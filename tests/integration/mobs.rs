use glam::Vec3;
use minecraft_clone::combat::model::Damage;
use minecraft_clone::combat::service::{Outcome, hit};
use minecraft_clone::crafting::registry::{RecipeId, next};
use minecraft_clone::crafting::service::craft;
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::mobs::model::Zombie;
use minecraft_clone::mobs::service::{step, strike};
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
use minecraft_clone::survival::model::Stats;
use minecraft_clone::world::model::World;

const DT: f32 = 0.05;

fn flat_world() -> World {
    let mut world = World::default();
    for x in -20..20 {
        for z in -20..20 {
            let pos = WorldPos { x, y: 0, z };
            let event = Event::BlockSet {
                pos,
                old: world.block_at(pos),
                new: BlockId::STONE,
            };
            assert_eq!(world.apply(&event), Ok(()));
        }
    }
    world
}

#[test]
fn a_chasing_zombie_wears_the_player_down_and_a_sword_beats_it() {
    let world = flat_world();
    let mut stats = Stats::default();
    let mut zombie = Zombie::try_new(Vec3::new(8.5, 1.0, 0.5)).unwrap();
    let player = Vec3::new(0.5, 1.0, 0.5);

    // Left alone, the zombie arrives and the player eventually dies.
    let mut steps = 0;
    while !stats.is_dead() {
        if let Some(damage) = step(&mut zombie, player, &world, DT) {
            hit(&mut stats, damage);
        }
        steps += 1;
        assert!(steps < 2000, "the zombie never killed the player");
    }

    // With a sword, five hits kill a fresh zombie.
    let mut inventory = Inventory::default();
    inventory.add(ItemId::STONE, 6).unwrap();
    inventory.add(ItemId::DIRT, 2).unwrap();
    let mut id = RecipeId::FIRST;
    let crafted = (0..3).any(|_| {
        let made = craft(&mut inventory, id).is_ok();
        id = next(id);
        made
    });
    assert!(crafted);
    assert_eq!(inventory.count_of(ItemId::SWORD), 1);

    let mut fresh = Zombie::try_new(Vec3::new(8.5, 1.0, 0.5)).unwrap();
    let blow = minecraft_clone::combat::service::strike(&inventory);
    assert_eq!(blow, Damage::try_new(4).unwrap());
    let outcomes: Vec<Outcome> = (0..5).map(|_| strike(&mut fresh, blow)).collect();
    assert_eq!(outcomes.last(), Some(&Outcome::Died));
    assert!(outcomes[..4].iter().all(|o| *o == Outcome::Survived));
}
