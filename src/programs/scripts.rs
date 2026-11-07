//! The edits a player saved to the ready-made scripts.
//!
//! A ready-made script is a fixed program in [`library`]. When the player saves a change from
//! the block editor, the changed program is kept here under the script's ID and is what a start
//! (from a panel, the command line or the editor) runs from then on; resetting a script drops
//! the edit and brings the built-in program back. A script is saved to intent tracks as the
//! commands it really ran, never as a program, so an edit cannot change an old recording.

use thiserror::Error;

use crate::scripting::library::{self, ScriptId};
use crate::scripting::program::Program;

/// Why an edit was not kept.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScriptEditError {
    #[error("no such script")]
    NoSuchScript,
}

/// The programs the player changed, at most one per ready-made script, ordered by script ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptEdits {
    list: Vec<(ScriptId, Program)>,
}

impl ScriptEdits {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The edited scripts with their programs, in script order.
    pub fn iter(&self) -> impl Iterator<Item = (ScriptId, &Program)> {
        self.list.iter().map(|(id, program)| (*id, program))
    }

    /// The player's version of script `id`, if it was edited.
    pub fn edited(&self, id: ScriptId) -> Option<&Program> {
        self.list
            .iter()
            .find(|(edited, _)| edited.0 == id.0)
            .map(|(_, program)| program)
    }

    /// The program script `id` runs now: the player's version, or the built-in one. `None` for
    /// an ID no script has.
    pub fn program_of(&self, id: ScriptId) -> Option<&Program> {
        match self.edited(id) {
            Some(program) => Some(program),
            None => library::get(id).map(library::Script::program),
        }
    }

    /// The most fuel script `id` can burn as it runs now, or `None` for an ID no script has.
    pub fn fuel_cost(&self, id: ScriptId) -> Option<u32> {
        self.program_of(id).map(Program::fuel_cost)
    }

    /// Keeps `program` as script `id`. A program equal to the built-in one is not an edit, so
    /// it clears any earlier edit instead.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptEditError::NoSuchScript`] for an ID no script has.
    pub fn set(&mut self, id: ScriptId, program: Program) -> Result<(), ScriptEditError> {
        let built_in = library::get(id).ok_or(ScriptEditError::NoSuchScript)?;
        self.list.retain(|(edited, _)| edited.0 != id.0);
        if &program != built_in.program() {
            let at = self.list.partition_point(|(edited, _)| edited.0 < id.0);
            self.list.insert(at, (id, program));
        }
        Ok(())
    }

    /// Brings script `id`'s built-in program back; returns whether there was an edit to drop.
    pub fn reset(&mut self, id: ScriptId) -> bool {
        let before = self.list.len();
        self.list.retain(|(edited, _)| edited.0 != id.0);
        self.list.len() != before
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::scripting::program::{Block, Plan};

    fn other() -> Program {
        Program::try_new(vec![Block::Do(Plan::Hop { by: [0, 5, 0] })]).unwrap()
    }

    #[test]
    fn an_unedited_script_runs_its_built_in_program() {
        let edits = ScriptEdits::default();
        for id in library::ids() {
            assert_eq!(
                edits.program_of(id),
                library::get(id).map(library::Script::program)
            );
        }
        assert_eq!(edits.program_of(ScriptId(250)), None);
    }

    #[test]
    fn a_saved_edit_replaces_the_program_and_its_cost() {
        let mut edits = ScriptEdits::default();
        let id = library::ids().next().unwrap();
        edits.set(id, other()).unwrap();
        assert_eq!(edits.program_of(id), Some(&other()));
        assert_eq!(edits.fuel_cost(id), Some(other().fuel_cost()));
        assert!(edits.reset(id));
        assert!(!edits.reset(id));
        assert_eq!(edits.edited(id), None);
    }

    #[test]
    fn saving_the_built_in_program_is_not_an_edit() {
        let mut edits = ScriptEdits::default();
        let id = library::ids().next().unwrap();
        edits.set(id, other()).unwrap();
        let built_in = library::get(id).unwrap().program().clone();
        edits.set(id, built_in).unwrap();
        assert!(edits.is_empty());
    }

    #[test]
    fn an_unknown_script_is_refused() {
        let mut edits = ScriptEdits::default();
        assert_eq!(
            edits.set(ScriptId(250), other()),
            Err(ScriptEditError::NoSuchScript)
        );
    }

    proptest! {
        // However edits come and go, there is at most one per script and they stay in order.
        #[test]
        fn edits_stay_unique_and_ordered(picks in prop::collection::vec((0usize..8, any::<bool>()), 0..24)) {
            let ids: Vec<_> = library::ids().collect();
            let mut edits = ScriptEdits::default();
            for (pick, keep) in picks {
                let id = ids[pick % ids.len()];
                if keep {
                    edits.set(id, other()).unwrap();
                } else {
                    edits.reset(id);
                }
            }
            let order: Vec<u8> = edits.iter().map(|(id, _)| id.0).collect();
            prop_assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}
