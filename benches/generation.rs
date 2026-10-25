//! Hot path: generating one chunk of terrain, at the surface and in the sky.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use minecraft_clone::shared::voxel::ChunkPos;
use minecraft_clone::world::generation::TerrainGenerator;

fn generate_chunk(c: &mut Criterion) {
    let terrain = TerrainGenerator::new(42);
    let surface = ChunkPos { x: 7, y: 0, z: -3 };
    let sky = ChunkPos { x: 7, y: 8, z: -3 };
    c.bench_function("generate_surface_chunk", |b| {
        b.iter(|| terrain.generate_chunk(black_box(surface)));
    });
    c.bench_function("generate_sky_chunk", |b| {
        b.iter(|| terrain.generate_chunk(black_box(sky)));
    });
}

criterion_group!(benches, generate_chunk);
criterion_main!(benches);
