//! Rebuilding world state from events.

use crate::events::log::EventLog;
use crate::shared::event::Event;
use crate::shared::voxel::ChunkPos;
use crate::world::model::{ApplyError, World};

/// Re-applies the logged edits of one chunk, oldest first: first the base, the final blocks of
/// history that was folded away, then the events kept since.
///
/// Call it right after the chunk's generated baseline is installed. Each event touches a
/// single block, so replaying one chunk on its own gives the same blocks the live world
/// held before the chunk was unloaded.
///
/// # Errors
///
/// Returns the first [`ApplyError`] if an event does not match the baseline, for example
/// when the terrain came from a different seed. Events before it stay applied.
pub fn replay_chunk(world: &mut World, log: &EventLog, pos: ChunkPos) -> Result<(), ApplyError> {
    // Base positions are all different, so their order does not matter.
    for (at, new) in log.chunk_base(pos) {
        let old = world.block_at(at);
        world.apply(&Event::BlockSet { pos: at, old, new })?;
    }
    log.chunk_events(pos)
        .try_for_each(|event| world.apply(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::event::Event;
    use crate::shared::voxel::{BlockId, WorldPos};

    #[test]
    fn replaying_a_chunk_leaves_other_chunks_alone() {
        let mut world = World::default();
        let mut log = EventLog::new();
        for x in [1, 40] {
            let pos = WorldPos { x, y: 0, z: 0 };
            let event = Event::BlockSet {
                pos,
                old: BlockId::AIR,
                new: BlockId::STONE,
            };
            world.apply(&event).unwrap();
            log.push(event);
        }
        let mut fresh = World::default();

        replay_chunk(&mut fresh, &log, ChunkPos { x: 0, y: 0, z: 0 }).unwrap();

        assert_eq!(
            fresh.block_at(WorldPos { x: 1, y: 0, z: 0 }),
            BlockId::STONE
        );
        assert_eq!(fresh.block_at(WorldPos { x: 40, y: 0, z: 0 }), BlockId::AIR);
    }

    #[test]
    fn a_stale_event_is_reported() {
        let mut log = EventLog::new();
        log.push(Event::BlockSet {
            pos: WorldPos { x: 0, y: 0, z: 0 },
            old: BlockId::STONE,
            new: BlockId::AIR,
        });

        let result = replay_chunk(&mut World::default(), &log, ChunkPos { x: 0, y: 0, z: 0 });

        assert!(matches!(result, Err(ApplyError::StaleEvent { .. })));
    }
}
