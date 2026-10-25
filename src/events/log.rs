//! Append-only event log.

use crate::shared::event::Event;

/// Every event applied to the world, in order. Events are never removed or reordered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventLog {
    events: Vec<Event>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: Event) {
        self.events.push(event);
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
    }
}
