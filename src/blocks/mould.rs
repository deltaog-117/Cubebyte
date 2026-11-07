//! Moulded blocks: a block cut into 4×4×4 tiny cubes, some of them gone.
//!
//! A [`Shape`] says which tiny cubes are left. A moulded block is stored as a block ID at
//! or above [`MOULD_FIRST`], which names a (material, shape) pair in a [`Moulds`] table; every
//! edit is therefore still an ordinary block change from one ID to another, so undo, redo,
//! zone rewind, the log and replay treat a moulded block like any other and stay exact. The
//! table only ever grows, so an ID keeps its meaning, and it is saved beside the world.
//!
//! Chipping every tiny cube away is air, and filling them all again is the plain material,
//! so a block that looks whole is never stored as moulded: [`Cut::of`] decides that.

use std::collections::HashMap;

use thiserror::Error;

use crate::blocks::registry;
use crate::shared::voxel::{BlockId, TINY_SIDE, tiny_bit};

pub mod persist;

/// Tiny cubes along one side of a block.
pub const SIDE: i32 = TINY_SIDE;

/// Tiny cubes in one block.
pub const CELLS: u8 = 64;

/// The first moulded block ID. Every ID below is a plain block, as before.
pub const MOULD_FIRST: u16 = 0x8000;

/// Most distinct moulded blocks a world can hold, which is every ID from [`MOULD_FIRST`] up.
pub const MAX_MOULDS: usize = 0x8000;

/// The bit of the tiny cube at `(x, y, z)`, each in `0..SIDE`, or `None` outside the block.
pub fn bit(x: i32, y: i32, z: i32) -> Option<u64> {
    tiny_bit(x, y, z)
}

/// The tiny cubes left of a block that is neither whole nor gone: at least one present and at
/// least one missing. The only way to get one is [`Cut::of`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shape(u64);

impl Shape {
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Whether the tiny cube at `(x, y, z)` is still there; `false` outside the block.
    pub fn has(self, x: i32, y: i32, z: i32) -> bool {
        bit(x, y, z).is_some_and(|bit| self.0 & bit != 0)
    }
}

/// What a set of tiny cubes makes of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Nothing is left: the cell is air.
    Gone,
    /// Some are left: a moulded block.
    Partial(Shape),
    /// All are left: the plain material.
    Whole,
}

impl Cut {
    /// Sorts `bits` (one bit per tiny cube) into gone, partial or whole.
    pub const fn of(bits: u64) -> Self {
        match bits {
            0 => Self::Gone,
            u64::MAX => Self::Whole,
            _ => Self::Partial(Shape(bits)),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MouldError {
    #[error("this world already holds {MAX_MOULDS} different moulded blocks")]
    Full,
    #[error("only a plain solid block can be moulded")]
    NotPlain,
}

/// Every moulded block a world has made, by ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Moulds {
    list: Vec<(BlockId, Shape)>,
    index: HashMap<(BlockId, Shape), BlockId>,
}

impl Moulds {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The material and shape a moulded ID names, or `None` for a plain ID or one the table
    /// does not hold.
    pub fn get(&self, id: BlockId) -> Option<(BlockId, Shape)> {
        self.list
            .get(usize::from(id.0.checked_sub(MOULD_FIRST)?))
            .copied()
    }

    /// Every entry in ID order, for saving.
    pub fn entries(&self) -> &[(BlockId, Shape)] {
        &self.list
    }

    /// The ID of `shape` cut from `base`, adding it to the table the first time.
    ///
    /// # Errors
    ///
    /// - [`MouldError::NotPlain`] when `base` is air, a moulded ID or has no solid definition.
    /// - [`MouldError::Full`] when the table holds [`MAX_MOULDS`] entries.
    pub fn intern(&mut self, base: BlockId, shape: Shape) -> Result<BlockId, MouldError> {
        if !registry::is_solid(base) {
            return Err(MouldError::NotPlain);
        }
        if let Some(&id) = self.index.get(&(base, shape)) {
            return Ok(id);
        }
        if self.list.len() >= MAX_MOULDS {
            return Err(MouldError::Full);
        }
        // The table is shorter than `MAX_MOULDS`, so the offset fits `u16`.
        let id = BlockId(MOULD_FIRST + self.list.len() as u16);
        self.list.push((base, shape));
        self.index.insert((base, shape), id);
        Ok(id)
    }

    /// The ID for `bits` cut from `base`: air when nothing is left, `base` when all is, and a
    /// moulded ID otherwise.
    ///
    /// # Errors
    ///
    /// The same as [`Self::intern`] when the result is moulded.
    pub fn id_for(&mut self, base: BlockId, bits: u64) -> Result<BlockId, MouldError> {
        match Cut::of(bits) {
            Cut::Gone => Ok(BlockId::AIR),
            Cut::Whole => Ok(base),
            Cut::Partial(shape) => self.intern(base, shape),
        }
    }

    /// The tiny cubes left in the block `id` names: all of them for a plain solid block,
    /// none for air or an unknown ID.
    pub fn bits_of(&self, id: BlockId) -> u64 {
        match self.get(id) {
            Some((_, shape)) => shape.bits(),
            None if registry::is_solid(id) => u64::MAX,
            None => 0,
        }
    }

    /// The material of `id`: the base of a moulded ID, or `id` itself.
    pub fn base_of(&self, id: BlockId) -> BlockId {
        self.get(id).map_or(id, |(base, _)| base)
    }

    /// A table rebuilt from saved entries, in order.
    ///
    /// # Errors
    ///
    /// Returns [`MouldError::NotPlain`] for an entry whose material cannot be moulded, and
    /// [`MouldError::Full`] for too many entries. A repeated entry is refused as `NotPlain`
    /// too, since it would give one shape two IDs.
    pub fn from_entries(entries: &[(BlockId, Shape)]) -> Result<Self, MouldError> {
        let mut table = Self::default();
        for &(base, shape) in entries {
            let before = table.len();
            table.intern(base, shape)?;
            if table.len() == before {
                return Err(MouldError::NotPlain);
            }
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_cut_is_gone_partial_or_whole() {
        assert_eq!(Cut::of(0), Cut::Gone);
        assert_eq!(Cut::of(u64::MAX), Cut::Whole);
        assert!(matches!(Cut::of(1), Cut::Partial(shape) if shape.has(0, 0, 0)));
    }

    #[test]
    fn a_tiny_cube_is_found_by_its_place_and_nowhere_outside() {
        assert_eq!(bit(0, 0, 0), Some(1));
        assert_eq!(bit(1, 0, 0), Some(2));
        assert_eq!(bit(0, 1, 0), Some(16));
        assert_eq!(bit(3, 3, 3), Some(1 << 63));
        assert_eq!(bit(4, 0, 0), None);
        assert_eq!(bit(0, -1, 0), None);
    }

    #[test]
    fn the_same_cut_gets_the_same_id_and_ids_are_handed_out_in_order() {
        let mut moulds = Moulds::default();
        let a = moulds.id_for(BlockId::STONE, 0xF).unwrap();
        let b = moulds.id_for(BlockId::DIRT, 0xF).unwrap();
        assert_eq!((a.0, b.0), (MOULD_FIRST, MOULD_FIRST + 1));
        assert_eq!(moulds.id_for(BlockId::STONE, 0xF), Ok(a));
        assert_eq!(moulds.len(), 2);
        assert_eq!(moulds.base_of(b), BlockId::DIRT);
    }

    #[test]
    fn whole_and_gone_never_enter_the_table() {
        let mut moulds = Moulds::default();
        assert_eq!(moulds.id_for(BlockId::STONE, u64::MAX), Ok(BlockId::STONE));
        assert_eq!(moulds.id_for(BlockId::STONE, 0), Ok(BlockId::AIR));
        assert!(moulds.is_empty());
    }

    #[test]
    fn only_plain_solid_blocks_can_be_moulded() {
        let mut moulds = Moulds::default();
        assert_eq!(moulds.id_for(BlockId::AIR, 1), Err(MouldError::NotPlain));
        assert_eq!(moulds.id_for(BlockId(999), 1), Err(MouldError::NotPlain));
        let moulded = moulds.id_for(BlockId::STONE, 1).unwrap();
        assert_eq!(moulds.id_for(moulded, 3), Err(MouldError::NotPlain));
    }

    #[test]
    fn bits_of_reads_plain_moulded_and_missing_blocks() {
        let mut moulds = Moulds::default();
        let id = moulds.id_for(BlockId::STONE, 0b101).unwrap();
        assert_eq!(moulds.bits_of(id), 0b101);
        assert_eq!(moulds.bits_of(BlockId::STONE), u64::MAX);
        assert_eq!(moulds.bits_of(BlockId::AIR), 0);
        assert_eq!(moulds.bits_of(BlockId(MOULD_FIRST + 9)), 0);
    }

    proptest! {
        #[test]
        fn any_cut_gives_back_the_bits_it_was_made_from(bits in any::<u64>()) {
            let mut moulds = Moulds::default();
            let id = moulds.id_for(BlockId::GRASS, bits).unwrap();
            prop_assert_eq!(moulds.bits_of(id), bits);
        }

        #[test]
        fn a_table_rebuilt_from_its_entries_is_the_same_table(
            cuts in proptest::collection::vec(1u64..u64::MAX, 0..20),
        ) {
            let mut moulds = Moulds::default();
            for bits in cuts {
                moulds.id_for(BlockId::STONE, bits).unwrap();
            }
            prop_assert_eq!(Moulds::from_entries(moulds.entries()), Ok(moulds));
        }
    }
}
