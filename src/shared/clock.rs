//! Fixed-timestep simulation clock.
//!
//! The simulation advances in whole ticks, so its state depends only on the tick count and
//! never on wall-clock time or frame rate. That is what makes replay and rewind exact.

use std::time::Duration;

use thiserror::Error;

/// Number of simulation steps taken since the world began.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u64);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClockError {
    #[error("the simulation step must be longer than zero")]
    ZeroStep,
    #[error("at least one catch-up step per advance is required")]
    ZeroCatchUp,
}

/// Turns elapsed real time into a whole number of simulation steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedClock {
    step_nanos: u128,
    max_steps_per_advance: u32,
    // Real time received but not yet spent on a step. Always below one step.
    remainder_nanos: u128,
    tick: Tick,
}

impl FixedClock {
    /// `max_steps_per_advance` bounds the catch-up after a long stall, so one slow frame
    /// cannot demand so many steps that the next frame is slow too.
    ///
    /// # Errors
    ///
    /// Returns [`ClockError::ZeroStep`] if `step` is zero, and [`ClockError::ZeroCatchUp`]
    /// if `max_steps_per_advance` is zero.
    pub fn try_new(step: Duration, max_steps_per_advance: u32) -> Result<Self, ClockError> {
        if step.is_zero() {
            return Err(ClockError::ZeroStep);
        }
        if max_steps_per_advance == 0 {
            return Err(ClockError::ZeroCatchUp);
        }
        Ok(Self {
            step_nanos: step.as_nanos(),
            max_steps_per_advance,
            remainder_nanos: 0,
            tick: Tick(0),
        })
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Adds `elapsed` real time and returns how many simulation steps are now due.
    ///
    /// If more than the cap are due, the extra backlog is dropped and the game runs slow
    /// instead of spiralling.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        let total = self.remainder_nanos + elapsed.as_nanos();
        let due = total / self.step_nanos;
        self.remainder_nanos = total % self.step_nanos;

        let steps = u32::try_from(due).map_or(self.max_steps_per_advance, |due| {
            due.min(self.max_steps_per_advance)
        });
        if u128::from(steps) < due {
            tracing::warn!(due = %due, steps, "simulation fell behind; dropping backlog");
        }
        self.tick = Tick(self.tick.0.saturating_add(u64::from(steps)));
        steps
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn clock(step_ms: u64, cap: u32) -> FixedClock {
        FixedClock::try_new(Duration::from_millis(step_ms), cap).unwrap()
    }

    #[test]
    fn zero_step_and_zero_cap_are_rejected() {
        assert_eq!(
            FixedClock::try_new(Duration::ZERO, 1),
            Err(ClockError::ZeroStep)
        );
        assert_eq!(
            FixedClock::try_new(Duration::from_millis(1), 0),
            Err(ClockError::ZeroCatchUp)
        );
    }

    #[test]
    fn partial_steps_carry_over_to_the_next_advance() {
        let mut clock = clock(50, 10);
        assert_eq!(clock.advance(Duration::from_millis(30)), 0);
        assert_eq!(clock.advance(Duration::from_millis(30)), 1);
        assert_eq!(clock.tick(), Tick(1));
    }

    #[test]
    fn a_long_stall_is_capped_and_the_backlog_dropped() {
        let mut clock = clock(10, 5);
        assert_eq!(clock.advance(Duration::from_secs(10)), 5);
        assert_eq!(clock.tick(), Tick(5));
        assert_eq!(clock.advance(Duration::ZERO), 0);
    }

    proptest! {
        #[test]
        fn how_time_is_split_into_frames_does_not_change_the_tick_count(
            step_ms in 1u64..50,
            frames in prop::collection::vec(0u64..200, 0..100),
        ) {
            let mut clock = clock(step_ms, u32::MAX);
            let steps: u64 = frames
                .iter()
                .map(|ms| u64::from(clock.advance(Duration::from_millis(*ms))))
                .sum();
            let total_ms: u64 = frames.iter().sum();
            prop_assert_eq!(steps, total_ms / step_ms);
            prop_assert_eq!(clock.tick(), Tick(steps));
        }

        #[test]
        fn steps_per_advance_never_exceed_the_cap(
            cap in 1u32..20,
            frames in prop::collection::vec(0u64..5_000, 0..50),
        ) {
            let mut clock = clock(10, cap);
            prop_assert!(frames
                .iter()
                .all(|ms| clock.advance(Duration::from_millis(*ms)) <= cap));
        }
    }
}
