//! Shaped recipes: where the items sit matters, not only which items there are.
//!
//! A [`Pattern`] is the picture a recipe asks for, at most 3 by 3 cells, trimmed so it has no
//! empty border. A [`Grid`] is what the player has laid out. They match when the grid's own
//! trimmed picture equals the pattern or its left-right mirror, wherever it sits in the grid.
//! The cost of a shaped recipe is one item per filled cell, so [`program_for`] turns a
//! pattern and its output into an ordinary [`RecipeProgram`], with the same worth rule.

use thiserror::Error;

use crate::crafting::program::{RecipeError, RecipeProgram, Step};
use crate::items::registry::{self as items, ItemId};

/// Cells along one side of the grid.
pub const SIDE: usize = 3;

/// Cells in the grid.
pub const CELLS: usize = SIDE * SIDE;

/// Why a picture is not a pattern.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PatternError {
    #[error("a pattern needs at least one item")]
    Empty,
    #[error("a pattern fits in {SIDE} by {SIDE} cells")]
    TooBig,
    #[error("every row of a pattern must be as wide as the others")]
    Ragged,
    #[error("a pattern names an item that does not exist")]
    UnknownItem,
}

/// A checked picture: 1 to [`SIDE`] cells each way, at least one filled cell, and a filled
/// cell on every edge. The only way to get one is [`Pattern::try_new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    width: usize,
    cells: Vec<Option<ItemId>>,
}

impl Pattern {
    /// Checks `rows` and trims the empty rows and columns around the items.
    ///
    /// # Errors
    ///
    /// Returns a [`PatternError`] for no rows or no items, more than [`SIDE`] rows or columns,
    /// rows of different widths, or an item that does not exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use minecraft_clone::crafting::shaped::Pattern;
    /// use minecraft_clone::items::registry::ItemId;
    ///
    /// let rows = vec![vec![None, None], vec![None, Some(ItemId::STONE)]];
    /// let pattern = Pattern::try_new(&rows).unwrap();
    /// assert_eq!((pattern.width(), pattern.height()), (1, 1));
    /// ```
    pub fn try_new(rows: &[Vec<Option<ItemId>>]) -> Result<Self, PatternError> {
        let width = rows.first().map_or(0, Vec::len);
        if rows.is_empty() || width == 0 {
            return Err(PatternError::Empty);
        }
        if rows.len() > SIDE || width > SIDE {
            return Err(PatternError::TooBig);
        }
        if rows.iter().any(|row| row.len() != width) {
            return Err(PatternError::Ragged);
        }
        let filled = |item: &Option<ItemId>| item.is_some();
        if rows
            .iter()
            .flatten()
            .flatten()
            .any(|&item| items::get(item).is_none())
        {
            return Err(PatternError::UnknownItem);
        }
        let used_rows: Vec<usize> = (0..rows.len())
            .filter(|&y| rows[y].iter().any(filled))
            .collect();
        let used_columns: Vec<usize> = (0..width)
            .filter(|&x| rows.iter().any(|row| filled(&row[x])))
            .collect();
        let (Some(&top), Some(&bottom), Some(&left), Some(&right)) = (
            used_rows.first(),
            used_rows.last(),
            used_columns.first(),
            used_columns.last(),
        ) else {
            return Err(PatternError::Empty);
        };
        let cells = rows[top..=bottom]
            .iter()
            .flat_map(|row| row[left..=right].iter().copied())
            .collect();
        Ok(Self {
            width: right - left + 1,
            cells,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.cells.len() / self.width
    }

    /// The rows, top first.
    pub fn rows(&self) -> impl Iterator<Item = &[Option<ItemId>]> {
        self.cells.chunks(self.width)
    }

    /// How many of each item the pattern asks for, in the order the items first appear.
    pub fn needs(&self) -> Vec<(ItemId, u8)> {
        let mut needs: Vec<(ItemId, u8)> = Vec::new();
        for item in self.cells.iter().flatten() {
            match needs.iter_mut().find(|(seen, _)| seen == item) {
                Some((_, count)) => *count = count.saturating_add(1),
                None => needs.push((*item, 1)),
            }
        }
        needs
    }

    /// The picture reflected left to right.
    pub fn mirrored(&self) -> Self {
        let cells = self
            .rows()
            .flat_map(|row| row.iter().rev().copied())
            .collect();
        Self {
            width: self.width,
            cells,
        }
    }

    /// Whether a grid showing `other` makes this recipe: the same picture or its mirror.
    pub fn accepts(&self, other: &Self) -> bool {
        self == other || self.mirrored() == *other
    }
}

/// What the player has laid out: [`CELLS`] cells, row by row from the top left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grid {
    cells: [Option<ItemId>; CELLS],
}

impl Grid {
    /// The item in cell `index`, or `None` for an empty cell or one past the grid.
    pub fn get(&self, index: usize) -> Option<ItemId> {
        self.cells.get(index).copied().flatten()
    }

    /// Gives cell `index` the next item, and after the last item makes it empty again.
    /// Reports whether the cell exists.
    pub fn cycle(&mut self, index: usize) -> bool {
        let Some(cell) = self.cells.get_mut(index) else {
            return false;
        };
        *cell = match *cell {
            None => items::ids().next(),
            Some(item) => items::ids().find(|candidate| candidate.0 > item.0),
        };
        true
    }

    pub fn clear(&mut self) {
        self.cells = [None; CELLS];
    }

    /// The picture on the grid, trimmed, or `None` when the grid is empty.
    pub fn pattern(&self) -> Option<Pattern> {
        let rows: Vec<Vec<Option<ItemId>>> = self.cells.chunks(SIDE).map(<[_]>::to_vec).collect();
        Pattern::try_new(&rows).ok()
    }
}

/// The recipe that spends one item per filled cell of `pattern` and makes `makes`.
///
/// # Errors
///
/// Returns the [`RecipeError`] that an ordinary recipe of the same steps would, such as
/// making more worth than the pattern takes, or making nothing.
pub fn program_for(
    pattern: &Pattern,
    makes: &[(ItemId, u8)],
) -> Result<RecipeProgram, RecipeError> {
    let steps = pattern
        .needs()
        .into_iter()
        .map(|(item, count)| Step::Take { item, count })
        .chain(
            makes
                .iter()
                .map(|&(item, count)| Step::Make { item, count }),
        )
        .collect();
    RecipeProgram::try_new(steps)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const S: Option<ItemId> = Some(ItemId::STONE);
    const D: Option<ItemId> = Some(ItemId::DIRT);
    const N: Option<ItemId> = None;

    fn pattern(rows: &[&[Option<ItemId>]]) -> Pattern {
        let rows: Vec<_> = rows.iter().map(|row| row.to_vec()).collect();
        Pattern::try_new(&rows).unwrap()
    }

    #[test]
    fn empty_borders_are_trimmed() {
        let loose = pattern(&[&[N, N, N], &[N, S, N], &[N, S, S]]);
        assert_eq!(loose, pattern(&[&[S, N], &[S, S]]));
        assert_eq!((loose.width(), loose.height()), (2, 2));
    }

    #[test]
    fn bad_pictures_are_refused() {
        let row = |n: usize| vec![S; n];
        assert_eq!(Pattern::try_new(&[]), Err(PatternError::Empty));
        assert_eq!(Pattern::try_new(&[vec![N, N]]), Err(PatternError::Empty));
        assert_eq!(Pattern::try_new(&[row(4)]), Err(PatternError::TooBig));
        assert_eq!(
            Pattern::try_new(&[row(1), row(1), row(1), row(1)]),
            Err(PatternError::TooBig)
        );
        assert_eq!(
            Pattern::try_new(&[row(2), row(1)]),
            Err(PatternError::Ragged)
        );
        assert_eq!(
            Pattern::try_new(&[vec![Some(ItemId(99))]]),
            Err(PatternError::UnknownItem)
        );
    }

    #[test]
    fn a_pattern_matches_itself_and_its_mirror_but_not_a_rotation() {
        let stairs = pattern(&[&[S, N], &[S, S]]);
        assert!(stairs.accepts(&pattern(&[&[S, N], &[S, S]])));
        assert!(stairs.accepts(&pattern(&[&[N, S], &[S, S]])));
        assert!(!stairs.accepts(&pattern(&[&[S, S], &[S, N]])));
        assert!(!stairs.accepts(&pattern(&[&[S, N], &[S, D]])));
    }

    #[test]
    fn needs_counts_each_item_once_in_order() {
        let shape = pattern(&[&[S, D], &[S, S]]);
        assert_eq!(shape.needs(), vec![(ItemId::STONE, 3), (ItemId::DIRT, 1)]);
    }

    #[test]
    fn a_cell_cycles_through_every_item_and_back_to_empty() {
        let mut grid = Grid::default();
        let mut seen = Vec::new();
        for _ in 0..=items::ids().count() {
            grid.cycle(4);
            seen.push(grid.get(4));
        }
        assert_eq!(seen.first(), Some(&Some(ItemId::STONE)));
        assert_eq!(seen.last(), Some(&None));
        assert!(!grid.cycle(CELLS));
    }

    #[test]
    fn a_fair_pattern_becomes_a_recipe_and_an_unfair_one_is_refused() {
        let shape = pattern(&[&[S, S], &[D, N]]);
        assert!(program_for(&shape, &[(ItemId::SWORD, 1)]).is_err());
        assert!(program_for(&shape, &[(ItemId::FUEL, 1)]).is_ok());
    }

    proptest! {
        #[test]
        fn where_a_picture_sits_in_the_grid_does_not_change_the_pattern(
            dx in 0usize..2,
            dy in 0usize..2,
        ) {
            let figure = [(0usize, 0usize, ItemId::STONE), (1, 0, ItemId::DIRT), (1, 1, ItemId::STONE)];
            let mut grid = Grid::default();
            for (x, y, item) in figure {
                grid.cells[(y + dy) * SIDE + x + dx] = Some(item);
            }
            let mut home = Grid::default();
            for (x, y, item) in figure {
                home.cells[y * SIDE + x] = Some(item);
            }
            prop_assert_eq!(grid.pattern(), home.pattern());
        }

        #[test]
        fn mirroring_twice_gives_the_pattern_back(cells in proptest::collection::vec(0u16..4, CELLS)) {
            let mut grid = Grid::default();
            for (index, item) in cells.into_iter().enumerate() {
                grid.cells[index] = (item > 0).then_some(ItemId(item));
            }
            if let Some(pattern) = grid.pattern() {
                prop_assert_eq!(pattern.mirrored().mirrored(), pattern.clone());
                let filled = grid.cells.iter().flatten().count();
                prop_assert_eq!(
                    pattern.needs().iter().map(|&(_, n)| usize::from(n)).sum::<usize>(),
                    filled
                );
            }
        }
    }
}
