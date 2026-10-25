//! Append-only event log.

use std::collections::HashMap;

use crate::shared::event::Event;
use crate::shared::voxel::ChunkPos;

/// Every event applied to the world, in order. Events are never removed or reordered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventLog {
    events: Vec<Event>,
    // Indices into `events`, per chunk and in log order, so a chunk that streams back in
    // replays only its own edits.
    by_chunk: HashMap<ChunkPos, Vec<usize>>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: Event) {
        self.by_chunk
            .entry(event.chunk())
            .or_default()
            .push(self.events.len());
        self.events.push(event);
    }

    /// The events that changed `pos`, in the order they were logged.
    pub fn chunk_events(&self, pos: ChunkPos) -> impl Iterator<Item = &Event> {
        self.by_chunk
            .get(&pos)
            .into_iter()
            .flatten()
            .map(|&index| &self.events[index])
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::voxel::{BlockId, WorldPos};

    proptest! {
        #[test]
        fn iteration_yields_events_in_the_order_they_were_pushed(
            blocks in prop::collection::vec((any::<u16>(), any::<u16>()), 0..50),
        ) {
            let events: Vec<Event> = blocks
                .into_iter()
                .map(|(old, new)| Event::BlockSet {
                    pos: WorldPos { x: 0, y: 0, z: 0 },
                    old: BlockId(old),
                    new: BlockId(new),
                })
                .collect();
            let mut log = EventLog::new();
            for event in &events {
                log.push(*event);
            }
            prop_assert_eq!(log.len(), events.len());
            prop_assert!(log.iter().eq(events.iter()));
        }

        #[test]
        fn chunk_events_are_that_chunks_events_in_log_order(
            edits in prop::collection::vec(((-40i32..40, -4i32..4, -40i32..40), 0u16..4), 0..80),
            chunk in (-3i32..3, -1i32..1, -3i32..3),
        ) {
            let chunk = ChunkPos { x: chunk.0, y: chunk.1, z: chunk.2 };
            let events: Vec<Event> = edits
                .into_iter()
                .map(|((x, y, z), new)| Event::BlockSet {
                    pos: WorldPos { x, y, z },
                    old: BlockId(0),
                    new: BlockId(new),
                })
                .collect();
            let mut log = EventLog::new();
            for event in &events {
                log.push(*event);
            }
            let expected = events.iter().filter(|event| event.chunk() == chunk);
            prop_assert!(log.chunk_events(chunk).eq(expected));
        }
    }
}
