//! Distance fog: far terrain fades into the sky colour.
//!
//! The shader computes the blend per pixel with the same formula as [`Fog::factor`], which is
//! the version that is tested without a GPU. The range is set from the chunk load radius, so
//! chunks appear while they are fully fogged and not as a visible pop.

use thiserror::Error;

/// A fog range that cannot be built.
#[derive(Debug, Error, PartialEq)]
#[error("fog must start at 0 or more, finite, and end after it (start {start}, end {end})")]
pub struct BadFog {
    pub start: f32,
    pub end: f32,
}

/// Where fog begins and where it hides everything, in blocks from the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    start: f32,
    end: f32,
}

impl Fog {
    /// Fog pushed past any view distance, so nothing is hidden. The renderer starts with it
    /// until the game sets a real range.
    pub const OFF: Self = Self {
        start: 1.0e6,
        end: 2.0e6,
    };

    /// # Errors
    ///
    /// Returns [`BadFog`] unless `0 <= start < end` and both are finite.
    pub fn try_new(start: f32, end: f32) -> Result<Self, BadFog> {
        if start.is_finite() && end.is_finite() && start >= 0.0 && start < end {
            Ok(Self { start, end })
        } else {
            Err(BadFog { start, end })
        }
    }

    pub const fn start(self) -> f32 {
        self.start
    }

    pub const fn end(self) -> f32 {
        self.end
    }

    /// How much sky shows at `distance` blocks: 0 up to `start`, 1 from `end` on, rising
    /// evenly between.
    pub fn factor(self, distance: f32) -> f32 {
        ((distance - self.start) / (self.end - self.start)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_bad_range_is_refused() {
        assert!(Fog::try_new(10.0, 10.0).is_err());
        assert!(Fog::try_new(-1.0, 10.0).is_err());
        assert!(Fog::try_new(0.0, f32::NAN).is_err());
        assert!(Fog::try_new(0.0, f32::INFINITY).is_err());
        assert!(Fog::try_new(0.0, 1.0).is_ok());
    }

    proptest! {
        #[test]
        fn fog_is_clear_before_start_solid_after_end_and_never_thins_with_distance(
            start in 0.0f32..500.0,
            width in 0.5f32..500.0,
            a in 0.0f32..2000.0,
            b in 0.0f32..2000.0,
        ) {
            let fog = Fog::try_new(start, start + width).unwrap();
            prop_assert_eq!(fog.factor(start), 0.0);
            prop_assert_eq!(fog.factor(start + width), 1.0);
            let (near, far) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(fog.factor(near) <= fog.factor(far));
            prop_assert!((0.0..=1.0).contains(&fog.factor(a)));
        }
    }
}
