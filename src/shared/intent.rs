//! What an actor wants to do in one tick, as whole numbers.
//!
//! Walking direction is stored as two small integers, not floats, so the same intent is the
//! same bits on every machine and can be saved and replayed exactly. Whatever works out a
//! direction in floating point, such as a camera or a chase, rounds it here once, before the
//! body sees it.

use glam::Vec2;

/// Largest size of one walking axis. `-128` is not used, so left and right are symmetric.
pub const AXIS_MAX: i8 = 127;

/// A walking direction: `x` is right and `z` is forward, each from `-127` to `127`, where 127
/// means full speed along that axis.
///
/// The fields are private, so a value outside that range cannot exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Walk {
    x: i8,
    z: i8,
}

impl Walk {
    /// Standing still.
    pub const STILL: Self = Self { x: 0, z: 0 };

    /// A walk from raw axes. `-128` counts as `-127`.
    pub fn new(x: i8, z: i8) -> Self {
        Self {
            x: x.max(-AXIS_MAX),
            z: z.max(-AXIS_MAX),
        }
    }

    /// Rounds a wanted direction to the nearest whole axes.
    ///
    /// A length above 1 is cut to 1 first, and a non-finite direction counts as standing
    /// still. The rounding step is about 0.45 degrees of direction and under 0.4% of speed.
    pub fn from_direction(direction: Vec2) -> Self {
        if !direction.is_finite() {
            return Self::STILL;
        }
        let unit = direction.clamp_length_max(1.0) * f32::from(AXIS_MAX);
        Self::new(unit.x.round() as i8, unit.y.round() as i8)
    }

    pub const fn x(self) -> i8 {
        self.x
    }

    pub const fn z(self) -> i8 {
        self.z
    }

    /// The direction as x = right, y = forward. Its length can be a hair above 1 on a
    /// diagonal, so a body cuts it to 1 before using it.
    pub fn direction(self) -> Vec2 {
        Vec2::new(f32::from(self.x), f32::from(self.z)) / f32::from(AXIS_MAX)
    }
}

/// What a body asks for in one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Intent {
    pub walk: Walk,
    pub jump: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_lowest_raw_axis_is_raised_so_both_directions_match() {
        let walk = Walk::new(i8::MIN, i8::MIN);
        assert_eq!((walk.x(), walk.z()), (-AXIS_MAX, -AXIS_MAX));
    }

    #[test]
    fn a_non_finite_direction_stands_still() {
        for direction in [
            Vec2::new(f32::NAN, 0.0),
            Vec2::new(0.0, f32::INFINITY),
            Vec2::splat(f32::NEG_INFINITY),
        ] {
            assert_eq!(Walk::from_direction(direction), Walk::STILL);
        }
    }

    #[test]
    fn straight_and_full_speed_map_to_the_axis_limits() {
        assert_eq!(Walk::from_direction(Vec2::X), Walk::new(AXIS_MAX, 0));
        assert_eq!(Walk::from_direction(Vec2::NEG_Y), Walk::new(0, -AXIS_MAX));
        assert_eq!(
            Walk::from_direction(Vec2::new(40.0, 0.0)),
            Walk::new(AXIS_MAX, 0)
        );
    }

    proptest! {
        #[test]
        fn rounding_moves_a_direction_by_less_than_one_step(
            x in -1.0f32..1.0,
            z in -1.0f32..1.0,
        ) {
            let wanted = Vec2::new(x, z).clamp_length_max(1.0);
            let got = Walk::from_direction(Vec2::new(x, z)).direction();
            // Half a step on each axis at most, so the corner of the rounding square.
            let limit = 0.5 * 2.0_f32.sqrt() / f32::from(AXIS_MAX) + 1e-6;
            prop_assert!((got - wanted).length() <= limit);
        }

        #[test]
        fn any_float_direction_gives_a_walk_inside_the_range(x in any::<f32>(), z in any::<f32>()) {
            let walk = Walk::from_direction(Vec2::new(x, z));
            prop_assert!(walk.x() >= -AXIS_MAX && walk.z() >= -AXIS_MAX);
        }

        #[test]
        fn a_walk_survives_the_trip_through_a_direction(x in -AXIS_MAX..=AXIS_MAX, z in -AXIS_MAX..=AXIS_MAX) {
            let walk = Walk::new(x, z);
            // Only a unit-length walk is guaranteed to come back; longer ones are cut to 1.
            if walk.direction().length() <= 1.0 {
                prop_assert_eq!(Walk::from_direction(walk.direction()), walk);
            }
        }
    }
}
