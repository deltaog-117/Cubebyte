//! Saving the inventory to disk and reading it back.
//!
//! Layout, all integers little-endian:
//!
//! | Part | Bytes | Meaning |
//! |---|---|---|
//! | magic | 4 | `MCIN` |
//! | version | 2 | [`VERSION`] |
//! | slots | 3 each, [`SLOTS`] of them | item ID (2), count (1); item 0 is an empty slot |
//!
//! The slot count is fixed, so a file of any other length is rejected.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::inventory::model::{Inventory, SLOTS, Stack, StackError};
use crate::items::registry::ItemId;

const MAGIC: [u8; 4] = *b"MCIN";

/// Bump when the layout changes; older files are then refused, not guessed at.
pub const VERSION: u16 = 1;

const HEADER_LEN: usize = 4 + 2;
const SLOT_LEN: usize = 2 + 1;
const FILE_LEN: usize = HEADER_LEN + SLOTS * SLOT_LEN;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("not an inventory file")]
    BadMagic,
    #[error("inventory version {0} is not supported (expected {VERSION})")]
    UnsupportedVersion(u16),
    #[error("the file is {0} bytes, not {FILE_LEN}")]
    WrongLength(usize),
    #[error("slot {slot} holds an invalid stack")]
    BadStack {
        slot: usize,
        #[source]
        source: StackError,
    },
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("cannot read the file")]
    Io(#[from] io::Error),
    #[error("the file is damaged")]
    Decode(#[from] DecodeError),
}

/// Serialises `inventory` in the layout described in the module docs.
pub fn encode(inventory: &Inventory) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FILE_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    for slot in inventory.slots() {
        let (item, count) = slot.map_or((0, 0), |stack| (stack.item().0, stack.count()));
        bytes.extend_from_slice(&item.to_le_bytes());
        bytes.push(count);
    }
    bytes
}

/// # Errors
///
/// Returns a [`DecodeError`] if the bytes are not a complete, supported inventory whose
/// every stack is valid.
pub fn decode(bytes: &[u8]) -> Result<Inventory, DecodeError> {
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
    if body.len() != SLOTS * SLOT_LEN {
        return Err(DecodeError::WrongLength(bytes.len()));
    }

    let mut slots = [None; SLOTS];
    for (slot, (record, out)) in body.chunks_exact(SLOT_LEN).zip(&mut slots).enumerate() {
        let item = u16::from_le_bytes([record[0], record[1]]);
        if item == 0 {
            continue;
        }
        *out = Some(
            Stack::try_new(ItemId(item), record[2])
                .map_err(|source| DecodeError::BadStack { slot, source })?,
        );
    }
    Ok(Inventory::from_slots(slots))
}

/// Writes the inventory to `path` without ever leaving a half-written file there: the
/// bytes go to a sibling file first, which then replaces `path` in one step.
///
/// # Errors
///
/// Returns the I/O error if the sibling file cannot be written or moved into place.
pub fn save(path: &Path, inventory: &Inventory) -> io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, encode(inventory))?;
    std::fs::rename(&staging, path)
}

/// Reads a saved inventory, or `None` if there is no file at `path` yet.
///
/// # Errors
///
/// Returns [`LoadError`] if the file exists but cannot be read or is not a valid inventory.
pub fn load(path: &Path) -> Result<Option<Inventory>, LoadError> {
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
    fn an_empty_inventory_round_trips() {
        let inventory = Inventory::default();
        assert_eq!(decode(&encode(&inventory)), Ok(inventory));
    }

    #[test]
    fn the_layout_is_pinned() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(ItemId::DIRT, 5), Ok(0));
        let bytes = encode(&inventory);
        assert_eq!(&bytes[..6], b"MCIN\x01\x00");
        assert_eq!(&bytes[6..9], &[2, 0, 5]);
        assert_eq!(bytes.len(), FILE_LEN);
    }

    #[test]
    fn damaged_files_are_refused() {
        let good = encode(&Inventory::default());
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

        let mut unknown_item = good.clone();
        unknown_item[6] = 99;
        unknown_item[8] = 1;
        assert!(matches!(
            decode(&unknown_item),
            Err(DecodeError::BadStack { slot: 0, .. })
        ));

        let mut too_tall = good;
        too_tall[6] = 1;
        too_tall[8] = 200;
        assert!(matches!(
            decode(&too_tall),
            Err(DecodeError::BadStack { slot: 0, .. })
        ));
    }

    proptest! {
        #[test]
        fn any_inventory_round_trips(
            adds in prop::collection::vec((1u16..=3, 0u32..300), 0..30),
        ) {
            let mut inventory = Inventory::default();
            for (item, count) in adds {
                inventory.add(ItemId(item), count).unwrap();
            }
            prop_assert_eq!(decode(&encode(&inventory)), Ok(inventory));
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..120)) {
            let _ = decode(&bytes);
        }
    }
}
