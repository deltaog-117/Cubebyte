//! Stepping the game back one action.
//!
//! A rewind never removes anything from the log. It works out the events that undo the
//! latest edit that is still in effect; the caller applies them like any other action and
//! logs them as a [`ActionKind::Rewind`], so replay and a restart land on the rewound game.

use crate::events::log::{ActionKind, EventLog};
use crate::shared::event::Event;

/// The events that undo the latest edit still in effect, newest change first, or `None` when
/// nothing is left to undo.
pub fn rewind_events(log: &EventLog) -> Option<Vec<Event>> {
    let edit = log.latest_live_edit()?;
    Some(edit.iter().rev().map(|event| event.inverse()).collect())
}

/// Logs `events` as one action of `kind`.
pub fn log_action(log: &mut EventLog, events: &[Event], kind: ActionKind) {
    events.iter().for_each(|&event| log.push(event));
    log.end_action(kind);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::voxel::{BlockId, WorldPos};
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
