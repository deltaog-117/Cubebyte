//! Movement keys held by the player.

use glam::Vec3;

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

    proptest! {
        #[test]
        fn the_axis_is_zero_or_unit_length(bits in 0u8..64) {
            let keys = MoveKeys {
                forward: bits & 1 != 0,
                back: bits & 2 != 0,
                left: bits & 4 != 0,
                right: bits & 8 != 0,
                up: bits & 16 != 0,
                down: bits & 32 != 0,
            };
            let length = keys.axis().length();
            prop_assert!(length == 0.0 || (length - 1.0).abs() < 1e-5);
        }
    }
}
