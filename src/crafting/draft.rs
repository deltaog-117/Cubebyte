//! The recipe editor's core: a draft of a recipe that clicks change one small step at a time.
//!
//! A [`RecipeDraft`] is a flat list of `take` and `make` lines with one selected. It can hold
//! a recipe that is not fair yet (nothing made, or more worth made than taken); the only way
//! out is [`RecipeDraft::program`], which runs the same checks as [`RecipeProgram::try_new`],
//! so a player can build nothing here that a hand-written recipe file could not. `repeat` and
//! `if` are not offered: they are written in the programs file.
//!
//! Nothing here draws or reads the mouse; the window turns clicks into the calls below.

use std::ops::Range;

use crate::crafting::program::{MAX_STEPS, RecipeError, RecipeProgram, Step};
use crate::items::registry::{self as items, ItemId};

/// Largest count a click can reach.
pub const COUNT_MAX: u8 = 99;

/// Whether a line spends items or adds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Take,
    Make,
}

impl Verb {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Take => "take",
            Self::Make => "make",
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::Take => Self::Make,
            Self::Make => Self::Take,
        }
    }
}

/// One line of the draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub verb: Verb,
    pub item: ItemId,
    /// 1 to [`COUNT_MAX`].
    pub count: u8,
}

/// The recipe being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeDraft {
    name: String,
    lines: Vec<Line>,
    selected: usize,
}

/// The item after `item` in the item table, wrapping to the first.
fn next_item(item: ItemId) -> ItemId {
    let all: Vec<ItemId> = items::ids().collect();
    let at = all.iter().position(|&other| other == item);
    match at {
        Some(index) => all[(index + 1) % all.len()],
        None => all.first().copied().unwrap_or(item),
    }
}

impl RecipeDraft {
    /// An empty draft that will be kept as `name`.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            lines: Vec::new(),
            selected: 0,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// A line as the player reads it: `take grass 3`.
    pub fn text(line: Line) -> String {
        let name = items::get(line.item).map_or("?", |def| def.name);
        format!("{} {name} {}", line.verb.name(), line.count)
    }

    /// Selects line `index`, if there is one.
    pub fn select(&mut self, index: usize) {
        if index < self.lines.len() {
            self.selected = index;
        }
    }

    pub fn up(&mut self) {
        self.select(self.selected.saturating_sub(1));
    }

    pub fn down(&mut self) {
        self.select(self.selected + 1);
    }

    /// The lines to show when only `size` fit: a stretch holding the selected one.
    pub fn window(&self, size: usize) -> Range<usize> {
        let len = self.lines.len();
        let start = self
            .selected
            .saturating_sub(size / 2)
            .min(len.saturating_sub(size));
        start..(start + size).min(len)
    }

    /// Adds a line after the selected one and selects it: a copy of the selected line, or
    /// `take` of the first item when the draft is empty. Refused (`false`) at [`MAX_STEPS`].
    pub fn add(&mut self) -> bool {
        if self.lines.len() >= MAX_STEPS {
            return false;
        }
        let Some(first) = items::ids().next() else {
            return false;
        };
        let line = self.lines.get(self.selected).copied().unwrap_or(Line {
            verb: Verb::Take,
            item: first,
            count: 1,
        });
        let at = if self.lines.is_empty() {
            0
        } else {
            self.selected + 1
        };
        self.lines.insert(at, line);
        self.selected = at;
        true
    }

    /// Removes the selected line; returns whether there was one.
    pub fn delete(&mut self) -> bool {
        if self.selected >= self.lines.len() {
            return false;
        }
        self.lines.remove(self.selected);
        self.selected = self.selected.min(self.lines.len().saturating_sub(1));
        true
    }

    /// Turns the selected `take` into a `make` or the other way round.
    pub fn flip_verb(&mut self) -> bool {
        match self.lines.get_mut(self.selected) {
            Some(line) => {
                line.verb = line.verb.other();
                true
            }
            None => false,
        }
    }

    /// Gives the selected line the next item.
    pub fn cycle_item(&mut self) -> bool {
        match self.lines.get_mut(self.selected) {
            Some(line) => {
                line.item = next_item(line.item);
                true
            }
            None => false,
        }
    }

    /// Changes the selected line's count by `delta`, staying within `1..=`[`COUNT_MAX`].
    /// Returns whether the count changed.
    pub fn nudge(&mut self, delta: i32) -> bool {
        let Some(line) = self.lines.get_mut(self.selected) else {
            return false;
        };
        let wanted = (i32::from(line.count) + delta).clamp(1, i32::from(COUNT_MAX));
        let wanted = u8::try_from(wanted).unwrap_or(COUNT_MAX);
        let changed = wanted != line.count;
        line.count = wanted;
        changed
    }

    /// The draft as a checked recipe.
    ///
    /// # Errors
    ///
    /// Returns the [`RecipeError`] that says what to change, such as a recipe that makes more
    /// worth than it takes.
    pub fn program(&self) -> Result<RecipeProgram, RecipeError> {
        RecipeProgram::try_new(
            self.lines
                .iter()
                .map(|line| match line.verb {
                    Verb::Take => Step::Take {
                        item: line.item,
                        count: line.count,
                    },
                    Verb::Make => Step::Make {
                        item: line.item,
                        count: line.count,
                    },
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn draft_of(lines: &[(Verb, ItemId, u8)]) -> RecipeDraft {
        let mut draft = RecipeDraft::new("t");
        draft.lines = lines
            .iter()
            .map(|&(verb, item, count)| Line { verb, item, count })
            .collect();
        draft
    }

    #[test]
    fn a_fair_draft_becomes_a_recipe_and_an_unfair_one_says_why() {
        let fair = draft_of(&[
            (Verb::Take, ItemId::GRASS, 3),
            (Verb::Make, ItemId::BREAD, 1),
        ]);
        assert!(fair.program().is_ok());
        let free = draft_of(&[(Verb::Make, ItemId::SWORD, 1)]);
        assert!(matches!(
            free.program(),
            Err(RecipeError::MakesWorth { .. })
        ));
        assert_eq!(RecipeDraft::new("t").program(), Err(RecipeError::Empty));
    }

    #[test]
    fn adding_copies_the_selected_line_after_it() {
        let mut draft = RecipeDraft::new("t");
        assert!(draft.add());
        assert_eq!(draft.lines().len(), 1);
        draft.flip_verb();
        draft.nudge(2);
        assert!(draft.add());
        assert_eq!(draft.selected(), 1);
        assert_eq!(draft.lines()[0], draft.lines()[1]);
    }

    #[test]
    fn a_line_reads_as_its_words() {
        let line = Line {
            verb: Verb::Take,
            item: ItemId::GRASS,
            count: 3,
        };
        assert_eq!(RecipeDraft::text(line), "take grass 3");
    }

    #[test]
    fn delete_keeps_a_valid_selection() {
        let mut draft = draft_of(&[
            (Verb::Take, ItemId::GRASS, 1),
            (Verb::Take, ItemId::DIRT, 1),
        ]);
        draft.select(1);
        assert!(draft.delete());
        assert_eq!(draft.selected(), 0);
        assert!(draft.delete());
        assert!(!draft.delete());
        assert_eq!(draft.selected(), 0);
    }

    #[test]
    fn an_empty_draft_has_nothing_to_change() {
        let mut draft = RecipeDraft::new("t");
        assert!(!draft.flip_verb() && !draft.cycle_item() && !draft.nudge(1));
    }

    proptest! {
        // Whatever is clicked, counts stay in range, the selection stays on a line (or 0 when
        // there is none), the list stays within the step limit, and `program` never panics.
        #[test]
        fn any_clicks_leave_a_sound_draft(clicks in prop::collection::vec(0u8..9, 0..200)) {
            let mut draft = RecipeDraft::new("t");
            for click in clicks {
                match click {
                    0 => { draft.add(); }
                    1 => { draft.delete(); }
                    2 => { draft.flip_verb(); }
                    3 => { draft.cycle_item(); }
                    4 => { draft.nudge(-1); }
                    5 => { draft.nudge(1); }
                    6 => draft.up(),
                    7 => draft.down(),
                    _ => draft.select(draft.selected() / 2),
                }
                prop_assert!(draft.lines().len() <= MAX_STEPS);
                prop_assert!(draft.lines().iter().all(|l| (1..=COUNT_MAX).contains(&l.count)));
                prop_assert!(draft.selected() < draft.lines().len().max(1));
                let _ = draft.program();
            }
        }

        // What a draft says is what the recipe does: reading its program back gives its lines.
        #[test]
        fn a_checked_program_has_one_step_per_line(clicks in prop::collection::vec(0u8..6, 0..60)) {
            let mut draft = RecipeDraft::new("t");
            for click in clicks {
                match click {
                    0 | 1 => { draft.add(); }
                    2 => { draft.flip_verb(); }
                    3 => { draft.cycle_item(); }
                    4 => { draft.nudge(1); }
                    _ => { draft.delete(); }
                }
            }
            if let Ok(program) = draft.program() {
                prop_assert_eq!(program.steps().len(), draft.lines().len());
            }
        }
    }
}
