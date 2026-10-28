//! Where the crafting panel and its rows sit on screen, in pixels.
//!
//! Pure arithmetic with no GPU, so placement and click-testing are tested directly.

use std::fmt::Write;

use crate::crafting::registry::{self, RecipeId};
use crate::crafting::service::can_craft;
use crate::inventory::model::Inventory;
use crate::items::registry as items;
use crate::ui::font::{GLYPH_HEIGHT, GLYPH_WIDTH};

const TITLE: &str = "crafting";
const HINT: &str = "click to craft - tab closes";

/// Window height in pixels per step of text scale: 720 high gives 3 pixels per font pixel.
const PIXELS_PER_SCALE: u32 = 200;

/// Space around the panel's contents, and inside each row, in font pixels.
const MARGIN: u32 = 4;
const ROW_PAD: u32 = 2;

/// A rectangle in window pixels, with the origin at the top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Whether the point is inside; the left and top edges count, the right and bottom do not.
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// A line of text and where its top left corner goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
}

/// One clickable recipe.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub recipe: RecipeId,
    pub rect: Rect,
    /// Where the row's text goes, inside `rect`.
    pub label: Label,
    /// Whether a click would craft it right now.
    pub affordable: bool,
}

/// The whole panel for one window size and one inventory.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    /// Screen pixels per font pixel.
    pub scale: u32,
    pub rect: Rect,
    pub title: Label,
    pub hint: Label,
    pub rows: Vec<Row>,
}

impl Panel {
    /// The recipe whose row is under the point, if any.
    pub fn row_at(&self, x: f32, y: f32) -> Option<RecipeId> {
        self.rows
            .iter()
            .find(|row| row.rect.contains(x, y))
            .map(|row| row.recipe)
    }
}

/// "sword - 6 stone, 2 dirt", with " x2" after the name when a craft makes several.
fn describe(id: RecipeId) -> Option<String> {
    let recipe = registry::get(id)?;
    let mut text = recipe.name.to_string();
    if recipe.output.1 > 1 {
        let _ = write!(text, " x{}", recipe.output.1);
    }
    text.push_str(" - ");
    for (index, &(item, count)) in recipe.inputs.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        let name = items::get(item).map_or("?", |def| def.name);
        let _ = write!(text, "{count} {name}");
    }
    Some(text)
}

/// Lays the panel out centred in a window of `size` pixels, one row per recipe.
pub fn panel((width, height): (u32, u32), inventory: &Inventory) -> Panel {
    let described: Vec<_> = registry::ids()
        .filter_map(|id| Some((id, describe(id)?)))
        .collect();
    let widest = described
        .iter()
        .map(|(_, text)| text.chars().count())
        .chain([TITLE, HINT].map(|text| text.chars().count()))
        .max()
        .unwrap_or(0);
    // Big windows get big text, but never so big that the widest line leaves the window.
    let panel_units = widest as u32 * (GLYPH_WIDTH + 1) + 2 * ROW_PAD + 2 * MARGIN;
    let scale = (height / PIXELS_PER_SCALE).min(width / panel_units).max(1);
    let unit = scale as f32;
    let advance = (GLYPH_WIDTH + 1) as f32 * unit;
    let text_height = GLYPH_HEIGHT as f32 * unit;
    let margin = MARGIN as f32 * unit;
    let row_pad = ROW_PAD as f32 * unit;
    let gap = 2.0 * unit;

    let row_width = widest as f32 * advance + 2.0 * row_pad;
    let row_height = text_height + 2.0 * row_pad;
    let panel_width = row_width + 2.0 * margin;

    // Lay out from the top of the panel, then move everything to the panel's place.
    let mut y = margin;
    let title_y = y;
    y += text_height + margin;
    let mut rows = Vec::with_capacity(described.len());
    for (recipe, text) in described {
        let rect = Rect {
            x: margin,
            y,
            width: row_width,
            height: row_height,
        };
        rows.push(Row {
            recipe,
            rect,
            label: Label {
                x: rect.x + row_pad,
                y: rect.y + row_pad,
                text,
            },
            affordable: can_craft(inventory, recipe),
        });
        y += row_height + gap;
    }
    let hint_y = y;
    let panel_height = y + text_height + margin;

    let origin_x = ((width as f32 - panel_width) / 2.0).max(0.0);
    let origin_y = ((height as f32 - panel_height) / 2.0).max(0.0);
    let moved = |rect: Rect| Rect {
        x: rect.x + origin_x,
        y: rect.y + origin_y,
        ..rect
    };
    let label = |x: f32, y: f32, text: &str| Label {
        x: x + origin_x,
        y: y + origin_y,
        text: text.to_string(),
    };
    for row in &mut rows {
        row.rect = moved(row.rect);
        row.label = label(row.label.x, row.label.y, &row.label.text);
    }
    Panel {
        scale,
        rect: Rect {
            x: origin_x,
            y: origin_y,
            width: panel_width,
            height: panel_height,
        },
        title: label(margin, title_y, TITLE),
        hint: label(margin, hint_y, HINT),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::registry::ItemId;
    use proptest::prelude::*;

    fn size() -> impl Strategy<Value = (u32, u32)> {
        (320u32..4096, 240u32..2160)
    }

    #[test]
    fn a_row_reads_as_its_name_output_and_inputs() {
        let sword =
            registry::ids().find(|&id| registry::get(id).is_some_and(|r| r.name == "sword"));
        assert_eq!(
            describe(sword.expect("sword recipe exists")).as_deref(),
            Some("sword - 6 stone, 2 dirt")
        );
        let stew = registry::ids().find(|&id| registry::get(id).is_some_and(|r| r.name == "stew"));
        assert_eq!(
            describe(stew.expect("stew recipe exists")).as_deref(),
            Some("stew x2 - 4 grass, 2 dirt")
        );
        assert_eq!(describe(RecipeId(200)), None);
    }

    #[test]
    fn only_rows_that_can_be_crafted_are_marked_affordable() {
        let mut inventory = Inventory::default();
        inventory.add(ItemId::STONE, 6).unwrap();
        inventory.add(ItemId::DIRT, 2).unwrap();
        let panel = panel((1280, 720), &inventory);

        for row in &panel.rows {
            let name = registry::get(row.recipe).map(|r| r.name);
            assert_eq!(row.affordable, name == Some("sword"), "{name:?}");
        }
    }

    proptest! {
        // Every recipe has a row, rows sit inside the panel and the window, and never overlap.
        #[test]
        fn rows_fit_without_overlapping(size in size()) {
            let panel = panel(size, &Inventory::default());
            prop_assert_eq!(panel.rows.len(), registry::ids().count());
            let (width, height) = (size.0 as f32, size.1 as f32);
            prop_assert!(panel.rect.x >= 0.0 && panel.rect.x + panel.rect.width <= width);
            prop_assert!(panel.rect.y >= 0.0 && panel.rect.y + panel.rect.height <= height);
            for pair in panel.rows.windows(2) {
                prop_assert!(pair[0].rect.y + pair[0].rect.height <= pair[1].rect.y);
            }
            for row in &panel.rows {
                let (r, p) = (row.rect, panel.rect);
                prop_assert!(r.x >= p.x && r.x + r.width <= p.x + p.width);
                prop_assert!(r.y >= p.y && r.y + r.height <= p.y + p.height);
            }
        }

        // A click anywhere in a row finds that row's recipe; a click off every row finds none.
        #[test]
        fn clicks_find_the_row_under_them(size in size(), fx in 0.0f32..1.0, fy in 0.0f32..1.0) {
            let panel = panel(size, &Inventory::default());
            for row in &panel.rows {
                let (x, y) = (row.rect.x + fx * row.rect.width, row.rect.y + fy * row.rect.height);
                prop_assert_eq!(panel.row_at(x, y), Some(row.recipe));
            }
            prop_assert_eq!(panel.row_at(-1.0, -1.0), None);
            let below = panel.rect.y + panel.rect.height + 1.0;
            prop_assert_eq!(panel.row_at(panel.rect.x, below), None);
        }
    }
}
