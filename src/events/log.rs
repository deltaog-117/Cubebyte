//! Append-only event log.

use std::collections::HashMap;

use crate::shared::event::Event;
use crate::shared::voxel::{BlockId, ChunkPos, WorldPos};

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

/// Every event applied to the world, in order. Events are never reordered, and only
/// [`EventLog::compact`] removes any: it folds the oldest into a base of final blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventLog {
    events: Vec<Event>,
    // Where each finished action ends, oldest first. Events pushed after the last mark are
    // not part of any action yet, and rewind never touches them.
    marks: Vec<Mark>,
    // Indices into `events`, per chunk and in log order, so a chunk that streams back in
    // replays only its own edits.
    by_chunk: HashMap<ChunkPos, Vec<usize>>,
    // The block every position ended up as in the history folded away by `compact`, per
    // chunk. It sits under `events`: a chunk is rebuilt as terrain, then this, then its events.
    base: HashMap<ChunkPos, HashMap<WorldPos, BlockId>>,
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

    /// Sets what `pos` is in the folded-away history.
    pub fn set_base(&mut self, pos: WorldPos, block: BlockId) {
        self.base
            .entry(pos.split().0)
            .or_default()
            .insert(pos, block);
    }

    /// What the folded-away history left in `pos`, in no particular order.
    pub fn chunk_base(&self, pos: ChunkPos) -> impl Iterator<Item = (WorldPos, BlockId)> + '_ {
        self.base
            .get(&pos)
            .into_iter()
            .flatten()
            .map(|(&at, &block)| (at, block))
    }

    /// Every block of the folded-away history, in no particular order.
    pub fn base_blocks(&self) -> impl Iterator<Item = (WorldPos, BlockId)> + '_ {
        self.base
            .values()
            .flatten()
            .map(|(&at, &block)| (at, block))
    }

    /// How many finished actions the log still holds, which is as far back as a rewind can
    /// reach.
    pub fn actions(&self) -> usize {
        self.marks.len()
    }

    /// Folds every action but the newest `keep` into the base, so the log stops growing
    /// with play. Only the final block at each position is kept, which is all a chunk needs
    /// to be rebuilt; the folded events, and rewinding past them, are gone. Does nothing when
    /// there are `keep` actions or fewer.
    pub fn compact(&mut self, keep: usize) {
        let Some(folded) = self.marks.len().checked_sub(keep).filter(|&n| n > 0) else {
            return;
        };
        let cut = self.marks[folded - 1].end;
        for event in self.events.drain(..cut) {
            if let Event::BlockSet { pos, new, .. } = event {
                self.base.entry(pos.split().0).or_default().insert(pos, new);
            }
        }
        self.marks.drain(..folded);
        for mark in &mut self.marks {
            mark.end -= cut;
        }
        self.by_chunk.clear();
        for (index, event) in self.events.iter().enumerate() {
            if let Some(chunk) = event.chunk() {
                self.by_chunk.entry(chunk).or_default().push(index);
            }
        }
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

    fn set(x: i32, old: u16, new: u16) -> Event {
        Event::BlockSet {
            pos: WorldPos { x, y: 0, z: 0 },
            old: BlockId(old),
            new: BlockId(new),
        }
    }

    proptest! {
        #[test]
        fn compacting_keeps_exactly_the_newest_actions(
            sizes in prop::collection::vec(1usize..4, 0..12),
            keep in 0usize..14,
        ) {
            let mut log = EventLog::new();
            for &size in &sizes {
                for _ in 0..size {
                    log.push(set(0, 0, 1));
                }
                log.end_action(ActionKind::Edit);
            }
            log.compact(keep);

            let kept = sizes.len().min(keep);
            prop_assert_eq!(log.actions(), kept);
            prop_assert_eq!(log.len(), sizes[sizes.len() - kept..].iter().sum::<usize>());
            let chunk = ChunkPos { x: 0, y: 0, z: 0 };
            prop_assert_eq!(log.chunk_events(chunk).count(), log.len());
        }
    }

    #[test]
    fn compacting_folds_old_actions_into_the_final_block_of_each_position() {
        let mut log = EventLog::new();
        for (old, new) in [(0, 1), (1, 2), (2, 3)] {
            log.push(set(5, old, new));
            log.end_action(ActionKind::Edit);
        }
        let before = log.clone();

        log.compact(3);
        assert_eq!(log, before);

        log.compact(1);
        let chunk = ChunkPos { x: 0, y: 0, z: 0 };
        let pos = WorldPos { x: 5, y: 0, z: 0 };
        assert!(log.chunk_base(chunk).eq([(pos, BlockId(2))]));
        assert!(log.chunk_events(chunk).eq([&set(5, 2, 3)]));
        assert_eq!(log.latest_live_edit(), Some(&[set(5, 2, 3)][..]));
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
