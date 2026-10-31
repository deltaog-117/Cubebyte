//! Saving the intent log to disk and reading it back.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCIT` |
//! | version | 2 | [`VERSION`] |
//! | records | by tag | a tag byte, then the fields below |
//!
//! | Tag | Record | Fields |
//! |---|---|---|
//! | 1 | run | actor (4), first tick (8), length (4), walk x (1), walk z (1), jump (1) |
//! | 2 | action | tick (8), kind (1), then by kind |
//!
//! | Kind | Action | Fields |
//! |---|---|---|
//! | 1 | break | x, y, z (4 each) |
//! | 2 | place | x, y, z (4 each), block (2) |
//! | 3 | craft | recipe (1) |
//! | 4 | eat | none |
//! | 5 | strike | actor (4) |
//! | 6 | rewind | none |
//! | 7 | redo | none |
//!
//! A run is one intent held on consecutive ticks, so standing still for an hour costs one
//! run instead of one entry per tick. The writer joins every such stretch into a single run
//! and lists the runs by actor, then by tick, and the actions after them; the reader accepts
//! any run order within an actor as long as ticks keep rising, and actions as long as their
//! ticks never fall. The record count is not stored: records go to the end of the file, so a
//! file cut short mid-record is rejected by [`decode`]; [`decode_recovering`] keeps the whole
//! records before the cut instead, for a file left behind by a crash.
//!
//! Version 1 had no tags and no actions, only runs; it is still read, unchanged. Version 2 had
//! kinds 1 to 4 only; it is still read, unchanged.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::shared::clock::Tick;
use crate::shared::intent::{AXIS_MAX, Intent, Walk};
use crate::shared::voxel::{BlockId, WorldPos};
use crate::simulation::action::Action;
use crate::simulation::intent_log::{ActionEntry, ActorId, Entry, IntentLog, RecordError};

const MAGIC: [u8; 4] = *b"MCIT";

/// Bump when the layout changes.
pub const VERSION: u16 = 3;

/// The oldest version still read. A layout that cannot be read unchanged is refused, not
/// guessed at.
pub const MIN_VERSION: u16 = 1;

const HEADER_LEN: usize = 4 + 2;

/// The first bytes of every file; `simulation::stream` writes them before any run.
pub(super) const HEADER: [u8; HEADER_LEN] = {
    let version = VERSION.to_le_bytes();
    [
        MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], version[0], version[1],
    ]
};
/// Bytes a run takes after its tag.
const RUN_LEN: usize = 4 + 8 + 4 + 1 + 1 + 1;

/// Record tags, from version 2 on.
const TAG_RUN: u8 = 1;
const TAG_ACTION: u8 = 2;

/// Action kinds.
const KIND_BREAK: u8 = 1;
const KIND_PLACE: u8 = 2;
const KIND_CRAFT: u8 = 3;
const KIND_EAT: u8 = 4;
const KIND_STRIKE: u8 = 5;
const KIND_REWIND: u8 = 6;
const KIND_REDO: u8 = 7;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("not an intent log file")]
    BadMagic,
    #[error(
        "intent log version {0} is not supported (this build reads {MIN_VERSION} to {VERSION})"
    )]
    UnsupportedVersion(u16),
    #[error("the file ends in the middle of the header or a record")]
    Truncated,
    #[error("record tag {0} is not a run or an action")]
    BadTag(u8),
    #[error("action kind {0} is not known")]
    BadActionKind(u8),
    #[error("a run has no length")]
    EmptyRun,
    #[error("a run goes past the last tick")]
    TickOverflow,
    #[error("a walk axis is outside -{AXIS_MAX} to {AXIS_MAX}")]
    WalkOutOfRange,
    #[error("the jump flag is {0}, not 0 or 1")]
    BadJump(u8),
    #[error("the runs of one actor, or the actions, are out of order")]
    OutOfOrder(#[from] RecordError),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("cannot read the file")]
    Io(#[from] io::Error),
    #[error("the file is damaged")]
    Decode(#[from] DecodeError),
}

/// Serialises `log` in the layout described in the module docs.
pub fn encode(log: &IntentLog) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN);
    bytes.extend_from_slice(&HEADER);
    for (actor, track) in log.tracks() {
        push_runs(&mut bytes, actor, track);
    }
    push_actions(&mut bytes, log.actions());
    bytes
}

/// Appends `track` as runs, joining every stretch of one intent on consecutive ticks.
pub(super) fn push_runs(bytes: &mut Vec<u8>, actor: ActorId, track: &[Entry]) {
    let mut entries = track.iter();
    let Some(first) = entries.next() else {
        return;
    };
    let (mut start, mut length, mut intent) = (first.tick.0, 1u32, first.intent);
    for entry in entries {
        // The track's ticks rise, so `start + length` cannot overflow before this check.
        let continues = entry.intent == intent
            && entry.tick.0 == start + u64::from(length)
            && length < u32::MAX;
        if continues {
            length += 1;
        } else {
            push_run(bytes, actor, start, length, intent);
            (start, length, intent) = (entry.tick.0, 1, entry.intent);
        }
    }
    push_run(bytes, actor, start, length, intent);
}

fn push_run(bytes: &mut Vec<u8>, actor: ActorId, start: u64, length: u32, intent: Intent) {
    bytes.reserve(1 + RUN_LEN);
    bytes.push(TAG_RUN);
    bytes.extend_from_slice(&actor.0.to_le_bytes());
    bytes.extend_from_slice(&start.to_le_bytes());
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&intent.walk.x().to_le_bytes());
    bytes.extend_from_slice(&intent.walk.z().to_le_bytes());
    bytes.push(u8::from(intent.jump));
}

/// Appends `actions` as records, one each, in the order given.
pub(super) fn push_actions(bytes: &mut Vec<u8>, actions: &[ActionEntry]) {
    for entry in actions {
        bytes.push(TAG_ACTION);
        bytes.extend_from_slice(&entry.tick.0.to_le_bytes());
        match entry.action {
            Action::Break(pos) => {
                bytes.push(KIND_BREAK);
                push_pos(bytes, pos);
            }
            Action::Place { pos, block } => {
                bytes.push(KIND_PLACE);
                push_pos(bytes, pos);
                bytes.extend_from_slice(&block.0.to_le_bytes());
            }
            Action::Craft(recipe) => {
                bytes.push(KIND_CRAFT);
                bytes.push(recipe);
            }
            Action::Eat => bytes.push(KIND_EAT),
            Action::Strike(actor) => {
                bytes.push(KIND_STRIKE);
                bytes.extend_from_slice(&actor.0.to_le_bytes());
            }
            Action::Rewind => bytes.push(KIND_REWIND),
            Action::Redo => bytes.push(KIND_REDO),
        }
    }
}

fn push_pos(bytes: &mut Vec<u8>, pos: WorldPos) {
    for axis in [pos.x, pos.y, pos.z] {
        bytes.extend_from_slice(&axis.to_le_bytes());
    }
}

/// # Errors
///
/// Returns a [`DecodeError`] if the bytes are not a complete, supported and well-ordered
/// intent log.
pub fn decode(bytes: &[u8]) -> Result<IntentLog, DecodeError> {
    decode_runs(bytes, false).map(|(log, _)| log)
}

/// Like [`decode`], but a file that ends in the middle of a record (a crash or power loss
/// while the game was appending to it) gives the records before the cut, plus how many bytes
/// were left over. Every other fault is still an error.
///
/// # Errors
///
/// Returns a [`DecodeError`] for a damaged header or a record that is complete but invalid.
pub fn decode_recovering(bytes: &[u8]) -> Result<(IntentLog, usize), DecodeError> {
    decode_runs(bytes, true)
}

fn decode_runs(bytes: &[u8], allow_torn_tail: bool) -> Result<(IntentLog, usize), DecodeError> {
    let mut reader = Reader(bytes);
    if reader.take::<4>()? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = u16::from_le_bytes(reader.take()?);
    if !(MIN_VERSION..=VERSION).contains(&version) {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    // Version 1 files are runs end to end, with no tag byte before each.
    let tagged = version >= 2;

    let mut log = IntentLog::default();
    let mut discarded = 0;
    while !reader.0.is_empty() {
        let left = reader.0.len();
        match read_record(&mut reader, tagged, &mut log) {
            Ok(()) => {}
            Err(DecodeError::Truncated) if allow_torn_tail => {
                discarded = left;
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok((log, discarded))
}

/// Reads one record from `reader` into `log`.
fn read_record(reader: &mut Reader, tagged: bool, log: &mut IntentLog) -> Result<(), DecodeError> {
    let tag = if tagged {
        reader.take::<1>()?[0]
    } else {
        TAG_RUN
    };
    match tag {
        TAG_RUN => read_run(reader, log),
        TAG_ACTION => read_action(reader, log),
        other => Err(DecodeError::BadTag(other)),
    }
}

fn read_run(reader: &mut Reader, log: &mut IntentLog) -> Result<(), DecodeError> {
    let actor = ActorId(u32::from_le_bytes(reader.take()?));
    let start = u64::from_le_bytes(reader.take()?);
    let length = u32::from_le_bytes(reader.take()?);
    let [x, z, jump] = reader.take()?;
    let (x, z) = (i8::from_le_bytes([x]), i8::from_le_bytes([z]));
    if length == 0 {
        return Err(DecodeError::EmptyRun);
    }
    if x < -AXIS_MAX || z < -AXIS_MAX {
        return Err(DecodeError::WalkOutOfRange);
    }
    let jump = match jump {
        0 => false,
        1 => true,
        other => return Err(DecodeError::BadJump(other)),
    };
    let end = start
        .checked_add(u64::from(length) - 1)
        .ok_or(DecodeError::TickOverflow)?;
    let intent = Intent {
        walk: Walk::new(x, z),
        jump,
    };
    for tick in start..=end {
        log.record(actor, Tick(tick), intent)?;
    }
    Ok(())
}

fn read_action(reader: &mut Reader, log: &mut IntentLog) -> Result<(), DecodeError> {
    let tick = Tick(u64::from_le_bytes(reader.take()?));
    let action = match reader.take::<1>()?[0] {
        KIND_BREAK => Action::Break(read_pos(reader)?),
        KIND_PLACE => Action::Place {
            pos: read_pos(reader)?,
            block: BlockId(u16::from_le_bytes(reader.take()?)),
        },
        KIND_CRAFT => Action::Craft(reader.take::<1>()?[0]),
        KIND_EAT => Action::Eat,
        KIND_STRIKE => Action::Strike(ActorId(u32::from_le_bytes(reader.take()?))),
        KIND_REWIND => Action::Rewind,
        KIND_REDO => Action::Redo,
        other => return Err(DecodeError::BadActionKind(other)),
    };
    log.record_action(tick, action)?;
    Ok(())
}

fn read_pos(reader: &mut Reader) -> Result<WorldPos, DecodeError> {
    Ok(WorldPos {
        x: i32::from_le_bytes(reader.take()?),
        y: i32::from_le_bytes(reader.take()?),
        z: i32::from_le_bytes(reader.take()?),
    })
}

/// Writes the log to `path` without ever leaving a half-written file there: the bytes go
/// to a sibling file first, which then replaces `path` in one step.
///
/// # Errors
///
/// Returns the I/O error if the sibling file cannot be written or moved into place.
pub fn save(path: &Path, log: &IntentLog) -> io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, encode(log))?;
    std::fs::rename(&staging, path)
}

/// Reads a saved log, or `None` if there is no file at `path` yet.
///
/// # Errors
///
/// Returns [`LoadError`] if the file exists but cannot be read or is not a valid log.
pub fn load(path: &Path) -> Result<Option<IntentLog>, LoadError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(decode(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Consumes a byte slice from the front.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(DecodeError::Truncated)?;
        self.0 = rest;
        Ok(*head)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const ZOMBIE: ActorId = ActorId(1);

    /// A run with its tag.
    const RECORD_LEN: usize = 1 + RUN_LEN;

    fn still() -> Intent {
        Intent::default()
    }

    fn walking(x: i8) -> Intent {
        Intent {
            walk: Walk::new(x, 0),
            jump: false,
        }
    }

    /// Records `(gap, x, jump)` steps for each actor in turn; small value ranges make
    /// repeated intents, so runs actually form.
    fn log_of(steps: &[(u8, u64, i8, bool)]) -> IntentLog {
        let mut log = IntentLog::default();
        let mut ticks = [0u64; 4];
        for &(actor, gap, x, jump) in steps {
            let slot = usize::from(actor % 4);
            ticks[slot] += gap;
            let intent = Intent {
                walk: Walk::new(x, 0),
                jump,
            };
            log.record(ActorId(u32::from(actor % 4)), Tick(ticks[slot]), intent)
                .unwrap();
        }
        log
    }

    fn steps() -> impl Strategy<Value = Vec<(u8, u64, i8, bool)>> {
        prop::collection::vec((0u8..4, 1u64..4, -1i8..2, any::<bool>()), 0..120)
    }

    proptest! {
        #[test]
        fn decoding_an_encoded_log_gives_it_back(steps in steps()) {
            let log = log_of(&steps);
            prop_assert_eq!(decode(&encode(&log)), Ok(log));
        }

        #[test]
        fn the_same_log_always_gives_the_same_bytes(steps in steps()) {
            let log = log_of(&steps);
            prop_assert_eq!(encode(&log), encode(&log.clone()));
        }

        #[test]
        fn a_file_cut_inside_a_run_is_rejected(steps in steps(), cut in 1usize..RECORD_LEN) {
            let bytes = encode(&log_of(&steps));
            prop_assume!(bytes.len() > HEADER_LEN);
            prop_assert_eq!(decode(&bytes[..bytes.len() - cut]), Err(DecodeError::Truncated));
        }

        #[test]
        fn decoding_arbitrary_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..200)) {
            let _ = decode(&bytes);
        }
    }

    proptest! {
        #[test]
        fn recovering_a_cut_file_keeps_the_whole_runs_before_the_cut(
            steps in steps(),
            cut in 1usize..RECORD_LEN,
        ) {
            let bytes = encode(&log_of(&steps));
            prop_assume!(bytes.len() > HEADER_LEN);
            let whole = &bytes[..bytes.len() - RECORD_LEN];
            let cut_file = &bytes[..bytes.len() - cut];
            prop_assert_eq!(
                decode_recovering(cut_file),
                Ok((decode(whole)?, RECORD_LEN - cut))
            );
        }

        #[test]
        fn recovering_an_intact_file_discards_nothing(steps in steps()) {
            let log = log_of(&steps);
            prop_assert_eq!(decode_recovering(&encode(&log)), Ok((log, 0)));
        }
    }

    #[test]
    fn recovering_still_rejects_a_cut_header_and_a_damaged_run() {
        let bytes = encode(&IntentLog::default());
        assert_eq!(
            decode_recovering(&bytes[..HEADER_LEN - 1]),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            decode_recovering(&one_run(1, 0, 0, 2)),
            Err(DecodeError::BadJump(2))
        );
    }

    #[test]
    fn a_long_stretch_of_one_intent_takes_one_run() {
        let mut log = IntentLog::default();
        for tick in 0..10_000 {
            log.record(ZOMBIE, Tick(tick), still()).unwrap();
        }
        assert_eq!(encode(&log).len(), HEADER_LEN + RECORD_LEN);
    }

    #[test]
    fn a_gap_or_a_change_starts_a_new_run() {
        let mut log = IntentLog::default();
        for (tick, intent) in [(0, still()), (1, still()), (2, walking(5)), (4, walking(5))] {
            log.record(ZOMBIE, Tick(tick), intent).unwrap();
        }
        let bytes = encode(&log);
        assert_eq!(bytes.len(), HEADER_LEN + 3 * RECORD_LEN);
        assert_eq!(decode(&bytes), Ok(log));
    }

    #[test]
    fn an_empty_log_round_trips() {
        let bytes = encode(&IntentLog::default());
        assert_eq!(bytes.len(), HEADER_LEN);
        assert_eq!(decode(&bytes), Ok(IntentLog::default()));
    }

    fn one_run(length: u32, x: i8, z: i8, jump: u8) -> Vec<u8> {
        let mut log = IntentLog::default();
        log.record(ZOMBIE, Tick(0), still()).unwrap();
        let mut bytes = encode(&log);
        bytes[HEADER_LEN + 13..HEADER_LEN + 17].copy_from_slice(&length.to_le_bytes());
        bytes[HEADER_LEN + 17..HEADER_LEN + 19]
            .copy_from_slice(&[x.to_le_bytes()[0], z.to_le_bytes()[0]]);
        bytes[HEADER_LEN + 19] = jump;
        bytes
    }

    #[test]
    fn a_run_with_no_length_is_rejected() {
        assert_eq!(decode(&one_run(0, 0, 0, 0)), Err(DecodeError::EmptyRun));
    }

    #[test]
    fn the_lowest_raw_axis_is_rejected_so_nothing_is_silently_changed() {
        assert_eq!(
            decode(&one_run(1, i8::MIN, 0, 0)),
            Err(DecodeError::WalkOutOfRange)
        );
        assert_eq!(
            decode(&one_run(1, 0, i8::MIN, 0)),
            Err(DecodeError::WalkOutOfRange)
        );
    }

    #[test]
    fn a_jump_flag_other_than_zero_or_one_is_rejected() {
        assert_eq!(decode(&one_run(1, 0, 0, 2)), Err(DecodeError::BadJump(2)));
    }

    #[test]
    fn a_run_past_the_last_tick_is_rejected() {
        let mut bytes = one_run(2, 0, 0, 0);
        bytes[HEADER_LEN + 5..HEADER_LEN + 13].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Err(DecodeError::TickOverflow));
    }

    #[test]
    fn runs_that_go_back_in_time_are_rejected() {
        let mut bytes = encode(&IntentLog::default());
        for start in [10u64, 5] {
            let mut log = IntentLog::default();
            log.record(ZOMBIE, Tick(start), still()).unwrap();
            bytes.extend_from_slice(&encode(&log)[HEADER_LEN..]);
        }
        assert!(matches!(decode(&bytes), Err(DecodeError::OutOfOrder(_))));
    }

    #[test]
    fn foreign_magic_and_other_versions_are_rejected() {
        let mut bytes = encode(&IntentLog::default());
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(DecodeError::BadMagic));

        let mut bytes = encode(&IntentLog::default());
        bytes[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::UnsupportedVersion(VERSION + 1))
        );
    }

    #[test]
    fn a_file_cut_inside_the_header_is_rejected() {
        let bytes = encode(&IntentLog::default());
        for len in 0..HEADER_LEN {
            assert!(decode(&bytes[..len]).is_err(), "len {len}");
        }
    }

    #[test]
    fn saving_then_loading_round_trips_and_a_missing_file_is_none() {
        let dir = std::env::temp_dir().join(format!("mcit-persist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("intents.mcit");
        assert_eq!(load(&path).unwrap(), None);

        let log = log_of(&[(1, 1, 3, false), (1, 1, 3, false), (0, 2, 0, true)]);
        save(&path, &log).unwrap();

        assert_eq!(load(&path).unwrap(), Some(log));
        assert!(!dir.join("intents.mcit.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn pos(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    fn action_strategy() -> impl Strategy<Value = Action> {
        prop_oneof![
            (any::<i32>(), any::<i32>(), any::<i32>())
                .prop_map(|(x, y, z)| Action::Break(pos(x, y, z))),
            (any::<i32>(), any::<i32>(), any::<i32>(), any::<u16>()).prop_map(
                |(x, y, z, block)| Action::Place {
                    pos: pos(x, y, z),
                    block: BlockId(block)
                }
            ),
            any::<u8>().prop_map(Action::Craft),
            Just(Action::Eat),
            any::<u32>().prop_map(|id| Action::Strike(ActorId(id))),
            Just(Action::Rewind),
            Just(Action::Redo),
        ]
    }

    /// `steps` plus actions on rising ticks; gaps of zero put several on one tick.
    fn log_with_actions(steps: &[(u8, u64, i8, bool)], actions: &[(u64, Action)]) -> IntentLog {
        let mut log = log_of(steps);
        let mut tick = 0;
        for &(gap, action) in actions {
            tick += gap;
            log.record_action(Tick(tick), action).unwrap();
        }
        log
    }

    proptest! {
        #[test]
        fn actions_survive_a_round_trip_with_the_runs(
            steps in steps(),
            actions in prop::collection::vec((0u64..3, action_strategy()), 0..40),
        ) {
            let log = log_with_actions(&steps, &actions);
            prop_assert_eq!(decode(&encode(&log)), Ok(log));
        }

        #[test]
        fn a_file_cut_inside_an_action_keeps_every_record_before_it(
            actions in prop::collection::vec((0u64..3, action_strategy()), 1..20),
            cut in 1usize..8,
        ) {
            let log = log_with_actions(&[], &actions);
            let bytes = encode(&log);
            let last = log.actions().last().copied().unwrap();
            let mut whole = IntentLog::default();
            for entry in &log.actions()[..log.actions().len() - 1] {
                whole.record_action(entry.tick, entry.action).unwrap();
            }
            let mut one = Vec::new();
            push_actions(&mut one, &[last]);
            prop_assume!(cut < one.len());
            let cut_file = &bytes[..bytes.len() - cut];
            prop_assert_eq!(decode(cut_file), Err(DecodeError::Truncated));
            prop_assert_eq!(decode_recovering(cut_file), Ok((whole, one.len() - cut)));
        }
    }

    #[test]
    fn an_unknown_tag_or_action_kind_is_rejected() {
        let mut bytes = encode(&IntentLog::default());
        bytes.push(9);
        assert_eq!(decode(&bytes), Err(DecodeError::BadTag(9)));

        let mut bytes = encode(&IntentLog::default());
        bytes.push(TAG_ACTION);
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.push(9);
        assert_eq!(decode(&bytes), Err(DecodeError::BadActionKind(9)));
    }

    #[test]
    fn actions_that_go_back_in_time_are_rejected() {
        let mut bytes = encode(&IntentLog::default());
        for tick in [10, 5] {
            push_actions(
                &mut bytes,
                &[ActionEntry {
                    tick: Tick(tick),
                    action: Action::Eat,
                }],
            );
        }
        assert!(matches!(decode(&bytes), Err(DecodeError::OutOfOrder(_))));
    }

    #[test]
    fn a_version_one_file_is_still_read_as_untagged_runs() {
        let mut log = IntentLog::default();
        for tick in 0..3 {
            log.record(ZOMBIE, Tick(tick), walking(2)).unwrap();
        }
        // The version 1 layout: the old header, then the run without its tag.
        let mut bytes = encode(&log);
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes.remove(HEADER_LEN);
        assert_eq!(bytes.len(), HEADER_LEN + RUN_LEN);
        assert_eq!(decode(&bytes), Ok(log));
    }

    #[test]
    fn a_version_two_file_is_still_read() {
        let log = log_with_actions(
            &[(2, 0, 1, false)],
            &[(0, Action::Eat), (1, Action::Craft(3))],
        );
        let mut bytes = encode(&log);
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(decode(&bytes), Ok(log));
    }

    #[test]
    fn strikes_rewinds_and_redos_keep_their_order_on_one_tick() {
        let log = log_with_actions(
            &[],
            &[
                (0, Action::Strike(ActorId(4))),
                (0, Action::Rewind),
                (0, Action::Redo),
            ],
        );
        assert_eq!(decode(&encode(&log)), Ok(log));
    }

    #[test]
    fn version_zero_is_rejected() {
        let mut bytes = encode(&IntentLog::default());
        bytes[4..6].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(decode(&bytes), Err(DecodeError::UnsupportedVersion(0)));
    }
}
