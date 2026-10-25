//! Saving the event log to disk and reading it back.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCEL` |
//! | version | 2 | [`VERSION`] |
//! | seed | 8 | seed of the terrain the events were made on |
//! | records | 17 each | tag (1), x, y, z (4 each), old (2), new (2) |
//!
//! The record count is not stored: the records run to the end of the file, so a file cut
//! short mid-record is rejected instead of silently losing its last edit.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::events::log::EventLog;
use crate::shared::event::Event;
use crate::shared::voxel::{BlockId, WorldPos};

const MAGIC: [u8; 4] = *b"MCEL";

/// Bump when the layout changes; older files are then refused, not guessed at.
pub const VERSION: u16 = 1;

const HEADER_LEN: usize = 4 + 2 + 8;
const RECORD_LEN: usize = 1 + 3 * 4 + 2 + 2;

/// Record tag of [`Event::BlockSet`].
const TAG_BLOCK_SET: u8 = 1;

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
    #[error("event log version {0} is not supported (expected {VERSION})")]
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
    let mut bytes = Vec::with_capacity(HEADER_LEN + log.len() * RECORD_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&seed.to_le_bytes());
    for event in log.iter() {
        match *event {
            Event::BlockSet { pos, old, new } => {
                bytes.push(TAG_BLOCK_SET);
                bytes.extend_from_slice(&pos.x.to_le_bytes());
                bytes.extend_from_slice(&pos.y.to_le_bytes());
                bytes.extend_from_slice(&pos.z.to_le_bytes());
                bytes.extend_from_slice(&old.0.to_le_bytes());
                bytes.extend_from_slice(&new.0.to_le_bytes());
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
    if version != VERSION {
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
            })
    }

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
            prop_assert_eq!(bytes.len(), HEADER_LEN + events.len() * RECORD_LEN);
            prop_assert_eq!(decode(&bytes), Ok(SavedLog { seed, log }));
        }

        #[test]
        fn a_file_cut_inside_a_record_is_rejected(
            events in prop::collection::vec(event(), 1..20),
            cut in 1..RECORD_LEN,
        ) {
            let bytes = encode(0, &log_of(&events));
            prop_assert_eq!(decode(&bytes[..bytes.len() - cut]), Err(DecodeError::Truncated));
        }

        #[test]
        fn decoding_arbitrary_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..200)) {
            let _ = decode(&bytes);
        }
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
    fn an_unknown_tag_is_rejected() {
        let mut bytes = encode(7, &EventLog::new());
        bytes.extend_from_slice(&[9; RECORD_LEN]);
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
