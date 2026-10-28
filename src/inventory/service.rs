//! Gathering and placing with items: building edits that cost and yield stacks.
//!
//! A break yields the block's item and a place spends one, in the same step as the block
//! edit. Every check that can refuse runs before the world changes, so a refused edit
//! leaves the world, the log and the inventory as they were.

use glam::Vec3;
use thiserror::Error;

use crate::building::model::{EditError, SolidBlock};
use crate::building::raycast::cast;
use crate::building::service::{
    REACH, block_event, break_block, break_target, place_block, place_target,
};
use crate::events::apply::slot_events;
use crate::events::log::EventLog;
use crate::inventory::model::Inventory;
use crate::items::registry::{self, ItemId};
use crate::shared::event::Event;
use crate::shared::voxel::{BlockId, WorldPos};
use crate::world::model::World;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StockError {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("the inventory has no room for what the block yields")]
    Full,
    #[error("the inventory holds none of that block")]
    Missing,
}

/// Breaks the block the viewer looks at and adds what it yields to `inventory`.
///
/// # Errors
///
/// - [`StockError::Edit`] when the break itself fails (see [`break_block`]).
/// - [`StockError::Full`] when the yield does not fit; the block is left standing.
pub fn gather_block(
    world: &mut World,
    log: &mut EventLog,
    inventory: &mut Inventory,
    origin: Vec3,
    direction: Vec3,
) -> Result<WorldPos, StockError> {
    let hit = cast(world, origin, direction, REACH).ok_or(EditError::NothingInReach)?;
    let drop = registry::drop_of(world.block_at(hit.block));
    if let Some(item) = drop
        && inventory.room_for(item).unwrap_or(0) == 0
    {
        return Err(StockError::Full);
    }
    let broken = break_block(world, log, origin, direction)?;
    if let Some(item) = drop
        && let Err(error) = inventory.add(item, 1)
    {
        // `room_for` already vouched for the item, so this cannot happen.
        tracing::error!(%error, "yield refused after the room check");
    }
    Ok(broken)
}

/// Places `block` against the face the viewer looks at, spending one of its item.
///
/// # Errors
///
/// - [`StockError::Missing`] when the inventory holds no item for `block`.
/// - [`StockError::Edit`] when the placement itself fails (see [`place_block`]); nothing
///   is spent.
pub fn place_from_stock(
    world: &mut World,
    log: &mut EventLog,
    inventory: &mut Inventory,
    origin: Vec3,
    direction: Vec3,
    block: SolidBlock,
) -> Result<WorldPos, StockError> {
    let item: ItemId = registry::drop_of(block.id()).ok_or(StockError::Missing)?;
    if inventory.count_of(item) == 0 {
        return Err(StockError::Missing);
    }
    let placed = place_block(world, log, origin, direction, block)?;
    if let Err(error) = inventory.take(item, 1) {
        // The count was checked above and nothing touched the inventory since.
        tracing::error!(%error, "stack vanished during placement");
    }
    Ok(placed)
}

/// The events for breaking the block the viewer looks at: the block change, then the slot
/// changes that add its yield. Nothing is applied.
///
/// # Errors
///
/// The same as [`gather_block`].
pub fn gather_events(
    world: &World,
    inventory: &Inventory,
    origin: Vec3,
    direction: Vec3,
) -> Result<(WorldPos, Vec<Event>), StockError> {
    let pos = break_target(world, origin, direction)?;
    let mut events = vec![block_event(world, pos, BlockId::AIR)];
    if let Some(item) = registry::drop_of(world.block_at(pos)) {
        let mut after = inventory.clone();
        if after.room_for(item).unwrap_or(0) == 0 || after.add(item, 1).is_err() {
            return Err(StockError::Full);
        }
        events.extend(slot_events(inventory, &after));
    }
    Ok((pos, events))
}

/// The events for placing `block` against the face the viewer looks at: the block change,
/// then the slot change that spends one of its item. Nothing is applied.
///
/// # Errors
///
/// The same as [`place_from_stock`].
pub fn place_events(
    world: &World,
    inventory: &Inventory,
    origin: Vec3,
    direction: Vec3,
    block: SolidBlock,
) -> Result<(WorldPos, Vec<Event>), StockError> {
    let item: ItemId = registry::drop_of(block.id()).ok_or(StockError::Missing)?;
    let mut after = inventory.clone();
    after.take(item, 1).map_err(|_| StockError::Missing)?;
    let pos = place_target(world, origin, direction)?;
    let mut events = vec![block_event(world, pos, block.id())];
    events.extend(slot_events(inventory, &after));
    Ok((pos, events))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::event::Event;
    use crate::shared::voxel::BlockId;

    const EYE: Vec3 = Vec3::new(0.5, 0.5, 0.5);

    fn world_with(block: BlockId, x: i32) -> World {
        let mut world = World::default();
        let pos = WorldPos { x, y: 0, z: 0 };
        let event = Event::BlockSet {
            pos,
            old: BlockId::AIR,
            new: block,
        };
        world.apply(&event).unwrap();
        world
    }

    #[test]
    fn breaking_a_block_adds_its_item() {
        let mut world = world_with(BlockId::DIRT, 3);
        let (mut log, mut inventory) = (EventLog::new(), Inventory::default());

        gather_block(&mut world, &mut log, &mut inventory, EYE, Vec3::X).unwrap();

        assert_eq!(inventory.count_of(ItemId::DIRT), 1);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn a_full_inventory_leaves_the_block_standing() {
        let mut world = world_with(BlockId::STONE, 3);
        let (mut log, mut inventory) = (EventLog::new(), Inventory::default());
        inventory.add(ItemId::DIRT, 27 * 64).unwrap();
        let before = world.clone();

        let result = gather_block(&mut world, &mut log, &mut inventory, EYE, Vec3::X);

        assert_eq!(result, Err(StockError::Full));
        assert_eq!(world, before);
        assert!(log.is_empty());
    }

    #[test]
    fn placing_spends_one_item() {
        let mut world = world_with(BlockId::STONE, 3);
        let (mut log, mut inventory) = (EventLog::new(), Inventory::default());
        inventory.add(ItemId::GRASS, 2).unwrap();

        let placed = place_from_stock(
            &mut world,
            &mut log,
            &mut inventory,
            EYE,
            Vec3::X,
            SolidBlock::GRASS,
        )
        .unwrap();

        assert_eq!(world.block_at(placed), BlockId::GRASS);
        assert_eq!(inventory.count_of(ItemId::GRASS), 1);
    }

    #[test]
    fn placing_without_the_item_changes_nothing() {
        let mut world = world_with(BlockId::STONE, 3);
        let (mut log, mut inventory) = (EventLog::new(), Inventory::default());
        let before = world.clone();

        let result = place_from_stock(
            &mut world,
            &mut log,
            &mut inventory,
            EYE,
            Vec3::X,
            SolidBlock::STONE,
        );

        assert_eq!(result, Err(StockError::Missing));
        assert_eq!(world, before);
        assert!(log.is_empty());
    }

    #[test]
    fn a_failed_placement_spends_nothing() {
        let mut world = World::default();
        let (mut log, mut inventory) = (EventLog::new(), Inventory::default());
        inventory.add(ItemId::STONE, 1).unwrap();

        let result = place_from_stock(
            &mut world,
            &mut log,
            &mut inventory,
            EYE,
            Vec3::X,
            SolidBlock::STONE,
        );

        assert_eq!(result, Err(StockError::Edit(EditError::NothingInReach)));
        assert_eq!(inventory.count_of(ItemId::STONE), 1);
    }
}
