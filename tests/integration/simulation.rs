use std::time::Duration;

use minecraft_clone::shared::clock::FixedClock;
use minecraft_clone::simulation::model::Simulation;
use proptest::prelude::*;

fn simulation(seed: u64) -> Result<Simulation, TestCaseError> {
    let clock = FixedClock::try_new(Duration::from_millis(50), u32::MAX)
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    Ok(Simulation::new(seed, clock))
}

proptest! {
    #[test]
    fn frame_rate_changes_neither_the_tick_nor_the_random_streams(
        seed in any::<u64>(),
        key in any::<u64>(),
        frames in prop::collection::vec(0u64..200, 0..100),
    ) {
        // One run sees the time in many small frames, the other in a single frame.
        let mut stepped = simulation(seed)?;
        for ms in &frames {
            stepped.advance(Duration::from_millis(*ms));
        }
        let mut whole = simulation(seed)?;
        whole.advance(Duration::from_millis(frames.iter().sum()));

        prop_assert_eq!(stepped.tick(), whole.tick());
        prop_assert_eq!(stepped.stream(key), whole.stream(key));
    }
}
