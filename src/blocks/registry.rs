//! The one table of block names, colours and properties.
//!
//! A block's ID is its index in [`BLOCKS`]. IDs are written into saved worlds, so a new
//! block is only ever appended, never inserted or reordered.

use crate::shared::voxel::BlockId;

/// What the game knows about one block type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockDef {
    pub name: &'static str,
    /// Flat colour with every channel in `0.0..=1.0`.
    pub color: [f32; 3],
    /// Whether the block can be placed and stops the viewer's ray.
    pub solid: bool,
}

/// Colour of a block with no definition; magenta stands out on screen.
pub const UNKNOWN_COLOR: [f32; 3] = [1.0, 0.0, 1.0];

/// Indexed by [`BlockId`]. The `BlockId` constants must match these positions.
const BLOCKS: [BlockDef; 4] = [
    BlockDef {
        name: "air",
        color: [0.0, 0.0, 0.0],
        solid: false,
    },
    BlockDef {
        name: "stone",
        color: [0.50, 0.50, 0.52],
        solid: true,
    },
    BlockDef {
        name: "dirt",
        color: [0.45, 0.30, 0.18],
        solid: true,
    },
    BlockDef {
        name: "grass",
        color: [0.30, 0.65, 0.20],
        solid: true,
    },
];

/// The definition of `id`, or `None` for an ID no block uses.
pub fn get(id: BlockId) -> Option<&'static BlockDef> {
    BLOCKS.get(usize::from(id.0))
}

/// Colour of `id`, or [`UNKNOWN_COLOR`] when it has no definition.
pub fn color(id: BlockId) -> [f32; 3] {
    get(id).map_or(UNKNOWN_COLOR, |def| def.color)
}

/// Whether `id` is a defined block that can be placed.
pub fn is_solid(id: BlockId) -> bool {
    get(id).is_some_and(|def| def.solid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn id_constants_match_their_table_entries() {
        let named = [
            (BlockId::AIR, "air"),
            (BlockId::STONE, "stone"),
            (BlockId::DIRT, "dirt"),
            (BlockId::GRASS, "grass"),
        ];
        for (id, name) in named {
            assert_eq!(get(id).map(|def| def.name), Some(name));
        }
        assert_eq!(named.len(), BLOCKS.len());
    }

    #[test]
    fn only_air_is_not_solid() {
        for index in 0..BLOCKS.len() {
            let id = BlockId(u16::try_from(index).expect("small table"));
            assert_eq!(is_solid(id), !id.is_air());
        }
    }

    proptest! {
        #[test]
        fn every_id_has_a_displayable_colour(raw in any::<u16>()) {
            let id = BlockId(raw);
            prop_assert!(color(id).iter().all(|c| (0.0..=1.0).contains(c)));
            prop_assert_eq!(get(id).is_some(), usize::from(raw) < BLOCKS.len());
            if get(id).is_none() {
                prop_assert_eq!(color(id), UNKNOWN_COLOR);
                prop_assert!(!is_solid(id));
            }
        }
    }
}
