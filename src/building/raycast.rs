//! Voxel ray cast: which block the viewer is looking at.
//!
//! Pure maths over [`World::block_at`], so it runs and is tested without a window.

use glam::{IVec3, Vec3};

use crate::shared::voxel::WorldPos;
use crate::world::model::World;

/// Rays that start farther out than this are refused, so stepping from the start cell can
/// never overflow `i32`.
const COORD_LIMIT: f32 = 1_000_000.0;

/// The first solid block along a ray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayHit {
    pub block: WorldPos,
    /// The cell the ray crossed just before `block`, where a new block would go. `None`
    /// when the ray starts inside `block`, since no face was crossed.
    pub before: Option<WorldPos>,
}

/// Walks the cells along a ray (Amanatides-Woo) and returns the first solid block whose
/// entry point is within `reach`.
///
/// Returns `None` when nothing solid is in reach, and for a zero or non-finite
/// `direction`, a non-finite or negative `reach`, or an `origin` that is non-finite or
/// beyond one million blocks from the world origin. `direction` need not be normalized.
pub fn cast(world: &World, origin: Vec3, direction: Vec3, reach: f32) -> Option<RayHit> {
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO
        || !reach.is_finite()
        || reach < 0.0
        || !origin.is_finite()
        || origin.abs().max_element() > COORD_LIMIT
    {
        return None;
    }

    let mut cell = origin.floor().as_ivec3();
    let step = IVec3::new(sign(direction.x), sign(direction.y), sign(direction.z));
    // Distance along the ray between two cell borders on each axis; infinite when the ray
    // never moves on that axis, so that axis is never picked.
    let t_delta = direction.recip().abs();
    let mut t_max = Vec3::from_array(std::array::from_fn(|axis| {
        let to_border = match step[axis] {
            1 => (cell[axis] + 1) as f32 - origin[axis],
            -1 => origin[axis] - cell[axis] as f32,
            _ => return f32::INFINITY,
        };
        to_border * t_delta[axis]
    }));

    let mut before = None;
    loop {
        if !world.block_at(world_pos(cell)).is_air() {
            return Some(RayHit {
                block: world_pos(cell),
                before: before.map(world_pos),
            });
        }
        let axis = nearest_axis(t_max);
        if t_max[axis] > reach {
            return None;
        }
        before = Some(cell);
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
    }
}

fn sign(value: f32) -> i32 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

fn nearest_axis(t: Vec3) -> usize {
    if t.x <= t.y && t.x <= t.z {
        0
    } else if t.y <= t.z {
        1
    } else {
        2
    }
}

fn world_pos(cell: IVec3) -> WorldPos {
    WorldPos {
        x: cell.x,
        y: cell.y,
        z: cell.z,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::event::Event;
    use crate::shared::voxel::BlockId;

    const REACH: f32 = 12.0;

    fn world_with(cells: &[IVec3]) -> World {
        let mut world = World::default();
        for &cell in cells {
            let pos = world_pos(cell);
            let event = Event::BlockSet {
                pos,
                old: world.block_at(pos),
                new: BlockId::STONE,
            };
            world.apply(&event).unwrap();
        }
        world
    }

    /// Distance at which the ray enters the unit cube at `cell`, found with the slab method
    /// so it shares no code with the cell walk.
    fn entry_distance(origin: Vec3, direction: Vec3, cell: IVec3) -> Option<f32> {
        let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
        for axis in 0..3 {
            let low = cell[axis] as f32;
            let high = low + 1.0;
            if direction[axis].abs() < 1e-6 {
                if origin[axis] < low || origin[axis] > high {
                    return None;
                }
                continue;
            }
            let a = (low - origin[axis]) / direction[axis];
            let b = (high - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        let near = near.max(0.0);
        (near <= far && near <= REACH).then_some(near)
    }

    #[test]
    fn a_ray_along_an_axis_hits_the_block_and_reports_the_cell_before_it() {
        let world = world_with(&[IVec3::new(3, 0, 0)]);
        let hit = cast(&world, Vec3::new(0.5, 0.5, 0.5), Vec3::X, 5.0).unwrap();
        assert_eq!(hit.block, world_pos(IVec3::new(3, 0, 0)));
        assert_eq!(hit.before, Some(world_pos(IVec3::new(2, 0, 0))));
    }

    #[test]
    fn a_ray_looking_the_other_way_hits_the_far_face() {
        let world = world_with(&[IVec3::new(-3, 0, 0)]);
        let hit = cast(&world, Vec3::new(0.5, 0.5, 0.5), Vec3::NEG_X, 5.0).unwrap();
        assert_eq!(hit.block, world_pos(IVec3::new(-3, 0, 0)));
        assert_eq!(hit.before, Some(world_pos(IVec3::new(-2, 0, 0))));
    }

    #[test]
    fn a_ray_that_starts_inside_a_block_has_no_cell_before_it() {
        let world = world_with(&[IVec3::ZERO]);
        let hit = cast(&world, Vec3::splat(0.5), Vec3::X, 5.0).unwrap();
        assert_eq!(hit.block, world_pos(IVec3::ZERO));
        assert_eq!(hit.before, None);
    }

    #[test]
    fn a_block_just_past_the_reach_is_missed() {
        let world = world_with(&[IVec3::new(6, 0, 0)]);
        // The block's near face is 5.5 away from the origin.
        assert_eq!(cast(&world, Vec3::splat(0.5), Vec3::X, 5.0), None);
        assert!(cast(&world, Vec3::splat(0.5), Vec3::X, 5.6).is_some());
    }

    #[test]
    fn an_empty_world_has_nothing_to_hit() {
        assert_eq!(cast(&World::default(), Vec3::ZERO, Vec3::ONE, REACH), None);
    }

    #[test]
    fn unusable_input_hits_nothing() {
        let world = world_with(&[IVec3::new(1, 0, 0)]);
        let origin = Vec3::splat(0.5);
        assert_eq!(cast(&world, origin, Vec3::ZERO, REACH), None);
        assert_eq!(cast(&world, origin, Vec3::NAN, REACH), None);
        assert_eq!(cast(&world, origin, Vec3::X, f32::NAN), None);
        assert_eq!(cast(&world, origin, Vec3::X, f32::INFINITY), None);
        assert_eq!(cast(&world, origin, Vec3::X, -1.0), None);
        assert_eq!(cast(&world, Vec3::NAN, Vec3::X, REACH), None);
        assert_eq!(cast(&world, Vec3::splat(1e30), Vec3::X, REACH), None);
    }

    fn scene() -> impl Strategy<Value = (Vec<IVec3>, Vec3, Vec3)> {
        let cell = (-4i32..4, -4i32..4, -4i32..4).prop_map(|(x, y, z)| IVec3::new(x, y, z));
        let component = -1.0f32..1.0;
        (
            prop::collection::vec(cell, 0..40),
            (-5.0f32..5.0, -5.0f32..5.0, -5.0f32..5.0).prop_map(|(x, y, z)| Vec3::new(x, y, z)),
            (component.clone(), component.clone(), component)
                .prop_map(|(x, y, z)| Vec3::new(x, y, z))
                .prop_filter("direction must be usable", |d| d.length() > 0.1),
        )
    }

    proptest! {
        #[test]
        fn the_cast_finds_the_nearest_solid_block_the_ray_enters(
            (cells, origin, direction) in scene(),
        ) {
            let world = world_with(&cells);
            let direction = direction.normalize();
            let expected = cells
                .iter()
                .filter_map(|&cell| entry_distance(origin, direction, cell).map(|d| (cell, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(cell, _)| world_pos(cell));
            prop_assert_eq!(cast(&world, origin, direction, REACH).map(|hit| hit.block), expected);
        }

        #[test]
        fn the_cell_before_a_hit_is_empty_and_touches_the_block_by_one_face(
            (cells, origin, direction) in scene(),
        ) {
            let world = world_with(&cells);
            if let Some(RayHit { block, before: Some(before) }) =
                cast(&world, origin, direction, REACH)
            {
                prop_assert!(world.block_at(block) != BlockId::AIR);
                prop_assert!(world.block_at(before).is_air());
                let apart = (block.x - before.x).abs()
                    + (block.y - before.y).abs()
                    + (block.z - before.z).abs();
                prop_assert_eq!(apart, 1);
            }
        }

        #[test]
        fn a_longer_reach_never_loses_a_hit(
            (cells, origin, direction) in scene(),
            reach in 0.0f32..12.0,
        ) {
            let world = world_with(&cells);
            if let Some(near) = cast(&world, origin, direction, reach) {
                prop_assert_eq!(cast(&world, origin, direction, REACH), Some(near));
            }
        }
    }
}
