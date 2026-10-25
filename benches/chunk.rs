//! Hot paths: reading every cell of a chunk, and applying place/break events.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
use minecraft_clone::world::model::{Chunk, World};

fn chunk_get(c: &mut Criterion) {
    let chunk = Chunk::default();
    let positions: Vec<_> = (0..16)
        .flat_map(|x| {
            (0..16).flat_map(move |y| (0..16).map(move |z| WorldPos { x, y, z }.split().1))
        })
        .collect();
    c.bench_function("chunk_get_all_cells", |b| {
        b.iter(|| {
            positions
                .iter()
                .map(|p| chunk.get(*p).0 as u32)
                .sum::<u32>()
        });
    });
}

fn world_apply(c: &mut Criterion) {
    let pos = WorldPos { x: 5, y: 5, z: 5 };
    let place = Event::BlockSet {
        pos,
        old: BlockId::AIR,
        new: BlockId(1),
    };
    let remove = place.inverse();
    let mut world = World::default();
    c.bench_function("world_apply_place_then_remove", |b| {
        b.iter(|| {
            let _ = black_box(world.apply(&place));
            let _ = black_box(world.apply(&remove));
        });
    });
}

criterion_group!(benches, chunk_get, world_apply);
criterion_main!(benches);
