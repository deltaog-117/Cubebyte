//! The zombie's state, and the store of every mob that is alive.

use std::collections::BTreeMap;

use glam::Vec3;
use thiserror::Error;

use crate::shared::body::{Body, BodyError};
use crate::shared::event::MobId;

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

    /// Like [`Zombie::try_new`], with `health` instead of full health: a zombie put back by
    /// an undo comes back as it left.
    fn with_health(position: Vec3, health: u8) -> Result<Self, MobError> {
        if health > MAX_HEALTH {
            return Err(MobError::BadHealth(health));
        }
        Ok(Self {
            health,
            ..Self::try_new(position)?
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MobError {
    #[error("mob {0:?} is already in the world")]
    Exists(MobId),
    #[error("mob {0:?} is not in the world")]
    Missing(MobId),
    /// The mob does not have the health the event was recorded against.
    #[error("stale mob {id:?}: expected {expected} health but found {found}")]
    StaleHealth { id: MobId, expected: u8, found: u8 },
    #[error("health {0} is above the most a mob has ({MAX_HEALTH})")]
    BadHealth(u8),
    /// A health change that would leave `0..=MAX_HEALTH`.
    #[error("mob {id:?} cannot go from {from} health by {by}")]
    HealthRange { id: MobId, from: u8, by: i8 },
    #[error(transparent)]
    Body(#[from] BodyError),
}

/// Every mob that is alive, by id.
///
/// Mobs come and go and lose health only through the three methods below, which the event
/// door calls, so the log always says how the store got to be what it is. Walking is not an
/// event yet, so [`Mobs::get_mut`] hands out a mob to be stepped; its health is private to
/// this feature and only the store changes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mobs {
    zombies: BTreeMap<MobId, Zombie>,
}

impl Mobs {
    /// Puts mob `id` at `at` with `health`.
    ///
    /// # Errors
    ///
    /// - [`MobError::Exists`] if `id` is already in the world.
    /// - [`MobError::BadHealth`] if `health` is above [`MAX_HEALTH`].
    /// - [`MobError::Body`] if `at` is not finite.
    ///
    /// The store is unchanged on any error.
    pub fn spawn(&mut self, id: MobId, at: Vec3, health: u8) -> Result<(), MobError> {
        if self.zombies.contains_key(&id) {
            return Err(MobError::Exists(id));
        }
        self.zombies.insert(id, Zombie::with_health(at, health)?);
        Ok(())
    }

    /// Takes mob `id` out, if it has `health`. Where it stands is not checked: it has walked
    /// since it appeared, and walking is not an event.
    ///
    /// # Errors
    ///
    /// - [`MobError::Missing`] if `id` is not in the world.
    /// - [`MobError::StaleHealth`] if it has other health than `health`.
    ///
    /// The store is unchanged on any error.
    pub fn remove(&mut self, id: MobId, health: u8) -> Result<(), MobError> {
        let found = self.zombies.get(&id).ok_or(MobError::Missing(id))?.health;
        if found != health {
            return Err(MobError::StaleHealth {
                id,
                expected: health,
                found,
            });
        }
        self.zombies.remove(&id);
        Ok(())
    }

    /// Moves the health of mob `id` by `by`.
    ///
    /// # Errors
    ///
    /// - [`MobError::Missing`] if `id` is not in the world.
    /// - [`MobError::HealthRange`] if the result would leave `0..=MAX_HEALTH`.
    ///
    /// The store is unchanged on any error.
    pub fn shift_health(&mut self, id: MobId, by: i8) -> Result<(), MobError> {
        let zombie = self.zombies.get_mut(&id).ok_or(MobError::Missing(id))?;
        zombie.health = zombie
            .health
            .checked_add_signed(by)
            .filter(|&health| health <= MAX_HEALTH)
            .ok_or(MobError::HealthRange {
                id,
                from: zombie.health,
                by,
            })?;
        Ok(())
    }

    pub fn get(&self, id: MobId) -> Option<&Zombie> {
        self.zombies.get(&id)
    }

    /// The mob to step. Moving it is not an event; its health cannot be changed through it.
    pub fn get_mut(&mut self, id: MobId) -> Option<&mut Zombie> {
        self.zombies.get_mut(&id)
    }

    /// Every mob, lowest id first.
    pub fn iter(&self) -> impl Iterator<Item = (MobId, &Zombie)> {
        self.zombies.iter().map(|(&id, zombie)| (id, zombie))
    }

    pub fn is_empty(&self) -> bool {
        self.zombies.is_empty()
    }

    pub fn len(&self) -> usize {
        self.zombies.len()
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

    #[test]
    fn a_spawned_mob_keeps_the_health_it_was_given() {
        let mut mobs = Mobs::default();
        mobs.spawn(MobId(1), Vec3::ZERO, 7).unwrap();
        assert_eq!(mobs.get(MobId(1)).map(Zombie::health), Some(7));
    }

    #[test]
    fn a_spawn_that_cannot_be_is_refused_and_changes_nothing() {
        let mut mobs = Mobs::default();
        mobs.spawn(MobId(1), Vec3::ZERO, 7).unwrap();
        let before = mobs.clone();
        assert_eq!(
            mobs.spawn(MobId(1), Vec3::ONE, 7),
            Err(MobError::Exists(MobId(1)))
        );
        assert_eq!(
            mobs.spawn(MobId(2), Vec3::ONE, MAX_HEALTH + 1),
            Err(MobError::BadHealth(MAX_HEALTH + 1))
        );
        assert_eq!(
            mobs.spawn(MobId(2), Vec3::new(f32::NAN, 0.0, 0.0), 1),
            Err(MobError::Body(BodyError::NonFinite))
        );
        assert_eq!(mobs, before);
    }

    #[test]
    fn a_removal_needs_the_mob_and_its_health_but_not_its_place() {
        let mut mobs = Mobs::default();
        mobs.spawn(MobId(1), Vec3::ZERO, 7).unwrap();
        assert_eq!(mobs.remove(MobId(2), 7), Err(MobError::Missing(MobId(2))));
        assert_eq!(
            mobs.remove(MobId(1), 8),
            Err(MobError::StaleHealth {
                id: MobId(1),
                expected: 8,
                found: 7
            })
        );
        assert_eq!(mobs.len(), 1);
        assert_eq!(mobs.remove(MobId(1), 7), Ok(()));
        assert!(mobs.is_empty());
    }

    #[test]
    fn health_moves_within_zero_and_the_most_and_no_further() {
        let mut mobs = Mobs::default();
        mobs.spawn(MobId(1), Vec3::ZERO, 5).unwrap();
        assert_eq!(mobs.shift_health(MobId(1), -5), Ok(()));
        assert_eq!(mobs.get(MobId(1)).map(Zombie::health), Some(0));
        let range = Err(MobError::HealthRange {
            id: MobId(1),
            from: 0,
            by: -1,
        });
        assert_eq!(mobs.shift_health(MobId(1), -1), range);
        assert_eq!(
            mobs.shift_health(MobId(1), 21),
            Err(MobError::HealthRange {
                id: MobId(1),
                from: 0,
                by: 21,
            })
        );
        assert_eq!(
            mobs.shift_health(MobId(9), 1),
            Err(MobError::Missing(MobId(9)))
        );
    }
}
