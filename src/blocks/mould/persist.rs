//! Saving the moulded-block table (`moulds.mcmd`) and reading it back.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCMD` |
//! | version | 2 | [`VERSION`] |
//! | count | 4 | number of entries |
//! | entries | 10 each | material block ID (2), shape bits (8) |
//!
//! An entry's ID is [`MOULD_FIRST`] plus its position, so the saved world's moulded blocks keep
//! their meaning. A file that does not decode is refused whole, and the game leaves it alone.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::blocks::mould::{Cut, MAX_MOULDS, MouldError, Moulds, Shape};
use crate::shared::voxel::BlockId;

const MAGIC: [u8; 4] = *b"MCMD";

/// Bump when the layout changes; older files are then refused, not guessed at.
pub const VERSION: u16 = 1;

const HEADER_LEN: usize = 4 + 2 + 4;
const ENTRY_LEN: usize = 2 + 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("not a moulded-blocks file")]
    BadMagic,
    #[error("moulded-blocks version {0} is not supported (expected {VERSION})")]
    UnsupportedVersion(u16),
    #[error("the file is {0} bytes, which does not fit its count")]
    WrongLength(usize),
    #[error("entry {0} is not a partial cut")]
    BadShape(usize),
    #[error("the table is not valid")]
    BadTable(#[from] MouldError),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("cannot read the file")]
    Io(#[from] io::Error),
    #[error("the file is damaged")]
    Decode(#[from] DecodeError),
}

/// The bytes of `moulds` in the layout above.
pub fn encode(moulds: &Moulds) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + moulds.len() * ENTRY_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    // The table never holds more than `MAX_MOULDS` entries, which fits `u32`.
    bytes.extend_from_slice(&(moulds.len() as u32).to_le_bytes());
    for &(base, shape) in moulds.entries() {
        bytes.extend_from_slice(&base.0.to_le_bytes());
        bytes.extend_from_slice(&shape.bits().to_le_bytes());
    }
    bytes
}

/// # Errors
///
/// Returns a [`DecodeError`] for a file that is not a complete, supported table of valid
/// entries.
pub fn decode(bytes: &[u8]) -> Result<Moulds, DecodeError> {
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(DecodeError::WrongLength(bytes.len()));
    };
    if header[..4] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let count = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as usize;
    if count > MAX_MOULDS || body.len() != count * ENTRY_LEN {
        return Err(DecodeError::WrongLength(bytes.len()));
    }
    let entries = body
        .chunks_exact(ENTRY_LEN)
        .enumerate()
        .map(|(index, record)| {
            let base = BlockId(u16::from_le_bytes([record[0], record[1]]));
            let mut bits = [0u8; 8];
            bits.copy_from_slice(&record[2..]);
            match Cut::of(u64::from_le_bytes(bits)) {
                Cut::Partial(shape) => Ok((base, shape)),
                Cut::Gone | Cut::Whole => Err(DecodeError::BadShape(index)),
            }
        })
        .collect::<Result<Vec<(BlockId, Shape)>, _>>()?;
    Ok(Moulds::from_entries(&entries)?)
}

/// Writes the table to `path` without leaving a half-written file there.
///
/// # Errors
///
/// Returns the I/O error if the sibling file cannot be written or moved into place.
pub fn save(path: &Path, moulds: &Moulds) -> io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, encode(moulds))?;
    std::fs::rename(&staging, path)
}

/// Reads a saved table, or `None` if there is no file at `path` yet.
///
/// # Errors
///
/// Returns [`LoadError`] if the file exists but cannot be read or is not a valid table.
pub fn load(path: &Path) -> Result<Option<Moulds>, LoadError> {
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

    #[test]
    fn an_empty_table_round_trips() {
        let moulds = Moulds::default();
        assert_eq!(decode(&encode(&moulds)), Ok(moulds));
    }

    #[test]
    fn the_layout_is_pinned() {
        let mut moulds = Moulds::default();
        moulds.id_for(BlockId::DIRT, 0x0102).unwrap();
        assert_eq!(
            encode(&moulds),
            [
                b'M', b'C', b'M', b'D', 1, 0, 1, 0, 0, 0, 2, 0, 0x02, 0x01, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn damaged_files_are_refused() {
        let mut moulds = Moulds::default();
        moulds.id_for(BlockId::STONE, 5).unwrap();
        let good = encode(&moulds);
        assert_eq!(
            decode(&good[..good.len() - 1]),
            Err(DecodeError::WrongLength(good.len() - 1))
        );
        let mut wrong_magic = good.clone();
        wrong_magic[0] = b'X';
        assert_eq!(decode(&wrong_magic), Err(DecodeError::BadMagic));
        let mut future = good.clone();
        future[4] = 9;
        assert_eq!(decode(&future), Err(DecodeError::UnsupportedVersion(9)));
        let mut whole = good.clone();
        whole[HEADER_LEN + 2..].fill(0xFF);
        assert_eq!(decode(&whole), Err(DecodeError::BadShape(0)));
        let mut air = good;
        air[HEADER_LEN] = 0;
        assert_eq!(
            decode(&air),
            Err(DecodeError::BadTable(MouldError::NotPlain))
        );
    }

    proptest! {
        #[test]
        fn any_table_round_trips(cuts in proptest::collection::vec((1u16..=3, 1u64..u64::MAX), 0..30)) {
            let mut moulds = Moulds::default();
            for (base, bits) in cuts {
                moulds.id_for(BlockId(base), bits).unwrap();
            }
            prop_assert_eq!(decode(&encode(&moulds)), Ok(moulds));
        }

        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = decode(&bytes);
        }
    }
}
