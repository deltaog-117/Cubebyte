//! Stepping forward again after a rewind.
//!
//! A rewind hands back the edit it undid; the stack keeps those edits in memory, newest last.
//! Redo applies the newest one again and the caller logs it as a fresh edit, so the log
//! stays append-only and the rewind logic needs no change. Any new edit must [`clear`] the
//! stack, since the undone edits were made against a game that has moved on.
//!
//! The stack is not saved on its own. The log already says how it came to be, so a restart
//! rebuilds it with [`RedoStack::from_log`]: each rewind pushes, each redo pops, and an edit or
//! a respawn empties it.
//!
//! [`clear`]: RedoStack::clear

use crate::events::log::{ActionKind, EventLog, Record};
use crate::events::rewind::inverse_run;
use crate::shared::event::Event;

/// Edits that rewinds undid and nothing has replaced since.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedoStack {
    undone: Vec<Vec<Event>>,
}

impl RedoStack {
    /// The stack a game that logged `log` ended with. History the log has folded away is gone,
    /// so a rewind older than the kept actions cannot be redone.
    pub fn from_log(log: &EventLog) -> Self {
        let mut stack = Self::default();
        let mut action = Vec::new();
        for record in log.records() {
            match record {
                Record::Event(event) => action.push(event),
                Record::ActionEnd(kind) => {
                    match kind {
                        ActionKind::Rewind => stack.record(&action),
                        ActionKind::Redo => {
                            stack.take();
                        }
                        ActionKind::Edit | ActionKind::Respawn => stack.clear(),
                        ActionKind::Passive => {}
                    }
                    action.clear();
                }
            }
        }
        stack
    }

    /// Remembers the edit that `rewind`, the events just applied by a rewind, undid.
    pub fn record(&mut self, rewind: &[Event]) {
        self.undone.push(inverse_run(rewind));
    }

    /// Takes the newest undone edit: the events that redo it, in the order they first
    /// happened.
    pub fn take(&mut self) -> Option<Vec<Event>> {
        self.undone.pop()
    }

    /// Forgets everything that could be redone.
    pub fn clear(&mut self) {
        self.undone.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.undone.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::events::persist::{decode, encode};
    use crate::events::rewind::{log_action, rewind_events};
    use crate::shared::voxel::{BlockId, WorldPos};
    use crate::world::model::World;

    fn set(x: i32, old: u16, new: u16) -> Event {
        Event::BlockSet {
            pos: WorldPos { x, y: 0, z: 0 },
            old: BlockId(old),
            new: BlockId(new),
        }
    }

    #[test]
    fn redo_gives_back_the_undone_edit_newest_first() {
        let (first, second) = ([set(0, 0, 1), set(1, 0, 1)], [set(2, 0, 1)]);
        let mut stack = RedoStack::default();
        stack.record(&inverse_run(&first));
        stack.record(&inverse_run(&second));

        assert_eq!(stack.take(), Some(second.to_vec()));
        assert_eq!(stack.take(), Some(first.to_vec()));
        assert_eq!(stack.take(), None);
    }

    /// What the game does, as the window does it: every step is applied, logged, and keeps the
    /// live stack the way the window keeps it.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        Edit(i32, u16),
        Rewind,
        Redo,
        Passive,
        Respawn,
    }

    fn play(steps: &[Step]) -> (EventLog, RedoStack) {
        let (mut world, mut log, mut live) =
            (World::default(), EventLog::new(), RedoStack::default());
        for &step in steps {
            match step {
                Step::Edit(x, new) => {
                    let pos = WorldPos { x, y: 0, z: 0 };
                    let event = Event::BlockSet {
                        pos,
                        old: world.block_at(pos),
                        new: BlockId(new),
                    };
                    world.apply(&event).unwrap();
                    log_action(&mut log, &[event], ActionKind::Edit);
                    live.clear();
                }
                Step::Rewind => {
                    if let Some(events) = rewind_events(&log) {
                        events.iter().for_each(|e| world.apply(e).unwrap());
                        log_action(&mut log, &events, ActionKind::Rewind);
                        live.record(&events);
                    }
                }
                Step::Redo => {
                    if let Some(events) = live.take() {
                        events.iter().for_each(|e| world.apply(e).unwrap());
                        log_action(&mut log, &events, ActionKind::Redo);
                    }
                }
                Step::Passive => {
                    log_action(
                        &mut log,
                        &[Event::HungerChanged { by: -1 }],
                        ActionKind::Passive,
                    );
                }
                Step::Respawn => {
                    log_action(
                        &mut log,
                        &[Event::HealthChanged { by: 1 }],
                        ActionKind::Respawn,
                    );
                    live.clear();
                }
            }
        }
        (log, live)
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            (0i32..4, 0u16..4).prop_map(|(x, new)| Step::Edit(x, new)),
            Just(Step::Rewind),
            Just(Step::Redo),
            Just(Step::Passive),
            Just(Step::Respawn),
        ]
    }

    proptest! {
        #[test]
        fn the_stack_rebuilt_from_the_log_is_the_stack_the_game_ended_with(
            steps in prop::collection::vec(step(), 0..40),
        ) {
            let (log, live) = play(&steps);
            prop_assert_eq!(RedoStack::from_log(&log), live);
        }

        #[test]
        fn the_rebuilt_stack_survives_a_save_and_redoes_what_was_undone(
            edits in prop::collection::vec((0i32..4, 0u16..4), 1..6),
            back in 0usize..6,
        ) {
            let mut steps: Vec<Step> = edits.iter().map(|&(x, n)| Step::Edit(x, n)).collect();
            steps.extend(std::iter::repeat_n(Step::Rewind, back));
            let (log, live) = play(&steps);

            let saved = decode(&encode(117, &log)).unwrap().log;
            let restored = RedoStack::from_log(&saved);
            prop_assert_eq!(&restored, &live);
            prop_assert_eq!(restored.is_empty(), back == 0);
        }
    }

    #[test]
    fn a_redo_does_not_empty_the_stack_but_a_new_edit_does() {
        let (log, live) = play(&[
            Step::Edit(0, 1),
            Step::Edit(1, 1),
            Step::Rewind,
            Step::Rewind,
            Step::Redo,
        ]);
        assert_eq!(RedoStack::from_log(&log), live);
        assert!(!live.is_empty());

        let (log, live) = play(&[Step::Edit(0, 1), Step::Rewind, Step::Edit(1, 1)]);
        assert_eq!(RedoStack::from_log(&log), live);
        assert!(live.is_empty());
    }

    #[test]
    fn clearing_forgets_every_undone_edit() {
        let mut stack = RedoStack::default();
        stack.record(&inverse_run(&[set(0, 0, 1)]));
        stack.clear();

        assert!(stack.is_empty());
        assert_eq!(stack.take(), None);
    }
}
