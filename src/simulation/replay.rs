//! Playing a saved session back: the intents it recorded for the player and each mob, looked
//! up by tick.
//!
//! A replay only supplies intents. The caller feeds them to fresh bodies in place of the
//! keyboard and the mob's own choices, which reproduces the run as long as the world is the
//! same as when it was recorded; edits, crafting and eating are not in the tracks.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::shared::clock::Tick;
use crate::shared::intent::Intent;
use crate::simulation::intent_log::{ActorId, IntentLog};
use crate::simulation::persist::{DecodeError, decode_recovering};

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("there is no session file at {0}")]
    Missing(PathBuf),
    #[error("cannot read the session file")]
    Io(#[from] io::Error),
    #[error("the session file is damaged")]
    Decode(#[from] DecodeError),
    #[error("the session has no player steps from the first tick")]
    NoPlayerTrack,
}

/// A loaded session whose player track starts on tick 0, so playing it from a fresh start
/// lines up with the clock.
#[derive(Clone, Debug)]
pub struct Replay {
    log: IntentLog,
}

impl Replay {
    /// # Errors
    ///
    /// Returns [`ReplayError::NoPlayerTrack`] if the player has no intent on tick 0.
    pub fn try_new(log: IntentLog) -> Result<Self, ReplayError> {
        match log.track(ActorId::PLAYER).first() {
            Some(first) if first.tick == Tick(0) => Ok(Self { log }),
            _ => Err(ReplayError::NoPlayerTrack),
        }
    }

    /// Reads the session at `path`. A file cut short by a crash gives the steps before the
    /// cut.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError`] if there is no file, it cannot be read, it is damaged beyond a
    /// cut-off end, or it holds no player steps from tick 0.
    pub fn load(path: &Path) -> Result<Self, ReplayError> {
        let bytes = fs::read(path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => ReplayError::Missing(path.to_owned()),
            _ => ReplayError::Io(error),
        })?;
        let (log, discarded) = decode_recovering(&bytes)?;
        if discarded > 0 {
            tracing::warn!(
                discarded,
                "the session file was cut short; replaying what is left"
            );
        }
        Self::try_new(log)
    }

    /// What `actor` asked for on `tick`, or `None` if the session has nothing for it: the
    /// actor was not there, or the session ended.
    pub fn intent(&self, actor: ActorId, tick: Tick) -> Option<Intent> {
        self.log.intent_at(actor, tick)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArgsError {
    #[error("--replay needs a session number, for example --replay 3")]
    MissingNumber,
    #[error("`{0}` is not a session number")]
    BadNumber(String),
    #[error("unknown argument `{0}`")]
    Unknown(String),
}

/// The session number given by `--replay <number>` in `args` (without the program name), if
/// any. When it is given more than once, the last one counts.
///
/// # Errors
///
/// Returns [`ArgsError`] for a missing or non-numeric number, or any other argument.
pub fn replay_session(args: impl IntoIterator<Item = String>) -> Result<Option<u32>, ArgsError> {
    let mut args = args.into_iter();
    let mut session = None;
    while let Some(arg) = args.next() {
        if arg != "--replay" {
            return Err(ArgsError::Unknown(arg));
        }
        let number = args.next().ok_or(ArgsError::MissingNumber)?;
        session = Some(number.parse().map_err(|_| ArgsError::BadNumber(number))?);
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::shared::intent::Walk;
    use crate::simulation::persist::encode;

    fn intent(x: i8) -> Intent {
        Intent {
            walk: Walk::new(x, 0),
            jump: false,
        }
    }

    fn log_from(first_tick: u64, count: u64) -> IntentLog {
        let mut log = IntentLog::default();
        for tick in first_tick..first_tick + count {
            assert_eq!(
                log.record(ActorId::PLAYER, Tick(tick), intent(tick as i8)),
                Ok(())
            );
        }
        log
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    /// A scratch file path that is removed when the test ends, even if it fails.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            Self(std::env::temp_dir().join(format!(
                "mcit-replay-{}-{}.mcit",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn a_replay_gives_each_recorded_tick_and_nothing_after() {
        let replay = Replay::try_new(log_from(0, 3));
        let Ok(replay) = replay else {
            panic!("a track from tick 0 must be accepted");
        };
        assert_eq!(replay.intent(ActorId::PLAYER, Tick(0)), Some(intent(0)));
        assert_eq!(replay.intent(ActorId::PLAYER, Tick(2)), Some(intent(2)));
        assert_eq!(replay.intent(ActorId::PLAYER, Tick(3)), None);
        assert_eq!(replay.intent(ActorId::PLAYER.next(), Tick(0)), None);
    }

    #[test]
    fn a_session_without_the_first_tick_is_refused() {
        assert!(matches!(
            Replay::try_new(IntentLog::default()),
            Err(ReplayError::NoPlayerTrack)
        ));
        assert!(matches!(
            Replay::try_new(log_from(1, 3)),
            Err(ReplayError::NoPlayerTrack)
        ));
    }

    #[test]
    fn a_missing_file_is_reported_with_its_path() {
        let file = TempFile::new();
        assert!(matches!(
            Replay::load(&file.0),
            Err(ReplayError::Missing(path)) if path == file.0
        ));
    }

    #[test]
    fn a_file_cut_short_replays_the_steps_before_the_cut() {
        let file = TempFile::new();
        let mut bytes = encode(&log_from(0, 4));
        bytes.extend_from_slice(&[7, 7, 7]);
        assert!(fs::write(&file.0, bytes).is_ok());
        let Ok(replay) = Replay::load(&file.0) else {
            panic!("a torn tail must still load");
        };
        assert_eq!(replay.intent(ActorId::PLAYER, Tick(3)), Some(intent(3)));
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let file = TempFile::new();
        assert!(fs::write(&file.0, b"not a session").is_ok());
        assert!(matches!(
            Replay::load(&file.0),
            Err(ReplayError::Decode(DecodeError::BadMagic))
        ));
    }

    #[test]
    fn the_replay_flag_reads_a_session_number() {
        assert_eq!(replay_session(args(&[])), Ok(None));
        assert_eq!(replay_session(args(&["--replay", "3"])), Ok(Some(3)));
        assert_eq!(
            replay_session(args(&["--replay", "3", "--replay", "5"])),
            Ok(Some(5))
        );
    }

    #[test]
    fn a_bad_command_line_is_refused() {
        assert_eq!(
            replay_session(args(&["--replay"])),
            Err(ArgsError::MissingNumber)
        );
        assert_eq!(
            replay_session(args(&["--replay", "x"])),
            Err(ArgsError::BadNumber("x".to_owned()))
        );
        assert_eq!(
            replay_session(args(&["3"])),
            Err(ArgsError::Unknown("3".to_owned()))
        );
    }
}
