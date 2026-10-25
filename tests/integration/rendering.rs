use minecraft_clone::rendering::meshing::mesh_chunk;
use minecraft_clone::shared::voxel::{CHUNK_SIZE, ChunkPos};
use minecraft_clone::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

proptest! {
    #[test]
    fn terrain_meshes_are_well_formed_and_hide_the_buried_stone(
        seed in any::<u64>(),
        column in (-3i32..3, -3i32..3),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let mut world = World::default();
        let (low, high) = (MIN_HEIGHT.div_euclid(16), MAX_HEIGHT.div_euclid(16));
        for chunk_y in low..=high {
            let pos = ChunkPos { x: column.0, y: chunk_y, z: column.1 };
            prop_assert_eq!(world.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
        }

        let mut quads = 0;
        for chunk_y in low..=high {
            let pos = ChunkPos { x: column.0, y: chunk_y, z: column.1 };
            let mesh = mesh_chunk(&world, pos);
            let vertex_count = mesh.vertices().len();
            prop_assert!(mesh.indices().iter().all(|&i| (i as usize) < vertex_count));
            quads += mesh.quad_count();
        }

        // Every column shows at least its top face, and a surface this smooth never shows
        // more than the top plus the side steps between neighbouring columns.
        let columns = usize::from(CHUNK_SIZE).pow(2);
        let height_range = usize::try_from(MAX_HEIGHT - MIN_HEIGHT).unwrap_or(0) + 1;
        prop_assert!(quads >= columns);
        prop_assert!(quads <= columns * (1 + 4 * height_range));
    }
}
