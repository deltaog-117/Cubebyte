use minecraft_clone::events::log::EventLog;
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

proptest! {
    #[test]
    fn replaying_the_log_rebuilds_the_same_world(
        edits in prop::collection::vec(
            ((-20i32..20, -20i32..20, -20i32..20), 0u16..4),
            0..100,
        ),
    ) {
        let mut live = World::default();
        let mut log = EventLog::new();
        for ((x, y, z), block) in edits {
            let pos = WorldPos { x, y, z };
            let event = Event::BlockSet { pos, old: live.block_at(pos), new: BlockId(block) };
            prop_assert_eq!(live.apply(&event), Ok(()));
            log.push(event);
        }

        let mut replayed = World::default();
        for event in log.iter() {
            prop_assert_eq!(replayed.apply(event), Ok(()));
        }
        prop_assert_eq!(live, replayed);
    }
}
