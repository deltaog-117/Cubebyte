use minecraft_clone::combat::model::Damage;
use minecraft_clone::combat::service::{Outcome, hit, respawn, strike};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::survival::model::{METER_MAX, Stats};
use proptest::prelude::*;

proptest! {
    // However hits, hunger steps and respawns interleave, health stays in range, `Died`
    // is reported exactly when health is zero, and a respawn always starts clean.
    #[test]
    fn health_stays_in_range_and_death_means_zero(
        actions in prop::collection::vec((0u8..3, 1u8..=30), 0..80),
    ) {
        let (mut stats, mut inventory) = (Stats::default(), Inventory::default());
        inventory.add(ItemId::DIRT, 5).unwrap();
        for (action, amount) in actions {
            match action {
                0 => {
                    let before = stats.health().value();
                    let outcome = hit(&mut stats, Damage::try_new(amount).unwrap());
                    prop_assert_eq!(stats.health().value(), before.saturating_sub(amount));
                    prop_assert_eq!(outcome == Outcome::Died, stats.health().value() == 0);
                }
                1 => {
                    for _ in 0..u32::from(amount) * 40 {
                        stats.step();
                    }
                }
                _ => {
                    respawn(&mut stats, &mut inventory);
                    prop_assert_eq!(stats, Stats::default());
                    prop_assert!(inventory.is_empty());
                }
            }
            prop_assert!(stats.health().value() <= METER_MAX);
            prop_assert_eq!(stats.is_dead(), stats.health().value() == 0);
        }
    }

    // Holding more never makes a hit weaker, and the sword always beats a fist.
    #[test]
    fn carrying_more_never_weakens_the_strike(count in 0u32..60, sword in any::<bool>()) {
        let mut inventory = Inventory::default();
        inventory.add(ItemId::STONE, count).unwrap();
        let bare = strike(&inventory);
        if sword {
            inventory.add(ItemId::SWORD, 1).unwrap();
            prop_assert!(strike(&inventory) > bare);
        } else {
            prop_assert_eq!(bare, Damage::FIST);
        }
    }
}
