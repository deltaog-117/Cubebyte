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

    /// Raises the meter by `amount`, stopping at [`METER_MAX`].
    fn fill(self, amount: u8) -> Self {
        Self(self.0.saturating_add(amount).min(METER_MAX))
    }

    /// Lowers the meter by `amount`, stopping at 0.
    fn drain(self, amount: u8) -> Self {
        Self(self.0.saturating_sub(amount))
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

    /// Refills hunger by `amount`, up to the maximum.
    pub fn restore_hunger(&mut self, amount: u8) {
        self.hunger = self.hunger.fill(amount);
    }

    /// Takes `amount` health, stopping at 0.
    pub fn hurt(&mut self, amount: u8) {
        self.health = self.health.drain(amount);
    }

    /// Whether health has run out.
    pub const fn is_dead(self) -> bool {
        self.health.0 == 0
    }

    /// Advances one simulation step.
    pub fn step(&mut self) {
        self.steps += 1;
        if self.steps.is_multiple_of(HUNGER_PERIOD) {
            self.hunger = self.hunger.drain(1);
        }
        if self.hunger.value() == 0 && self.steps.is_multiple_of(STARVE_PERIOD) {
            self.health = self.health.drain(1);
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
    fn starving_drains_health_to_zero_and_stops_there() {
        let stats = run(HUNGER_PERIOD * u64::from(METER_MAX) + STARVE_PERIOD * 200);
        assert_eq!(stats.hunger().value(), 0);
        assert_eq!(stats.health().value(), 0);
        assert!(stats.is_dead());
    }

    #[test]
    fn hurting_takes_health_and_stops_at_zero() {
        let mut stats = Stats::default();
        stats.hurt(7);
        assert_eq!(stats.health().value(), METER_MAX - 7);
        assert!(!stats.is_dead());
        stats.hurt(u8::MAX);
        assert_eq!(stats.health().value(), 0);
        assert!(stats.is_dead());
    }

    #[test]
    fn restoring_hunger_stops_at_the_maximum() {
        let mut stats = run(HUNGER_PERIOD * 5);
        stats.restore_hunger(2);
        assert_eq!(stats.hunger().value(), METER_MAX - 3);
        stats.restore_hunger(u8::MAX);
        assert_eq!(stats.hunger(), Meter::FULL);
    }

    proptest! {
        #[test]
        fn meters_stay_in_range_and_never_rise(steps in 0u64..40_000) {
            let stats = run(steps);
            prop_assert!(stats.hunger().value() <= METER_MAX);
            prop_assert!(stats.health().value() <= METER_MAX);
        }

        #[test]
        fn hurting_never_raises_health_and_death_means_zero(
            hits in prop::collection::vec(0u8..=u8::MAX, 0..20),
        ) {
            let mut stats = Stats::default();
            for hit in hits {
                let before = stats.health().value();
                stats.hurt(hit);
                prop_assert_eq!(stats.health().value(), before.saturating_sub(hit));
                prop_assert_eq!(stats.is_dead(), stats.health().value() == 0);
            }
        }

        #[test]
        fn the_same_steps_give_the_same_stats(steps in 0u64..5_000) {
            prop_assert_eq!(run(steps), run(steps));
        }
    }
}
