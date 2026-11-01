//! Eating: turn a food item from the inventory into hunger points.

use thiserror::Error;

use crate::events::apply::slot_events;
use crate::inventory::model::Inventory;
use crate::items::registry::{self, ItemId};
use crate::shared::event::Event;
use crate::survival::model::{Meter, Stats};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EatError {
    #[error("hunger is already full")]
    NotHungry,
    #[error("the inventory holds no food")]
    NoFood,
}

/// Eats one food item, the first found in the lowest slot, and returns which.
///
/// Nothing is spent when hunger is already full, so food is never wasted.
///
/// # Errors
///
/// - [`EatError::NotHungry`] when hunger is full.
/// - [`EatError::NoFood`] when no held item restores hunger.
pub fn eat(inventory: &mut Inventory, stats: &mut Stats) -> Result<ItemId, EatError> {
    let (item, restored) = pick_food(inventory, stats)?;
    // The stack was just seen, so one is always there to take.
    inventory.take(item, 1).map_err(|_| EatError::NoFood)?;
    stats.restore_hunger(restored);
    Ok(item)
}

/// One meal worked out but not yet eaten.
#[derive(Debug, PartialEq, Eq)]
pub struct Meal {
    pub item: ItemId,
    /// The hunger points the food is worth. Near a full stomach fewer are gained, and
    /// `events` carry the points really gained.
    pub restored: u8,
    /// The slot change that spends the food, then the hunger it restores.
    pub events: Vec<Event>,
}

/// The meal [`eat`] would take, without changing `inventory` or `stats`.
///
/// # Errors
///
/// The same as [`eat`].
pub fn eat_events(inventory: &Inventory, stats: &Stats) -> Result<Meal, EatError> {
    let (item, restored) = pick_food(inventory, stats)?;
    let mut after = inventory.clone();
    after.take(item, 1).map_err(|_| EatError::NoFood)?;
    let mut events = slot_events(inventory, &after);
    // Hunger below full means at least a point is gained; a food worth nothing gains none.
    let gained = stats.hunger_gain(restored);
    if gained != 0 {
        events.push(Event::HungerChanged { by: gained });
    }
    Ok(Meal {
        item,
        restored,
        events,
    })
}

/// The food [`eat`] would choose and the points it restores.
fn pick_food(inventory: &Inventory, stats: &Stats) -> Result<(ItemId, u8), EatError> {
    if stats.hunger() == Meter::FULL {
        return Err(EatError::NotHungry);
    }
    inventory
        .slots()
        .iter()
        .flatten()
        .find_map(|stack| {
            let restored = registry::get(stack.item())?.restores_hunger?;
            Some((stack.item(), restored))
        })
        .ok_or(EatError::NoFood)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::survival::model::HUNGER_PERIOD;

    fn hungry() -> Stats {
        let mut stats = Stats::default();
        for _ in 0..HUNGER_PERIOD * 10 {
            stats.step();
        }
        stats
    }

    #[test]
    fn eating_spends_one_food_and_restores_its_points() {
        let (mut inventory, mut stats) = (Inventory::default(), hungry());
        inventory.add(ItemId::BREAD, 2).unwrap();
        let before = stats.hunger().value();

        assert_eq!(eat(&mut inventory, &mut stats), Ok(ItemId::BREAD));

        assert_eq!(inventory.count_of(ItemId::BREAD), 1);
        assert_eq!(stats.hunger().value(), before + 4);
    }

    #[test]
    fn a_meal_logs_the_hunger_really_gained() {
        let mut inventory = Inventory::default();
        inventory.add(ItemId::BREAD, 1).unwrap();
        let mut stats = Stats::default();
        for _ in 0..HUNGER_PERIOD * 2 {
            stats.step();
        }

        let meal = eat_events(&inventory, &stats).unwrap();

        assert_eq!(meal.restored, 4);
        assert_eq!(meal.events.last(), Some(&Event::HungerChanged { by: 2 }));
    }

    #[test]
    fn blocks_are_not_food() {
        let (mut inventory, mut stats) = (Inventory::default(), hungry());
        inventory.add(ItemId::DIRT, 5).unwrap();

        assert_eq!(eat(&mut inventory, &mut stats), Err(EatError::NoFood));
        assert_eq!(inventory.count_of(ItemId::DIRT), 5);
    }

    #[test]
    fn a_full_stomach_keeps_the_food() {
        let (mut inventory, mut stats) = (Inventory::default(), Stats::default());
        inventory.add(ItemId::STEW, 1).unwrap();

        assert_eq!(eat(&mut inventory, &mut stats), Err(EatError::NotHungry));
        assert_eq!(inventory.count_of(ItemId::STEW), 1);
    }
}
