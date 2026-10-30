use std::time::Duration;

use glam::Vec3;
use minecraft_clone::player::model::Player;
use minecraft_clone::player::service::step;
use minecraft_clone::shared::clock::{FixedClock, Tick};
use minecraft_clone::shared::intent::{AXIS_MAX, Intent, Walk};
use minecraft_clone::simulation::intent_log::ActorId;
use minecraft_clone::simulation::model::Simulation;
use minecraft_clone::simulation::replay::Replay;
use minecraft_clone::simulation::stream::IntentStream;
use minecraft_clone::world::model::World;
use proptest::prelude::*;

const DT: f32 = 0.05;

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

/// A scratch folder that is removed when the test ends, even if it fails.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn start() -> Result<Player, TestCaseError> {
    Player::try_new(Vec3::new(0.5, 20.0, 0.5))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn a_streamed_session_replays_the_run_exactly(
        intents in prop::collection::vec(
            ((-AXIS_MAX..=AXIS_MAX, -AXIS_MAX..=AXIS_MAX), any::<bool>()).prop_map(
                |((x, z), jump)| Intent { walk: Walk::new(x, z), jump },
            ),
            1..300,
        ),
        id in any::<u32>(),
    ) {
        let dir = TempDir(std::env::temp_dir().join(format!(
            "mcit-replay-run-{}-{id}",
            std::process::id()
        )));
        let path = dir.0.join("0001.mcit");
        let world = World::default();

        // Record a run to a session file, the way the game does.
        let mut stream = IntentStream::new(path.clone());
        let mut live = start()?;
        for (tick, intent) in (0u64..).zip(intents.iter().copied()) {
            prop_assert!(stream.record(ActorId::PLAYER, Tick(tick), intent).is_ok());
            step(&mut live, intent, &world, DT);
        }
        prop_assert!(stream.finish().is_ok());

        // Load it and feed it to a fresh body, tick by tick, until the replay runs out.
        let replay = Replay::load(&path)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut replayed = start()?;
        let mut tick = 0u64;
        while let Some(intent) = replay.intent(ActorId::PLAYER, Tick(tick)) {
            step(&mut replayed, intent, &world, DT);
            tick += 1;
        }
        prop_assert_eq!(tick, intents.len() as u64);
        prop_assert_eq!(live, replayed);
    }
}
