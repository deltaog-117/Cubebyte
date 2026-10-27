//! The one table of item names and properties.
//!
//! An item's ID is its position in [`ITEMS`] plus one, so ID 0 is "no item" and a zeroed
//! save slot is an empty slot. IDs are written into saved inventories, so a new item is
//! only ever appended, never inserted or reordered.

use crate::shared::voxel::BlockId;

/// Identifies an item type. ID 0 names no item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(pub u16);

impl ItemId {
    pub const STONE: Self = Self(1);
    pub const DIRT: Self = Self(2);
    pub const GRASS: Self = Self(3);
    pub const BREAD: Self = Self(4);
    pub const STEW: Self = Self(5);
    pub const SWORD: Self = Self(6);
}

/// What the game knows about one item type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemDef {
    pub name: &'static str,
    /// Most items one stack holds; at least 1.
    pub stack_limit: u8,
    /// The block this item places, if it places one.
    pub places: Option<BlockId>,
    /// Hunger points eating this item restores, if it is food.
    pub restores_hunger: Option<u8>,
    /// Damage one hit deals while this item is held, if it is a weapon.
    pub damage: Option<u8>,
}

/// Indexed by `ItemId - 1`. The `ItemId` constants must match these positions.
const ITEMS: [ItemDef; 6] = [
    ItemDef {
        name: "stone",
        stack_limit: 64,
        places: Some(BlockId::STONE),
        restores_hunger: None,
        damage: None,
    },
    ItemDef {
        name: "dirt",
        stack_limit: 64,
        places: Some(BlockId::DIRT),
        restores_hunger: None,
        damage: None,
    },
    ItemDef {
        name: "grass",
        stack_limit: 64,
        places: Some(BlockId::GRASS),
        restores_hunger: None,
        damage: None,
    },
    ItemDef {
        name: "bread",
        stack_limit: 64,
        places: None,
        restores_hunger: Some(4),
        damage: None,
    },
    ItemDef {
        name: "stew",
        stack_limit: 16,
        places: None,
        restores_hunger: Some(8),
        damage: None,
    },
    ItemDef {
        name: "sword",
        stack_limit: 1,
        places: None,
        restores_hunger: None,
        damage: Some(4),
    },
];

/// The definition of `id`, or `None` for an ID no item uses.
pub fn get(id: ItemId) -> Option<&'static ItemDef> {
    let index = usize::from(id.0.checked_sub(1)?);
    ITEMS.get(index)
}

/// Every defined item, in ID order.
pub fn ids() -> impl Iterator<Item = ItemId> {
    (1..=ITEMS.len() as u16).map(ItemId)
}

/// The item breaking `block` yields, or `None` when it yields nothing (air, or an
/// undefined block).
pub fn drop_of(block: BlockId) -> Option<ItemId> {
    let index = ITEMS.iter().position(|def| def.places == Some(block))?;
    u16::try_from(index + 1).ok().map(ItemId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::registry as blocks;

    #[test]
    fn the_id_constants_match_their_table_rows() {
        assert_eq!(get(ItemId::STONE).map(|def| def.name), Some("stone"));
        assert_eq!(get(ItemId::DIRT).map(|def| def.name), Some("dirt"));
        assert_eq!(get(ItemId::GRASS).map(|def| def.name), Some("grass"));
        assert_eq!(get(ItemId::BREAD).map(|def| def.name), Some("bread"));
        assert_eq!(get(ItemId::STEW).map(|def| def.name), Some("stew"));
        assert_eq!(get(ItemId::SWORD).map(|def| def.name), Some("sword"));
    }

    #[test]
    fn id_zero_and_ids_past_the_table_are_not_items() {
        assert_eq!(get(ItemId(0)), None);
        assert_eq!(get(ItemId(ITEMS.len() as u16 + 1)), None);
    }

    #[test]
    fn every_stack_limit_is_at_least_one() {
        assert!(ITEMS.iter().all(|def| def.stack_limit >= 1));
    }

    #[test]
    fn ids_lists_every_item_once() {
        let ids: Vec<_> = ids().collect();
        assert_eq!(ids.len(), ITEMS.len());
        assert!(ids.iter().all(|&id| get(id).is_some()));
    }

    #[test]
    fn food_restores_something_and_is_not_placeable() {
        for def in &ITEMS {
            if let Some(restored) = def.restores_hunger {
                assert!(restored > 0, "{} restores nothing", def.name);
                assert_eq!(def.places, None, "{} is food and a block", def.name);
            }
        }
    }

    #[test]
    fn a_weapon_hurts_and_is_neither_food_nor_a_block() {
        for def in &ITEMS {
            let Some(damage) = def.damage else { continue };
            assert!(damage > 0, "{} deals no damage", def.name);
            assert_eq!(
                def.restores_hunger, None,
                "{} is food and a weapon",
                def.name
            );
            assert_eq!(def.places, None, "{} is a block and a weapon", def.name);
        }
    }

    #[test]
    fn every_placed_block_is_a_solid_block() {
        for def in &ITEMS {
            let Some(block) = def.places else { continue };
            assert!(
                blocks::is_solid(block),
                "{} places a non-solid block",
                def.name
            );
        }
    }

    #[test]
    fn breaking_a_block_yields_the_item_that_places_it() {
        assert_eq!(drop_of(BlockId::STONE), Some(ItemId::STONE));
        assert_eq!(drop_of(BlockId::DIRT), Some(ItemId::DIRT));
        assert_eq!(drop_of(BlockId::GRASS), Some(ItemId::GRASS));
        assert_eq!(drop_of(BlockId::AIR), None);
        assert_eq!(drop_of(BlockId(999)), None);
    }
}
