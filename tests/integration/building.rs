use glam::Vec3;
use minecraft_clone::building::model::SolidBlock;
use minecraft_clone::building::service::{break_block, place_block};
use minecraft_clone::events::log::EventLog;
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
    #[test]
    fn edits_made_on_terrain_replay_onto_fresh_terrain(
        seed in any::<u64>(),
        edits in prop::collection::vec(
            (any::<bool>(), (-1.0f32..1.0, -1.0f32..0.0, -1.0f32..1.0)),
            0..40,
        ),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let mut live = terrain_world(&terrain);
        let mut log = EventLog::new();
        let eye = Vec3::new(8.0, terrain.height_at(8, 8) as f32 + 3.0, 8.0);
        for (placing, (x, y, z)) in edits {
            let direction = Vec3::new(x, y, z);
            let _ = if placing {
                place_block(&mut live, &mut log, eye, direction, SolidBlock::STONE)
            } else {
                break_block(&mut live, &mut log, eye, direction)
            };
        }

        // Terrain is a pure function of the seed, so the log alone carries the edits.
        let mut replayed = terrain_world(&terrain);
        for event in log.iter() {
            prop_assert_eq!(replayed.apply(event), Ok(()));
        }
        prop_assert_eq!(replayed, live);
    }
}
