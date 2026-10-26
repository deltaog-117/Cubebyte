//! Hot path: one player step, run once per simulation tick.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::{Vec2, Vec3};
use minecraft_clone::player::model::Player;
use minecraft_clone::player::service::{Intent, step};
use minecraft_clone::shared::voxel::ChunkPos;
use minecraft_clone::world::generation::TerrainGenerator;
use minecraft_clone::world::model::World;

fn walking_over_terrain(c: &mut Criterion) {
    let terrain = TerrainGenerator::new(117);
    let mut world = World::default();
    for y in -2..4 {
        let pos = ChunkPos { x: 0, y, z: 0 };
        let inserted = world.insert_generated(pos, terrain.generate_chunk(pos));
        assert_eq!(
            inserted,
            Ok(()),
            "each chunk is inserted once into an empty world"
        );
    }
    let spawn = Vec3::new(8.5, terrain.height_at(8, 8) as f32 + 1.0, 8.5);
    let spawned = Player::try_new(spawn);
    assert!(spawned.is_ok(), "the spawn point is finite");
    let Ok(mut player) = spawned else {
        return;
    };
    let intent = Intent {
        walk: Vec2::new(0.6, 0.8),
        jump: false,
    };
    c.bench_function("player_step_walking", |b| {
        b.iter(|| step(black_box(&mut player), intent, black_box(&world), 0.05));
    });
}

criterion_group!(benches, walking_over_terrain);
criterion_main!(benches);
