//! Taking hits, dying and starting over, and how hard the held items hit.

use crate::combat::model::Damage;
use crate::inventory::model::Inventory;
use crate::items::registry;
use crate::survival::model::Stats;

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
}
