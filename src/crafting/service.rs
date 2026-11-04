//! Crafting: spend a recipe's inputs and add its output, or change nothing.

use thiserror::Error;

use crate::crafting::custom::CustomRecipes;
use crate::crafting::program::RunError;
use crate::crafting::registry::{self, RecipeId};
use crate::events::apply::slot_events;
use crate::inventory::model::{Inventory, NotEnough, UnknownItem};
use crate::scripting::model::{Answer, Denied, Query};
use crate::shared::event::Event;
use crate::shared::voxel::WorldPos;

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
    #[error("the recipe could not run: {0}")]
    Recipe(#[from] RunError),
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
    *inventory = crafted(inventory, id)?;
    Ok(())
}

/// The slot events that crafting `id` once from `inventory` would apply, without changing it.
///
/// Applying them through [`crate::events::apply::apply_all`] gives the same inventory as
/// [`craft`].
///
/// # Errors
///
/// The same as [`craft`].
pub fn craft_events(inventory: &Inventory, id: RecipeId) -> Result<Vec<Event>, CraftError> {
    Ok(slot_events(inventory, &crafted(inventory, id)?))
}

/// The slot events that crafting the player recipe `id` once would apply, without changing
/// `inventory`. `cell` is where the recipe's `look` reads count from (the player's feet), and
/// `ask` answers the reads the pack cannot.
///
/// # Errors
///
/// - [`CraftError::UnknownRecipe`] when `id` names no player recipe.
/// - [`CraftError::Recipe`] when the recipe cannot run (a short input, no room, a refused read).
pub fn custom_craft_events(
    custom: &CustomRecipes,
    inventory: &Inventory,
    id: RecipeId,
    cell: WorldPos,
    ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
) -> Result<Vec<Event>, CraftError> {
    let recipe = custom.get(id).ok_or(CraftError::UnknownRecipe(id.0))?;
    let after = recipe.program().run(inventory, cell, ask)?;
    Ok(slot_events(inventory, &after))
}

/// `inventory` after one craft of `id`, or why it cannot be done.
fn crafted(inventory: &Inventory, id: RecipeId) -> Result<Inventory, CraftError> {
    let recipe = registry::get(id).ok_or(CraftError::UnknownRecipe(id.0))?;
    let mut after = inventory.clone();
    for &(item, count) in recipe.inputs {
        after.take(item, u32::from(count))?;
    }
    let (output, count) = recipe.output;
    if after.add(output, u32::from(count))? > 0 {
        return Err(CraftError::Full);
    }
    Ok(after)
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
    fn a_player_recipe_crafts_through_the_same_slot_events() {
        use crate::crafting::program::{RecipeProgram, Step};

        let mut custom = CustomRecipes::default();
        let id = custom
            .add(
                "loaf",
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
                .unwrap(),
            )
            .unwrap();
        let mut inventory = Inventory::default();
        inventory.add(ItemId::GRASS, 4).unwrap();
        let origin = WorldPos { x: 0, y: 0, z: 0 };
        let mut no_world = |_| Err(Denied::NothingToDo);

        let events = custom_craft_events(&custom, &inventory, id, origin, &mut no_world).unwrap();
        let mut expected = inventory.clone();
        expected.take(ItemId::GRASS, 3).unwrap();
        expected.add(ItemId::BREAD, 1).unwrap();
        assert_eq!(events, slot_events(&inventory, &expected));
        assert!(matches!(
            custom_craft_events(&custom, &Inventory::default(), id, origin, &mut no_world),
            Err(CraftError::Recipe(RunError::Missing(_)))
        ));
        assert_eq!(
            custom_craft_events(&custom, &inventory, RecipeId(0), origin, &mut no_world),
            Err(CraftError::UnknownRecipe(0))
        );
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
