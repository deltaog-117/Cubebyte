//! Stepping forward again after a rewind.
//!
//! A rewind hands back the edit it undid; the stack keeps those edits in memory, newest last.
//! Redo applies the newest one again and the caller logs it as a fresh edit, so the log
//! stays append-only and the rewind logic needs no change. Any new edit must [`clear`] the
//! stack, since the undone edits were made against a game that has moved on.
//!
//! [`clear`]: RedoStack::clear

use crate::events::rewind::inverse_run;
use crate::shared::event::Event;

/// Edits that rewinds undid and nothing has replaced since. It is not saved: a restart
/// forgets what could be redone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedoStack {
    undone: Vec<Vec<Event>>,
}

impl RedoStack {
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
    use super::*;
    use crate::shared::voxel::{BlockId, WorldPos};

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

    #[test]
    fn clearing_forgets_every_undone_edit() {
        let mut stack = RedoStack::default();
        stack.record(&inverse_run(&[set(0, 0, 1)]));
        stack.clear();

        assert!(stack.is_empty());
        assert_eq!(stack.take(), None);
    }
}
