//! Saving the event log to disk and reading it back.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCEL` |
//! | version | 2 | [`VERSION`] |
//! | seed | 8 | seed of the terrain the events were made on |
//! | records | by tag | a tag byte, then the fields below |
//!
//! | Tag | Event | Fields |
//! |---|---|---|
//! | 1 | block set | x, y, z (4 each), old (2), new (2) |
//! | 2 | player moved | old x, y, z bits (4 each), new x, y, z bits (4 each) |
//! | 3 | slot set | slot (1), old item (2), old count (1), new item (2), new count (1) |
//! | 4 | edit ends | none: the events since the last end form one edit |
//! | 5 | rewind ends | none: the events since the last end undo the latest live edit |
//!
//! The record count is not stored: the records run to the end of the file, so a file cut
//! short mid-record is rejected instead of silently losing its last event. Version 1 held
//! only tag 1 and reads unchanged under version 2. Tags 4 and 5 came with version 3; older
//! files have no action ends, so their history cannot be rewound.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::events::log::{ActionKind, EventLog, Record};
use crate::shared::event::{Event, Held, Spot};
use crate::shared::voxel::{BlockId, WorldPos};

const MAGIC: [u8; 4] = *b"MCEL";

/// The version written. Bump it when the layout changes.
pub const VERSION: u16 = 3;

/// The oldest version still read. A layout that cannot be read unchanged is refused, not
/// guessed at.
pub const MIN_VERSION: u16 = 1;

const HEADER_LEN: usize = 4 + 2 + 8;

/// Record tags, one per [`Event`] variant.
const TAG_BLOCK_SET: u8 = 1;
const TAG_PLAYER_MOVED: u8 = 2;
const TAG_SLOT_SET: u8 = 3;
const TAG_EDIT_END: u8 = 4;
const TAG_REWIND_END: u8 = 5;

/// Bytes `event` takes in a file, tag included.
const fn record_len(event: &Event) -> usize {
    match event {
        Event::BlockSet { .. } => 1 + 3 * 4 + 2 + 2,
        Event::PlayerMoved { .. } => 1 + 6 * 4,
        Event::SlotSet { .. } => 1 + 1 + 3 + 3,
    }
}

/// A log read back from disk, with the seed its terrain must be regenerated from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedLog {
    pub seed: u64,
    pub log: EventLog,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("not an event log file")]
    BadMagic,
    #[error("event log version {0} is not supported (expected {MIN_VERSION} to {VERSION})")]
    UnsupportedVersion(u16),
    #[error("the file ends in the middle of the header or a record")]
    Truncated,
    #[error("unknown event tag {0}")]
    UnknownEvent(u8),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("cannot read the file")]
    Io(#[from] io::Error),
    #[error("the file is damaged")]
    Decode(#[from] DecodeError),
}

/// Serialises `log` in the layout described in the module docs.
pub fn encode(seed: u64, log: &EventLog) -> Vec<u8> {
    let len = log.iter().map(record_len).sum::<usize>();
    let mut bytes = Vec::with_capacity(HEADER_LEN + len);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&seed.to_le_bytes());
    for record in log.records() {
        let event = match record {
            Record::Event(event) => event,
            Record::ActionEnd(ActionKind::Edit) => {
                bytes.push(TAG_EDIT_END);
                continue;
            }
            Record::ActionEnd(ActionKind::Rewind) => {
                bytes.push(TAG_REWIND_END);
                continue;
            }
        };
        match event {
            Event::BlockSet { pos, old, new } => {
                bytes.push(TAG_BLOCK_SET);
                bytes.extend_from_slice(&pos.x.to_le_bytes());
                bytes.extend_from_slice(&pos.y.to_le_bytes());
                bytes.extend_from_slice(&pos.z.to_le_bytes());
                bytes.extend_from_slice(&old.0.to_le_bytes());
                bytes.extend_from_slice(&new.0.to_le_bytes());
            }
            Event::PlayerMoved { old, new } => {
                bytes.push(TAG_PLAYER_MOVED);
                for bits in old.to_bits().into_iter().chain(new.to_bits()) {
                    bytes.extend_from_slice(&bits.to_le_bytes());
                }
            }
            Event::SlotSet { slot, old, new } => {
                bytes.push(TAG_SLOT_SET);
                bytes.push(slot);
                for held in [old, new] {
                    bytes.extend_from_slice(&held.item.to_le_bytes());
                    bytes.push(held.count);
                }
            }
        }
    }
    bytes
}

/// # Errors
///
/// Returns a [`DecodeError`] if the bytes are not a complete, supported event log.
pub fn decode(bytes: &[u8]) -> Result<SavedLog, DecodeError> {
    let mut reader = Reader(bytes);
    if reader.take::<4>()? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = u16::from_le_bytes(reader.take()?);
    if !(MIN_VERSION..=VERSION).contains(&version) {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let seed = u64::from_le_bytes(reader.take()?);

    let mut log = EventLog::new();
    while !reader.0.is_empty() {
        let [tag] = reader.take()?;
        match tag {
            TAG_BLOCK_SET => log.push(Event::BlockSet {
                pos: WorldPos {
                    x: i32::from_le_bytes(reader.take()?),
                    y: i32::from_le_bytes(reader.take()?),
                    z: i32::from_le_bytes(reader.take()?),
                },
                old: BlockId(u16::from_le_bytes(reader.take()?)),
                new: BlockId(u16::from_le_bytes(reader.take()?)),
            }),
            TAG_PLAYER_MOVED => {
                let mut spot = || -> Result<Spot, DecodeError> {
                    Ok(Spot::from_bits([
                        u32::from_le_bytes(reader.take()?),
                        u32::from_le_bytes(reader.take()?),
                        u32::from_le_bytes(reader.take()?),
                    ]))
                };
                let old = spot()?;
                let new = spot()?;
                log.push(Event::PlayerMoved { old, new });
            }
            TAG_SLOT_SET => {
                let [slot] = reader.take()?;
                let mut held = || -> Result<Held, DecodeError> {
                    let item = u16::from_le_bytes(reader.take()?);
                    let [count] = reader.take()?;
                    Ok(Held { item, count })
                };
                let old = held()?;
                let new = held()?;
                log.push(Event::SlotSet { slot, old, new });
            }
            TAG_EDIT_END if version >= 3 => log.end_action(ActionKind::Edit),
            TAG_REWIND_END if version >= 3 => log.end_action(ActionKind::Rewind),
            other => return Err(DecodeError::UnknownEvent(other)),
        }
    }
    Ok(SavedLog { seed, log })
}

/// Writes the log to `path` without ever leaving a half-written file there: the bytes go
/// to a sibling file first, which then replaces `path` in one step.
///
/// # Errors
///
/// Returns the I/O error if the sibling file cannot be written or moved into place.
pub fn save(path: &Path, seed: u64, log: &EventLog) -> io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, encode(seed, log))?;
    std::fs::rename(&staging, path)
}

/// Reads a saved log, or `None` if there is no file at `path` yet.
///
/// # Errors
///
/// Returns [`LoadError`] if the file exists but cannot be read or is not a valid log.
pub fn load(path: &Path) -> Result<Option<SavedLog>, LoadError> {
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

    fn event() -> impl Strategy<Value = Event> {
        prop_oneof![
            (
                any::<i32>(),
                any::<i32>(),
                any::<i32>(),
                any::<u16>(),
                any::<u16>(),
            )
                .prop_map(|(x, y, z, old, new)| Event::BlockSet {
                    pos: WorldPos { x, y, z },
                    old: BlockId(old),
                    new: BlockId(new),
                }),
            (any::<[u32; 3]>(), any::<[u32; 3]>()).prop_map(|(old, new)| Event::PlayerMoved {
                old: Spot::from_bits(old),
                new: Spot::from_bits(new),
            }),
            (
                any::<u8>(),
                (any::<u16>(), any::<u8>()),
                (any::<u16>(), any::<u8>())
            )
                .prop_map(|(slot, old, new)| Event::SlotSet {
                    slot,
                    old: Held {
                        item: old.0,
                        count: old.1
                    },
                    new: Held {
                        item: new.0,
                        count: new.1
                    },
                }),
        ]
    }

    /// A block record, the only kind version 1 files hold.
    const BLOCK_RECORD_LEN: usize = 17;

    fn log_of(events: &[Event]) -> EventLog {
        let mut log = EventLog::new();
        for event in events {
            log.push(*event);
        }
        log
    }

    proptest! {
        #[test]
        fn decoding_an_encoded_log_gives_it_back(
            seed in any::<u64>(),
            events in prop::collection::vec(event(), 0..60),
        ) {
            let log = log_of(&events);
            let bytes = encode(seed, &log);
            prop_assert_eq!(
                bytes.len(),
                HEADER_LEN + events.iter().map(record_len).sum::<usize>()
            );
            prop_assert_eq!(decode(&bytes), Ok(SavedLog { seed, log }));
        }

        #[test]
        fn a_file_cut_inside_a_record_is_rejected(
            events in prop::collection::vec(event(), 1..20),
            cut in 1usize..32,
        ) {
            // Cutting more than the last record would drop whole records and still decode.
            let last = record_len(&events[events.len() - 1]);
            let cut = 1 + (cut - 1) % (last - 1);
            let bytes = encode(0, &log_of(&events));
            prop_assert_eq!(decode(&bytes[..bytes.len() - cut]), Err(DecodeError::Truncated));
        }

        #[test]
        fn decoding_arbitrary_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..200)) {
            let _ = decode(&bytes);
        }
    }

    #[test]
    fn action_ends_survive_a_save_and_decide_what_rewind_undoes() {
        let events = [
            Event::SlotSet {
                slot: 0,
                old: Held::EMPTY,
                new: Held { item: 1, count: 1 },
            },
            Event::SlotSet {
                slot: 0,
                old: Held { item: 1, count: 1 },
                new: Held::EMPTY,
            },
        ];
        let mut log = EventLog::new();
        log.push(events[0]);
        log.end_action(ActionKind::Edit);
        log.push(events[1]);
        log.end_action(ActionKind::Rewind);

        let SavedLog { log: back, .. } = decode(&encode(1, &log)).unwrap();

        assert_eq!(back, log);
        assert_eq!(back.latest_live_edit(), None);
    }

    #[test]
    fn a_version_two_file_with_an_action_end_is_rejected() {
        let mut log = log_of(&[]);
        log.push(Event::PlayerMoved {
            old: Spot::from_bits([0; 3]),
            new: Spot::from_bits([1; 3]),
        });
        log.end_action(ActionKind::Edit);
        let mut bytes = encode(1, &log);
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());

        assert_eq!(decode(&bytes), Err(DecodeError::UnknownEvent(TAG_EDIT_END)));
    }

    #[test]
    fn a_file_cut_inside_the_header_is_rejected() {
        let bytes = encode(7, &EventLog::new());
        for len in 0..HEADER_LEN {
            assert!(decode(&bytes[..len]).is_err(), "len {len}");
        }
    }

    #[test]
    fn foreign_magic_is_rejected() {
        let mut bytes = encode(7, &EventLog::new());
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(DecodeError::BadMagic));
    }

    #[test]
    fn another_version_is_rejected() {
        let mut bytes = encode(7, &EventLog::new());
        bytes[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::UnsupportedVersion(VERSION + 1))
        );
    }

    #[test]
    fn a_version_one_file_still_reads() {
        let block = Event::BlockSet {
            pos: WorldPos { x: -3, y: 4, z: 5 },
            old: BlockId::AIR,
            new: BlockId::STONE,
        };
        let mut bytes = encode(7, &log_of(&[block]));
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());

        assert_eq!(
            decode(&bytes),
            Ok(SavedLog {
                seed: 7,
                log: log_of(&[block])
            })
        );
    }

    #[test]
    fn version_zero_is_rejected() {
        let mut bytes = encode(7, &EventLog::new());
        bytes[4..6].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(decode(&bytes), Err(DecodeError::UnsupportedVersion(0)));
    }

    #[test]
    fn an_unknown_tag_is_rejected() {
        let mut bytes = encode(7, &EventLog::new());
        bytes.extend_from_slice(&[9; BLOCK_RECORD_LEN]);
        assert_eq!(decode(&bytes), Err(DecodeError::UnknownEvent(9)));
    }

    #[test]
    fn saving_then_loading_round_trips_and_a_missing_file_is_none() {
        let dir = std::env::temp_dir().join(format!("mcel-persist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("world.mcel");
        assert_eq!(load(&path).unwrap(), None);

        let log = log_of(&[Event::BlockSet {
            pos: WorldPos { x: -3, y: 4, z: 5 },
            old: BlockId::AIR,
            new: BlockId::STONE,
        }]);
        save(&path, 117, &log).unwrap();

        assert_eq!(load(&path).unwrap(), Some(SavedLog { seed: 117, log }));
        assert!(!dir.join("world.mcel.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
