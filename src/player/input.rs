//! Movement keys held by the player.

use glam::{Vec2, Vec3};

/// Which movement keys are down right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveKeys {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
}

impl MoveKeys {
    /// Intended direction as x = right, y = up, z = forward.
    ///
    /// Opposite keys cancel. The result is normalized, so moving diagonally is not faster
    /// than moving straight, and it is zero when nothing is held.
    pub fn axis(self) -> Vec3 {
        let signed = |positive: bool, negative: bool| f32::from(positive) - f32::from(negative);
        Vec3::new(
            signed(self.right, self.left),
            signed(self.up, self.down),
            signed(self.forward, self.back),
        )
        .normalize_or_zero()
    }

    /// Intended horizontal direction as x = right, y = forward, ignoring up and down.
    ///
    /// Normalized like [`MoveKeys::axis`], so holding jump while walking is not slower.
    pub fn walk(self) -> Vec2 {
        let axis = Self {
            up: false,
            down: false,
            ..self
        }
        .axis();
        Vec2::new(axis.x, axis.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn opposite_keys_cancel() {
        let keys = MoveKeys {
            forward: true,
            back: true,
            ..MoveKeys::default()
        };
        assert_eq!(keys.axis(), Vec3::ZERO);
    }

    fn keys_from_bits(bits: u8) -> MoveKeys {
        MoveKeys {
            forward: bits & 1 != 0,
            back: bits & 2 != 0,
            left: bits & 4 != 0,
            right: bits & 8 != 0,
            up: bits & 16 != 0,
            down: bits & 32 != 0,
        }
    }

    proptest! {
        #[test]
        fn the_axis_is_zero_or_unit_length(bits in 0u8..64) {
            let length = keys_from_bits(bits).axis().length();
            prop_assert!(length == 0.0 || (length - 1.0).abs() < 1e-5);
        }

        #[test]
        fn the_walk_direction_ignores_up_and_down_and_is_zero_or_unit_length(bits in 0u8..64) {
            let keys = keys_from_bits(bits);
            let flat = MoveKeys { up: false, down: false, ..keys };
            prop_assert_eq!(keys.walk(), flat.walk());
            let length = keys.walk().length();
            prop_assert!(length == 0.0 || (length - 1.0).abs() < 1e-5);
        }
    }
}
