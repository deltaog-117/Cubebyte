//! Stepping the game back one action.
//!
//! A rewind never removes anything from the log. It works out the events that undo the
//! latest edit that is still in effect; the caller applies them like any other action and
//! logs them as a [`ActionKind::Rewind`], so replay and a restart land on the rewound game.
//!
//! The log keeps only the last [`HISTORY_WINDOW`] actions as history; older ones are folded
//! away, so a rewind cannot reach past them.

use crate::events::log::{ActionKind, EventLog};
use crate::shared::event::Event;

/// How many actions of history the log keeps rewindable. Compaction runs once the log holds
/// twice this many, so it happens once per window of play and not on every action.
pub const HISTORY_WINDOW: usize = 256;

/// The events that undo `events`, newest change first. Inverting the result gives `events`
/// back, which is how a rewound edit is replayed.
pub fn inverse_run(events: &[Event]) -> Vec<Event> {
    events.iter().rev().map(|event| event.inverse()).collect()
}

/// The events that undo the latest edit still in effect, newest change first, or `None` when
/// nothing is left to undo.
pub fn rewind_events(log: &EventLog) -> Option<Vec<Event>> {
    Some(inverse_run(log.latest_live_edit()?))
}

/// Logs `events` as one action of `kind`, then folds the oldest history away once it has
/// grown past twice [`HISTORY_WINDOW`].
pub fn log_action(log: &mut EventLog, events: &[Event], kind: ActionKind) {
    events.iter().for_each(|&event| log.push(event));
    log.end_action(kind);
    if log.actions() >= 2 * HISTORY_WINDOW {
        log.compact(HISTORY_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::events::replay::replay_chunk;
    use crate::shared::voxel::{BlockId, ChunkPos, WorldPos};
    use crate::world::model::World;

    /// One action: a few blocks in one chunk set to new values.
    fn action() -> impl Strategy<Value = Vec<(i32, u16)>> {
        prop::collection::vec((0i32..4, 0u16..4), 1..4)
    }

    /// Records `action` against `world`, applies it and logs it as an edit.
    fn edit(world: &mut World, log: &mut EventLog, action: &[(i32, u16)]) {
        let events: Vec<Event> = action
            .iter()
            .map(|&(x, new)| {
                let pos = WorldPos { x, y: 0, z: 0 };
                let event = Event::BlockSet {
                    pos,
                    old: world.block_at(pos),
                    new: BlockId(new),
                };
                world.apply(&event).unwrap();
                event
            })
            .collect();
        log_action(log, &events, ActionKind::Edit);
    }

    fn rewind(world: &mut World, log: &mut EventLog) -> bool {
        let Some(events) = rewind_events(log) else {
            return false;
        };
        for event in &events {
            world.apply(event).unwrap();
        }
        log_action(log, &events, ActionKind::Rewind);
        true
    }

    fn replayed(log: &EventLog) -> World {
        let mut world = World::default();
        for event in log.iter() {
            world.apply(event).unwrap();
        }
        world
    }

    proptest! {
        #[test]
        fn compacting_keeps_the_newest_actions_rewindable_and_the_world_rebuildable(
            actions in prop::collection::vec(action(), 1..12),
            keep in 0usize..14,
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            let mut after_each = vec![World::default()];
            for step in &actions {
                edit(&mut world, &mut log, step);
                after_each.push(world.clone());
            }
            log.compact(keep);

            // A restart sees the base and the kept events, and lands on the same world.
            let chunk = ChunkPos { x: 0, y: 0, z: 0 };
            let mut restored = World::default();
            prop_assert_eq!(replay_chunk(&mut restored, &log, chunk), Ok(()));
            prop_assert_eq!(&restored, &world);

            let mut stepped = 0;
            while rewind(&mut world, &mut log) {
                stepped += 1;
            }
            prop_assert_eq!(stepped, keep.min(actions.len()));
            prop_assert_eq!(&world, &after_each[actions.len() - stepped]);

            // The rewinds are in the log too, so a restart still lands on the rewound world.
            let mut restored = World::default();
            prop_assert_eq!(replay_chunk(&mut restored, &log, chunk), Ok(()));
            prop_assert_eq!(restored, world);
        }

        #[test]
        fn rewinding_k_actions_restores_the_world_after_the_earlier_ones(
            actions in prop::collection::vec(action(), 1..8),
            back in 0usize..10,
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            let mut after_each = vec![World::default()];
            for step in &actions {
                edit(&mut world, &mut log, step);
                after_each.push(world.clone());
            }

            let mut stepped = 0;
            while stepped < back && rewind(&mut world, &mut log) {
                stepped += 1;
            }

            prop_assert_eq!(stepped, back.min(actions.len()));
            prop_assert_eq!(&world, &after_each[actions.len() - stepped]);
            // The log holds the whole story, so replaying it gives the same world.
            prop_assert_eq!(replayed(&log), world);
        }

        #[test]
        fn edits_after_a_rewind_are_themselves_rewindable(
            before in action(), undone in action(), after in action(),
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            edit(&mut world, &mut log, &before);
            let kept = world.clone();
            edit(&mut world, &mut log, &undone);
            rewind(&mut world, &mut log);
            edit(&mut world, &mut log, &after);

            rewind(&mut world, &mut log);
            prop_assert_eq!(&world, &kept);
        }
    }

    #[test]
    fn an_empty_log_has_nothing_to_rewind() {
        assert_eq!(rewind_events(&EventLog::new()), None);
    }

    #[test]
    fn events_outside_any_action_are_never_rewound() {
        let mut log = EventLog::new();
        log.push(Event::BlockSet {
            pos: WorldPos { x: 0, y: 0, z: 0 },
            old: BlockId::AIR,
            new: BlockId::STONE,
        });
        assert_eq!(rewind_events(&log), None);
    }

    #[test]
    fn rewinding_past_the_first_edit_stops() {
        let (mut world, mut log) = (World::default(), EventLog::new());
        edit(&mut world, &mut log, &[(0, 1)]);
        assert!(rewind(&mut world, &mut log));
        assert!(!rewind(&mut world, &mut log));
    }
}
