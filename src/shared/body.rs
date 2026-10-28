//! A walking box: gravity, jumping and collision with solid cells, one tick at a time.
//!
//! Players and mobs move with this one set of rules. A body knows nothing about worlds: the
//! caller says which cells are solid.
//!
//! Only `+ - * /` and comparisons touch positions, with no trigonometry, and nothing reads
//! wall-clock time, so the same start and the same [`Intent`]s always give the same result.

use glam::Vec3;
use thiserror::Error;

pub use crate::shared::intent::Intent;

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

/// Moves `body` through one tick of `dt` seconds. `solid` says whether the cell at the given
/// integer coordinates blocks movement.
///
/// Gravity pulls first, then the box moves along Y, X and Z in turn and is stopped by solid
/// cells, so it slides along walls. A jump starts only from the ground. A box that already
/// overlaps a cell, for example one placed on top of the body, can still move out of it.
pub fn step(body: &mut Body, intent: Intent, solid: &impl Fn([i32; 3]) -> bool, dt: f32) {
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
    for axis in [1, 0, 2] {
        let wanted = velocity[axis] * dt;
        let allowed = clip(solid, body, axis, wanted);
        body.position[axis] += allowed;
        if allowed != wanted {
            velocity[axis] = 0.0;
            body.on_ground |= axis == 1 && wanted < 0.0;
        }
    }
    body.velocity = velocity;
}

/// Whether the body's box overlaps a solid cell.
pub fn collides(body: &Body, solid: &impl Fn([i32; 3]) -> bool) -> bool {
    let (min, max) = body.bounds();
    cell_range(min.x, max.x).any(|x| {
        cell_range(min.y, max.y).any(|y| cell_range(min.z, max.z).any(|z| solid([x, y, z])))
    })
}

/// How far the box can move along `axis` towards `delta` before touching a solid cell,
/// never more than `delta` and never in the opposite direction.
fn clip(solid: &impl Fn([i32; 3]) -> bool, body: &Body, axis: usize, delta: f32) -> f32 {
    let (min, max) = body.bounds();
    let (first, second) = ((axis + 1) % 3, (axis + 2) % 3);
    let blocked = |cell: i32| {
        cell_range(min[first], max[first]).any(|a| {
            cell_range(min[second], max[second]).any(|b| {
                let mut at = [0; 3];
                at[axis] = cell;
                at[first] = a;
                at[second] = b;
                solid(at)
            })
        })
    };

    if delta > 0.0 {
        // Cells whose near face lies at or beyond the front of the box.
        let mut cells = max[axis].ceil() as i32..=(max[axis] + delta).floor() as i32;
        cells.find(|&cell| blocked(cell)).map_or(delta, |cell| {
            (cell as f32 - max[axis] - SKIN).clamp(0.0, delta)
        })
    } else if delta < 0.0 {
        // Cells whose near face lies at or behind the back of the box, nearest first.
        let mut cells =
            ((min[axis] + delta).ceil() as i32 - 1..=min[axis].floor() as i32 - 1).rev();
        cells.find(|&cell| blocked(cell)).map_or(delta, |cell| {
            ((cell + 1) as f32 - min[axis] + SKIN).clamp(delta, 0.0)
        })
    } else {
        0.0
    }
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
