use minecraft_clone::crafting::registry;
use minecraft_clone::crafting::service::craft;
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::ui::draw::overlay;
use minecraft_clone::ui::layout::panel;

#[test]
fn clicking_the_sword_row_crafts_a_sword_and_greys_the_row_out() {
    let size = (1280, 720);
    let sword = registry::ids()
        .find(|&id| registry::get(id).is_some_and(|recipe| recipe.name == "sword"))
        .unwrap();
    let mut inventory = Inventory::default();
    inventory.add(ItemId::STONE, 6).unwrap();
    inventory.add(ItemId::DIRT, 2).unwrap();

    let before = panel(size, &inventory);
    let row = before.rows.iter().find(|r| r.recipe == sword).unwrap();
    assert!(row.affordable);
    let (x, y) = (
        row.rect.x + row.rect.width / 2.0,
        row.rect.y + row.rect.height / 2.0,
    );
    let clicked = before.row_at(x, y).unwrap();
    assert_eq!(craft(&mut inventory, clicked), Ok(()));
    assert_eq!(inventory.count_of(ItemId::SWORD), 1);

    let after = panel(size, &inventory);
    assert!(after.rows.iter().all(|r| !r.affordable));
    let (vertices, indices) = overlay(&after, size, Some(clicked));
    assert!(!vertices.is_empty() && indices.len() % 3 == 0);
}
