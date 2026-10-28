use glam::Vec3;
use minecraft_clone::events::apply::{Stage, apply};
use minecraft_clone::events::log::EventLog;
use minecraft_clone::events::persist::{decode, encode};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::player::model::Player;
use minecraft_clone::scripting::model::{Command, Limits, Permission, Permissions};
use minecraft_clone::scripting::service::run;
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
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
            .with(Permission::Build);
        let mut world = floor();
        let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let mut inventory = Inventory::default();
        let mut log = EventLog::new();
        for commands in &ticks {
            let mut stage = Stage {
                world: &mut world,
                player: &mut player,
                inventory: &mut inventory,
            };
            run(commands, granted, Limits::standard(), &mut stage, &mut log);
        }

        // A restart: only the file survives, and the log is replayed onto the baseline.
        let saved = decode(&encode(117, &log)).unwrap();
        prop_assert_eq!(&saved.log, &log);
        let mut replayed_world = floor();
        let mut replayed_player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let mut replayed_inventory = Inventory::default();
        for event in saved.log.iter() {
            let mut stage = Stage {
                world: &mut replayed_world,
                player: &mut replayed_player,
                inventory: &mut replayed_inventory,
            };
            prop_assert_eq!(apply(&mut stage, event), Ok(()));
        }

        prop_assert_eq!(replayed_world, world);
        prop_assert_eq!(replayed_player, player);
        prop_assert_eq!(replayed_inventory, inventory);
    }
}
