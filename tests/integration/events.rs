use glam::Vec3;
use minecraft_clone::building::model::SolidBlock;
use minecraft_clone::building::service::{break_block, place_block};
use minecraft_clone::events::log::EventLog;
use minecraft_clone::events::persist::{decode, encode};
use minecraft_clone::events::replay::replay_chunk;
use minecraft_clone::shared::voxel::ChunkPos;
use minecraft_clone::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

fn chunk_range() -> impl Iterator<Item = ChunkPos> {
    let (low, high) = (MIN_HEIGHT.div_euclid(16), MAX_HEIGHT.div_euclid(16));
    (-1..=1).flat_map(move |x| {
        (-1..=1).flat_map(move |z| (low..=high).map(move |y| ChunkPos { x, y, z }))
    })
}

fn terrain_world(terrain: &TerrainGenerator) -> World {
    let mut world = World::default();
    for pos in chunk_range() {
        assert_eq!(
            world.insert_generated(pos, terrain.generate_chunk(pos)),
            Ok(())
        );
    }
    world
}

proptest! {
    #[test]
    fn a_saved_log_rebuilds_the_edited_world_chunk_by_chunk(
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

        // A restart: only the file survives. Streaming then replays each chunk on its own.
        let saved = decode(&encode(seed, &log)).unwrap();
        prop_assert_eq!(saved.seed, seed);
        let terrain = TerrainGenerator::new(saved.seed);
        let mut restored = World::default();
        for pos in chunk_range() {
            prop_assert_eq!(restored.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
            prop_assert_eq!(replay_chunk(&mut restored, &saved.log, pos), Ok(()));
        }
        prop_assert_eq!(restored, live);
    }
}
