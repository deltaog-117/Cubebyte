//! The text form of a program: one block per line, braces for the blocks inside.
//!
//! ```text
//! shift 2 0 0
//! repeat 3 {
//!   build stone 0 0 0
//!   shift 0 1 0
//! }
//! if hunger below 8 {
//!   give bread 1
//! } else {
//!   hop 0 1 0
//! }
//! ```
//!
//! Text is only another way to write a [`Program`]: parsing ends in [`Program::try_new`], so a
//! file can never hold a program the blocks could not (too deep, too big, a bad repeat), and
//! [`print`] followed by [`parse`] gives back an equal program. A `#` starts a comment. A `{`
//! ends the line that opens a body, and a `}` stands alone on its line (or in `} else {`), each
//! set off by spaces, so a line splits into words on spaces alone.
//!
//! The line reader here ([`Cursor`], [`Node`]) is shared by the other text forms (rules and
//! recipes), which differ only in the lines they allow.

use std::fmt::Write as _;

use thiserror::Error;

use crate::blocks::registry as blocks;
use crate::items::registry::{self as items, ItemId};
use crate::scripting::program::{Block, Compare, Plan, Program, Read, Test};
use crate::shared::voxel::BlockId;

/// Deepest nesting of braces the reader follows, so a hostile file cannot overflow the stack.
/// Programs may nest less (see [`crate::scripting::program::MAX_DEPTH`]); that is checked when
/// the program is built.
const MAX_NESTING: usize = 16;

/// Why a text is not what it should be, with the line to fix. Lines count from 1.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("line {line}: {message}")]
pub struct TextError {
    pub line: usize,
    pub message: String,
}

impl TextError {
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

/// One line with something on it: its number and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    pub number: usize,
    pub words: Vec<&'a str>,
}

/// One line and the lines inside its braces, if it opened any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node<'a> {
    pub line: usize,
    /// The words before the `{`, if any.
    pub words: Vec<&'a str>,
    pub opens: bool,
    pub then: Vec<Node<'a>>,
    /// The lines after `} else {`; empty when there was no else.
    pub otherwise: Vec<Node<'a>>,
}

/// What ended a run of lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A `}` line.
    Close,
    /// A `} else {` line.
    CloseElse,
    /// An `end` line.
    End,
    /// The text ran out.
    Eof,
}

/// A reader over the lines of a text.
#[derive(Clone, Debug)]
pub struct Cursor<'a> {
    lines: Vec<Line<'a>>,
    at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(text: &'a str) -> Self {
        let lines = text
            .lines()
            .enumerate()
            .filter_map(|(index, raw)| {
                let content = raw.split('#').next().unwrap_or("");
                let words: Vec<&str> = content.split_whitespace().collect();
                (!words.is_empty()).then_some(Line {
                    number: index + 1,
                    words,
                })
            })
            .collect();
        Self { lines, at: 0 }
    }

    /// The next line without taking it.
    pub fn peek(&self) -> Option<&Line<'a>> {
        self.lines.get(self.at)
    }

    /// Takes the next line.
    pub fn take(&mut self) -> Option<Line<'a>> {
        let line = self.lines.get(self.at).cloned();
        if line.is_some() {
            self.at += 1;
        }
        line
    }

    /// The number of the last line, for an error about running out of text.
    pub fn last_line(&self) -> usize {
        self.lines.last().map_or(1, |line| line.number)
    }

    /// Reads lines into nodes until a closing line, an `end` line or the end of the text.
    ///
    /// # Errors
    ///
    /// Returns a [`TextError`] for a brace in the wrong place, a body that is never closed, or
    /// braces nested deeper than the reader follows.
    pub fn body(&mut self, depth: usize) -> Result<(Vec<Node<'a>>, Stop), TextError> {
        let mut nodes = Vec::new();
        while let Some(line) = self.take() {
            match line.words.as_slice() {
                ["}"] => return Ok((nodes, Stop::Close)),
                ["}", "else", "{"] => return Ok((nodes, Stop::CloseElse)),
                ["end"] => return Ok((nodes, Stop::End)),
                ["}", ..] => {
                    return Err(TextError::new(
                        line.number,
                        "a } stands alone, or as } else {",
                    ));
                }
                _ => {}
            }
            let mut words = line.words;
            if words.contains(&"{") && words.last() != Some(&"{") {
                return Err(TextError::new(
                    line.number,
                    "a { ends its line, set off by a space",
                ));
            }
            let opens = words.last() == Some(&"{");
            if opens {
                words.pop();
                if words.is_empty() {
                    return Err(TextError::new(line.number, "a { needs a block before it"));
                }
            }
            let mut node = Node {
                line: line.number,
                words,
                opens,
                then: Vec::new(),
                otherwise: Vec::new(),
            };
            if opens {
                if depth >= MAX_NESTING {
                    return Err(TextError::new(line.number, "braces nest too deeply"));
                }
                let (then, stop) = self.body(depth + 1)?;
                node.then = then;
                match stop {
                    Stop::Close => {}
                    Stop::CloseElse => {
                        if node.words.first() != Some(&"if") {
                            return Err(TextError::new(line.number, "only an if can have an else"));
                        }
                        let (otherwise, stop) = self.body(depth + 1)?;
                        if stop != Stop::Close {
                            return Err(TextError::new(
                                line.number,
                                "this else is never closed with }",
                            ));
                        }
                        node.otherwise = otherwise;
                    }
                    Stop::End | Stop::Eof => {
                        return Err(TextError::new(line.number, "this { is never closed"));
                    }
                }
            }
            nodes.push(node);
        }
        Ok((nodes, Stop::Eof))
    }
}

/// A whole number from one word.
///
/// # Errors
///
/// Returns a [`TextError`] on `line` when the word is not a number in range.
pub fn number<T: std::str::FromStr>(word: &str, line: usize, what: &str) -> Result<T, TextError> {
    word.parse().map_err(|_| {
        TextError::new(
            line,
            format!("{what} must be a whole number in range, not '{word}'"),
        )
    })
}

/// Three whole numbers from three words.
///
/// # Errors
///
/// Returns a [`TextError`] on `line` when there are not three numbers.
pub fn offset(words: &[&str], line: usize) -> Result<[i32; 3], TextError> {
    match words {
        [x, y, z] => Ok([
            number(x, line, "an offset")?,
            number(y, line, "an offset")?,
            number(z, line, "an offset")?,
        ]),
        _ => Err(TextError::new(line, "an offset is three numbers: x y z")),
    }
}

/// The item a word names.
///
/// # Errors
///
/// Returns a [`TextError`] on `line` listing the names that exist.
pub fn item_named(word: &str, line: usize) -> Result<ItemId, TextError> {
    items::find(word).ok_or_else(|| {
        let names: Vec<_> = items::ids()
            .filter_map(|id| items::get(id).map(|def| def.name))
            .collect();
        TextError::new(
            line,
            format!("no item called '{word}' (items: {})", names.join(", ")),
        )
    })
}

fn block_named(word: &str, line: usize) -> Result<BlockId, TextError> {
    blocks::find(word)
        .ok_or_else(|| TextError::new(line, format!("no block called '{word}' (try stone)")))
}

/// Checks that a line has exactly `count` words after its keyword.
fn args<'w>(node: &Node<'w>, count: usize, usage: &str) -> Result<Vec<&'w str>, TextError> {
    let rest = &node.words[1..];
    if rest.len() == count {
        Ok(rest.to_vec())
    } else {
        Err(TextError::new(node.line, format!("write: {usage}")))
    }
}

/// Reads a test such as `hunger below 8` or `count dirt above 3`.
///
/// # Errors
///
/// Returns a [`TextError`] on `line` when the words are not a test.
pub fn test_of(words: &[&str], line: usize) -> Result<Test, TextError> {
    const USAGE: &str =
        "health|hunger|mobs|count <item>|look <x> <y> <z>, then below|is|above and a number";
    let bad = || TextError::new(line, format!("a test reads: {USAGE}"));
    let (read, rest) = match words {
        ["health", rest @ ..] => (Read::Health, rest),
        ["hunger", rest @ ..] => (Read::Hunger, rest),
        ["mobs", rest @ ..] => (Read::MobsNear, rest),
        ["count", item, rest @ ..] => (
            Read::Count {
                item: item_named(item, line)?,
            },
            rest,
        ),
        ["look", x, y, z, rest @ ..] => (
            Read::Look {
                by: offset(&[x, y, z], line)?,
            },
            rest,
        ),
        _ => return Err(bad()),
    };
    let [compare, than] = rest else {
        return Err(bad());
    };
    let compare = match *compare {
        "below" => Compare::Below,
        "is" => Compare::Is,
        "above" => Compare::Above,
        _ => return Err(bad()),
    };
    let than = match (read, blocks::find(than)) {
        (Read::Look { .. }, Some(block)) => u32::from(block.0),
        _ => number(than, line, "the number to compare with")?,
    };
    Ok(Test {
        read,
        compare,
        than,
    })
}

fn no_body(node: &Node<'_>) -> Result<(), TextError> {
    if node.opens {
        Err(TextError::new(
            node.line,
            format!("'{}' has no blocks inside it", node.words[0]),
        ))
    } else {
        Ok(())
    }
}

fn block_of(node: &Node<'_>) -> Result<Block, TextError> {
    let line = node.line;
    match node.words[0] {
        "build" => {
            no_body(node)?;
            let a = args(node, 4, "build <block> <x> <y> <z>")?;
            Ok(Block::Do(Plan::Build {
                by: offset(&a[1..], line)?,
                block: block_named(a[0], line)?,
            }))
        }
        "hop" => {
            no_body(node)?;
            let a = args(node, 3, "hop <x> <y> <z>")?;
            Ok(Block::Do(Plan::Hop {
                by: offset(&a, line)?,
            }))
        }
        "give" => {
            no_body(node)?;
            let a = args(node, 2, "give <item> <count>")?;
            Ok(Block::Do(Plan::Give {
                item: item_named(a[0], line)?,
                count: number(a[1], line, "a count")?,
            }))
        }
        "summon" => {
            no_body(node)?;
            let a = args(node, 3, "summon <x> <y> <z>")?;
            Ok(Block::Do(Plan::Summon {
                by: offset(&a, line)?,
            }))
        }
        "shift" => {
            no_body(node)?;
            let a = args(node, 3, "shift <x> <y> <z>")?;
            Ok(Block::Shift {
                by: offset(&a, line)?,
            })
        }
        "repeat" => {
            if !node.opens {
                return Err(TextError::new(line, "write: repeat <times> {"));
            }
            let a = args(node, 1, "repeat <times> {")?;
            Ok(Block::Repeat {
                times: number(a[0], line, "the repeat count")?,
                body: blocks_of(&node.then)?,
            })
        }
        "if" => {
            if !node.opens {
                return Err(TextError::new(line, "write: if <test> {"));
            }
            Ok(Block::If {
                test: test_of(&node.words[1..], line)?,
                then: blocks_of(&node.then)?,
                otherwise: blocks_of(&node.otherwise)?,
            })
        }
        other => Err(TextError::new(
            line,
            format!("unknown block '{other}' (build, hop, give, summon, shift, repeat, if)"),
        )),
    }
}

/// The blocks that nodes stand for.
///
/// # Errors
///
/// Returns a [`TextError`] for the first line that is not a block.
pub fn blocks_of(nodes: &[Node<'_>]) -> Result<Vec<Block>, TextError> {
    nodes.iter().map(block_of).collect()
}

/// Builds a program from nodes read by a [`Cursor`], reporting a rule of [`Program`] on
/// `line`.
///
/// # Errors
///
/// Returns a [`TextError`] for a line that is not a block, or the [`Program`] rule that the
/// blocks break.
pub fn program_of(nodes: &[Node<'_>], line: usize) -> Result<Program, TextError> {
    Program::try_new(blocks_of(nodes)?).map_err(|error| TextError::new(line, error.to_string()))
}

/// Reads a whole text as one program.
///
/// # Errors
///
/// Returns a [`TextError`] naming the line to fix.
///
/// # Examples
///
/// ```
/// use minecraft_clone::scripting::text::parse;
///
/// let program = parse("repeat 2 {\n  build stone 0 0 0\n  shift 0 1 0\n}\n").unwrap();
/// assert_eq!(program.blocks().len(), 1);
/// assert!(parse("build lava 0 0 0").is_err());
/// ```
pub fn parse(text: &str) -> Result<Program, TextError> {
    let mut cursor = Cursor::new(text);
    let (nodes, stop) = cursor.body(0)?;
    match stop {
        Stop::Eof => program_of(&nodes, 1),
        Stop::Close | Stop::CloseElse => Err(TextError::new(
            cursor.last_line(),
            "a } with no { before it",
        )),
        Stop::End => Err(TextError::new(
            cursor.last_line(),
            "an 'end' only closes a rule or a recipe",
        )),
    }
}

/// Writes the word an item goes by.
fn item_text(item: ItemId) -> String {
    items::get(item).map_or_else(|| format!("item{}", item.0), |def| def.name.to_owned())
}

fn block_text(block: BlockId) -> String {
    blocks::get(block).map_or_else(|| format!("block{}", block.0), |def| def.name.to_owned())
}

fn offset_text(by: [i32; 3]) -> String {
    format!("{} {} {}", by[0], by[1], by[2])
}

/// A test written the way [`test_of`] reads it.
pub fn test_text(test: Test) -> String {
    let compare = match test.compare {
        Compare::Below => "below",
        Compare::Is => "is",
        Compare::Above => "above",
    };
    match test.read {
        Read::Health => format!("health {compare} {}", test.than),
        Read::Hunger => format!("hunger {compare} {}", test.than),
        Read::MobsNear => format!("mobs {compare} {}", test.than),
        Read::Count { item } => format!("count {} {compare} {}", item_text(item), test.than),
        Read::Look { by } => {
            let than = u16::try_from(test.than)
                .ok()
                .and_then(|id| blocks::get(BlockId(id)))
                .map_or_else(|| test.than.to_string(), |def| def.name.to_owned());
            format!("look {} {compare} {than}", offset_text(by))
        }
    }
}

fn print_blocks(list: &[Block], indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for block in list {
        // Writing to a String cannot fail.
        let _ = match block {
            Block::Do(Plan::Build { by, block }) => {
                writeln!(
                    out,
                    "{pad}build {} {}",
                    block_text(*block),
                    offset_text(*by)
                )
            }
            Block::Do(Plan::Hop { by }) => writeln!(out, "{pad}hop {}", offset_text(*by)),
            Block::Do(Plan::Give { item, count }) => {
                writeln!(out, "{pad}give {} {count}", item_text(*item))
            }
            Block::Do(Plan::Summon { by }) => writeln!(out, "{pad}summon {}", offset_text(*by)),
            Block::Shift { by } => writeln!(out, "{pad}shift {}", offset_text(*by)),
            Block::Repeat { times, body } => {
                let _ = writeln!(out, "{pad}repeat {times} {{");
                print_blocks(body, indent + 1, out);
                writeln!(out, "{pad}}}")
            }
            Block::If {
                test,
                then,
                otherwise,
            } => {
                let _ = writeln!(out, "{pad}if {} {{", test_text(*test));
                print_blocks(then, indent + 1, out);
                if otherwise.is_empty() {
                    writeln!(out, "{pad}}}")
                } else {
                    let _ = writeln!(out, "{pad}}} else {{");
                    print_blocks(otherwise, indent + 1, out);
                    writeln!(out, "{pad}}}")
                }
            }
        };
    }
}

/// The blocks of `list` as text, indented by `indent` levels. [`parse`] reads it back.
pub fn print_list(list: &[Block], indent: usize) -> String {
    let mut out = String::new();
    print_blocks(list, indent, &mut out);
    out
}

/// A program as text. [`parse`] reads it back to an equal program.
pub fn print(program: &Program) -> String {
    print_list(program.blocks(), 0)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn fail(text: &str) -> TextError {
        parse(text).expect_err("the text should be refused")
    }

    #[test]
    fn a_program_reads_and_prints_back_the_same() {
        let text = "shift 2 0 0\nrepeat 3 {\n  build stone 0 0 0\n  shift 0 1 0\n}\n\
                    if hunger below 8 {\n  give bread 1\n} else {\n  hop 0 1 0\n}\n";
        let program = parse(text).unwrap();
        assert_eq!(print(&program), text);
    }

    #[test]
    fn comments_blank_lines_and_spacing_do_not_matter() {
        let program = parse("# a tower\n\n  hop   0 1 0   # up\n").unwrap();
        assert_eq!(print(&program), "hop 0 1 0\n");
    }

    #[test]
    fn a_look_test_may_name_a_block() {
        let program = parse("if look 0 -1 0 is grass {\n hop 0 1 0\n}\n").unwrap();
        assert_eq!(
            print(&program),
            "if look 0 -1 0 is grass {\n  hop 0 1 0\n}\n"
        );
    }

    #[test]
    fn mistakes_name_their_line() {
        assert_eq!(fail("hop 0 1 0\nbuild lava 0 0 0").line, 2);
        assert_eq!(fail("hop 0 1").line, 1);
        assert_eq!(fail("\n\nfly 1 2 3").line, 3);
        assert_eq!(fail("repeat 3 {\n hop 0 1 0\n").line, 1);
        assert_eq!(fail("hop 0 1 0\n}").line, 2);
        assert_eq!(fail("repeat 3 {\n hop 0 1 0\n} else {\n}\n").line, 1);
    }

    #[test]
    fn the_program_rules_still_apply_to_text() {
        assert!(
            fail("repeat 99 {\n hop 0 1 0\n}")
                .message
                .contains("repeat")
        );
        assert!(fail("# nothing here").message.contains("at least one"));
        assert!(fail("shift 5000 0 0").message.contains("shift"));
    }

    #[test]
    fn a_hostile_nesting_is_refused_without_overflowing() {
        let text = "repeat 2 {\n".repeat(5000);
        assert!(parse(&text).is_err());
    }

    #[test]
    fn braces_must_be_set_off() {
        assert!(parse("repeat 2 {hop 0 1 0}").is_err());
        assert!(parse("repeat 2{\n hop 0 1 0\n}").is_err());
    }

    fn arb_offset() -> impl Strategy<Value = [i32; 3]> {
        prop::array::uniform3(-50i32..50)
    }

    fn arb_item() -> impl Strategy<Value = ItemId> {
        (1u16..=7).prop_map(ItemId)
    }

    fn arb_test() -> impl Strategy<Value = Test> {
        let read = prop_oneof![
            Just(Read::Health),
            Just(Read::Hunger),
            Just(Read::MobsNear),
            arb_item().prop_map(|item| Read::Count { item }),
            arb_offset().prop_map(|by| Read::Look { by }),
        ];
        let compare = prop_oneof![
            Just(Compare::Below),
            Just(Compare::Is),
            Just(Compare::Above)
        ];
        (read, compare, 0u32..100).prop_map(|(read, compare, than)| Test {
            read,
            compare,
            than,
        })
    }

    fn arb_leaf() -> impl Strategy<Value = Block> {
        prop_oneof![
            (arb_offset(), 0u16..4).prop_map(|(by, id)| Block::Do(Plan::Build {
                by,
                block: BlockId(id)
            })),
            arb_offset().prop_map(|by| Block::Do(Plan::Hop { by })),
            (arb_item(), 1u16..500).prop_map(|(item, count)| Block::Do(Plan::Give { item, count })),
            arb_offset().prop_map(|by| Block::Do(Plan::Summon { by })),
            arb_offset().prop_map(|by| Block::Shift { by }),
        ]
    }

    fn arb_blocks() -> impl Strategy<Value = Vec<Block>> {
        let block = arb_leaf().prop_recursive(3, 24, 3, |inner| {
            let body = prop::collection::vec(inner, 1..3);
            prop_oneof![
                (1u8..=16, body.clone()).prop_map(|(times, body)| Block::Repeat { times, body }),
                (
                    arb_test(),
                    body.clone(),
                    prop::collection::vec(arb_leaf(), 0..2)
                )
                    .prop_map(|(test, then, otherwise)| Block::If {
                        test,
                        then,
                        otherwise
                    }),
            ]
        });
        prop::collection::vec(block, 1..4)
    }

    proptest! {
        // Whatever program the blocks can make, its text reads back as the same program.
        #[test]
        fn printing_then_parsing_gives_the_same_program(blocks in arb_blocks()) {
            let Ok(program) = Program::try_new(blocks) else { return Ok(()); };
            let text = print(&program);
            prop_assert_eq!(parse(&text), Ok(program));
        }

        // Arbitrary text is refused or accepted, never a panic.
        #[test]
        fn any_text_is_read_without_panicking(text in "[ -~\n]{0,200}") {
            let _ = parse(&text);
        }
    }
}
