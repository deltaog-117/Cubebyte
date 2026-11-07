//! Hot path: meshing one chunk of terrain, at the surface and in the sky.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use minecraft_clone::rendering::meshing::mesh_chunk;
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, ChunkPos, WorldPos};
use minecraft_clone::world::generation::TerrainGenerator;
use minecraft_clone::world::model::World;

fn mesh_terrain(c: &mut Criterion) {
    let terrain = TerrainGenerator::new(42);
    let mut world = World::default();
    // The 3x3x3 block around the surface chunk, so border faces hit loaded neighbours.
    for x in 6..=8 {
        for y in -1..=1 {
            for z in -4..=-2 {
                let pos = ChunkPos { x, y, z };
                let inserted = world.insert_generated(pos, terrain.generate_chunk(pos));
                assert_eq!(
                    inserted,
                    Ok(()),
                    "each chunk is inserted once into an empty world"
                );
            }
        }
    }
    let surface = ChunkPos { x: 7, y: 0, z: -3 };
    let sky = ChunkPos { x: 7, y: 8, z: -3 };
    c.bench_function("mesh_surface_chunk", |b| {
        b.iter(|| mesh_chunk(black_box(&world), black_box(surface)));
    });
    c.bench_function("mesh_sky_chunk", |b| {
        b.iter(|| mesh_chunk(black_box(&world), black_box(sky)));
    });
}

/// A chunk whose every block is moulded, each with a different quarter of its tiny cubes gone:
/// the worst case for the moulded pass, far beyond what play produces.
fn mesh_moulded(c: &mut Criterion) {
    let mut world = World::default();
    for x in 0..16 {
        for y in 0..16 {
            for z in 0..16 {
                let bits = 0x0123_4567_89AB_CDEF_u64.rotate_left((x + y + z) as u32) | 1;
                let Ok(id) = world.moulds_mut().id_for(BlockId::STONE, bits) else {
                    return;
                };
                let pos = WorldPos { x, y, z };
                let event = Event::BlockSet {
                    pos,
                    old: BlockId::AIR,
                    new: id,
                };
                if world.apply(&event).is_err() {
                    return;
                }
            }
        }
    }
    let chunk = ChunkPos { x: 0, y: 0, z: 0 };
    c.bench_function("mesh_fully_moulded_chunk", |b| {
        b.iter(|| mesh_chunk(black_box(&world), black_box(chunk)));
    });
}

criterion_group!(benches, mesh_terrain, mesh_moulded);
criterion_main!(benches);
