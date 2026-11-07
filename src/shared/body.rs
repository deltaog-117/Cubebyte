//! A walking box: gravity, jumping and collision with solid cells, one tick at a time.
//!
//! Players and mobs move with this one set of rules. A body knows nothing about worlds: the
//! caller says what is solid through a [`Terrain`], in tiny cubes a quarter of a block wide so
//! a moulded block can be walked on and bumped into by its real shape.
//!
//! Only `+ - * /` and comparisons touch positions, with no trigonometry, and nothing reads
//! wall-clock time, so the same start and the same [`Intent`]s always give the same result.

use glam::Vec3;
use thiserror::Error;

pub use crate::shared::intent::Intent;
use crate::shared::voxel::{TINY_SIDE, tiny_bit};

/// Tiny cubes along one side of a block, as a [`Terrain`] sees the world.
pub const FINE: i32 = TINY_SIDE;

/// What a body bumps into.
pub trait Terrain {
    /// The tiny cubes of the block at `cell` (in whole blocks) that are solid, one bit each as
    /// [`tiny_bit`] numbers them: all 64 for a whole block, none for an open one. A body asks
    /// once per block instead of once per tiny cube.
    fn solid_bits(&self, cell: [i32; 3]) -> u64;
}

/// A closure that says which whole blocks are solid is a terrain of whole blocks only.
impl<F: Fn([i32; 3]) -> bool> Terrain for F {
    fn solid_bits(&self, cell: [i32; 3]) -> u64 {
        if self(cell) { u64::MAX } else { 0 }
    }
}

/// Blocks per second on level ground.
pub const WALK_SPEED: f32 = 4.3;

/// Blocks per second squared.
pub const GRAVITY: f32 = 28.0;

/// Upward speed of a jump: about 1.4 blocks of height, enough to climb one block.
pub const JUMP_SPEED: f32 = 9.0;

/// Fastest fall, in blocks per second. At 20 steps per second it is 2.5 blocks a step.
pub const TERMINAL_SPEED: f32 = 50.0;

/// Gap kept between the box and a block it stops against.
///
/// Rounding can place the box a hair inside the face it was stopped at. Inside a block, the
/// next sweep would no longer see it and the body would fall through. Stopping short by
/// more than the rounding error rules that out.
pub const SKIN: f32 = 0.005;

/// Beyond this distance from the origin `f32` can no longer resolve [`SKIN`], so the body
/// stops moving instead of risking a fall through the floor.
const COORD_LIMIT: f32 = 8192.0;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BodyError {
    #[error("the body position must be finite")]
    NonFinite,
    #[error("a body must be wider and taller than zero, and finite")]
    BadSize,
}

/// A box standing at `position` (the middle of its feet) with a velocity.
///
/// The fields are private, so only [`step`] moves a body and its position and size are
/// always finite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    position: Vec3,
    /// Where the body stood before the latest step, so a view can blend between the two.
    previous: Vec3,
    /// Blocks per second.
    velocity: Vec3,
    on_ground: bool,
    half_width: f32,
    height: f32,
}

impl Body {
    /// A body standing still at `position`, with nothing known to be under the feet yet. The
    /// box is `2 * half_width` wide and deep and `height` tall.
    ///
    /// # Errors
    ///
    /// Returns [`BodyError::NonFinite`] if any coordinate is NaN or infinite, and
    /// [`BodyError::BadSize`] if a size is not finite and above zero.
    pub fn try_new(position: Vec3, half_width: f32, height: f32) -> Result<Self, BodyError> {
        if !position.is_finite() {
            return Err(BodyError::NonFinite);
        }
        if !(half_width.is_finite() && half_width > 0.0 && height.is_finite() && height > 0.0) {
            return Err(BodyError::BadSize);
        }
        Ok(Self {
            position,
            previous: position,
            velocity: Vec3::ZERO,
            on_ground: false,
            half_width,
            height,
        })
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Jumps to `position`, standing still. The previous position moves too, so a view does
    /// not blend across the gap, and velocity is dropped so the body does not keep falling or
    /// sliding from where it was.
    ///
    /// # Errors
    ///
    /// Returns [`BodyError::NonFinite`] if any coordinate is NaN or infinite; the body is left
    /// where it was.
    pub fn place(&mut self, position: Vec3) -> Result<(), BodyError> {
        if !position.is_finite() {
            return Err(BodyError::NonFinite);
        }
        self.position = position;
        self.previous = position;
        self.velocity = Vec3::ZERO;
        self.on_ground = false;
        Ok(())
    }

    pub fn velocity(&self) -> Vec3 {
        self.velocity
    }

    /// Whether the last step ended pressed against the floor, which is what allows a jump.
    pub fn on_ground(&self) -> bool {
        self.on_ground
    }

    /// The feet, `alpha` of the way from the previous step to the latest one.
    ///
    /// Blending keeps a view smooth at the slower simulation rate.
    pub fn blended(&self, alpha: f32) -> Vec3 {
        self.previous.lerp(self.position, alpha.clamp(0.0, 1.0))
    }

    /// Opposite corners of the box.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.bounds_at(self.position)
    }

    /// Opposite corners of the box if its feet were at `feet`.
    pub fn bounds_at(&self, feet: Vec3) -> (Vec3, Vec3) {
        let half = Vec3::new(self.half_width, 0.0, self.half_width);
        (feet - half, feet + half + Vec3::Y * self.height)
    }
}

/// Moves `body` through one tick of `dt` seconds over `terrain`.
///
/// Gravity pulls first, then the box moves along Y, X and Z in turn and is stopped by solid
/// cells, so it slides along walls. A jump starts only from the ground. A box that already
/// overlaps a cell, for example one placed on top of the body, can still move out of it.
pub fn step(body: &mut Body, intent: Intent, terrain: &impl Terrain, dt: f32) {
    body.previous = body.position;
    if !(dt.is_finite() && dt > 0.0) || body.position.abs().max_element() > COORD_LIMIT {
        return;
    }

    // A diagonal of whole axes can come out a hair above 1, and must not be faster.
    let walk = intent.walk.direction().clamp_length_max(1.0);
    let mut velocity = body.velocity;
    velocity.x = walk.x * WALK_SPEED;
    velocity.z = walk.y * WALK_SPEED;
    if intent.jump && body.on_ground {
        velocity.y = JUMP_SPEED;
    }
    velocity.y = (velocity.y - GRAVITY * dt).max(-TERMINAL_SPEED);

    body.on_ground = false;
    let mut cache = BlockCache::new(terrain);
    for axis in [1, 0, 2] {
        let wanted = velocity[axis] * dt;
        let allowed = clip(&mut cache, body, axis, wanted);
        body.position[axis] += allowed;
        if allowed != wanted {
            velocity[axis] = 0.0;
            body.on_ground |= axis == 1 && wanted < 0.0;
        }
    }
    body.velocity = velocity;
}

/// Whether a solid tiny cube lies in the box of tiny cubes from `low` to `high`, both ends
/// included. It asks the terrain once per block, and looks at single tiny cubes only inside a
/// block that is partly carved. A block with every tiny cube solid counts unless `skip_whole`
/// says to leave it out.
fn solid_in<T: Terrain>(
    cache: &mut BlockCache<'_, T>,
    low: [i32; 3],
    high: [i32; 3],
    skip_whole: impl Fn([i32; 3]) -> bool,
) -> bool {
    let block_range = |axis: usize| low[axis].div_euclid(FINE)..=high[axis].div_euclid(FINE);
    for bx in block_range(0) {
        for by in block_range(1) {
            for bz in block_range(2) {
                let block = [bx, by, bz];
                let found = match cache.bits(block) {
                    0 => false,
                    u64::MAX => !skip_whole(block),
                    bits => bits & tiny_cubes_in(block, low, high) != 0,
                };
                if found {
                    return true;
                }
            }
        }
    }
    false
}

/// The blocks of a terrain a step has already asked about, so the same block is not asked
/// about again for every tiny cube and every axis. It is small and fixed, so it never allocates;
/// a block that shares a slot with another is simply asked about again.
struct BlockCache<'a, T: Terrain> {
    terrain: &'a T,
    slots: [Option<([i32; 3], u64)>; 16],
}

impl<'a, T: Terrain> BlockCache<'a, T> {
    fn new(terrain: &'a T) -> Self {
        Self {
            terrain,
            slots: [None; 16],
        }
    }

    fn bits(&mut self, block: [i32; 3]) -> u64 {
        let hash = block[0]
            .wrapping_mul(3)
            .wrapping_add(block[1].wrapping_mul(5))
            .wrapping_add(block[2].wrapping_mul(7));
        let slot = &mut self.slots[hash.rem_euclid(16) as usize];
        match *slot {
            Some((seen, bits)) if seen == block => bits,
            _ => {
                let bits = self.terrain.solid_bits(block);
                *slot = Some((block, bits));
                bits
            }
        }
    }
}

/// The tiny cubes of `block` that lie inside the box from `low` to `high`, as a mask.
fn tiny_cubes_in(block: [i32; 3], low: [i32; 3], high: [i32; 3]) -> u64 {
    let span = |axis: usize| {
        let first = (low[axis] - block[axis] * FINE).max(0);
        let last = (high[axis] - block[axis] * FINE).min(FINE - 1);
        first..=last
    };
    let mut mask = 0;
    for x in span(0) {
        for y in span(1) {
            for z in span(2) {
                mask |= tiny_bit(x, y, z).unwrap_or(0);
            }
        }
    }
    mask
}

/// The tiny cubes a span from `low` to `high` overlaps, as a first and a last index.
fn fine_span(low: f32, high: f32) -> (i32, i32) {
    let cells = cell_range(low, high);
    (*cells.start(), *cells.end())
}

/// Whether the body's box overlaps a solid tiny cube.
pub fn collides(body: &Body, terrain: &impl Terrain) -> bool {
    let (min, max) = scaled(body.bounds());
    let spans = [0, 1, 2].map(|axis| fine_span(min[axis], max[axis]));
    let low = spans.map(|(first, _)| first);
    let high = spans.map(|(_, last)| last);
    solid_in(&mut BlockCache::new(terrain), low, high, |_| false)
}

/// Opposite corners of a box, in tiny cubes instead of blocks. Multiplying by a power of two
/// is exact, so everything below gives the same numbers as working in whole blocks.
fn scaled((low, high): (Vec3, Vec3)) -> (Vec3, Vec3) {
    (low * FINE as f32, high * FINE as f32)
}

/// How far the box can move along `axis` towards `delta` before touching a solid tiny cube,
/// never more than `delta` and never in the opposite direction.
///
/// A tiny cube inside a whole block that the box already overlaps along `axis` is ignored,
/// so a box that was put inside a block can still walk out of it, as before moulding existed.
fn clip<T: Terrain>(cache: &mut BlockCache<'_, T>, body: &Body, axis: usize, delta: f32) -> f32 {
    let (block_min, block_max) = body.bounds();
    let (min, max) = scaled((block_min, block_max));
    let (delta, skin) = (delta * FINE as f32, SKIN * FINE as f32);
    let (first, second) = ((axis + 1) % 3, (axis + 2) % 3);
    // The first block along the axis the box has not reached yet, going up, and the last one
    // it has left behind, going down: whole blocks beyond these are real obstacles.
    let (not_reached, left_behind) = (
        block_max[axis].ceil() as i32,
        block_min[axis].floor() as i32 - 1,
    );
    let across = [first, second].map(|axis| fine_span(min[axis], max[axis]));
    let mut blocked = |cell: i32| {
        let mut low = [0; 3];
        let mut high = [0; 3];
        low[axis] = cell;
        high[axis] = cell;
        low[first] = across[0].0;
        high[first] = across[0].1;
        low[second] = across[1].0;
        high[second] = across[1].1;
        solid_in(cache, low, high, |block| {
            if delta > 0.0 {
                block[axis] < not_reached
            } else {
                block[axis] > left_behind
            }
        })
    };

    let allowed = if delta > 0.0 {
        // Cells whose near face lies at or beyond the front of the box.
        let mut cells = max[axis].ceil() as i32..=(max[axis] + delta).floor() as i32;
        cells.find(|&cell| blocked(cell)).map_or(delta, |cell| {
            (cell as f32 - max[axis] - skin).clamp(0.0, delta)
        })
    } else if delta < 0.0 {
        // Cells whose near face lies at or behind the back of the box, nearest first.
        let mut cells =
            ((min[axis] + delta).ceil() as i32 - 1..=min[axis].floor() as i32 - 1).rev();
        cells.find(|&cell| blocked(cell)).map_or(delta, |cell| {
            ((cell + 1) as f32 - min[axis] + skin).clamp(delta, 0.0)
        })
    } else {
        0.0
    };
    allowed / FINE as f32
}

/// The cells a span from `low` to `high` overlaps. A span that only touches a cell's face
/// does not overlap it.
fn cell_range(low: f32, high: f32) -> std::ops::RangeInclusive<i32> {
    low.floor() as i32..=high.ceil() as i32 - 1
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::intent::{AXIS_MAX, Walk};

    const DT: f32 = 0.05;

    /// A floor whose top face is at y = 1, with one pillar of height 2 at x = 3.
    fn solid(cell: [i32; 3]) -> bool {
        cell[1] == 0 || (cell[0] == 3 && cell[1] < 3)
    }

    fn body(half_width: f32, height: f32, x: f32, y: f32) -> Body {
        Body::try_new(Vec3::new(x, y, 0.5), half_width, height).unwrap()
    }

    #[test]
    fn a_bad_position_or_size_is_rejected() {
        let at = Vec3::ZERO;
        assert_eq!(
            Body::try_new(Vec3::new(f32::NAN, 0.0, 0.0), 0.3, 1.8),
            Err(BodyError::NonFinite)
        );
        for (half_width, height) in [
            (0.0, 1.8),
            (-0.3, 1.8),
            (0.3, 0.0),
            (f32::NAN, 1.8),
            (0.3, f32::INFINITY),
        ] {
            assert_eq!(
                Body::try_new(at, half_width, height),
                Err(BodyError::BadSize)
            );
        }
    }

    #[test]
    fn placing_a_body_moves_it_without_a_trail_or_leftover_speed() {
        let mut falling = body(0.3, 1.8, 6.0, 5.0);
        step(&mut falling, Intent::default(), &solid, DT);
        assert_ne!(falling.velocity(), Vec3::ZERO);

        let to = Vec3::new(40.0, 9.0, -7.5);
        assert_eq!(falling.place(to), Ok(()));

        assert_eq!(falling.position(), to);
        assert_eq!(falling.blended(0.0), to);
        assert_eq!(falling.velocity(), Vec3::ZERO);
        assert!(!falling.on_ground());
    }

    #[test]
    fn placing_a_body_at_a_non_finite_position_changes_nothing() {
        let mut standing = body(0.3, 1.8, 4.0, 3.0);
        let before = standing;
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                standing.place(Vec3::new(0.0, bad, 0.0)),
                Err(BodyError::NonFinite)
            );
        }
        assert_eq!(standing, before);
    }

    #[test]
    fn a_wide_low_body_lands_and_a_narrow_tall_one_stops_at_the_wall() {
        let mut crawler = body(0.45, 0.9, 0.5, 6.0);
        for _ in 0..100 {
            step(&mut crawler, Intent::default(), &solid, DT);
        }
        assert!(crawler.on_ground());
        assert!((crawler.position().y - 1.0).abs() <= SKIN + 1e-4);

        let mut tall = body(0.2, 2.4, 0.5, 1.0);
        let east = Intent {
            walk: Walk::new(AXIS_MAX, 0),
            jump: false,
        };
        for _ in 0..60 {
            step(&mut tall, east, &solid, DT);
        }
        // The pillar's west face is at x = 3; the box is 0.2 wide on each side.
        assert!(tall.position().x <= 3.0 - 0.2);
        assert!(tall.position().x > 3.0 - 0.2 - 0.05);
    }

    #[test]
    fn the_blend_runs_from_the_previous_step_to_the_latest() {
        let mut walker = body(0.3, 1.8, 0.5, 1.0);
        step(&mut walker, Intent::default(), &solid, DT);
        step(
            &mut walker,
            Intent {
                walk: Walk::new(AXIS_MAX, 0),
                jump: false,
            },
            &solid,
            DT,
        );
        let (start, end) = (walker.blended(0.0).x, walker.blended(1.0).x);
        assert!(end > start);
        assert!((walker.blended(0.5).x - (start + end) / 2.0).abs() < 1e-5);
        assert_eq!(walker.blended(2.0), walker.blended(1.0));
    }

    proptest! {
        #[test]
        fn a_body_of_any_size_never_ends_a_step_inside_a_block(
            half_width in 0.1f32..0.6,
            height in 0.5f32..2.5,
            moves in prop::collection::vec((-AXIS_MAX..=AXIS_MAX, -AXIS_MAX..=AXIS_MAX, any::<bool>()), 1..80),
        ) {
            let mut walker = body(half_width, height, 0.5, 1.0);
            for (x, z, jump) in moves {
                step(&mut walker, Intent { walk: Walk::new(x, z), jump }, &solid, DT);
                prop_assert!(!collides(&walker, &solid));
            }
        }

        #[test]
        fn the_same_intents_give_the_same_body(
            moves in prop::collection::vec((-AXIS_MAX..=AXIS_MAX, -AXIS_MAX..=AXIS_MAX, any::<bool>()), 0..60),
        ) {
            let run = || {
                let mut walker = body(0.3, 1.8, 0.5, 1.0);
                for &(x, z, jump) in &moves {
                    step(&mut walker, Intent { walk: Walk::new(x, z), jump }, &solid, DT);
                }
                walker
            };
            prop_assert_eq!(run(), run());
        }
    }
}
