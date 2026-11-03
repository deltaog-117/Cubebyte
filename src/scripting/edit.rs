//! The block editor's core: a draft of a program that clicks change one small step at a time.
//!
//! A [`Editor`] holds blocks that may be unfinished (an empty repeat, say), a selected line and
//! a selected field of that line. Nothing here draws or reads the mouse; the window turns clicks
//! into the calls below. The only way out is [`Editor::program`], which runs the same checks as
//! [`Program::try_new`], so whatever a player builds can do nothing a hand-written program could
//! not. An edit that would break a limit the checks cannot repair later (too many blocks, nested
//! too deep) is refused on the spot.
//!
//! A draft for a rule also carries the rule's `when` test, shown as the first line.

use std::fmt::Write;
use std::ops::Range;

use crate::blocks::registry as blocks;
use crate::items::registry::{self as items, ItemId};
use crate::scripting::program::{
    Block, Compare, MAX_BLOCKS, MAX_DEPTH, MAX_REPEAT, MAX_SHIFT, Plan, Program, ProgramError,
    Read, Test,
};
use crate::shared::voxel::BlockId;

/// Largest number a test can compare with, and a give can hand out.
pub const NUMBER_MAX: u32 = 999;

/// Which list inside a block a step of a [`Place`] goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inside {
    Body,
    Then,
    Otherwise,
}

/// Where a block sits: the blocks it is nested in (each with the list entered), then its index
/// in the list it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    parents: Vec<(usize, Inside)>,
    index: usize,
}

/// One line of the draft as the player sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A rule's test.
    When,
    Block(Place),
    /// The divider between an `if`'s then part and its else part; holds the `if`'s place.
    Else(Place),
}

/// A line with how deep it is nested and the words that show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub depth: usize,
    pub line: Line,
    pub text: String,
}

/// What a click on a number can change. Which ones a line has depends on the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    X,
    Y,
    Z,
    Block,
    Item,
    Count,
    Times,
    Read,
    Compare,
    Than,
}

impl Field {
    const fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            Self::Block => "block",
            Self::Item => "item",
            Self::Count => "count",
            Self::Times => "times",
            Self::Read => "read",
            Self::Compare => "compare",
            Self::Than => "number",
        }
    }

    const fn axis(self) -> Option<usize> {
        match self {
            Self::X => Some(0),
            Self::Y => Some(1),
            Self::Z => Some(2),
            _ => None,
        }
    }
}

/// The kinds of block the editor can add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Build,
    Hop,
    Give,
    Summon,
    Shift,
    Repeat,
    If,
}

impl Kind {
    const ALL: [Self; 7] = [
        Self::Build,
        Self::Hop,
        Self::Give,
        Self::Summon,
        Self::Shift,
        Self::Repeat,
        Self::If,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Hop => "hop",
            Self::Give => "give",
            Self::Summon => "summon",
            Self::Shift => "shift",
            Self::Repeat => "repeat",
            Self::If => "if",
        }
    }

    fn next(self) -> Self {
        let at = Self::ALL.iter().position(|&kind| kind == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// A fresh block of this kind. A repeat or an if starts with nothing inside, so it is not
    /// a valid program until something is added in it.
    fn fresh(self) -> Block {
        match self {
            Self::Build => Block::Do(Plan::Build {
                by: [0; 3],
                block: BlockId::STONE,
            }),
            Self::Hop => Block::Do(Plan::Hop { by: [0, 1, 0] }),
            Self::Give => Block::Do(Plan::Give {
                item: ItemId::DIRT,
                count: 1,
            }),
            Self::Summon => Block::Do(Plan::Summon { by: [2, 0, 0] }),
            Self::Shift => Block::Shift { by: [0, 1, 0] },
            Self::Repeat => Block::Repeat {
                times: 2,
                body: Vec::new(),
            },
            Self::If => Block::If {
                test: Test {
                    read: Read::Hunger,
                    compare: Compare::Below,
                    than: 10,
                },
                then: Vec::new(),
                otherwise: Vec::new(),
            },
        }
    }
}

/// The program being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    when: Option<Test>,
    blocks: Vec<Block>,
    selected: usize,
    field: usize,
    kind: Kind,
}

impl Editor {
    /// A draft that starts as a copy of `program`, with a rule's `when` test if there is one.
    pub fn new(when: Option<Test>, program: &Program) -> Self {
        Self {
            when,
            blocks: program.blocks().to_vec(),
            selected: 0,
            field: 0,
            kind: Kind::Build,
        }
    }

    /// The rule's test, if this draft is a rule's.
    pub fn when(&self) -> Option<Test> {
        self.when
    }

    /// The draft as a checked program.
    ///
    /// # Errors
    ///
    /// Returns the [`ProgramError`] that says what to fix, such as an empty repeat.
    pub fn program(&self) -> Result<Program, ProgramError> {
        Program::try_new(self.blocks.clone())
    }

    pub const fn selected(&self) -> usize {
        self.selected
    }

    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Every line, top to bottom, with the words that show it.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(test) = self.when {
            rows.push(Row {
                depth: 0,
                line: Line::When,
                text: format!("when {}", test_text(test)),
            });
        }
        flatten(&self.blocks, &mut Vec::new(), 0, &mut rows);
        rows
    }

    /// Selects line `index`, if there is one.
    pub fn select(&mut self, index: usize) {
        if index < self.rows().len() {
            self.selected = index;
            self.field = 0;
        }
    }

    pub fn up(&mut self) {
        self.select(self.selected.saturating_sub(1));
    }

    pub fn down(&mut self) {
        self.select(self.selected + 1);
    }

    /// The lines to show when only `size` fit: a stretch holding the selected one.
    pub fn window(&self, size: usize) -> Range<usize> {
        let len = self.rows().len();
        let start = self
            .selected
            .saturating_sub(size / 2)
            .min(len.saturating_sub(size));
        start..(start + size).min(len)
    }

    /// The fields of the selected line.
    fn fields(&self) -> Vec<Field> {
        match self.rows().get(self.selected).map(|row| &row.line) {
            Some(Line::When) => self.when.map(test_fields).unwrap_or_default(),
            Some(Line::Block(place)) => block_at(&self.blocks, place)
                .map(block_fields)
                .unwrap_or_default(),
            Some(Line::Else(_)) | None => Vec::new(),
        }
    }

    /// The name of the field a click on less or more changes, or `None` when the selected line
    /// has nothing to change.
    pub fn field_name(&self) -> Option<&'static str> {
        let fields = self.fields();
        fields
            .get(self.field % fields.len().max(1))
            .map(|f| f.name())
    }

    /// Moves to the selected line's next field.
    pub fn next_field(&mut self) {
        let count = self.fields().len();
        if count > 0 {
            self.field = (self.field + 1) % count;
        }
    }

    /// Changes the selected field by `delta` steps; returns whether anything could change.
    pub fn nudge(&mut self, delta: i32) -> bool {
        let fields = self.fields();
        let Some(&field) = fields.get(self.field % fields.len().max(1)) else {
            return false;
        };
        let Some(row) = self.rows().into_iter().nth(self.selected) else {
            return false;
        };
        match row.line {
            Line::When => self
                .when
                .as_mut()
                .is_some_and(|test| nudge_test(test, field, delta)),
            Line::Block(place) => block_at_mut(&mut self.blocks, &place)
                .is_some_and(|block| nudge_block(block, field, delta)),
            Line::Else(_) => false,
        }
    }

    /// Picks the next kind of block to add.
    pub fn next_kind(&mut self) {
        self.kind = self.kind.next();
    }

    /// Adds a block of the chosen kind and selects it. It goes inside the selected repeat or if
    /// when that is still empty, at the end of the else part when the else line is selected,
    /// and otherwise right after the selected block. Refused (`false`) at [`MAX_BLOCKS`] blocks
    /// or beyond [`MAX_DEPTH`] levels.
    pub fn add(&mut self) -> bool {
        if count(&self.blocks) >= MAX_BLOCKS {
            return false;
        }
        let (parents, index) = match self.rows().get(self.selected).map(|row| row.line.clone()) {
            None => (Vec::new(), self.blocks.len()),
            Some(Line::When) => (Vec::new(), 0),
            Some(Line::Else(place)) => {
                let mut parents = place.parents;
                parents.push((place.index, Inside::Otherwise));
                let end = list_at(&self.blocks, &parents).map_or(0, <[Block]>::len);
                (parents, end)
            }
            Some(Line::Block(place)) => {
                let mut parents = place.parents.clone();
                match block_at(&self.blocks, &place) {
                    Some(Block::Repeat { body, .. }) if body.is_empty() => {
                        parents.push((place.index, Inside::Body));
                        (parents, 0)
                    }
                    Some(Block::If { then, .. }) if then.is_empty() => {
                        parents.push((place.index, Inside::Then));
                        (parents, 0)
                    }
                    _ => (parents, place.index + 1),
                }
            }
        };
        if parents.len() + 1 > MAX_DEPTH {
            return false;
        }
        let Some(list) = list_at_mut(&mut self.blocks, &parents) else {
            return false;
        };
        let at = index.min(list.len());
        list.insert(at, self.kind.fresh());
        let added = Place { parents, index: at };
        self.select_place(&added);
        true
    }

    /// Removes the selected block with everything inside it, or empties an else part. The
    /// `when` line cannot be removed.
    pub fn delete(&mut self) -> bool {
        let Some(row) = self.rows().into_iter().nth(self.selected) else {
            return false;
        };
        let done = match row.line {
            Line::When => false,
            Line::Block(place) => {
                list_at_mut(&mut self.blocks, &place.parents).is_some_and(|list| {
                    let present = place.index < list.len();
                    if present {
                        list.remove(place.index);
                    }
                    present
                })
            }
            Line::Else(place) => match block_at_mut(&mut self.blocks, &place) {
                Some(Block::If { otherwise, .. }) => {
                    otherwise.clear();
                    true
                }
                _ => false,
            },
        };
        let last = self.rows().len().saturating_sub(1);
        self.selected = self.selected.min(last);
        self.field = 0;
        done
    }

    fn select_place(&mut self, place: &Place) {
        if let Some(at) = self
            .rows()
            .iter()
            .position(|row| matches!(&row.line, Line::Block(found) if found == place))
        {
            self.selected = at;
            self.field = 0;
        }
    }
}

fn count(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| {
            1 + match block {
                Block::Repeat { body, .. } => count(body),
                Block::If {
                    then, otherwise, ..
                } => count(then) + count(otherwise),
                _ => 0,
            }
        })
        .sum()
}

fn flatten(list: &[Block], parents: &mut Vec<(usize, Inside)>, depth: usize, rows: &mut Vec<Row>) {
    for (index, block) in list.iter().enumerate() {
        let place = Place {
            parents: parents.clone(),
            index,
        };
        rows.push(Row {
            depth,
            line: Line::Block(place.clone()),
            text: block_text(block),
        });
        match block {
            Block::Repeat { body, .. } => {
                parents.push((index, Inside::Body));
                flatten(body, parents, depth + 1, rows);
                parents.pop();
            }
            Block::If {
                then, otherwise, ..
            } => {
                parents.push((index, Inside::Then));
                flatten(then, parents, depth + 1, rows);
                parents.pop();
                rows.push(Row {
                    depth,
                    line: Line::Else(place),
                    text: "else".to_owned(),
                });
                parents.push((index, Inside::Otherwise));
                flatten(otherwise, parents, depth + 1, rows);
                parents.pop();
            }
            Block::Do(_) | Block::Shift { .. } => {}
        }
    }
}

fn list_at<'a>(blocks: &'a [Block], parents: &[(usize, Inside)]) -> Option<&'a [Block]> {
    let mut list = blocks;
    for &(index, inside) in parents {
        list = match (list.get(index)?, inside) {
            (Block::Repeat { body, .. }, Inside::Body) => body,
            (Block::If { then, .. }, Inside::Then) => then,
            (Block::If { otherwise, .. }, Inside::Otherwise) => otherwise,
            _ => return None,
        };
    }
    Some(list)
}

fn list_at_mut<'a>(
    blocks: &'a mut Vec<Block>,
    parents: &[(usize, Inside)],
) -> Option<&'a mut Vec<Block>> {
    let mut list = blocks;
    for &(index, inside) in parents {
        list = match (list.get_mut(index)?, inside) {
            (Block::Repeat { body, .. }, Inside::Body) => body,
            (Block::If { then, .. }, Inside::Then) => then,
            (Block::If { otherwise, .. }, Inside::Otherwise) => otherwise,
            _ => return None,
        };
    }
    Some(list)
}

fn block_at<'a>(blocks: &'a [Block], place: &Place) -> Option<&'a Block> {
    list_at(blocks, &place.parents)?.get(place.index)
}

fn block_at_mut<'a>(blocks: &'a mut Vec<Block>, place: &Place) -> Option<&'a mut Block> {
    list_at_mut(blocks, &place.parents)?.get_mut(place.index)
}

fn test_fields(test: Test) -> Vec<Field> {
    let mut fields = vec![Field::Read, Field::Compare, Field::Than];
    match test.read {
        Read::Count { .. } => fields.push(Field::Item),
        Read::Look { .. } => fields.extend([Field::X, Field::Y, Field::Z]),
        Read::Health | Read::Hunger | Read::MobsNear => {}
    }
    fields
}

fn block_fields(block: &Block) -> Vec<Field> {
    match block {
        Block::Do(Plan::Build { .. }) => vec![Field::X, Field::Y, Field::Z, Field::Block],
        Block::Do(Plan::Give { .. }) => vec![Field::Item, Field::Count],
        Block::Do(Plan::Hop { .. } | Plan::Summon { .. }) | Block::Shift { .. } => {
            vec![Field::X, Field::Y, Field::Z]
        }
        Block::Repeat { .. } => vec![Field::Times],
        Block::If { test, .. } => test_fields(*test),
    }
}

/// `value` moved by `delta` and wrapped into `0..len`.
fn wrap(value: usize, delta: i32, len: usize) -> usize {
    let len = len.max(1) as i64;
    (value as i64 + i64::from(delta)).rem_euclid(len) as usize
}

fn block_count() -> usize {
    (0u16..)
        .take_while(|&id| blocks::get(BlockId(id)).is_some())
        .count()
}

fn nudge_axis(by: &mut [i32; 3], field: Field, delta: i32) -> bool {
    let Some(axis) = field.axis() else {
        return false;
    };
    by[axis] = by[axis].saturating_add(delta).clamp(-MAX_SHIFT, MAX_SHIFT);
    true
}

fn nudge_item(item: &mut ItemId, delta: i32) {
    let all: Vec<_> = items::ids().collect();
    let at = all.iter().position(|&known| known == *item).unwrap_or(0);
    *item = all[wrap(at, delta, all.len())];
}

fn nudge_test(test: &mut Test, field: Field, delta: i32) -> bool {
    match field {
        Field::Read => {
            let kinds = [
                Read::Health,
                Read::Hunger,
                Read::MobsNear,
                Read::Count { item: ItemId::DIRT },
                Read::Look { by: [0; 3] },
            ];
            let at = match test.read {
                Read::Health => 0,
                Read::Hunger => 1,
                Read::MobsNear => 2,
                Read::Count { .. } => 3,
                Read::Look { .. } => 4,
            };
            test.read = kinds[wrap(at, delta, kinds.len())];
            true
        }
        Field::Compare => {
            let all = [Compare::Below, Compare::Is, Compare::Above];
            let at = all.iter().position(|&c| c == test.compare).unwrap_or(0);
            test.compare = all[wrap(at, delta, all.len())];
            true
        }
        Field::Than => {
            let moved = i64::from(test.than) + i64::from(delta);
            test.than = moved.clamp(0, i64::from(NUMBER_MAX)) as u32;
            true
        }
        Field::Item => match &mut test.read {
            Read::Count { item } => {
                nudge_item(item, delta);
                true
            }
            _ => false,
        },
        Field::X | Field::Y | Field::Z => match &mut test.read {
            Read::Look { by } => nudge_axis(by, field, delta),
            _ => false,
        },
        Field::Block | Field::Count | Field::Times => false,
    }
}

fn nudge_block(block: &mut Block, field: Field, delta: i32) -> bool {
    match block {
        Block::Do(Plan::Build { by, block }) => match field {
            Field::Block => {
                *block = BlockId(wrap(usize::from(block.0), delta, block_count()) as u16);
                true
            }
            _ => nudge_axis(by, field, delta),
        },
        Block::Do(Plan::Hop { by } | Plan::Summon { by }) | Block::Shift { by } => {
            nudge_axis(by, field, delta)
        }
        Block::Do(Plan::Give { item, count }) => match field {
            Field::Item => {
                nudge_item(item, delta);
                true
            }
            Field::Count => {
                let moved = i64::from(*count) + i64::from(delta);
                *count = moved.clamp(1, i64::from(NUMBER_MAX)) as u16;
                true
            }
            _ => false,
        },
        Block::Repeat { times, .. } => {
            if field != Field::Times {
                return false;
            }
            let moved = i32::from(*times) + delta;
            *times = moved.clamp(1, i32::from(MAX_REPEAT)) as u8;
            true
        }
        Block::If { test, .. } => nudge_test(test, field, delta),
    }
}

/// The words that show a test, such as "hunger below 10".
pub fn test_text(test: Test) -> String {
    let compare = match test.compare {
        Compare::Below => "below",
        Compare::Is => "is",
        Compare::Above => "above",
    };
    let mut text = match test.read {
        Read::Health => "health".to_owned(),
        Read::Hunger => "hunger".to_owned(),
        Read::MobsNear => "mobs near".to_owned(),
        Read::Count { item } => format!("count {}", items::get(item).map_or("?", |d| d.name)),
        Read::Look { by } => format!("look {} {} {}", by[0], by[1], by[2]),
    };
    let _ = write!(text, " {compare} {}", test.than);
    text
}

fn block_text(block: &Block) -> String {
    match block {
        Block::Do(Plan::Build { by, block }) => format!(
            "build {} {} {} {}",
            blocks::get(*block).map_or("?", |def| def.name),
            by[0],
            by[1],
            by[2]
        ),
        Block::Do(Plan::Hop { by }) => format!("hop {} {} {}", by[0], by[1], by[2]),
        Block::Do(Plan::Give { item, count }) => format!(
            "give {count} {}",
            items::get(*item).map_or("?", |def| def.name)
        ),
        Block::Do(Plan::Summon { by }) => format!("summon {} {} {}", by[0], by[1], by[2]),
        Block::Shift { by } => format!("shift {} {} {}", by[0], by[1], by[2]),
        Block::Repeat { times, .. } => format!("repeat {times}"),
        Block::If { test, .. } => format!("if {}", test_text(*test)),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::scripting::library::{self, ScriptId};
    use crate::scripting::model::{Answer, Denied};
    use crate::shared::voxel::WorldPos;

    fn editor_of(name: &str) -> Editor {
        let (_, script) = library::find(name).expect("a ready-made script");
        Editor::new(None, script.program())
    }

    fn commands(program: &Program) -> Vec<crate::scripting::model::Command> {
        program
            .preview(WorldPos { x: 0, y: 0, z: 0 }, &mut |_| {
                Ok::<_, Denied>(Answer::Number(0))
            })
            .expect("a runnable program")
    }

    #[test]
    fn an_untouched_draft_is_the_program_it_started_as() {
        for id in library::ids() {
            let script = library::get(id).expect("listed");
            let editor = Editor::new(None, script.program());
            assert_eq!(editor.program().as_ref(), Ok(script.program()));
        }
    }

    #[test]
    fn rows_show_the_tree_with_its_depth() {
        let editor = editor_of("tower");
        let shown: Vec<_> = editor
            .rows()
            .into_iter()
            .map(|row| (row.depth, row.text))
            .collect();
        assert_eq!(
            shown,
            [
                (0, "shift 2 0 0".to_owned()),
                (0, "repeat 3".to_owned()),
                (1, "build stone 0 0 0".to_owned()),
                (1, "shift 0 1 0".to_owned()),
            ]
        );
    }

    #[test]
    fn changing_a_repeat_changes_what_the_program_does() {
        let mut editor = editor_of("tower");
        editor.select(1);
        assert_eq!(editor.field_name(), Some("times"));
        assert!(editor.nudge(2));
        let program = editor.program().unwrap();
        assert_eq!(commands(&program).len(), 5);
    }

    #[test]
    fn a_field_stops_at_its_limits() {
        let mut editor = editor_of("tower");
        editor.select(1);
        for _ in 0..40 {
            editor.nudge(1);
        }
        assert!(matches!(
            editor.blocks[1],
            Block::Repeat {
                times: MAX_REPEAT,
                ..
            }
        ));
        for _ in 0..40 {
            editor.nudge(-1);
        }
        assert!(matches!(editor.blocks[1], Block::Repeat { times: 1, .. }));
    }

    #[test]
    fn next_field_walks_the_fields_of_a_line() {
        let mut editor = editor_of("tower");
        editor.select(2);
        let names: Vec<_> = (0..5)
            .map(|_| {
                let name = editor.field_name();
                editor.next_field();
                name
            })
            .collect();
        assert_eq!(
            names,
            [Some("x"), Some("y"), Some("z"), Some("block"), Some("x")]
        );
    }

    #[test]
    fn a_new_repeat_needs_something_inside_before_it_is_a_program() {
        let mut editor = editor_of("hop");
        editor.kind = Kind::Repeat;
        assert!(editor.add());
        assert_eq!(editor.program(), Err(ProgramError::EmptyBody));
        editor.kind = Kind::Build;
        assert!(editor.add());
        assert!(editor.program().is_ok());
        let shown: Vec<_> = editor.rows().into_iter().map(|row| row.depth).collect();
        assert_eq!(shown, [0, 0, 1]);
    }

    #[test]
    fn adding_on_an_else_line_fills_the_else_part() {
        let mut editor = editor_of("ward");
        let at = editor
            .rows()
            .iter()
            .position(|row| {
                row.line
                    == Line::Else(Place {
                        parents: Vec::new(),
                        index: 0,
                    })
            })
            .unwrap();
        editor.select(at);
        editor.kind = Kind::Hop;
        assert!(editor.add());
        let Block::If { otherwise, .. } = &editor.blocks[0] else {
            panic!("ward is an if");
        };
        assert_eq!(otherwise.len(), 2);
    }

    #[test]
    fn deleting_removes_a_block_and_what_is_inside_it() {
        let mut editor = editor_of("tower");
        editor.select(1);
        assert!(editor.delete());
        assert_eq!(editor.blocks.len(), 1);
        assert!(editor.selected() < editor.rows().len());
    }

    #[test]
    fn a_rule_draft_starts_with_its_test_which_can_be_changed_but_not_removed() {
        let test = Test {
            read: Read::Hunger,
            compare: Compare::Below,
            than: 10,
        };
        let (_, script) = library::find("stock").unwrap();
        let mut editor = Editor::new(Some(test), script.program());
        assert_eq!(editor.rows()[0].text, "when hunger below 10");
        assert!(!editor.delete());
        editor.select(0);
        editor.next_field();
        editor.next_field();
        assert_eq!(editor.field_name(), Some("number"));
        assert!(editor.nudge(-4));
        assert_eq!(editor.rows()[0].text, "when hunger below 6");
        assert_eq!(editor.when().map(|t| t.than), Some(6));
    }

    #[test]
    fn the_read_of_a_test_can_be_swapped_and_brings_its_own_fields() {
        let test = Test {
            read: Read::Hunger,
            compare: Compare::Below,
            than: 10,
        };
        let (_, script) = library::find("stock").unwrap();
        let mut editor = Editor::new(Some(test), script.program());
        editor.select(0);
        assert_eq!(editor.field_name(), Some("read"));
        // hunger -> mobs near -> count -> look
        for _ in 0..3 {
            editor.nudge(1);
        }
        assert_eq!(editor.rows()[0].text, "when look 0 0 0 below 10");
        assert_eq!(editor.fields().len(), 6);
    }

    #[test]
    fn adding_is_refused_at_the_block_limit_and_beyond_the_depth_limit() {
        let mut editor = editor_of("hop");
        editor.kind = Kind::Hop;
        while editor.add() {}
        assert_eq!(count(&editor.blocks), MAX_BLOCKS);
        assert!(editor.program().is_ok());

        let mut editor = editor_of("hop");
        editor.kind = Kind::Repeat;
        let mut nested = 0;
        while editor.add() {
            nested += 1;
        }
        assert_eq!(nested, MAX_DEPTH);
    }

    #[test]
    fn the_window_always_holds_the_selected_line() {
        let mut editor = editor_of("hop");
        editor.kind = Kind::Hop;
        for _ in 0..12 {
            editor.add();
        }
        let len = editor.rows().len();
        for at in 0..len {
            editor.select(at);
            let window = editor.window(5);
            assert!(window.contains(&at), "{at} in {window:?}");
            assert!(window.end <= len);
        }
    }

    #[derive(Clone, Debug)]
    enum Op {
        Select(usize),
        Up,
        Down,
        Field,
        Nudge(i32),
        Kind,
        Add,
        Delete,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0usize..80).prop_map(Op::Select),
            Just(Op::Up),
            Just(Op::Down),
            Just(Op::Field),
            (-30i32..30).prop_map(Op::Nudge),
            Just(Op::Kind),
            Just(Op::Add),
            Just(Op::Delete),
        ]
    }

    proptest! {
        // However the clicks go, the selection stays on a line, the size limits hold, the
        // window holds the selection, and a draft that passes the checks always runs to its end.
        #[test]
        fn any_clicks_keep_the_draft_within_its_limits(
            script in 0u8..9,
            ops in prop::collection::vec(op(), 0..120),
        ) {
            let program = library::get(ScriptId(script)).expect("a built-in").program().clone();
            let mut editor = Editor::new(None, &program);
            for op in ops {
                match op {
                    Op::Select(at) => editor.select(at),
                    Op::Up => editor.up(),
                    Op::Down => editor.down(),
                    Op::Field => editor.next_field(),
                    Op::Nudge(delta) => { editor.nudge(delta); }
                    Op::Kind => editor.next_kind(),
                    Op::Add => { editor.add(); }
                    Op::Delete => { editor.delete(); }
                }
                let rows = editor.rows();
                prop_assert!(rows.is_empty() || editor.selected() < rows.len());
                prop_assert!(count(&editor.blocks) <= MAX_BLOCKS);
                prop_assert!(rows.iter().all(|row| row.depth < MAX_DEPTH));
                if !rows.is_empty() {
                    prop_assert!(editor.window(5).contains(&editor.selected()));
                }
                if let Ok(program) = editor.program() {
                    let ran = program.preview(
                        WorldPos { x: 0, y: 0, z: 0 },
                        &mut |_| Ok::<_, Denied>(Answer::Number(0)),
                    );
                    // A program may run out its budget, but it must never loop forever.
                    prop_assert!(matches!(ran, Ok(_) | Err(Denied::TooLong)));
                }
            }
        }
    }
}
