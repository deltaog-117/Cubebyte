//! Hot path: terrain generation will draw millions of numbers.

use std::hint::black_box;
use std::num::NonZeroU64;

use criterion::{Criterion, criterion_group, criterion_main};
use minecraft_clone::shared::rng::Rng;

fn rng_next_u64(c: &mut Criterion) {
    let mut rng = Rng::from_seed(1);
    c.bench_function("rng_next_u64_x1000", |b| {
        b.iter(|| {
            let mut acc = 0u64;
            for _ in 0..1000 {
                acc ^= rng.next_u64();
            }
            black_box(acc)
        });
    });
}

fn rng_below(c: &mut Criterion) {
    let mut rng = Rng::from_seed(1);
    let bound = NonZeroU64::MIN.saturating_add(5);
    c.bench_function("rng_below_6_x1000", |b| {
        b.iter(|| {
            let mut acc = 0u64;
            for _ in 0..1000 {
                acc ^= rng.below(bound);
            }
            black_box(acc)
        });
    });
}

criterion_group!(benches, rng_next_u64, rng_below);
criterion_main!(benches);
