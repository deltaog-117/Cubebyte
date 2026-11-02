//! The archive: history the log folded away, kept on disk for a timelapse or a timeline fork.
//!
//! The log keeps only its newest actions and folds the rest into a base of final blocks, which
//! is all a chunk needs. The folded events are not thrown away; they are appended here, in the
//! order they happened, with their action ends.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCEA` |
//! | version | 2 | [`VERSION`] |
//! | frames | by length | a frame is its length (4), then that many bytes |
//!
//! Each frame is a complete event log file (see [`crate::events::persist`]) holding the records
//! folded since the previous frame, so the record layout is defined once and a frame reads with
//! the log's own decoder. The game appends one frame at the end of a session, after the world
//! file is saved: a crash between the two loses that session's archive, but can never write the
//! same history twice. A frame cut short by a crash is dropped by the next append, so the file
//! always ends on a whole frame.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use thiserror::Error;

use crate::events::log::{EventLog, Record};
use crate::events::persist::{self, SavedLog};

const MAGIC: [u8; 4] = *b"MCEA";

/// The version written. Bump it when the layout changes.
pub const VERSION: u16 = 1;

const HEADER_LEN: u64 = 4 + 2;
const LEN_BYTES: usize = 4;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("cannot read or write the archive")]
    Io(#[from] io::Error),
    #[error("not an archive file")]
    BadMagic,
    #[error("archive version {0} is not supported (this build reads version {VERSION})")]
    UnsupportedVersion(u16),
    #[error("a frame of the archive is damaged")]
    Frame(#[from] persist::DecodeError),
    #[error("the file ends in the middle of the header or a frame")]
    Truncated,
    #[error("a frame is too long to be written")]
    FrameTooLong,
}

/// Every frame of an archive, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    pub frames: Vec<SavedLog>,
}

impl Archive {
    /// Every archived record in the order it happened.
    pub fn records(&self) -> impl Iterator<Item = Record> + '_ {
        self.frames.iter().flat_map(|frame| frame.log.records())
    }
}

/// Appends `records`, the history just folded away, to the archive at `path` as one frame,
/// creating the file if it is not there. Does nothing for no records.
///
/// A frame left half-written by an earlier crash is cut off first. The frame is synced to disk
/// before this returns.
///
/// # Errors
///
/// Returns [`ArchiveError`] if the file is not an archive this build reads, a frame is too
/// long, or the file cannot be read or written.
pub fn append(path: &Path, seed: u64, records: &[Record]) -> Result<(), ArchiveError> {
    if records.is_empty() {
        return Ok(());
    }
    let mut segment = EventLog::new();
    for &record in records {
        match record {
            Record::Event(event) => segment.push(event),
            Record::ActionEnd(kind) => segment.end_action(kind),
        }
    }
    let body = persist::encode(seed, &segment);
    let length = u32::try_from(body.len()).map_err(|_| ArchiveError::FrameTooLong)?;

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let end = whole_frames_end(&mut file)?;
    file.set_len(end)?;
    file.seek(SeekFrom::Start(end))?;

    let mut bytes = Vec::with_capacity(HEADER_LEN as usize + LEN_BYTES + body.len());
    if end == 0 {
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
    }
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&body);
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Where the last whole frame of `file` ends, or 0 if the file is empty or holds less than a
/// header, so a new frame starts the file over. Walks the frames by their lengths without
/// reading their contents.
fn whole_frames_end(file: &mut File) -> Result<u64, ArchiveError> {
    let size = file.metadata()?.len();
    if size < HEADER_LEN {
        return Ok(0);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut header)?;
    check_header(&header)?;
    let mut end = HEADER_LEN;
    while end + LEN_BYTES as u64 <= size {
        let mut length = [0u8; LEN_BYTES];
        file.read_exact(&mut length)?;
        let next = end + LEN_BYTES as u64 + u64::from(u32::from_le_bytes(length));
        if next > size {
            break;
        }
        end = next;
        file.seek(SeekFrom::Start(end))?;
    }
    Ok(end)
}

fn check_header(header: &[u8; HEADER_LEN as usize]) -> Result<(), ArchiveError> {
    if header[..4] != MAGIC {
        return Err(ArchiveError::BadMagic);
    }
    match u16::from_le_bytes([header[4], header[5]]) {
        VERSION => Ok(()),
        other => Err(ArchiveError::UnsupportedVersion(other)),
    }
}

/// # Errors
///
/// Returns [`ArchiveError`] if the bytes are not a complete archive of a supported version.
pub fn decode(bytes: &[u8]) -> Result<Archive, ArchiveError> {
    let (header, mut rest) = bytes
        .split_first_chunk::<{ HEADER_LEN as usize }>()
        .ok_or(ArchiveError::Truncated)?;
    check_header(header)?;
    let mut frames = Vec::new();
    while !rest.is_empty() {
        let (length, after) = rest
            .split_first_chunk::<LEN_BYTES>()
            .ok_or(ArchiveError::Truncated)?;
        let length = u32::from_le_bytes(*length) as usize;
        let (body, after) = after
            .split_at_checked(length)
            .ok_or(ArchiveError::Truncated)?;
        frames.push(persist::decode(body)?);
        rest = after;
    }
    Ok(Archive { frames })
}

/// Reads the archive at `path`, or `None` if there is no file there yet.
///
/// # Errors
///
/// Returns [`ArchiveError`] if the file exists but cannot be read or is not a valid archive.
pub fn load(path: &Path) -> Result<Option<Archive>, ArchiveError> {
    match std::fs::read(path) {
        Ok(bytes) => decode(&bytes).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use proptest::prelude::*;

    use super::*;
    use crate::events::log::ActionKind;
    use crate::shared::event::Event;
    use crate::shared::voxel::{BlockId, WorldPos};

    /// A scratch file path that is removed when the test ends, even if it fails.
    struct TempFile(std::path::PathBuf);

    impl TempFile {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            Self(std::env::temp_dir().join(format!(
                "mcea-{}-{}.mcea",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn set(x: i32, old: u16, new: u16) -> Event {
        Event::BlockSet {
            pos: WorldPos { x, y: 0, z: 0 },
            old: BlockId(old),
            new: BlockId(new),
        }
    }

    /// `count` one-event actions, the first at block `from`.
    fn records(from: i32, count: i32) -> Vec<Record> {
        (from..from + count)
            .flat_map(|x| {
                [
                    Record::Event(set(x, 0, 1)),
                    Record::ActionEnd(ActionKind::Edit),
                ]
            })
            .collect()
    }

    fn all(path: &Path) -> Vec<Record> {
        load(path)
            .expect("a readable archive")
            .expect("an existing archive")
            .records()
            .collect()
    }

    proptest! {
        #[test]
        fn appended_frames_read_back_as_one_history_in_order(
            sizes in prop::collection::vec(1i32..6, 1..5),
        ) {
            let file = TempFile::new();
            let mut expected = Vec::new();
            let mut next = 0;
            for size in sizes {
                let batch = records(next, size);
                next += size;
                prop_assert!(append(&file.0, 117, &batch).is_ok());
                expected.extend(batch);
            }
            prop_assert_eq!(all(&file.0), expected);
        }

        #[test]
        fn a_frame_cut_short_is_dropped_and_the_next_append_follows_the_whole_ones(
            cut in 1usize..40,
        ) {
            let file = TempFile::new();
            let (first, second, third) = (records(0, 3), records(10, 2), records(20, 2));
            prop_assert!(append(&file.0, 117, &first).is_ok());
            let whole = std::fs::metadata(&file.0).unwrap().len();
            prop_assert!(append(&file.0, 117, &second).is_ok());
            let size = std::fs::metadata(&file.0).unwrap().len();
            let cut = u64::try_from(cut).unwrap().min(size - whole - 1);
            OpenOptions::new().write(true).open(&file.0).unwrap().set_len(size - cut).unwrap();
            prop_assert!(matches!(
                decode(&std::fs::read(&file.0).unwrap()),
                Err(ArchiveError::Truncated)
            ));

            prop_assert!(append(&file.0, 117, &third).is_ok());

            let expected: Vec<Record> = first.into_iter().chain(third).collect();
            prop_assert_eq!(all(&file.0), expected);
        }
    }

    #[test]
    fn no_records_write_nothing_and_a_missing_file_loads_as_none() {
        let file = TempFile::new();
        assert!(append(&file.0, 117, &[]).is_ok());
        assert!(!file.0.exists());
        assert!(matches!(load(&file.0), Ok(None)));
    }

    #[test]
    fn a_file_that_is_not_an_archive_is_refused_and_left_alone() {
        let file = TempFile::new();
        std::fs::write(&file.0, b"not an archive at all").unwrap();
        assert!(matches!(
            append(&file.0, 117, &records(0, 1)),
            Err(ArchiveError::BadMagic)
        ));
        assert_eq!(std::fs::read(&file.0).unwrap(), b"not an archive at all");
    }

    #[test]
    fn another_version_is_refused() {
        let file = TempFile::new();
        let mut bytes = Vec::from(MAGIC);
        bytes.extend_from_slice(&(VERSION + 1).to_le_bytes());
        std::fs::write(&file.0, &bytes).unwrap();
        assert!(matches!(
            append(&file.0, 117, &records(0, 1)),
            Err(ArchiveError::UnsupportedVersion(v)) if v == VERSION + 1
        ));
        assert!(matches!(
            decode(&bytes),
            Err(ArchiveError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn a_cut_header_starts_the_file_over() {
        let file = TempFile::new();
        std::fs::write(&file.0, &MAGIC[..3]).unwrap();
        assert!(append(&file.0, 117, &records(0, 2)).is_ok());
        assert_eq!(all(&file.0), records(0, 2));
    }

    #[test]
    fn decoding_arbitrary_bytes_never_panics() {
        proptest!(|(bytes in prop::collection::vec(any::<u8>(), 0..200))| {
            let _ = decode(&bytes);
        });
    }
}
