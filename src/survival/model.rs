//! The player's survival stats.
//!
//! Stats change only by [`Stats::step`], once per simulation step, and use no randomness
//! and no wall-clock time, so a replay of the same steps gives the same stats.

use thiserror::Error;

/// Both meters run from 0 to this value.
pub const METER_MAX: u8 = 20;

/// Simulation steps between hunger losses: 30 seconds at 20 steps per second.
pub const HUNGER_PERIOD: u64 = 600;

/// Simulation steps between health losses while starving: 5 seconds.
pub const STARVE_PERIOD: u64 = 100;

/// Starvation never takes health below this, because nothing can heal or kill the player
/// yet. Lower it to 0 when combat adds death.
pub const STARVE_FLOOR: u8 = 1;

/// A value in `0..=METER_MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter(u8);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("a meter holds 0..={METER_MAX}, not {0}")]
pub struct OutOfRange(pub u8);

impl Meter {
    pub const FULL: Self = Self(METER_MAX);

    /// # Errors
    ///
    /// Returns [`OutOfRange`] above [`METER_MAX`].
    pub fn try_new(value: u8) -> Result<Self, OutOfRange> {
        if value > METER_MAX {
            Err(OutOfRange(value))
        } else {
            Ok(Self(value))
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }

    /// Lowers the meter by one, stopping at `floor`.
    fn drain(self, floor: u8) -> Self {
        Self(self.0.saturating_sub(1).max(floor.min(self.0)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    health: Meter,
    hunger: Meter,
    steps: u64,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            health: Meter::FULL,
            hunger: Meter::FULL,
            steps: 0,
        }
    }
}

impl Stats {
    pub const fn health(self) -> Meter {
        self.health
    }

    pub const fn hunger(self) -> Meter {
        self.hunger
    }

    /// Advances one simulation step.
    pub fn step(&mut self) {
        self.steps += 1;
        if self.steps.is_multiple_of(HUNGER_PERIOD) {
            self.hunger = self.hunger.drain(0);
        }
        if self.hunger.value() == 0 && self.steps.is_multiple_of(STARVE_PERIOD) {
            self.health = self.health.drain(STARVE_FLOOR);
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn run(steps: u64) -> Stats {
        let mut stats = Stats::default();
        for _ in 0..steps {
            stats.step();
        }
        stats
    }

    #[test]
    fn a_meter_rejects_values_above_the_maximum() {
        assert_eq!(Meter::try_new(METER_MAX), Ok(Meter::FULL));
        assert_eq!(
            Meter::try_new(METER_MAX + 1),
            Err(OutOfRange(METER_MAX + 1))
        );
    }

    #[test]
    fn a_new_player_is_at_full_health_and_hunger() {
        let stats = Stats::default();
        assert_eq!(stats.health(), Meter::FULL);
        assert_eq!(stats.hunger(), Meter::FULL);
    }

    #[test]
    fn hunger_drops_one_per_period_and_health_holds_while_fed() {
        let stats = run(HUNGER_PERIOD * 3);
        assert_eq!(stats.hunger().value(), METER_MAX - 3);
        assert_eq!(stats.health(), Meter::FULL);
    }

    #[test]
    fn starving_drains_health_down_to_the_floor_and_stops() {
        let stats = run(HUNGER_PERIOD * u64::from(METER_MAX) + STARVE_PERIOD * 200);
        assert_eq!(stats.hunger().value(), 0);
        assert_eq!(stats.health().value(), STARVE_FLOOR);
    }

    proptest! {
        #[test]
        fn meters_stay_in_range_and_never_rise(steps in 0u64..40_000) {
            let stats = run(steps);
            prop_assert!(stats.hunger().value() <= METER_MAX);
            prop_assert!(stats.health().value() <= METER_MAX);
            prop_assert!(stats.health().value() >= STARVE_FLOOR);
        }

        #[test]
        fn the_same_steps_give_the_same_stats(steps in 0u64..5_000) {
            prop_assert_eq!(run(steps), run(steps));
        }
    }
}
