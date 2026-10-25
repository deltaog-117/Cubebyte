//! Decides which chunks to load and unload around a centre chunk.
//!
//! It only tracks positions. Generating, replaying and meshing are the caller's job, so
//! this stays free of terrain, events and rendering.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use crate::shared::voxel::ChunkPos;

/// Extra chunks past the load radius before a chunk unloads, so a camera hovering on a
/// border does not load and unload the same chunks every frame.
const UNLOAD_MARGIN: i32 = 1;

/// What to change to match a new centre.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StreamPlan {
    /// Nearest first, so the ground under the player appears before the horizon.
    pub load: Vec<ChunkPos>,
    pub unload: Vec<ChunkPos>,
}

/// The set of chunks currently loaded around a moving centre.
///
/// Only a column of chunks over `y_range` is kept for each horizontal position, since
/// terrain has no blocks outside it.
#[derive(Debug)]
pub struct Streamer {
    load_radius: i32,
    y_range: RangeInclusive<i32>,
    loaded: HashSet<ChunkPos>,
}

impl Streamer {
    pub fn new(load_radius: u8, y_range: RangeInclusive<i32>) -> Self {
        Self {
            load_radius: i32::from(load_radius),
            y_range,
            loaded: HashSet::new(),
        }
    }

    pub fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.loaded.contains(&pos)
    }

    /// What to load and unload now that the centre is at horizontal chunk `(x, z)`.
    ///
    /// Distance is square: the largest of the two horizontal chunk offsets.
    pub fn plan(&self, centre_x: i32, centre_z: i32) -> StreamPlan {
        let distance = |pos: ChunkPos| {
            (pos.x.saturating_sub(centre_x))
                .abs()
                .max((pos.z.saturating_sub(centre_z)).abs())
        };
        let r = self.load_radius;
        let mut load: Vec<ChunkPos> = (centre_x.saturating_sub(r)..=centre_x.saturating_add(r))
            .flat_map(|x| {
                (centre_z.saturating_sub(r)..=centre_z.saturating_add(r))
                    .flat_map(move |z| self.y_range.clone().map(move |y| ChunkPos { x, y, z }))
            })
            .filter(|pos| !self.loaded.contains(pos))
            .collect();
        load.sort_by_key(|&pos| (distance(pos), pos.x, pos.z, pos.y));

        let mut unload: Vec<ChunkPos> = self
            .loaded
            .iter()
            .copied()
            .filter(|&pos| distance(pos) > r + UNLOAD_MARGIN)
            .collect();
        unload.sort_by_key(|pos| (pos.x, pos.z, pos.y));
        StreamPlan { load, unload }
    }

    pub fn mark_loaded(&mut self, pos: ChunkPos) {
        self.loaded.insert(pos);
    }

    pub fn mark_unloaded(&mut self, pos: ChunkPos) {
        self.loaded.remove(&pos);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn streamer(radius: u8) -> Streamer {
        Streamer::new(radius, -1..=2)
    }

    fn apply(streamer: &mut Streamer, plan: &StreamPlan) {
        plan.load.iter().for_each(|&pos| streamer.mark_loaded(pos));
        plan.unload
            .iter()
            .for_each(|&pos| streamer.mark_unloaded(pos));
    }

    #[test]
    fn the_first_plan_loads_a_full_square_of_columns() {
        let plan = streamer(2).plan(0, 0);

        assert_eq!(plan.load.len(), 5 * 5 * 4);
        assert!(plan.unload.is_empty());
    }

    #[test]
    fn the_nearest_chunks_load_first() {
        let plan = streamer(3).plan(10, -4);

        let near = plan.load.first().unwrap();
        assert_eq!((near.x, near.z), (10, -4));
    }

    #[test]
    fn a_chunk_just_past_the_load_radius_stays_loaded() {
        let mut streamer = streamer(1);
        streamer.mark_loaded(ChunkPos { x: 2, y: 0, z: 0 });

        assert!(streamer.plan(0, 0).unload.is_empty());
        assert_eq!(streamer.plan(-1, 0).unload.len(), 1);
    }

    proptest! {
        #[test]
        fn following_the_plans_settles_on_exactly_the_square_around_the_centre(
            radius in 0u8..4,
            path in prop::collection::vec((-6i32..6, -6i32..6), 1..8),
        ) {
            let mut streamer = streamer(radius);
            for &(x, z) in &path {
                let plan = streamer.plan(x, z);
                apply(&mut streamer, &plan);
            }
            let &(x, z) = path.last().unwrap();

            // Nothing is left to load, and everything within the radius is loaded.
            let plan = streamer.plan(x, z);
            prop_assert!(plan.load.is_empty());
            prop_assert!(plan.unload.is_empty());
            let r = i32::from(radius);
            for cx in x - r..=x + r {
                for cz in z - r..=z + r {
                    for y in -1..=2 {
                        let pos = ChunkPos { x: cx, y, z: cz };
                        prop_assert!(streamer.is_loaded(pos));
                    }
                }
            }
        }

        #[test]
        fn a_plan_never_loads_and_unloads_the_same_chunk(
            radius in 0u8..4,
            from in (-6i32..6, -6i32..6),
            to in (-6i32..6, -6i32..6),
        ) {
            let mut streamer = streamer(radius);
            let first = streamer.plan(from.0, from.1);
            apply(&mut streamer, &first);

            let plan = streamer.plan(to.0, to.1);

            prop_assert!(plan.load.iter().all(|pos| !plan.unload.contains(pos)));
            prop_assert!(plan.load.iter().all(|&pos| !streamer.is_loaded(pos)));
        }
    }
}
