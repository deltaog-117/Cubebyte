//! Crafting: spend a recipe's inputs and add its output, or change nothing.

use thiserror::Error;

use crate::crafting::registry::{self, RecipeId};
use crate::inventory::model::{Inventory, NotEnough, UnknownItem};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CraftError {
    #[error("recipe {0} is not defined")]
    UnknownRecipe(u8),
    #[error("missing ingredients")]
    Missing(#[from] NotEnough),
    #[error("a recipe uses an undefined item")]
    Item(#[from] UnknownItem),
    #[error("the inventory has no room for the result")]
    Full,
}

/// Crafts `id` once from `inventory`.
///
/// The work happens on a copy and replaces the inventory only on success, so a refused
/// craft leaves the inventory exactly as it was.
///
/// # Errors
///
/// - [`CraftError::UnknownRecipe`] when `id` names no recipe.
/// - [`CraftError::Missing`] when an input is short.
/// - [`CraftError::Item`] when the recipe names an undefined item.
/// - [`CraftError::Full`] when the output does not fit once the inputs are spent.
pub fn craft(inventory: &mut Inventory, id: RecipeId) -> Result<(), CraftError> {
    let recipe = registry::get(id).ok_or(CraftError::UnknownRecipe(id.0))?;
    let mut after = inventory.clone();
    for &(item, count) in recipe.inputs {
        after.take(item, u32::from(count))?;
    }
    let (output, count) = recipe.output;
    if after.add(output, u32::from(count))? > 0 {
        return Err(CraftError::Full);
    }
    *inventory = after;
    Ok(())
}

/// Whether [`craft`] would succeed for `id` right now, without changing `inventory`.
pub fn can_craft(inventory: &Inventory, id: RecipeId) -> bool {
    craft(&mut inventory.clone(), id).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::model::SLOTS;
    use crate::items::registry::ItemId;

    const BREAD: RecipeId = RecipeId(0);

    #[test]
    fn crafting_spends_the_inputs_and_adds_the_output() {
        let mut inventory = Inventory::default();
        inventory.add(ItemId::GRASS, 7).unwrap();

        assert_eq!(craft(&mut inventory, BREAD), Ok(()));

        assert_eq!(inventory.count_of(ItemId::GRASS), 4);
        assert_eq!(inventory.count_of(ItemId::BREAD), 1);
    }

    #[test]
    fn a_short_input_changes_nothing() {
        let mut inventory = Inventory::default();
        inventory.add(ItemId::GRASS, 2).unwrap();
        let before = inventory.clone();

        assert_eq!(
            craft(&mut inventory, BREAD),
            Err(CraftError::Missing(NotEnough { held: 2, wanted: 3 }))
        );
        assert_eq!(inventory, before);
    }

    #[test]
    fn spending_the_last_input_frees_a_slot_for_the_output() {
        let mut inventory = Inventory::default();
        inventory
            .add(ItemId::STONE, (SLOTS as u32 - 1) * 64)
            .unwrap();
        inventory.add(ItemId::GRASS, 3).unwrap();

        assert_eq!(craft(&mut inventory, BREAD), Ok(()));
        assert_eq!(inventory.count_of(ItemId::BREAD), 1);
    }

    #[test]
    fn an_output_with_no_slot_to_land_in_is_refused() {
        let mut inventory = Inventory::default();
        inventory
            .add(ItemId::STONE, (SLOTS as u32 - 1) * 64)
            .unwrap();
        // 4 grass spend 3 and leave 1, so no slot is freed and bread has nowhere to go.
        inventory.add(ItemId::GRASS, 4).unwrap();
        let before = inventory.clone();

        assert_eq!(craft(&mut inventory, BREAD), Err(CraftError::Full));
        assert_eq!(inventory, before);
    }

    #[test]
    fn an_unknown_recipe_is_refused() {
        let mut inventory = Inventory::default();
        assert_eq!(
            craft(&mut inventory, RecipeId(200)),
            Err(CraftError::UnknownRecipe(200))
        );
    }

    #[test]
    fn can_craft_agrees_with_craft_and_changes_nothing() {
        let mut inventory = Inventory::default();
        assert!(!can_craft(&inventory, BREAD));
        inventory.add(ItemId::GRASS, 3).unwrap();
        let before = inventory.clone();

        assert!(can_craft(&inventory, BREAD));
        assert_eq!(inventory, before);
        assert!(!can_craft(&inventory, RecipeId(200)));
    }
}
