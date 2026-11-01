//! The player's survival stats.
//!
//! Stats change only by [`Stats::step`], once per simulation step, and use no randomness
//! and no wall-clock time, so a replay of the same steps gives the same stats.
//!
//! The game changes the meters through events: [`Stats::advance`] says what a step drains
//! without applying it, and [`Stats::shift_hunger`] and [`Stats::shift_health`] apply the
//! change an event carries.

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

/// A change that would take a meter out of `0..=METER_MAX`.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("a meter at {value} cannot move by {by}: it holds 0..={METER_MAX}")]
pub struct ShiftRefused {
    pub value: u8,
    pub by: i8,
}

/// What one step takes from each meter, as the signed change to apply: 0 or -1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drain {
    pub hunger: i8,
    pub health: i8,
}

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

    /// The meter moved by `by`, or `None` if that leaves `0..=METER_MAX`.
    fn shifted(self, by: i8) -> Option<Self> {
        u8::try_from(i16::from(self.0) + i16::from(by))
            .ok()
            .and_then(|value| Self::try_new(value).ok())
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

    /// Starts the step count over, so the next hunger drain is a full period away. The
    /// count is a clock, not a meter, so no event carries it.
    pub fn restart_clock(&mut self) {
        self.steps = 0;
    }

    /// How far hunger would really rise if `amount` were restored: less than `amount` near
    /// the top, which is what an event records.
    pub fn hunger_gain(self, amount: u8) -> i8 {
        i8::try_from(self.hunger.fill(amount).0 - self.hunger.0).unwrap_or(0)
    }

    /// How far health would really rise if `amount` were restored.
    pub fn health_gain(self, amount: u8) -> i8 {
        i8::try_from(self.health.fill(amount).0 - self.health.0).unwrap_or(0)
    }

    /// How far health would really fall if `amount` were taken, as a negative number.
    pub fn health_loss(self, amount: u8) -> i8 {
        i8::try_from(self.health.0 - self.health.drain(amount).0).map_or(0, |lost| -lost)
    }

    /// Moves hunger by `by`, as an event says it moved.
    ///
    /// # Errors
    ///
    /// Returns [`ShiftRefused`], changing nothing, if that leaves `0..=METER_MAX`.
    pub fn shift_hunger(&mut self, by: i8) -> Result<(), ShiftRefused> {
        self.hunger = self.hunger.shifted(by).ok_or(ShiftRefused {
            value: self.hunger.0,
            by,
        })?;
        Ok(())
    }

    /// Moves health by `by`, as an event says it moved.
    ///
    /// # Errors
    ///
    /// Returns [`ShiftRefused`], changing nothing, if that leaves `0..=METER_MAX`.
    pub fn shift_health(&mut self, by: i8) -> Result<(), ShiftRefused> {
        self.health = self.health.shifted(by).ok_or(ShiftRefused {
            value: self.health.0,
            by,
        })?;
        Ok(())
    }

    /// Counts one simulation step and returns what it drains, without draining it: the
    /// caller applies the change as events. Nothing is drained from a meter already at 0.
    pub fn advance(&mut self) -> Drain {
        self.steps += 1;
        let hunger_drop = self.steps.is_multiple_of(HUNGER_PERIOD) && self.hunger.0 > 0;
        let hunger_after = self.hunger.0 - u8::from(hunger_drop);
        // Starvation reads hunger after this step's drain, so the last point of hunger and
        // the first starving hit can fall on the same step.
        let starving = hunger_after == 0 && self.steps.is_multiple_of(STARVE_PERIOD);
        Drain {
            hunger: -i8::from(hunger_drop),
            health: -i8::from(starving && self.health.0 > 0),
        }
    }

    /// Advances one simulation step and drains the meters directly.
    pub fn step(&mut self) {
        let drain = self.advance();
        self.hunger = self.hunger.drain(drain.hunger.unsigned_abs());
        self.health = self.health.drain(drain.health.unsigned_abs());
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

    #[test]
    fn a_shift_that_leaves_the_range_is_refused_and_changes_nothing() {
        let mut stats = Stats::default();
        assert_eq!(
            stats.shift_hunger(1),
            Err(ShiftRefused {
                value: METER_MAX,
                by: 1
            })
        );
        stats.hurt(METER_MAX);
        assert_eq!(
            stats.shift_health(-1),
            Err(ShiftRefused { value: 0, by: -1 })
        );
        assert_eq!(stats.health().value(), 0);
        assert_eq!(stats.shift_health(i8::MIN).map_err(|e| e.by), Err(i8::MIN));
    }

    #[test]
    fn gains_and_losses_stop_where_the_meter_stops() {
        let mut stats = run(HUNGER_PERIOD * 2);
        assert_eq!(stats.hunger_gain(5), 2);
        assert_eq!(stats.hunger_gain(1), 1);
        stats.hurt(15);
        assert_eq!(stats.health_loss(9), -5);
        assert_eq!(stats.health_loss(3), -3);
        assert_eq!(Stats::default().health_loss(0), 0);
    }

    proptest! {
        #[test]
        fn applying_the_advance_drain_matches_step(steps in 0u64..40_000) {
            let (mut stepped, mut advanced) = (Stats::default(), Stats::default());
            for _ in 0..steps {
                stepped.step();
                let drain = advanced.advance();
                advanced.shift_hunger(drain.hunger).unwrap();
                advanced.shift_health(drain.health).unwrap();
            }
            prop_assert_eq!(stepped, advanced);
        }

        #[test]
        fn a_shift_and_its_opposite_cancel(start in 0u64..30_000, by in -20i8..=20) {
            let mut stats = run(start);
            let before = stats;
            if stats.shift_hunger(by).is_ok() {
                prop_assert_eq!(stats.shift_hunger(-by), Ok(()));
            }
            prop_assert_eq!(stats, before);
        }

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
