use glam::Vec3;
use minecraft_clone::building::model::SolidBlock;
use minecraft_clone::events::log::EventLog;
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::inventory::persist::{decode, encode};
use minecraft_clone::inventory::service::{gather_block, place_from_stock};
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::shared::voxel::ChunkPos;
use minecraft_clone::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

fn terrain_world(terrain: &TerrainGenerator) -> World {
    let mut world = World::default();
    let (low, high) = (MIN_HEIGHT.div_euclid(16), MAX_HEIGHT.div_euclid(16));
    for x in -1..=1 {
        for z in -1..=1 {
            for y in low..=high {
                let pos = ChunkPos { x, y, z };
                assert_eq!(
                    world.insert_generated(pos, terrain.generate_chunk(pos)),
                    Ok(())
                );
            }
        }
    }
    world
}

proptest! {
    // Blocks are neither made nor lost: every block taken from the world is in the pack
    // or was put back, and the pack survives a save and load unchanged.
    #[test]
    fn gathering_and_placing_conserve_blocks(
        seed in any::<u64>(),
        edits in prop::collection::vec(
            (any::<bool>(), (-1.0f32..1.0, -1.0f32..0.0, -1.0f32..1.0)),
            0..60,
        ),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let mut world = terrain_world(&terrain);
        let mut log = EventLog::new();
        let mut inventory = Inventory::default();
        let eye = Vec3::new(8.0, terrain.height_at(8, 8) as f32 + 3.0, 8.0);
        let (mut gathered, mut placed) = (0u32, 0u32);
        for (placing, (x, y, z)) in edits {
            let direction = Vec3::new(x, y, z);
            if placing {
                if place_from_stock(&mut world, &mut log, &mut inventory, eye, direction, SolidBlock::STONE).is_ok() {
                    placed += 1;
                }
            } else if gather_block(&mut world, &mut log, &mut inventory, eye, direction).is_ok() {
                gathered += 1;
            }
        }

        let held: u32 = [ItemId::STONE, ItemId::DIRT, ItemId::GRASS]
            .into_iter()
            .map(|item| inventory.count_of(item))
            .sum();
        // Only stone is placed, so every other item gathered is still held.
        prop_assert_eq!(held + placed, gathered);
        prop_assert_eq!(log.len() as u32, gathered + placed);
        prop_assert_eq!(decode(&encode(&inventory)), Ok(inventory));
    }
}
