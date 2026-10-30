//! Writing the intent log to disk while the game runs, so memory stays small and a crash
//! loses little.
//!
//! A session file is a `persist` file built by appending: the header first, then runs as
//! entries settle. Reading the whole history means reading the file and then the tail still
//! in memory. What keeps that true:
//!
//! - the file is always the header plus whole runs, up to `good_len`;
//! - an entry leaves memory only after the run holding it was written;
//! - each track's newest entry stays in memory, so a run is never cut off before it ends and
//!   the rising-ticks rule keeps checking the next record;
//! - a failed write is cut back to `good_len` before anything else is written, or, if it
//!   cannot be cut back, nothing more is written to that file.
//!
//! The file is created on the first write, so a session that records nothing leaves nothing.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::shared::clock::Tick;
use crate::shared::intent::Intent;
use crate::simulation::intent_log::{ActorId, IntentLog, RecordError};
use crate::simulation::persist::{HEADER, push_runs};

/// Entries held in memory before they are written out. At this size an idle hour costs a
/// handful of small writes, and a crash loses a minute or two of play at most.
const FLUSH_AT: usize = 2048;

const EXTENSION: &str = "mcit";

#[derive(Debug, Error)]
pub enum StreamError {
    /// The intent was refused and nothing changed.
    #[error(transparent)]
    Record(#[from] RecordError),
    /// The intent was kept, but the write that followed failed. Nothing is lost: the entries
    /// stay in memory and the write is tried again after another [`FLUSH_AT`] of them.
    #[error("cannot write the intent track: {0}")]
    Write(#[from] io::Error),
}

/// The session file, once it exists.
struct Sink {
    file: File,
    /// File length after the last whole write.
    good_len: u64,
    /// A failed write could not be cut back, so the file may hold a torn run.
    broken: bool,
}

impl Sink {
    fn create(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // `create_new` so a second game in the same folder never appends to this session.
        let mut file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)?;
        if let Err(error) = file.write_all(&HEADER) {
            // A torn header would leave a file that every later attempt finds in its way.
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(Self {
            file,
            good_len: HEADER.len() as u64,
            broken: false,
        })
    }

    fn append(&mut self, runs: &[u8]) -> io::Result<()> {
        if self.broken {
            return Err(io::Error::other(
                "an earlier failed write could not be undone",
            ));
        }
        if let Err(error) = self.file.write_all(runs) {
            // Cut back to the last whole run, so a retry cannot bury a torn run mid-file.
            self.broken = self.file.set_len(self.good_len).is_err();
            return Err(error);
        }
        self.good_len += runs.len() as u64;
        Ok(())
    }
}

/// An [`IntentLog`] that moves settled entries to one session file as it grows.
pub struct IntentStream {
    path: PathBuf,
    sink: Option<Sink>,
    /// Entries not yet in the file.
    tail: IntentLog,
    /// Entries held before a flush is tried.
    limit: usize,
    /// The tail length that triggers the next flush; pushed back after a failed one so a
    /// broken disk costs one warning per `limit` entries, not one per tick.
    flush_at: usize,
}

impl IntentStream {
    /// A stream that will write to `path`. Nothing touches the disk until entries settle.
    pub fn new(path: PathBuf) -> Self {
        Self::with_limit(path, FLUSH_AT)
    }

    fn with_limit(path: PathBuf, limit: usize) -> Self {
        Self {
            path,
            sink: None,
            tail: IntentLog::default(),
            limit,
            flush_at: limit,
        }
    }

    /// Adds `intent` for `actor` on `tick`, and writes the settled entries out once enough
    /// have gathered.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Record`] if `tick` is not after the actor's latest one, or
    /// [`StreamError::Write`] if the intent was kept but the file could not be written.
    pub fn record(
        &mut self,
        actor: ActorId,
        tick: Tick,
        intent: Intent,
    ) -> Result<(), StreamError> {
        self.tail.record(actor, tick, intent)?;
        if self.tail.len() >= self.flush_at {
            self.flush().inspect_err(|_| {
                self.flush_at = self.tail.len().saturating_add(self.limit);
            })?;
        }
        Ok(())
    }

    /// Writes everything but each track's newest entry and drops it from memory.
    fn flush(&mut self) -> io::Result<()> {
        let mut runs = Vec::new();
        for (actor, track) in self.tail.tracks() {
            let settled = track.split_last().map_or(&[][..], |(_, rest)| rest);
            push_runs(&mut runs, actor, settled);
        }
        self.append(&runs)?;
        self.tail.retain_newest();
        self.flush_at = self.limit;
        Ok(())
    }

    fn append(&mut self, runs: &[u8]) -> io::Result<()> {
        if runs.is_empty() {
            return Ok(());
        }
        let mut sink = match self.sink.take() {
            Some(sink) => sink,
            None => Sink::create(&self.path)?,
        };
        let result = sink.append(runs);
        self.sink = Some(sink);
        result
    }

    /// Writes what is still in memory and syncs the file to disk. Call it when the session
    /// ends.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if the file cannot be written or synced.
    pub fn finish(mut self) -> io::Result<()> {
        let mut runs = Vec::new();
        for (actor, track) in self.tail.tracks() {
            push_runs(&mut runs, actor, track);
        }
        self.append(&runs)?;
        self.sink.map_or(Ok(()), |sink| sink.file.sync_all())
    }
}

/// The path for a new session in `dir`: one more than the highest numbered session file
/// there, so earlier sessions are kept and never overwritten. A missing `dir` is the
/// first session.
///
/// # Errors
///
/// Returns the I/O error if `dir` exists but cannot be listed, or if every number is taken.
pub fn next_session_path(dir: &Path) -> io::Result<PathBuf> {
    let mut newest = 0u32;
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let path = entry?.path();
                let number = path
                    .extension()
                    .filter(|extension| *extension == EXTENSION)
                    .and_then(|_| path.file_stem())
                    .and_then(OsStr::to_str)
                    .and_then(|stem| stem.parse::<u32>().ok());
                newest = newest.max(number.unwrap_or(0));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let next = newest
        .checked_add(1)
        .ok_or_else(|| io::Error::other("every session number is taken"))?;
    Ok(session_path(dir, next))
}

/// The path of session `number` in `dir`, the name [`next_session_path`] gives it.
pub fn session_path(dir: &Path, number: u32) -> PathBuf {
    dir.join(format!("{number:04}.{EXTENSION}"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use proptest::prelude::*;

    use super::*;
    use crate::shared::intent::Walk;
    use crate::simulation::persist::{decode_recovering, load};

    /// A scratch folder that is removed when the test ends, even if it fails.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "mcit-stream-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            Self(dir)
        }

        fn session(&self) -> PathBuf {
            self.0.join("0001.mcit")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn intent(x: i8, jump: bool) -> Intent {
        Intent {
            walk: Walk::new(x, 0),
            jump,
        }
    }

    /// `(actor, gap, x, jump)` steps; small ranges make repeated intents, so runs form.
    fn steps() -> impl Strategy<Value = Vec<(u8, u64, i8, bool)>> {
        prop::collection::vec((0u8..4, 1u64..4, -1i8..2, any::<bool>()), 0..150)
    }

    /// Feeds `steps` to a stream and to a plain log, which then agree on the history.
    fn play(stream: &mut IntentStream, steps: &[(u8, u64, i8, bool)]) -> IntentLog {
        let mut reference = IntentLog::default();
        let mut ticks = [0u64; 4];
        for &(actor, gap, x, jump) in steps {
            let slot = usize::from(actor % 4);
            ticks[slot] += gap;
            let (actor, tick) = (ActorId(u32::from(actor % 4)), Tick(ticks[slot]));
            stream.record(actor, tick, intent(x, jump)).unwrap();
            reference.record(actor, tick, intent(x, jump)).unwrap();
        }
        reference
    }

    proptest! {
        #[test]
        fn a_finished_session_reads_back_exactly_what_was_recorded(
            steps in steps(),
            limit in 1usize..12,
        ) {
            let dir = TempDir::new();
            let mut stream = IntentStream::with_limit(dir.session(), limit);
            let reference = play(&mut stream, &steps);
            prop_assert!(stream.tail.len() <= limit.max(4));
            stream.finish().unwrap();

            let expected = (!reference.is_empty()).then_some(reference);
            prop_assert_eq!(load(&dir.session()).unwrap(), expected);
        }

        #[test]
        fn the_file_and_the_tail_together_are_the_whole_history(
            steps in steps(),
            limit in 1usize..12,
        ) {
            let dir = TempDir::new();
            let mut stream = IntentStream::with_limit(dir.session(), limit);
            let reference = play(&mut stream, &steps);

            // Stopping here is a crash: the game never calls `finish`.
            let saved = match fs::read(dir.session()) {
                Ok(bytes) => decode_recovering(&bytes).unwrap().0,
                Err(_) => IntentLog::default(),
            };
            prop_assert_eq!(saved.len() + stream.tail.len(), reference.len());
            for (actor, track) in saved.tracks() {
                prop_assert_eq!(track, &reference.track(actor)[..track.len()]);
            }
        }
    }

    #[test]
    fn a_session_that_records_nothing_leaves_no_file() {
        let dir = TempDir::new();
        IntentStream::new(dir.session()).finish().unwrap();
        assert!(!dir.0.exists());
    }

    #[test]
    fn a_long_idle_session_stays_small_in_memory_and_on_disk() {
        let dir = TempDir::new();
        let mut stream = IntentStream::new(dir.session());
        for tick in 0..100_000 {
            stream
                .record(ActorId::PLAYER, Tick(tick), Intent::default())
                .unwrap();
            assert!(stream.tail.len() <= FLUSH_AT);
        }
        stream.finish().unwrap();
        let size = fs::metadata(dir.session()).unwrap().len();
        // One run per flush, not one per tick.
        assert!(size < 1_000, "{size} bytes");
        assert_eq!(load(&dir.session()).unwrap().unwrap().len(), 100_000);
    }

    #[test]
    fn a_refused_tick_changes_nothing() {
        let dir = TempDir::new();
        let mut stream = IntentStream::with_limit(dir.session(), 100);
        stream
            .record(ActorId::PLAYER, Tick(5), Intent::default())
            .unwrap();
        let refused = stream.record(ActorId::PLAYER, Tick(5), Intent::default());
        assert!(matches!(refused, Err(StreamError::Record(_))));
        assert_eq!(stream.tail.len(), 1);
    }

    #[test]
    fn a_failed_write_keeps_every_entry_and_is_not_retried_every_tick() {
        let dir = TempDir::new();
        fs::create_dir_all(&dir.0).unwrap();
        // A file where the session folder should be, so the session file cannot be created.
        let blocker = dir.0.join("blocker");
        fs::write(&blocker, b"").unwrap();
        let mut stream = IntentStream::with_limit(blocker.join("0001.mcit"), 3);

        let mut failures = 0;
        for tick in 0..8 {
            if let Err(StreamError::Write(_)) =
                stream.record(ActorId::PLAYER, Tick(tick), Intent::default())
            {
                failures += 1;
            }
        }
        assert_eq!(stream.tail.len(), 8);
        // Ticks 2 and 5 tried; the ones between did not.
        assert_eq!(failures, 2);
        assert!(stream.finish().is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_write_that_cannot_be_undone_stops_further_writes() {
        // `/dev/full` refuses every write and cannot be truncated.
        let file = OpenOptions::new().write(true).open("/dev/full").unwrap();
        let mut sink = Sink {
            file,
            good_len: 0,
            broken: false,
        };
        assert!(sink.append(b"run").is_err());
        assert!(sink.broken);
        let again = sink.append(b"run").unwrap_err();
        assert!(again.to_string().contains("could not be undone"));
    }

    #[test]
    fn session_numbers_continue_after_the_highest_and_skip_other_files() {
        let dir = TempDir::new();
        assert_eq!(
            next_session_path(&dir.0).unwrap(),
            dir.0.join("0001.mcit"),
            "a missing folder is the first session"
        );
        fs::create_dir_all(&dir.0).unwrap();
        for name in ["0001.mcit", "0007.mcit", "notes.txt", "x.mcit", "0009.txt"] {
            fs::write(dir.0.join(name), b"").unwrap();
        }
        assert_eq!(next_session_path(&dir.0).unwrap(), dir.0.join("0008.mcit"));
    }

    #[test]
    fn the_last_session_number_is_refused_instead_of_wrapping() {
        let dir = TempDir::new();
        fs::create_dir_all(&dir.0).unwrap();
        fs::write(dir.0.join(format!("{}.mcit", u32::MAX)), b"").unwrap();
        assert!(next_session_path(&dir.0).is_err());
    }
}
