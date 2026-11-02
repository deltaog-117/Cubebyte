use glam::Vec3;
use minecraft_clone::combat::model::Damage;
use minecraft_clone::combat::service::hit;
use minecraft_clone::crafting::registry::{RecipeId, next};
use minecraft_clone::crafting::service::craft;
use minecraft_clone::events::apply::{Stage, apply_all};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::mobs::model::{Mobs, Zombie};
use minecraft_clone::mobs::service::{spawn_event, step, strike_events};
use minecraft_clone::player::model::Player;
use minecraft_clone::shared::event::{Event, MobId};
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

    let id = MobId::FIRST;
    let blow = minecraft_clone::combat::service::strike(&inventory);
    assert_eq!(blow, Damage::try_new(4).unwrap());
    let (mut world, mut player, mut stats) = (
        World::default(),
        Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap(),
        Stats::default(),
    );
    let mut mobs = Mobs::default();
    let mut commit = |mobs: &mut Mobs, events: &[Event]| {
        let mut stage = Stage {
            world: &mut world,
            player: &mut player,
            inventory: &mut inventory,
            stats: &mut stats,
            mobs,
        };
        assert_eq!(apply_all(&mut stage, events), Ok(()));
    };
    commit(&mut mobs, &[spawn_event(id, Vec3::new(8.5, 1.0, 0.5))]);

    // Four blows leave it alive; the fifth takes the rest and removes it.
    for _ in 0..4 {
        let events = strike_events(id, mobs.get(id).unwrap(), blow);
        assert_eq!(events.len(), 1);
        commit(&mut mobs, &events);
        assert!(mobs.get(id).is_some());
    }
    let events = strike_events(id, mobs.get(id).unwrap(), blow);
    assert_eq!(events.len(), 2);
    commit(&mut mobs, &events);
    assert!(mobs.is_empty());
}
