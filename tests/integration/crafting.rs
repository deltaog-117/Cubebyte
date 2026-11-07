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

#[test]
fn a_recipe_typed_on_the_command_line_can_be_crafted() {
    use minecraft_clone::command::{parse::parse, plan::Plan, plan::plan};
    use minecraft_clone::crafting::custom::CustomRecipes;
    use minecraft_clone::scripting::model::Denied;
    use minecraft_clone::shared::voxel::WorldPos;

    let feet = WorldPos { x: 0, y: 0, z: 0 };
    let request = parse("recipe loaf take grass 3 make bread 1").unwrap();
    let Plan::Recipe {
        name,
        program: Some(program),
    } = plan(request, feet)
    else {
        panic!("a line with steps makes a recipe");
    };
    let mut recipes = CustomRecipes::default();
    let id = recipes.add(&name, program).unwrap();
    assert_eq!(recipes.name_of(id), Some("loaf"));

    let mut pack = Inventory::default();
    pack.add(ItemId::GRASS, 3).unwrap();
    let recipe = recipes.get(id).unwrap();
    let after = recipe
        .program()
        .run(&pack, feet, &mut |_| Err(Denied::NothingToDo))
        .unwrap();
    assert_eq!(after.count_of(ItemId::BREAD), 1);
    assert_eq!(after.count_of(ItemId::GRASS), 0);
    // The pack it ran on is untouched: a recipe works on a copy.
    assert_eq!(pack.count_of(ItemId::GRASS), 3);
}

#[test]
fn a_recipe_line_that_makes_worth_from_nothing_is_refused_before_it_exists() {
    use minecraft_clone::command::parse::parse;
    assert!(parse("recipe free make sword 1").is_err());
}
