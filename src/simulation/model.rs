//! The simulation's only sources of time and randomness.
//!
//! Anything that changes game state must take its time from [`Simulation::advance`] and its
//! randomness from [`Simulation::stream`]. Wall-clock time belongs to rendering alone.

use std::time::Duration;

use crate::shared::clock::{FixedClock, Tick};
use crate::shared::rng::Rng;

#[derive(Clone, Debug)]
pub struct Simulation {
    clock: FixedClock,
    // Never drawn from, only forked, so a stream depends on its key and not on call order.
    root: Rng,
}

impl Simulation {
    pub fn new(seed: u64, clock: FixedClock) -> Self {
        Self {
            clock,
            root: Rng::from_seed(seed),
        }
    }

    pub fn tick(&self) -> Tick {
        self.clock.tick()
    }

    /// Adds `elapsed` real time and returns how many simulation steps are now due.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.clock.advance(elapsed)
    }

    /// The random stream for one system, for example a mob kind or a chunk position.
    ///
    /// The same `key` always yields the same stream, whatever else was asked for before.
    pub fn stream(&self, key: u64) -> Rng {
        self.root.fork(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulation(seed: u64) -> Simulation {
        let clock = FixedClock::try_new(Duration::from_millis(50), 5).unwrap();
        Simulation::new(seed, clock)
    }

    #[test]
    fn a_new_simulation_starts_at_tick_zero() {
        assert_eq!(simulation(1).tick(), Tick(0));
    }

    #[test]
    fn advancing_moves_the_tick_by_the_steps_due() {
        let mut sim = simulation(1);
        assert_eq!(sim.advance(Duration::from_millis(120)), 2);
        assert_eq!(sim.tick(), Tick(2));
    }

    #[test]
    fn a_stream_does_not_depend_on_which_streams_were_asked_for_first() {
        let sim = simulation(7);
        let direct = sim.stream(3).next_u64();
        let _ = sim.stream(1).next_u64();
        let _ = sim.stream(2).next_u64();
        assert_eq!(sim.stream(3).next_u64(), direct);
    }

    #[test]
    fn different_keys_and_seeds_give_different_streams() {
        let sim = simulation(7);
        assert_ne!(sim.stream(1).next_u64(), sim.stream(2).next_u64());
        assert_ne!(
            simulation(7).stream(1).next_u64(),
            simulation(8).stream(1).next_u64()
        );
    }
}
