//! The one table of recipes.
//!
//! A recipe is shapeless: it needs its inputs somewhere in the inventory and nothing about
//! where they sit. A recipe's ID is its position in [`RECIPES`], so a new recipe is only
//! ever appended.

use crate::items::registry::ItemId;

/// Identifies a recipe by its position in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecipeId(pub u8);

impl RecipeId {
    pub const FIRST: Self = Self(0);
}

/// What one craft costs and what it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub name: &'static str,
    pub inputs: &'static [(ItemId, u8)],
    pub output: (ItemId, u8),
}

const RECIPES: [Recipe; 2] = [
    Recipe {
        name: "bread",
        inputs: &[(ItemId::GRASS, 3)],
        output: (ItemId::BREAD, 1),
    },
    Recipe {
        name: "stew",
        inputs: &[(ItemId::GRASS, 4), (ItemId::DIRT, 2)],
        output: (ItemId::STEW, 2),
    },
];

/// The recipe `id` names, or `None` for an ID past the table.
pub fn get(id: RecipeId) -> Option<&'static Recipe> {
    RECIPES.get(usize::from(id.0))
}

/// The recipe after `id`, wrapping to the first.
pub fn next(id: RecipeId) -> RecipeId {
    match usize::from(id.0) + 1 {
        n if n < RECIPES.len() => RecipeId(n as u8),
        _ => RecipeId::FIRST,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::registry as items;

    #[test]
    fn every_recipe_uses_defined_items_and_positive_counts() {
        for recipe in &RECIPES {
            assert!(!recipe.inputs.is_empty(), "{} has no inputs", recipe.name);
            for &(item, count) in recipe.inputs.iter().chain([&recipe.output]) {
                assert!(
                    items::get(item).is_some(),
                    "{} uses item {}",
                    recipe.name,
                    item.0
                );
                assert!(count > 0, "{} has a zero count", recipe.name);
            }
        }
    }

    #[test]
    fn a_recipe_never_consumes_what_it_makes() {
        for recipe in &RECIPES {
            assert!(
                recipe
                    .inputs
                    .iter()
                    .all(|&(item, _)| item != recipe.output.0)
            );
        }
    }

    #[test]
    fn an_output_fits_in_one_stack() {
        for recipe in &RECIPES {
            let (item, count) = recipe.output;
            let limit = items::get(item).map_or(0, |def| def.stack_limit);
            assert!(count <= limit, "{} overfills a stack", recipe.name);
        }
    }

    #[test]
    fn next_walks_the_table_and_wraps() {
        let mut id = RecipeId::FIRST;
        for expected in 1..RECIPES.len() {
            id = next(id);
            assert_eq!(id, RecipeId(expected as u8));
        }
        assert_eq!(next(id), RecipeId::FIRST);
    }

    #[test]
    fn ids_past_the_table_are_not_recipes() {
        assert!(get(RecipeId(RECIPES.len() as u8)).is_none());
    }
}
