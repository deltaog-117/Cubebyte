//! The recipes a player has written, after the built-in table.
//!
//! A player recipe gets the ID that follows the last built-in recipe plus its position in the
//! list, so the craft key, the panel and the intent tracks all name it the way they name a
//! built-in one. Recipes are loaded once at startup and fixed for the session, which is why a
//! recorded craft means the same recipe when it is replayed (the file is copied into the
//! session's starting world).

use std::sync::LazyLock;

use thiserror::Error;

use crate::crafting::program::RecipeProgram;
use crate::crafting::registry::{self, RecipeId};
use crate::crafting::shaped::{Pattern, program_for};
use crate::items::registry::ItemId;

/// Most player recipes at once.
pub const MAX_CUSTOM: usize = 16;

/// Longest recipe name, in characters.
pub const MAX_NAME: usize = 16;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CustomError {
    #[error("at most {MAX_CUSTOM} recipes of your own")]
    Full,
    #[error("a recipe name is 1 to {MAX_NAME} letters, digits, - or _ (lower case)")]
    BadName,
    #[error("a recipe called '{0}' already exists")]
    Taken(String),
}

/// The number of the first built-in shaped recipe. These sit at the top of the numbers, far
/// from the table's and the player's, so adding one never changes the number a recording
/// names for a recipe that already existed.
pub const SHAPED_FIRST: u8 = 200;

const _: () = assert!(
    registry::RECIPE_COUNT + MAX_CUSTOM < SHAPED_FIRST as usize,
    "player recipe numbers must stay below the built-in shaped ones"
);

/// One recipe of the player's, or a built-in shaped one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomRecipe {
    name: String,
    /// The picture the grid must show, for a shaped recipe. A shaped recipe is crafted only
    /// from the grid, so the crafting panel and the craft key leave it out.
    pattern: Option<Pattern>,
    program: RecipeProgram,
}

/// Built-in shaped recipes: a name, rows of items and what they make.
type ShapedDef = (
    &'static str,
    &'static [&'static [Option<ItemId>]],
    (ItemId, u8),
);

const SHAPED: [ShapedDef; 1] = [(
    "chisel",
    &[
        &[Some(ItemId::STONE), Some(ItemId::STONE)],
        &[Some(ItemId::DIRT), None],
    ],
    (ItemId::CHISEL, 1),
)];

// A definition that fails its checks is left out; a test proves none is.
static BUILTIN_SHAPED: LazyLock<Vec<CustomRecipe>> = LazyLock::new(|| {
    SHAPED
        .iter()
        .filter_map(|&(name, rows, output)| {
            let rows: Vec<Vec<Option<ItemId>>> = rows.iter().map(|row| row.to_vec()).collect();
            let pattern = Pattern::try_new(&rows).ok()?;
            let program = program_for(&pattern, &[output]).ok()?;
            Some(CustomRecipe {
                name: name.to_owned(),
                pattern: Some(pattern),
                program,
            })
        })
        .collect()
});

impl CustomRecipe {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The picture the grid must show, or `None` for a recipe crafted from the panel.
    pub fn pattern(&self) -> Option<&Pattern> {
        self.pattern.as_ref()
    }

    pub fn program(&self) -> &RecipeProgram {
        &self.program
    }
}

/// The player's recipes, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomRecipes {
    list: Vec<CustomRecipe>,
}

/// The ID of the first player recipe: the one after the last built-in.
fn first_id() -> u8 {
    u8::try_from(registry::ids().count()).unwrap_or(u8::MAX)
}

/// Whether `name` is a name a recipe may have: 1 to [`MAX_NAME`] lower case letters, digits, `-`
/// or `_`. It says nothing about whether the name is taken.
pub fn name_is_valid(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl CustomRecipes {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Every recipe with its ID, in order.
    pub fn iter(&self) -> impl Iterator<Item = (RecipeId, &CustomRecipe)> {
        let first = first_id();
        self.list
            .iter()
            .enumerate()
            .filter_map(move |(index, recipe)| {
                let id = u8::try_from(index).ok()?.checked_add(first)?;
                Some((RecipeId(id), recipe))
            })
    }

    /// The recipe after `id` across the built-in table and then the player's recipes, wrapping
    /// to the first.
    pub fn next(&self, id: RecipeId) -> RecipeId {
        let total = usize::from(first_id()) + self.list.len();
        // A shaped recipe is skipped: the craft key would bypass its picture.
        (1..=total)
            .map(|step| (usize::from(id.0) + step) % total)
            .filter_map(|n| u8::try_from(n).ok().map(RecipeId))
            .find(|&candidate| {
                self.get(candidate)
                    .is_none_or(|recipe| recipe.pattern.is_none())
            })
            .unwrap_or(RecipeId::FIRST)
    }

    /// The name of recipe `id`, built-in or the player's.
    pub fn name_of(&self, id: RecipeId) -> Option<&str> {
        registry::get(id)
            .map(|recipe| recipe.name)
            .or_else(|| self.get(id).map(CustomRecipe::name))
    }

    /// The recipe `id` names, or `None` for a table recipe or an ID past the lists. Built-in
    /// shaped recipes answer too.
    pub fn get(&self, id: RecipeId) -> Option<&CustomRecipe> {
        match id.0.checked_sub(SHAPED_FIRST) {
            Some(index) => BUILTIN_SHAPED.get(usize::from(index)),
            None => self.list.get(usize::from(id.0.checked_sub(first_id())?)),
        }
    }

    /// The recipe whose picture the grid shows, built-in ones first.
    pub fn find_shaped(&self, shown: &Pattern) -> Option<(RecipeId, &CustomRecipe)> {
        let built_in = BUILTIN_SHAPED
            .iter()
            .enumerate()
            .filter_map(|(index, recipe)| {
                let id = SHAPED_FIRST.checked_add(u8::try_from(index).ok()?)?;
                Some((RecipeId(id), recipe))
            });
        built_in.chain(self.iter()).find(|(_, recipe)| {
            recipe
                .pattern
                .as_ref()
                .is_some_and(|pattern| pattern.accepts(shown))
        })
    }

    /// A valid name no recipe has yet: `mine`, then `mine2`, `mine3` and so on, or `None` when
    /// all [`MAX_CUSTOM`] places are taken.
    pub fn suggest_name(&self) -> Option<String> {
        if self.list.len() >= MAX_CUSTOM {
            return None;
        }
        (1..=MAX_CUSTOM + 1)
            .map(|n| {
                if n == 1 {
                    "mine".to_owned()
                } else {
                    format!("mine{n}")
                }
            })
            .find(|name| self.is_free(name))
    }

    /// Whether no built-in recipe and no recipe of the player's has `name`.
    pub fn is_free(&self, name: &str) -> bool {
        !registry::ids()
            .filter_map(registry::get)
            .any(|recipe| recipe.name == name)
            && !self.list.iter().any(|recipe| recipe.name == name)
            && !BUILTIN_SHAPED.iter().any(|recipe| recipe.name == name)
    }

    /// Adds a recipe and returns its ID.
    ///
    /// # Errors
    ///
    /// - [`CustomError::Full`] when [`MAX_CUSTOM`] recipes exist.
    /// - [`CustomError::BadName`] for a name outside the allowed letters or length.
    /// - [`CustomError::Taken`] when a built-in or an earlier recipe has the name.
    pub fn add(&mut self, name: &str, program: RecipeProgram) -> Result<RecipeId, CustomError> {
        self.push(name, None, program)
    }

    /// Adds a shaped recipe, crafted from the grid, and returns its ID. `program` is what
    /// crafting it does; [`program_for`] makes the program that spends the pattern's items.
    ///
    /// # Errors
    ///
    /// The same as [`Self::add`].
    pub fn add_shaped(
        &mut self,
        name: &str,
        pattern: Pattern,
        program: RecipeProgram,
    ) -> Result<RecipeId, CustomError> {
        self.push(name, Some(pattern), program)
    }

    fn push(
        &mut self,
        name: &str,
        pattern: Option<Pattern>,
        program: RecipeProgram,
    ) -> Result<RecipeId, CustomError> {
        if !name_is_valid(name) {
            return Err(CustomError::BadName);
        }
        if !self.is_free(name) {
            return Err(CustomError::Taken(name.to_owned()));
        }
        if self.list.len() >= MAX_CUSTOM {
            return Err(CustomError::Full);
        }
        self.list.push(CustomRecipe {
            name: name.to_owned(),
            pattern,
            program,
        });
        Ok(RecipeId(first_id() + (self.list.len() - 1) as u8))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_built_in_shaped_recipe_is_valid_and_fair() {
        assert_eq!(super::BUILTIN_SHAPED.len(), super::SHAPED.len());
    }

    use super::*;
    use crate::crafting::program::Step;
    use crate::items::registry::ItemId;

    fn program() -> RecipeProgram {
        RecipeProgram::try_new(vec![
            Step::Take {
                item: ItemId::GRASS,
                count: 3,
            },
            Step::Make {
                item: ItemId::BREAD,
                count: 1,
            },
        ])
        .unwrap()
    }

    #[test]
    fn ids_follow_the_built_in_table_in_order() {
        let mut recipes = CustomRecipes::default();
        let a = recipes.add("one", program()).unwrap();
        let b = recipes.add("two", program()).unwrap();
        assert_eq!(a.0, first_id());
        assert_eq!(b.0, first_id() + 1);
        assert_eq!(recipes.get(b).map(CustomRecipe::name), Some("two"));
        let listed: Vec<_> = recipes.iter().map(|(id, _)| id).collect();
        assert_eq!(listed, [a, b]);
    }

    #[test]
    fn next_walks_the_built_ins_then_the_players_recipes_and_wraps() {
        let mut recipes = CustomRecipes::default();
        let last_built_in = RecipeId(first_id() - 1);
        assert_eq!(recipes.next(last_built_in), RecipeId::FIRST);
        let mine = recipes.add("mine", program()).unwrap();
        assert_eq!(recipes.next(last_built_in), mine);
        assert_eq!(recipes.next(mine), RecipeId::FIRST);
        assert_eq!(recipes.name_of(mine), Some("mine"));
        assert_eq!(recipes.name_of(RecipeId::FIRST), Some("bread"));
        assert_eq!(recipes.name_of(RecipeId(250)), None);
    }

    #[test]
    fn the_chisel_is_a_shaped_recipe_found_by_its_picture_and_its_mirror() {
        let recipes = CustomRecipes::default();
        let chisel = RecipeId(SHAPED_FIRST);
        assert_eq!(recipes.name_of(chisel), Some("chisel"));
        let shown = |rows: &[&[Option<ItemId>]]| {
            let rows: Vec<_> = rows.iter().map(|row| row.to_vec()).collect();
            Pattern::try_new(&rows).unwrap()
        };
        let (s, d, n) = (Some(ItemId::STONE), Some(ItemId::DIRT), None);
        assert_eq!(
            recipes
                .find_shaped(&shown(&[&[s, s], &[d, n]]))
                .map(|(id, _)| id),
            Some(chisel)
        );
        assert_eq!(
            recipes
                .find_shaped(&shown(&[&[s, s], &[n, d]]))
                .map(|(id, _)| id),
            Some(chisel)
        );
        assert!(recipes.find_shaped(&shown(&[&[s, s], &[n, n]])).is_none());
    }

    #[test]
    fn the_craft_key_skips_a_shaped_player_recipe() {
        let mut recipes = CustomRecipes::default();
        let pattern = Pattern::try_new(&[vec![Some(ItemId::STONE), Some(ItemId::STONE)]]).unwrap();
        let shaped = recipes
            .add_shaped(
                "pair",
                pattern.clone(),
                program_for(&pattern, &[(ItemId::FUEL, 1)]).unwrap(),
            )
            .unwrap();
        let last_built_in = RecipeId(first_id() - 1);
        assert_eq!(recipes.next(last_built_in), RecipeId::FIRST);
        assert!(recipes.get(shaped).is_some_and(|r| r.pattern().is_some()));
    }

    #[test]
    fn a_built_in_id_or_one_past_the_end_is_not_a_player_recipe() {
        let mut recipes = CustomRecipes::default();
        recipes.add("one", program()).unwrap();
        assert!(recipes.get(RecipeId::FIRST).is_none());
        assert!(recipes.get(RecipeId(first_id() + 1)).is_none());
    }

    #[test]
    fn names_must_be_new_and_tidy() {
        let mut recipes = CustomRecipes::default();
        assert_eq!(recipes.add("Bread", program()), Err(CustomError::BadName));
        assert_eq!(recipes.add("", program()), Err(CustomError::BadName));
        assert_eq!(
            recipes.add("a-name-that-is-too-long", program()),
            Err(CustomError::BadName)
        );
        assert_eq!(
            recipes.add("bread", program()),
            Err(CustomError::Taken("bread".to_owned()))
        );
        recipes.add("mine", program()).unwrap();
        assert_eq!(
            recipes.add("mine", program()),
            Err(CustomError::Taken("mine".to_owned()))
        );
    }

    #[test]
    fn a_suggested_name_is_valid_and_free() {
        let mut recipes = CustomRecipes::default();
        assert_eq!(recipes.suggest_name().as_deref(), Some("mine"));
        for _ in 0..MAX_CUSTOM {
            let name = recipes.suggest_name().unwrap();
            assert!(name_is_valid(&name));
            recipes.add(&name, program()).unwrap();
        }
        assert_eq!(recipes.suggest_name(), None);
    }

    #[test]
    fn there_is_a_limit() {
        let mut recipes = CustomRecipes::default();
        for index in 0..MAX_CUSTOM {
            recipes.add(&format!("r{index}"), program()).unwrap();
        }
        assert_eq!(recipes.add("extra", program()), Err(CustomError::Full));
    }
}
