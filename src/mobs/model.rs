//! The zombie's state.

use glam::Vec3;

use crate::shared::body::{Body, BodyError};

/// Half the width of the zombie's box, which is as deep as it is wide.
pub const HALF_WIDTH: f32 = 0.3;

/// Height of the zombie's box above the feet.
pub const HEIGHT: f32 = 1.8;

/// Hits-to-kill budget: a zombie starts with this much health.
pub const MAX_HEALTH: u8 = 20;

/// A walking body that chases the player.
///
/// The fields are private to the `mobs` feature, so only [`step`](super::service::step)
/// moves a zombie and only [`strike`](super::service::strike) hurts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zombie {
    pub(super) body: Body,
    pub(super) health: u8,
    /// Steps left before the zombie may hit again.
    pub(super) cooldown: u8,
}

impl Zombie {
    /// A zombie at full health standing at `position`.
    ///
    /// # Errors
    ///
    /// Returns [`BodyError::NonFinite`] if any coordinate is NaN or infinite.
    pub fn try_new(position: Vec3) -> Result<Self, BodyError> {
        Ok(Self {
            body: Body::try_new(position, HALF_WIDTH, HEIGHT)?,
            health: MAX_HEALTH,
            cooldown: 0,
        })
    }

    pub fn position(&self) -> Vec3 {
        self.body.position()
    }

    pub const fn health(&self) -> u8 {
        self.health
    }

    /// Opposite corners of the box, `alpha` of the way from the previous step to the latest
    /// one, for drawing.
    pub fn bounds(&self, alpha: f32) -> (Vec3, Vec3) {
        self.body.bounds_at(self.body.blended(alpha))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_zombie_has_full_health_and_a_player_sized_box() {
        let zombie = Zombie::try_new(Vec3::new(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(zombie.health(), MAX_HEALTH);
        let (min, max) = zombie.bounds(1.0);
        assert_eq!(min, Vec3::new(1.0 - HALF_WIDTH, 2.0, 3.0 - HALF_WIDTH));
        assert_eq!(
            max,
            Vec3::new(1.0 + HALF_WIDTH, 2.0 + HEIGHT, 3.0 + HALF_WIDTH)
        );
    }

    #[test]
    fn a_non_finite_position_is_rejected() {
        assert_eq!(
            Zombie::try_new(Vec3::new(f32::NAN, 0.0, 0.0)),
            Err(BodyError::NonFinite)
        );
    }
}
