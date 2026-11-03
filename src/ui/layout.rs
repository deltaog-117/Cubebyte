//! Where the crafting panel and its rows sit on screen, in pixels.
//!
//! Pure arithmetic with no GPU, so placement and click-testing are tested directly.

use std::fmt::Write;

use crate::command::model::CommandId;
use crate::command::table as commands;
use crate::crafting::registry::{self, RecipeId};
use crate::crafting::service::can_craft;
use crate::inventory::model::Inventory;
use crate::items::registry::{self as items, ItemId};
use crate::logic::model::{RuleId, Rules};
use crate::scripting::edit::{Editor, test_text};
use crate::scripting::library::{self, ScriptId, Target};
use crate::ui::font::{GLYPH_HEIGHT, GLYPH_WIDTH};

const TITLE: &str = "crafting";
const HINT: &str = "click to craft - tab closes";
const SCRIPT_TITLE: &str = "scripts";
const SCRIPT_HINT: &str = "click to run - t closes";
const HACK_TITLE: &str = "hacks";
const HACK_HINT: &str = "click to run - g closes";
const HACK_NONE: &str = "aim at a block or a mob - g closes";
const COMMAND_TITLE: &str = "commands";
const COMMAND_HINT: &str = "enter runs - tab table - esc closes";

/// Most characters of the typed line and of the status shown at once, so a long one cannot
/// widen the panel.
const PROMPT_SHOWN: usize = 40;

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

/// A panel's frame and where each of its lines of text sits, before the rows know what they
/// stand for.
struct Arranged {
    scale: u32,
    rect: Rect,
    title: Label,
    hint: Label,
    rows: Vec<(Rect, Label)>,
}

/// Lays a titled panel with one row per entry of `texts` out centred in a window of `size`
/// pixels.
fn arrange((width, height): (u32, u32), title: &str, hint: &str, texts: &[String]) -> Arranged {
    let widest = texts
        .iter()
        .map(|text| text.chars().count())
        .chain([title, hint].map(|text| text.chars().count()))
        .max()
        .unwrap_or(0);
    // Big windows get big text, but never so big that the widest line leaves the window.
    let panel_units = widest as u32 * (GLYPH_WIDTH + 1) + 2 * ROW_PAD + 2 * MARGIN;
    // A tall list must not leave the window either: title, rows, hint and the margins around.
    let panel_rows =
        3 * MARGIN + 2 * GLYPH_HEIGHT + texts.len() as u32 * (GLYPH_HEIGHT + 2 * ROW_PAD + 2);
    let scale = (height / PIXELS_PER_SCALE)
        .min(width / panel_units)
        .min(height / panel_rows)
        .max(1);
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
    let mut rows = Vec::with_capacity(texts.len());
    for text in texts {
        let rect = Rect {
            x: margin,
            y,
            width: row_width,
            height: row_height,
        };
        rows.push((
            rect,
            Label {
                x: rect.x + row_pad,
                y: rect.y + row_pad,
                text: text.clone(),
            },
        ));
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
    Arranged {
        scale,
        rect: Rect {
            x: origin_x,
            y: origin_y,
            width: panel_width,
            height: panel_height,
        },
        title: label(margin, title_y, title),
        hint: label(margin, hint_y, hint),
        rows: rows
            .into_iter()
            .map(|(rect, text)| (moved(rect), label(text.x, text.y, &text.text)))
            .collect(),
    }
}

/// Lays the panel out centred in a window of `size` pixels, one row per recipe.
pub fn panel(size: (u32, u32), inventory: &Inventory) -> Panel {
    let described: Vec<_> = registry::ids()
        .filter_map(|id| Some((id, describe(id)?)))
        .collect();
    let texts: Vec<_> = described.iter().map(|(_, text)| text.clone()).collect();
    let arranged = arrange(size, TITLE, HINT, &texts);
    let rows = described
        .into_iter()
        .zip(arranged.rows)
        .map(|((recipe, _), (rect, label))| Row {
            recipe,
            rect,
            label,
            affordable: can_craft(inventory, recipe),
        })
        .collect();
    Panel {
        scale: arranged.scale,
        rect: arranged.rect,
        title: arranged.title,
        hint: arranged.hint,
        rows,
    }
}

/// One clickable ready-made script.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptRow {
    pub script: ScriptId,
    pub rect: Rect,
    /// Where the row's text goes, inside `rect`.
    pub label: Label,
    /// Whether a click would start it right now.
    pub startable: bool,
}

/// The script panel for one window size and one inventory.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptPanel {
    /// Screen pixels per font pixel.
    pub scale: u32,
    pub rect: Rect,
    pub title: Label,
    pub hint: Label,
    pub rows: Vec<ScriptRow>,
}

impl ScriptPanel {
    /// The script whose row is under the point, if any.
    pub fn row_at(&self, x: f32, y: f32) -> Option<ScriptId> {
        self.rows
            .iter()
            .find(|row| row.rect.contains(x, y))
            .map(|row| row.script)
    }
}

/// "tower - 3 fuel".
fn describe_script(id: ScriptId) -> Option<String> {
    let script = library::get(id)?;
    Some(format!("{} - {} fuel", script.name, script.fuel_cost()))
}

/// Lays the script panel out centred in a window of `size` pixels, one row per script that needs
/// no aiming. A row can be started when the pack holds the fuel for the whole script and none
/// is running.
pub fn script_panel(size: (u32, u32), inventory: &Inventory, running: bool) -> ScriptPanel {
    let ids: Vec<_> = library::ids_for(Target::Feet).collect();
    script_rows(size, inventory, running, &ids, SCRIPT_TITLE, SCRIPT_HINT)
}

/// The quick-hack menu: one row per script in `ids`, which the game picks from what the player
/// is aiming at. With no rows it says what to aim at.
pub fn hack_panel(
    size: (u32, u32),
    inventory: &Inventory,
    running: bool,
    ids: &[ScriptId],
) -> ScriptPanel {
    let hint = if ids.is_empty() { HACK_NONE } else { HACK_HINT };
    script_rows(size, inventory, running, ids, HACK_TITLE, hint)
}

fn script_rows(
    size: (u32, u32),
    inventory: &Inventory,
    running: bool,
    ids: &[ScriptId],
    title: &str,
    hint: &str,
) -> ScriptPanel {
    let described: Vec<_> = ids
        .iter()
        .filter_map(|&id| Some((id, describe_script(id)?)))
        .collect();
    let texts: Vec<_> = described.iter().map(|(_, text)| text.clone()).collect();
    let arranged = arrange(size, title, hint, &texts);
    let fuel = inventory.count_of(ItemId::FUEL);
    let rows = described
        .into_iter()
        .zip(arranged.rows)
        .map(|((script, _), (rect, label))| ScriptRow {
            script,
            rect,
            label,
            startable: !running && library::get(script).is_some_and(|s| fuel >= s.fuel_cost()),
        })
        .collect();
    ScriptPanel {
        scale: arranged.scale,
        rect: arranged.rect,
        title: arranged.title,
        hint: arranged.hint,
        rows,
    }
}

/// One clickable line of a [`Menu`], before it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuLine<A> {
    pub act: A,
    pub text: String,
    /// Whether a click would do something right now.
    pub usable: bool,
    /// Whether the line is the one picked, such as the selected block.
    pub chosen: bool,
}

/// One placed line of a [`Menu`].
#[derive(Clone, Debug, PartialEq)]
pub struct MenuRow<A> {
    pub act: A,
    pub rect: Rect,
    pub label: Label,
    pub usable: bool,
    pub chosen: bool,
}

/// A titled list of clickable lines, each standing for an `A`: the form the rules panel and the
/// block editor share.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu<A> {
    /// Screen pixels per font pixel.
    pub scale: u32,
    pub rect: Rect,
    pub title: Label,
    pub hint: Label,
    pub rows: Vec<MenuRow<A>>,
}

impl<A: Copy> Menu<A> {
    /// What the line under the point stands for, if any.
    pub fn row_at(&self, x: f32, y: f32) -> Option<A> {
        self.rows
            .iter()
            .find(|row| row.rect.contains(x, y))
            .map(|row| row.act)
    }
}

/// Most characters of a menu's hint shown at once.
const MENU_HINT_SHOWN: usize = 56;

/// Lays a menu out centred in a window of `size` pixels.
pub fn menu<A: Copy>(
    size: (u32, u32),
    title: &str,
    hint: &str,
    lines: Vec<MenuLine<A>>,
) -> Menu<A> {
    let hint: String = hint.chars().take(MENU_HINT_SHOWN).collect();
    let texts: Vec<_> = lines.iter().map(|line| line.text.clone()).collect();
    let arranged = arrange(size, title, &hint, &texts);
    let rows = lines
        .into_iter()
        .zip(arranged.rows)
        .map(|(line, (rect, label))| MenuRow {
            act: line.act,
            rect,
            label,
            usable: line.usable,
            chosen: line.chosen,
        })
        .collect();
    Menu {
        scale: arranged.scale,
        rect: arranged.rect,
        title: arranged.title,
        hint: arranged.hint,
        rows,
    }
}

/// What a click in the rules panel asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesAct {
    Pick(RuleId),
    Switch,
    Edit,
    Remove,
    New,
}

/// The rules panel: one line per rule, then what can be done with the picked one.
pub fn rules_menu(
    size: (u32, u32),
    rules: &Rules,
    picked: Option<RuleId>,
    note: &str,
) -> Menu<RulesAct> {
    let mut lines: Vec<_> = rules
        .iter()
        .map(|(id, rule)| MenuLine {
            act: RulesAct::Pick(id),
            text: format!(
                "{} {} {}",
                u16::from(id.0) + 1,
                if rule.is_on() { "on" } else { "off" },
                test_text(rule.test())
            ),
            usable: true,
            chosen: picked == Some(id),
        })
        .collect();
    let has_pick = picked.is_some_and(|id| rules.get(id).is_some());
    lines.extend([
        MenuLine {
            act: RulesAct::Switch,
            text: "switch on or off".to_owned(),
            usable: has_pick,
            chosen: false,
        },
        MenuLine {
            act: RulesAct::Edit,
            text: "edit its blocks".to_owned(),
            usable: has_pick,
            chosen: false,
        },
        MenuLine {
            act: RulesAct::Remove,
            text: "remove".to_owned(),
            usable: has_pick,
            chosen: false,
        },
        MenuLine {
            act: RulesAct::New,
            text: "new rule".to_owned(),
            usable: true,
            chosen: false,
        },
    ]);
    let hint = if note.is_empty() {
        "click a rule - n closes"
    } else {
        note
    };
    menu(size, "rules", hint, lines)
}

/// What a click in the block editor asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAct {
    Row(usize),
    Up,
    Down,
    Field,
    Less,
    More,
    Kind,
    Add,
    Delete,
    Run,
    Keep,
    Other,
}

/// How many lines of the draft show at once.
pub const EDITOR_LINES: usize = 5;

/// What the editor panel needs to know besides the draft itself.
#[derive(Clone, Copy, Debug)]
pub struct EditorView<'a> {
    pub title: &'a str,
    /// What the keep line says, or `None` when keeping makes no sense.
    pub keep: &'a str,
    /// Whether the draft can start right now.
    pub can_run: bool,
    /// Whether another ready-made script can be opened in its place.
    pub can_switch: bool,
    pub note: &'a str,
}

/// The block editor: a stretch of the draft's lines, then the buttons.
pub fn editor_menu(size: (u32, u32), editor: &Editor, view: EditorView<'_>) -> Menu<EditAct> {
    let rows = editor.rows();
    let mut lines: Vec<_> = editor
        .window(EDITOR_LINES)
        .map(|index| MenuLine {
            act: EditAct::Row(index),
            text: format!("{}{}", "  ".repeat(rows[index].depth), rows[index].text),
            usable: true,
            chosen: index == editor.selected(),
        })
        .collect();
    let field = editor.field_name();
    let button = |act, text: &str, usable| MenuLine {
        act,
        text: text.to_owned(),
        usable,
        chosen: false,
    };
    lines.extend([
        button(EditAct::Up, "up", !rows.is_empty()),
        button(EditAct::Down, "down", !rows.is_empty()),
        button(
            EditAct::Field,
            &format!("field {}", field.unwrap_or("none")),
            field.is_some(),
        ),
        button(EditAct::Less, "less", field.is_some()),
        button(EditAct::More, "more", field.is_some()),
        button(
            EditAct::Kind,
            &format!("next kind - {}", editor.kind().name()),
            true,
        ),
        button(EditAct::Add, &format!("add {}", editor.kind().name()), true),
        button(EditAct::Delete, "delete", !rows.is_empty()),
        button(EditAct::Run, "run", view.can_run),
        button(EditAct::Keep, view.keep, !view.keep.is_empty()),
        button(EditAct::Other, "other script", view.can_switch),
    ]);
    let hint = if view.note.is_empty() {
        "click to edit - b closes"
    } else {
        view.note
    };
    menu(size, view.title, hint, lines)
}

/// One clickable command of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRow {
    pub command: CommandId,
    pub rect: Rect,
    /// Where the row's text goes, inside `rect`.
    pub label: Label,
}

/// The command panel: the typed line and the last answer, and, when asked for, the table of
/// every command.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandPanel {
    /// Screen pixels per font pixel.
    pub scale: u32,
    pub rect: Rect,
    pub title: Label,
    pub hint: Label,
    /// Empty when the table is hidden.
    pub rows: Vec<CommandRow>,
    /// Where the typed line sits, and the box behind it.
    pub prompt_rect: Rect,
    pub prompt: Label,
    pub status: Label,
}

impl CommandPanel {
    /// The command whose row is under the point, if any.
    pub fn row_at(&self, x: f32, y: f32) -> Option<CommandId> {
        self.rows
            .iter()
            .find(|row| row.rect.contains(x, y))
            .map(|row| row.command)
    }
}

/// The last `limit` characters of `text`, so the end of what is being typed stays in view.
fn tail(text: &str, limit: usize) -> String {
    let skip = text.chars().count().saturating_sub(limit);
    text.chars().skip(skip).collect()
}

/// The first `limit` characters of `text`.
fn head(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Lays the command panel out centred in a window of `size` pixels. `typed` is the line being
/// written and `status` the answer to the last one. With `table` the panel lists every command
/// above the line, one clickable row each; without it only the line and the answer show.
pub fn command_panel(size: (u32, u32), typed: &str, status: &str, table: bool) -> CommandPanel {
    let described: Vec<_> = if table {
        commands::ids()
            .filter_map(|id| {
                let spec = commands::get(id)?;
                Some((id, format!("{} - {}", commands::usage(spec), spec.summary)))
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut texts: Vec<_> = described.iter().map(|(_, text)| text.clone()).collect();
    let rows_wide = texts.iter().map(|t| t.chars().count()).max().unwrap_or(0);
    // The line and the answer never make the panel wider than the table or the hint.
    let width_limit = rows_wide
        .max(COMMAND_HINT.chars().count())
        .min(PROMPT_SHOWN + 2);
    let prompt = format!("/{}_", tail(typed, width_limit.saturating_sub(2)));
    texts.push(prompt);
    texts.push(head(status, width_limit));
    let arranged = arrange(size, COMMAND_TITLE, COMMAND_HINT, &texts);
    // `arrange` made one row per text: the table's rows, then the line, then the answer.
    let blank = || {
        (
            Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            Label {
                x: 0.0,
                y: 0.0,
                text: String::new(),
            },
        )
    };
    let mut parts = arranged.rows;
    let (_, status) = parts.pop().unwrap_or_else(blank);
    let (prompt_rect, prompt) = parts.pop().unwrap_or_else(blank);
    let rows: Vec<_> = described
        .iter()
        .zip(parts)
        .map(|((command, _), (rect, label))| CommandRow {
            command: *command,
            rect,
            label,
        })
        .collect();
    CommandPanel {
        scale: arranged.scale,
        rect: arranged.rect,
        title: arranged.title,
        hint: arranged.hint,
        rows,
        prompt_rect,
        prompt,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::registry::ItemId;
    use crate::scripting::program::{Compare, Read, Test};
    use proptest::prelude::*;

    fn size() -> impl Strategy<Value = (u32, u32)> {
        (320u32..4096, 240u32..2160)
    }

    fn view() -> EditorView<'static> {
        EditorView {
            title: "editing tower",
            keep: "make a rule of it",
            can_run: true,
            can_switch: true,
            note: "",
        }
    }

    fn tower() -> Editor {
        let (_, script) = library::find("tower").expect("tower exists");
        Editor::new(None, script.program())
    }

    #[test]
    fn the_editor_shows_only_a_window_of_lines_then_every_button() {
        let menu = editor_menu((1280, 720), &tower(), view());
        let acts: Vec<_> = menu.rows.iter().map(|row| row.act).collect();
        assert_eq!(acts[..4], (0..4).map(EditAct::Row).collect::<Vec<_>>()[..]);
        assert_eq!(acts.len(), 4 + 11);
        assert!(acts.contains(&EditAct::Keep) && acts.contains(&EditAct::Run));
        assert_eq!(menu.rows.iter().filter(|row| row.chosen).count(), 1);
    }

    #[test]
    fn a_nested_line_is_indented_and_a_click_finds_its_act() {
        let menu = editor_menu((1280, 720), &tower(), view());
        assert_eq!(menu.rows[2].label.text, "  build stone 0 0 0");
        let row = &menu.rows[2];
        let (x, y) = (row.rect.x + 1.0, row.rect.y + 1.0);
        assert_eq!(menu.row_at(x, y), Some(EditAct::Row(2)));
        assert_eq!(menu.row_at(-1.0, -1.0), None);
    }

    #[test]
    fn rules_are_listed_and_buttons_need_a_pick() {
        let mut rules = Rules::default();
        let (_, stock) = library::find("stock").expect("stock exists");
        let test = Test {
            read: Read::Hunger,
            compare: Compare::Below,
            than: 10,
        };
        let anchor = crate::shared::voxel::WorldPos { x: 0, y: 0, z: 0 };
        let id = rules.add(test, stock.program().clone(), anchor).unwrap();
        let none = rules_menu((1280, 720), &rules, None, "");
        assert_eq!(none.rows[0].label.text, "1 on hunger below 10");
        assert!(none.rows[1..4].iter().all(|row| !row.usable));
        let picked = rules_menu((1280, 720), &rules, Some(id), "");
        assert!(picked.rows[0].chosen);
        assert!(picked.rows[1..].iter().all(|row| row.usable));
    }

    proptest! {
        // The tallest editor always fits the window, whatever its size.
        #[test]
        fn the_editor_panel_always_fits_the_window((width, height) in size()) {
            let menu = editor_menu((width, height), &tower(), view());
            prop_assert!(menu.rect.y >= 0.0);
            prop_assert!(menu.rect.y + menu.rect.height <= height as f32 + 0.5
                || menu.scale == 1);
            prop_assert!(menu.rect.x + menu.rect.width <= width as f32 + 0.5
                || menu.scale == 1);
        }
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

    #[test]
    fn a_script_row_reads_as_its_name_and_fuel_cost() {
        assert_eq!(
            describe_script(ScriptId(0)).as_deref(),
            Some("tower - 3 fuel")
        );
        assert_eq!(describe_script(ScriptId(200)), None);
    }

    #[test]
    fn the_command_table_fits_the_smallest_window() {
        let (width, height) = (320.0, 240.0);
        let panel = command_panel((320, 240), "", "", true);
        assert!(panel.rect.x >= 0.0 && panel.rect.x + panel.rect.width <= width);
        assert!(panel.rect.y >= 0.0 && panel.rect.y + panel.rect.height <= height);
    }

    #[test]
    fn the_script_panel_lists_only_scripts_that_need_no_aim() {
        let panel = script_panel((1280, 720), &Inventory::default(), false);
        assert!(panel.rows.iter().all(|row| {
            library::get(row.script).is_some_and(|script| script.target == Target::Feet)
        }));
    }

    #[test]
    fn the_hack_menu_says_what_to_aim_at_when_it_is_empty() {
        let empty = hack_panel((1280, 720), &Inventory::default(), false, &[]);
        assert!(empty.rows.is_empty());
        assert_eq!(empty.hint.text, HACK_NONE);
        let ids: Vec<_> = library::ids_for(Target::Mob).collect();
        let full = hack_panel((1280, 720), &Inventory::default(), false, &ids);
        assert_eq!(full.hint.text, HACK_HINT);
        assert_eq!(full.title.text, HACK_TITLE);
    }

    #[test]
    fn a_script_can_start_only_with_its_fuel_and_nothing_running() {
        let mut inventory = Inventory::default();
        inventory.add(ItemId::FUEL, 3).unwrap();
        let startable = |running| {
            script_panel((1280, 720), &inventory, running)
                .rows
                .iter()
                .map(|row| (library::get(row.script).map(|s| s.name), row.startable))
                .collect::<Vec<_>>()
        };

        assert!(startable(false).contains(&(Some("tower"), true)));
        assert!(startable(false).contains(&(Some("summon"), false)));
        assert!(startable(true).iter().all(|&(_, ok)| !ok));
    }

    proptest! {
        // Script rows fit, never overlap, and a click in a row finds its script.
        #[test]
        fn script_rows_fit_and_clicks_find_them(
            size in size(), fx in 0.0f32..1.0, fy in 0.0f32..1.0,
        ) {
            let panel = script_panel(size, &Inventory::default(), false);
            prop_assert_eq!(panel.rows.len(), library::ids_for(Target::Feet).count());
            let (width, height) = (size.0 as f32, size.1 as f32);
            prop_assert!(panel.rect.x >= 0.0 && panel.rect.x + panel.rect.width <= width);
            prop_assert!(panel.rect.y >= 0.0 && panel.rect.y + panel.rect.height <= height);
            for pair in panel.rows.windows(2) {
                prop_assert!(pair[0].rect.y + pair[0].rect.height <= pair[1].rect.y);
            }
            for row in &panel.rows {
                let (x, y) = (row.rect.x + fx * row.rect.width, row.rect.y + fy * row.rect.height);
                prop_assert_eq!(panel.row_at(x, y), Some(row.script));
            }
            prop_assert_eq!(panel.row_at(-1.0, -1.0), None);
        }

        // The hack menu fits and finds its rows for any list of hacks, empty included, and the
        // empty menu says what to aim at.
        #[test]
        fn hack_rows_fit_and_clicks_find_them(
            size in size(), fx in 0.0f32..1.0, fy in 0.0f32..1.0, count in 0usize..6,
        ) {
            let ids: Vec<_> = library::ids_for(Target::Block).take(count).collect();
            let panel = hack_panel(size, &Inventory::default(), false, &ids);
            prop_assert_eq!(panel.rows.len(), ids.len());
            let (width, height) = (size.0 as f32, size.1 as f32);
            prop_assert!(panel.rect.x >= 0.0 && panel.rect.x + panel.rect.width <= width);
            prop_assert!(panel.rect.y >= 0.0 && panel.rect.y + panel.rect.height <= height);
            for row in &panel.rows {
                let (x, y) = (row.rect.x + fx * row.rect.width, row.rect.y + fy * row.rect.height);
                prop_assert_eq!(panel.row_at(x, y), Some(row.script));
            }
        }

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

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn the_table_has_one_clickable_row_per_command_and_each_is_found_by_its_centre() {
        let panel = command_panel((1280, 720), "gi", "done", true);
        assert_eq!(panel.rows.len(), commands::ids().count());
        for row in &panel.rows {
            let (x, y) = (
                row.rect.x + row.rect.width / 2.0,
                row.rect.y + row.rect.height / 2.0,
            );
            assert_eq!(panel.row_at(x, y), Some(row.command));
        }
        assert_eq!(panel.prompt.text, "/gi_");
    }

    #[test]
    fn without_the_table_only_the_line_and_the_answer_show() {
        let panel = command_panel((1280, 720), "", "done", false);
        assert!(panel.rows.is_empty());
        assert_eq!(panel.prompt.text, "/_");
        assert_eq!(panel.status.text, "done");
    }

    #[test]
    fn a_long_line_shows_its_end_and_does_not_widen_the_panel() {
        let short = command_panel((1280, 720), "a", "", false);
        let long = command_panel((1280, 720), &"b".repeat(48), &"c".repeat(200), false);
        assert!(long.prompt.text.ends_with("b_"));
        assert!(long.rect.width <= short.rect.width.max(1280.0));
        assert!(long.status.text.chars().count() <= PROMPT_SHOWN + 2);
    }
}
