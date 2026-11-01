//! Taking hits, dying and starting over, and how hard the held items hit.

use crate::combat::model::Damage;
use crate::events::apply::slot_events;
use crate::inventory::model::Inventory;
use crate::items::registry;
use crate::shared::event::Event;
use crate::survival::model::{METER_MAX, Stats};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Survived,
    Died,
}

/// Applies `damage` to the player's health. Hitting someone already dead reports `Died`
/// again and changes nothing.
pub fn hit(stats: &mut Stats, damage: Damage) -> Outcome {
    stats.hurt(damage.points());
    if stats.is_dead() {
        Outcome::Died
    } else {
        Outcome::Survived
    }
}

/// The event for `damage` taken at the current health, which is the points health really
/// loses: none when the player is already dead.
pub fn hit_events(stats: &Stats, damage: Damage) -> Vec<Event> {
    let lost = stats.health_loss(damage.points());
    if lost == 0 {
        Vec::new()
    } else {
        vec![Event::HealthChanged { by: lost }]
    }
}

/// The events [`respawn`] amounts to: both meters refilled and the pack emptied. Moving the
/// player to the spawn point and restarting the stats clock are left to the caller, which
/// knows where the spawn is and owns the clock.
pub fn respawn_events(stats: &Stats, inventory: &Inventory) -> Vec<Event> {
    let mut events = Vec::new();
    let health = stats.health_gain(METER_MAX);
    if health != 0 {
        events.push(Event::HealthChanged { by: health });
    }
    let hunger = stats.hunger_gain(METER_MAX);
    if hunger != 0 {
        events.push(Event::HungerChanged { by: hunger });
    }
    events.extend(slot_events(inventory, &Inventory::default()));
    events
}

/// Starts the player over: full meters and an empty inventory.
pub fn respawn(stats: &mut Stats, inventory: &mut Inventory) {
    *stats = Stats::default();
    *inventory = Inventory::default();
}

/// How hard the player hits: the best weapon in the inventory, or a fist without one.
pub fn strike(inventory: &Inventory) -> Damage {
    inventory
        .slots()
        .iter()
        .flatten()
        .filter_map(|stack| registry::get(stack.item())?.damage)
        .filter_map(|points| Damage::try_new(points).ok())
        .max()
        .unwrap_or(Damage::FIST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::registry::ItemId;

    fn damage(points: u8) -> Damage {
        Damage::try_new(points).unwrap()
    }

    #[test]
    fn a_hit_that_leaves_health_is_survived() {
        let mut stats = Stats::default();
        assert_eq!(hit(&mut stats, damage(5)), Outcome::Survived);
        assert_eq!(stats.health().value(), 15);
    }

    #[test]
    fn a_hit_that_empties_health_kills_and_stays_dead() {
        let mut stats = Stats::default();
        assert_eq!(hit(&mut stats, damage(200)), Outcome::Died);
        assert_eq!(stats.health().value(), 0);
        assert_eq!(hit(&mut stats, damage(1)), Outcome::Died);
        assert_eq!(stats.health().value(), 0);
    }

    #[test]
    fn respawning_restores_the_meters_and_empties_the_pack() {
        let (mut stats, mut inventory) = (Stats::default(), Inventory::default());
        inventory.add(ItemId::STONE, 30).unwrap();
        hit(&mut stats, damage(255));

        respawn(&mut stats, &mut inventory);

        assert_eq!(stats, Stats::default());
        assert!(inventory.is_empty());
    }

    #[test]
    fn bare_hands_hit_with_a_fist_and_a_sword_hits_harder() {
        let mut inventory = Inventory::default();
        assert_eq!(strike(&inventory), Damage::FIST);
        inventory.add(ItemId::STONE, 10).unwrap();
        assert_eq!(strike(&inventory), Damage::FIST);
        inventory.add(ItemId::SWORD, 1).unwrap();
        assert_eq!(strike(&inventory).points(), 4);
    }

    #[test]
    fn a_hit_event_carries_the_health_really_lost() {
        let mut stats = Stats::default();
        assert_eq!(
            hit_events(&stats, damage(5)),
            [Event::HealthChanged { by: -5 }]
        );
        stats.hurt(18);
        assert_eq!(
            hit_events(&stats, damage(5)),
            [Event::HealthChanged { by: -2 }]
        );
        stats.hurt(2);
        assert!(hit_events(&stats, damage(5)).is_empty());
    }

    #[test]
    fn respawn_events_refill_the_meters_and_empty_the_pack() {
        let (mut stats, mut inventory) = (Stats::default(), Inventory::default());
        inventory.add(ItemId::STONE, 30).unwrap();
        hit(&mut stats, damage(255));

        let events = respawn_events(&stats, &inventory);

        assert_eq!(events[0], Event::HealthChanged { by: 20 });
        assert!(events.len() > 1);
        assert!(respawn_events(&Stats::default(), &Inventory::default()).is_empty());
    }
}
