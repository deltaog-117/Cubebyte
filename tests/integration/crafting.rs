use minecraft_clone::crafting::registry::{RecipeId, get};
use minecraft_clone::crafting::service::craft;
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::inventory::persist::{decode, encode};
use minecraft_clone::items::registry::{ItemId, ids};
use minecraft_clone::survival::model::Stats;
use minecraft_clone::survival::service::eat;
use proptest::prelude::*;

fn any_item() -> impl Strategy<Value = ItemId> {
    (1u16..=6).prop_map(ItemId)
}

proptest! {
    // A craft either changes nothing or moves counts by exactly the recipe, and the
    // result survives a save and load.
    #[test]
    fn crafting_is_all_or_nothing(
        stock in prop::collection::vec((any_item(), 0u32..120), 0..10),
        picks in prop::collection::vec(0u8..4, 0..20),
    ) {
        let mut inventory = Inventory::default();
        for (item, count) in stock {
            inventory.add(item, count).unwrap();
        }
        for pick in picks {
            let id = RecipeId(pick);
            let before = inventory.clone();
            match (craft(&mut inventory, id), get(id)) {
                (Ok(()), Some(recipe)) => {
                    for item in ids() {
                        let spent: u32 = recipe.inputs.iter()
                            .filter(|&&(i, _)| i == item).map(|&(_, n)| u32::from(n)).sum();
                        let made = if recipe.output.0 == item { u32::from(recipe.output.1) } else { 0 };
                        prop_assert_eq!(inventory.count_of(item), before.count_of(item) - spent + made);
                    }
                }
                (Err(_), _) => prop_assert_eq!(&inventory, &before),
                (Ok(()), None) => prop_assert!(false, "crafted a recipe that does not exist"),
            }
        }
        prop_assert_eq!(decode(&encode(&inventory)), Ok(inventory));
    }
}

#[test]
fn grass_becomes_bread_which_refills_hunger() {
    let mut inventory = Inventory::default();
    let mut stats = Stats::default();
    for _ in 0..600 * 10 {
        stats.step();
    }
    let hungry = stats.hunger().value();
    inventory.add(ItemId::GRASS, 3).unwrap();

    assert_eq!(craft(&mut inventory, RecipeId::FIRST), Ok(()));
    assert_eq!(eat(&mut inventory, &mut stats), Ok(ItemId::BREAD));

    assert_eq!(stats.hunger().value(), hungry + 4);
    assert!(inventory.is_empty());
}
