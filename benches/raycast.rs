//! Hot path: casting a ray at full reach, once per click.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use minecraft_clone::building::raycast::cast;
use minecraft_clone::building::service::REACH;
use minecraft_clone::world::model::World;

fn cast_into_empty_world(c: &mut Criterion) {
    // The worst case: nothing to hit, so the walk covers the whole reach.
    let world = World::default();
    let origin = Vec3::new(0.3, 40.2, -0.7);
    let direction = Vec3::new(0.4, -0.5, 0.75);
    c.bench_function("raycast_full_reach_miss", |b| {
        b.iter(|| {
            cast(
                black_box(&world),
                black_box(origin),
                black_box(direction),
                REACH,
            )
        });
    });
}

criterion_group!(benches, cast_into_empty_world);
criterion_main!(benches);
