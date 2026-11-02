use glam::Vec3;
use minecraft_clone::events::apply::{Stage, apply};
use minecraft_clone::events::log::EventLog;
use minecraft_clone::events::persist::{decode, encode};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::mobs::model::Mobs;
use minecraft_clone::player::model::Player;
use minecraft_clone::scripting::model::{
    Answer, Command, Cooldowns, Facing, Limits, Pace, Permission, Permissions, Query,
};
use minecraft_clone::scripting::service::{ask, run};
use minecraft_clone::shared::clock::Tick;
use minecraft_clone::shared::event::{Event, MobId};
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
use minecraft_clone::survival::model::Stats;
use minecraft_clone::world::model::World;
use proptest::prelude::*;

fn cell(x: i32, y: i32, z: i32) -> WorldPos {
    WorldPos { x, y, z }
}

fn floor() -> World {
    let mut world = World::default();
    for x in -20..20 {
        for z in -20..20 {
            let event = Event::BlockSet {
                pos: cell(x, 0, z),
                old: BlockId::AIR,
                new: BlockId::STONE,
            };
            assert_eq!(world.apply(&event), Ok(()));
        }
    }
    world
}

/// A pack with fuel in it, since every script command burns some.
fn fuelled(fuel: u32) -> Inventory {
    let mut inventory = Inventory::default();
    assert!(
        inventory.add(ItemId::FUEL, fuel).is_ok(),
        "fuel is a defined item"
    );
    inventory
}

/// More fuel than any session below can burn.
const FUEL: u32 = 900;

fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        (-20i32..20, 1i32..3, -20i32..20)
            .prop_map(|(x, y, z)| Command::Teleport { to: cell(x, y, z) }),
        (1u16..4, 1u16..40).prop_map(|(item, count)| Command::GiveItem {
            item: ItemId(item),
            count
        }),
        ((-20i32..20, 0i32..3, -20i32..20), 0u16..4).prop_map(|((x, y, z), id)| {
            Command::SetBlock {
                pos: cell(x, y, z),
                block: BlockId(id),
            }
        }),
        (-20i32..20, 1i32..3, -20i32..20)
            .prop_map(|(x, y, z)| Command::SpawnMob { at: cell(x, y, z) }),
    ]
}

proptest! {
    #[test]
    fn a_scripted_session_survives_a_save_and_replays_to_the_same_game(
        ticks in prop::collection::vec(prop::collection::vec(command(), 0..10), 0..8),
    ) {
        let granted = Permissions::NONE
            .with(Permission::Move)
            .with(Permission::Give)
            .with(Permission::Build)
            .with(Permission::Spawn);
        let mut world = floor();
        let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let mut inventory = fuelled(FUEL);
        let mut stats = Stats::default();
        let mut mobs = Mobs::default();
        let mut next_mob = MobId::FIRST;
        let mut log = EventLog::new();
        let mut cooldowns = Cooldowns::default();
        for (index, commands) in ticks.iter().enumerate() {
            let mut stage = Stage {
                world: &mut world,
                player: &mut player,
                inventory: &mut inventory,
                stats: &mut stats,
                mobs: &mut mobs,
            };
            let mut pace = Pace {
                tick: Tick(index as u64 * 60),
                cooldowns: &mut cooldowns,
            };
            run(
                commands,
                granted,
                Limits::standard(),
                &mut pace,
                &mut stage,
                &mut log,
                &mut next_mob,
            );
        }

        // A restart: only the file survives, and the log is replayed onto the baseline.
        let saved = decode(&encode(117, &log)).unwrap();
        prop_assert_eq!(&saved.log, &log);
        let mut replayed_world = floor();
        let mut replayed_player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let mut replayed_inventory = fuelled(FUEL);
        let mut replayed_stats = Stats::default();
        let mut replayed_mobs = Mobs::default();
        for event in saved.log.iter() {
            let mut stage = Stage {
                world: &mut replayed_world,
                player: &mut replayed_player,
                inventory: &mut replayed_inventory,
                stats: &mut replayed_stats,
                mobs: &mut replayed_mobs,
            };
            prop_assert_eq!(apply(&mut stage, event), Ok(()));
        }

        prop_assert_eq!(replayed_world, world);
        prop_assert_eq!(replayed_player, player);
        prop_assert_eq!(replayed_inventory, inventory);
        prop_assert_eq!(replayed_mobs, mobs);
    }
}

#[test]
fn a_script_can_find_a_block_and_then_build_next_to_it() {
    let mut world = floor();
    let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
    let (mut inventory, mut stats, mut mobs) = (fuelled(10), Stats::default(), Mobs::default());
    let mut log = EventLog::new();
    let mut next_mob = MobId::FIRST;
    let granted = Permissions::NONE
        .with(Permission::Look)
        .with(Permission::Build);
    let marker = Event::BlockSet {
        pos: cell(4, 1, 0),
        old: BlockId::AIR,
        new: BlockId::DIRT,
    };
    assert_eq!(world.apply(&marker), Ok(()));

    let mut stage = Stage {
        world: &mut world,
        player: &mut player,
        inventory: &mut inventory,
        stats: &mut stats,
        mobs: &mut mobs,
    };
    let answers = ask(
        &[Query::Find {
            block: BlockId::DIRT,
        }],
        granted,
        Limits::standard(),
        Facing::East,
        &stage,
    );
    let Ok(Answer::Found(Some(found))) = answers[0] else {
        panic!("the dirt block is in reach, got {answers:?}");
    };
    assert_eq!(found, cell(4, 1, 0));
    assert!(log.is_empty());

    let next_to = cell(found.x + 1, found.y, found.z);
    let mut cooldowns = Cooldowns::default();
    let mut pace = Pace {
        tick: Tick(0),
        cooldowns: &mut cooldowns,
    };
    let results = run(
        &[Command::SetBlock {
            pos: next_to,
            block: BlockId::STONE,
        }],
        granted,
        Limits::standard(),
        &mut pace,
        &mut stage,
        &mut log,
        &mut next_mob,
    );
    assert_eq!(results, [Ok(())]);
    // The fuel the command burned, then the block it set.
    assert_eq!(log.len(), 2);
}
