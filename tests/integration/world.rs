use minecraft_clone::events::log::EventLog;
use minecraft_clone::events::replay::replay_chunk;
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, ChunkPos, WorldPos};
use minecraft_clone::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

proptest! {
    #[test]
    fn replaying_the_log_rebuilds_the_same_world(
        edits in prop::collection::vec(
            ((-20i32..20, -20i32..20, -20i32..20), 0u16..4),
            0..100,
        ),
    ) {
        let mut live = World::default();
        let mut log = EventLog::new();
        for ((x, y, z), block) in edits {
            let pos = WorldPos { x, y, z };
            let event = Event::BlockSet { pos, old: live.block_at(pos), new: BlockId(block) };
            prop_assert_eq!(live.apply(&event), Ok(()));
            log.push(event);
        }

        let mut replayed = World::default();
        for event in log.iter() {
            prop_assert_eq!(replayed.apply(event), Ok(()));
        }
        prop_assert_eq!(live, replayed);
    }
}

proptest! {
    #[test]
    fn generated_terrain_has_one_grass_surface_per_column_across_chunk_borders(
        seed in any::<u64>(),
        column in (-40i32..40, -40i32..40),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let mut world = World::default();
        let (low, high) = (MIN_HEIGHT.div_euclid(16), MAX_HEIGHT.div_euclid(16));
        let (chunk_x, chunk_z) = (column.0.div_euclid(16), column.1.div_euclid(16));
        for chunk_y in low..=high {
            let pos = ChunkPos { x: chunk_x, y: chunk_y, z: chunk_z };
            prop_assert_eq!(world.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
        }

        let surface = terrain.height_at(i64::from(column.0), i64::from(column.1));
        let block = |y| world.block_at(WorldPos { x: column.0, y, z: column.1 });
        prop_assert_eq!(block(surface), BlockId::GRASS);
        prop_assert_eq!(block(surface + 1), BlockId::AIR);
        prop_assert_eq!(block(surface - 1), BlockId::DIRT);
        prop_assert_eq!(block(MIN_HEIGHT - 4), BlockId::STONE);
    }
}

proptest! {
    #[test]
    fn edits_survive_unloading_and_reloading_every_chunk(
        seed in any::<u64>(),
        edits in prop::collection::vec(((-20i32..20, 0i32..40, -20i32..20), 0u16..4), 0..60),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let (low, high) = (MIN_HEIGHT.div_euclid(16), MAX_HEIGHT.div_euclid(16));
        let columns = (-2..2).flat_map(|x| (-2..2).map(move |z| (x, z)));
        let chunks: Vec<ChunkPos> = columns
            .flat_map(|(x, z)| (low..=high).map(move |y| ChunkPos { x, y, z }))
            .collect();

        let mut live = World::default();
        let mut log = EventLog::new();
        for &pos in &chunks {
            prop_assert_eq!(live.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
        }
        for ((x, y, z), block) in edits {
            let pos = WorldPos { x, y, z };
            let event = Event::BlockSet { pos, old: live.block_at(pos), new: BlockId(block) };
            prop_assert_eq!(live.apply(&event), Ok(()));
            log.push(event);
        }

        let mut streamed = live.clone();
        for &pos in &chunks {
            streamed.unload(pos);
        }
        prop_assert_eq!(streamed.chunk_count(), 0);
        for &pos in &chunks {
            prop_assert_eq!(streamed.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
            prop_assert_eq!(replay_chunk(&mut streamed, &log, pos), Ok(()));
        }
        prop_assert_eq!(live, streamed);
    }
}
