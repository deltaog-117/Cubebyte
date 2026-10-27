//! What a hit is.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("a hit must deal at least 1 damage")]
pub struct ZeroDamage;

/// Health points one hit takes, always at least 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Damage(u8);

impl Damage {
    /// A bare-handed hit.
    pub const FIST: Self = Self(1);

    /// # Errors
    ///
    /// Returns [`ZeroDamage`] for 0, since a hit that does nothing is not a hit.
    pub const fn try_new(points: u8) -> Result<Self, ZeroDamage> {
        if points == 0 {
            Err(ZeroDamage)
        } else {
            Ok(Self(points))
        }
    }

    pub const fn points(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_damage_is_rejected_and_the_fist_is_one() {
        assert_eq!(Damage::try_new(0), Err(ZeroDamage));
        assert_eq!(Damage::try_new(3).map(Damage::points), Ok(3));
        assert_eq!(Damage::FIST.points(), 1);
    }
}
