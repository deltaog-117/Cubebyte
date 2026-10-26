//! Player state.

use glam::Vec3;
use thiserror::Error;

/// Half the width of the player's box, which is as deep as it is wide.
pub const HALF_WIDTH: f32 = 0.3;

/// Height of the player's box above the feet.
pub const HEIGHT: f32 = 1.8;

/// Height of the eyes above the feet.
pub const EYE_HEIGHT: f32 = 1.62;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlayerError {
    #[error("the player position must be finite")]
    NonFinite,
}

/// A walking body: a box standing at `position` (the middle of its feet) with a velocity.
///
/// The fields are private to the `player` feature, so only [`step`](super::service::step)
/// moves the player and the position is always finite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub(super) position: Vec3,
    /// Where the player stood before the latest step, so the view can blend between the two.
    pub(super) previous: Vec3,
    /// Blocks per second.
    pub(super) velocity: Vec3,
    pub(super) on_ground: bool,
}

impl Player {
    /// A player standing still at `position`, with nothing known to be under the feet yet.
    ///
    /// # Errors
    ///
    /// Returns [`PlayerError::NonFinite`] if any coordinate is NaN or infinite.
    pub fn try_new(position: Vec3) -> Result<Self, PlayerError> {
        if !position.is_finite() {
            return Err(PlayerError::NonFinite);
        }
        Ok(Self {
            position,
            previous: position,
            velocity: Vec3::ZERO,
            on_ground: false,
        })
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn velocity(&self) -> Vec3 {
        self.velocity
    }

    /// Whether the last step ended pressed against the floor, which is what allows a jump.
    pub fn on_ground(&self) -> bool {
        self.on_ground
    }

    /// Where the eyes are, `alpha` of the way from the previous step to the latest one.
    ///
    /// Blending keeps the view smooth at the slower simulation rate.
    pub fn eye(&self, alpha: f32) -> Vec3 {
        self.previous.lerp(self.position, alpha.clamp(0.0, 1.0)) + Vec3::Y * EYE_HEIGHT
    }

    /// Opposite corners of the player's box.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let half = Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH);
        (
            self.position - half,
            self.position + half + Vec3::Y * HEIGHT,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_finite_position_is_rejected() {
        assert_eq!(
            Player::try_new(Vec3::new(0.0, f32::NAN, 0.0)),
            Err(PlayerError::NonFinite)
        );
        assert_eq!(
            Player::try_new(Vec3::new(f32::INFINITY, 0.0, 0.0)),
            Err(PlayerError::NonFinite)
        );
    }

    #[test]
    fn the_eye_sits_above_the_feet_and_the_box_spans_the_full_height() {
        let player = Player::try_new(Vec3::new(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(player.eye(1.0), Vec3::new(1.0, 2.0 + EYE_HEIGHT, 3.0));
        let (min, max) = player.bounds();
        assert_eq!(min, Vec3::new(1.0 - HALF_WIDTH, 2.0, 3.0 - HALF_WIDTH));
        assert_eq!(
            max,
            Vec3::new(1.0 + HALF_WIDTH, 2.0 + HEIGHT, 3.0 + HALF_WIDTH)
        );
    }
}
