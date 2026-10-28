//! Append-only event log.

use std::collections::HashMap;

use crate::shared::event::Event;
use crate::shared::voxel::ChunkPos;

/// What one logged action was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// A change the player or a script made.
    Edit,
    /// The undoing of the latest edit that had not been undone yet.
    Rewind,
}

/// Where an action's events end in the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mark {
    end: usize,
    kind: ActionKind,
}

/// One entry of the log as it is saved: an event, or the end of an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    Event(Event),
    ActionEnd(ActionKind),
}

/// Every event applied to the world, in order. Events are never removed or reordered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventLog {
    events: Vec<Event>,
    // Where each finished action ends, oldest first. Events pushed after the last mark are
    // not part of any action yet, and rewind never touches them.
    marks: Vec<Mark>,
    // Indices into `events`, per chunk and in log order, so a chunk that streams back in
    // replays only its own edits.
    by_chunk: HashMap<ChunkPos, Vec<usize>>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: Event) {
        if let Some(chunk) = event.chunk() {
            self.by_chunk
                .entry(chunk)
                .or_default()
                .push(self.events.len());
        }
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

    /// Closes the events pushed since the last action as one action of `kind`. Does nothing
    /// when there are none, so an action is never empty.
    pub fn end_action(&mut self, kind: ActionKind) {
        let end = self.events.len();
        if self.marks.last().map_or(0, |mark| mark.end) < end {
            self.marks.push(Mark { end, kind });
        }
    }

    /// The events and action ends in the order they were logged.
    pub fn records(&self) -> impl Iterator<Item = Record> + '_ {
        let mut marks = self.marks.iter().peekable();
        self.events
            .iter()
            .enumerate()
            .flat_map(move |(index, &event)| {
                let end = marks
                    .next_if(|mark| mark.end == index + 1)
                    .map(|mark| Record::ActionEnd(mark.kind));
                std::iter::once(Record::Event(event)).chain(end)
            })
    }

    /// The events of the latest edit that no later rewind has undone, in the order they
    /// happened. Each rewind cancels the closest edit before it, like matching brackets, so
    /// rewinding again steps back through older edits instead of redoing the last one.
    pub fn latest_live_edit(&self) -> Option<&[Event]> {
        let mut undone = 0usize;
        for (index, mark) in self.marks.iter().enumerate().rev() {
            match mark.kind {
                ActionKind::Rewind => undone += 1,
                ActionKind::Edit if undone > 0 => undone -= 1,
                ActionKind::Edit => {
                    let start = index
                        .checked_sub(1)
                        .map_or(0, |before| self.marks[before].end);
                    return Some(&self.events[start..mark.end]);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::event::Held;
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
            let expected = events.iter().filter(|event| event.chunk() == Some(chunk));
            prop_assert!(log.chunk_events(chunk).eq(expected));
        }
    }

    #[test]
    fn events_without_a_chunk_are_logged_but_never_replayed_per_chunk() {
        let mut log = EventLog::new();
        let block = Event::BlockSet {
            pos: WorldPos { x: 1, y: 0, z: 0 },
            old: BlockId::AIR,
            new: BlockId::STONE,
        };
        log.push(Event::SlotSet {
            slot: 0,
            old: Held::EMPTY,
            new: Held { item: 1, count: 5 },
        });
        log.push(block);

        assert_eq!(log.len(), 2);
        let chunk = ChunkPos { x: 0, y: 0, z: 0 };
        assert!(log.chunk_events(chunk).eq([&block]));
    }
}
