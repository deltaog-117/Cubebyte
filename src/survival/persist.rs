//! Saving the meters to disk and reading them back, so a restart keeps the health and hunger
//! the player left with.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCST` |
//! | version | 2 | [`VERSION`] |
//! | health | 1 | `0..=METER_MAX` |
//! | hunger | 1 | `0..=METER_MAX` |
//!
//! Only the meters are saved, not the clock that times the next drain, which starts a full
//! period over after a restart.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::survival::model::{Meter, OutOfRange, Stats};

const MAGIC: [u8; 4] = *b"MCST";

/// Bump when the layout changes; older files are then refused, not guessed at.
pub const VERSION: u16 = 1;

const FILE_LEN: usize = 4 + 2 + 1 + 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("not a meters file")]
    BadMagic,
    #[error("meters version {0} is not supported (expected {VERSION})")]
    UnsupportedVersion(u16),
    #[error("the file is {0} bytes, not {FILE_LEN}")]
    WrongLength(usize),
    #[error("a saved meter is out of range")]
    BadMeter(#[from] OutOfRange),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("cannot read the file")]
    Io(#[from] io::Error),
    #[error("the file is damaged")]
    Decode(#[from] DecodeError),
}

/// Serialises the meters of `stats` in the layout described in the module docs.
pub fn encode(stats: Stats) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FILE_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.push(stats.health().value());
    bytes.push(stats.hunger().value());
    bytes
}

/// # Errors
///
/// Returns a [`DecodeError`] if the bytes are not a complete, supported meters file whose
/// values are in range.
pub fn decode(bytes: &[u8]) -> Result<Stats, DecodeError> {
    let Ok::<&[u8; FILE_LEN], _>(bytes) = bytes.try_into() else {
        return Err(DecodeError::WrongLength(bytes.len()));
    };
    if bytes[..4] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    Ok(Stats::with_meters(
        Meter::try_new(bytes[6])?,
        Meter::try_new(bytes[7])?,
    ))
}

/// Writes the meters to `path` without ever leaving a half-written file there: the bytes go
/// to a sibling file first, which then replaces `path` in one step.
///
/// # Errors
///
/// Returns the I/O error if the sibling file cannot be written or moved into place.
pub fn save(path: &Path, stats: Stats) -> io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, encode(stats))?;
    std::fs::rename(&staging, path)
}

/// Reads saved meters, or `None` if there is no file at `path` yet.
///
/// # Errors
///
/// Returns [`LoadError`] if the file exists but cannot be read or is not a valid meters file.
pub fn load(path: &Path) -> Result<Option<Stats>, LoadError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(decode(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::survival::model::METER_MAX;

    fn stats(health: u8, hunger: u8) -> Stats {
        Stats::with_meters(
            Meter::try_new(health).unwrap(),
            Meter::try_new(hunger).unwrap(),
        )
    }

    #[test]
    fn fresh_stats_round_trip() {
        assert_eq!(decode(&encode(Stats::default())), Ok(Stats::default()));
    }

    #[test]
    fn the_layout_is_pinned() {
        assert_eq!(encode(stats(7, 13)), b"MCST\x01\x00\x07\x0d");
    }

    #[test]
    fn damaged_files_are_refused() {
        let good = encode(Stats::default());
        assert_eq!(
            decode(&good[..good.len() - 1]),
            Err(DecodeError::WrongLength(good.len() - 1))
        );
        assert_eq!(decode(&[]), Err(DecodeError::WrongLength(0)));

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert_eq!(decode(&bad_magic), Err(DecodeError::BadMagic));

        let mut bad_version = good.clone();
        bad_version[4] = 9;
        assert_eq!(
            decode(&bad_version),
            Err(DecodeError::UnsupportedVersion(9))
        );

        let mut too_full = good.clone();
        too_full[6] = METER_MAX + 1;
        assert_eq!(
            decode(&too_full),
            Err(DecodeError::BadMeter(OutOfRange(METER_MAX + 1)))
        );
        let mut hunger_too_full = good;
        hunger_too_full[7] = 255;
        assert_eq!(
            decode(&hunger_too_full),
            Err(DecodeError::BadMeter(OutOfRange(255)))
        );
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let path = std::env::temp_dir().join("cubebyte-no-such-meters-file.mcst");
        let _ = std::fs::remove_file(&path);
        assert!(matches!(load(&path), Ok(None)));
    }

    #[test]
    fn a_saved_file_loads_back_and_leaves_no_staging_file() {
        let dir = std::env::temp_dir().join(format!("cubebyte-meters-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stats.mcst");

        save(&path, stats(3, 17)).unwrap();

        assert!(matches!(load(&path), Ok(Some(loaded)) if loaded == stats(3, 17)));
        assert!(!dir.join("stats.mcst.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    proptest! {
        #[test]
        fn any_meters_round_trip(health in 0..=METER_MAX, hunger in 0..=METER_MAX) {
            prop_assert_eq!(decode(&encode(stats(health, hunger))), Ok(stats(health, hunger)));
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..24)) {
            let _ = decode(&bytes);
        }
    }
}
