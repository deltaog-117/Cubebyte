//! Reading a typed line.

use thiserror::Error;

use crate::blocks::registry as blocks;
use crate::command::model::{ArgKind, ArgSpec, Request, Value};
use crate::command::table::{self, OFFSET_MAX};
use crate::crafting::custom::{CustomError, name_is_valid};
use crate::crafting::program::{RecipeProgram, Step};
use crate::items::registry as items;
use crate::scripting::library;
use crate::shared::voxel::BlockId;

/// How many characters of a rejected word an error repeats, so a long paste cannot flood the
/// status line.
const ECHO: usize = 12;

/// Why a line did nothing. Each message says what to type instead.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("type a command - help opens the table")]
    Empty,
    #[error("no command called {0} - help opens the table")]
    UnknownCommand(String),
    #[error("too few words - use {0}")]
    TooFew(String),
    #[error("too many words - use {0}")]
    TooMany(String),
    #[error("{arg} must be a whole number, not {word}")]
    NotANumber { arg: &'static str, word: String },
    #[error("{arg} must be from {min} to {max}")]
    OutOfRange {
        arg: &'static str,
        min: i32,
        max: i32,
    },
    #[error("no item called {0}")]
    UnknownItem(String),
    #[error("no block called {0}")]
    UnknownBlock(String),
    #[error("no mob called {0} - try zombie")]
    UnknownMob(String),
    #[error("no hack called {0} - g lists them")]
    UnknownHack(String),
    #[error("{}", CustomError::BadName)]
    BadRecipeName,
    #[error("a step is take or make, an item and a count - as in take grass 3, not {0}")]
    BadStep(String),
    /// Why the steps are not a fair recipe, with the semicolon the font lacks turned into a
    /// comma.
    #[error("{0}")]
    Recipe(String),
    /// The table has a command whose build does not match its arguments: a bug, reported
    /// instead of a crash.
    #[error("the command {0} is broken")]
    Broken(&'static str),
}

fn clip(word: &str) -> String {
    word.chars().take(ECHO).collect()
}

/// Reads `line`: a command name, then one word per argument, separated by spaces. Letter case
/// does not matter.
///
/// # Errors
///
/// Returns a [`ParseError`] saying what was wrong with the line; nothing is changed.
pub fn parse(line: &str) -> Result<Request, ParseError> {
    let mut words = line.split_whitespace();
    let name = words.next().ok_or(ParseError::Empty)?;
    let (_, spec) = table::find(name).ok_or_else(|| ParseError::UnknownCommand(clip(name)))?;
    let rest: Vec<&str> = words.collect();
    // A last argument of kind `Steps` takes whatever words are left, however many.
    let open_ended = spec
        .args
        .last()
        .is_some_and(|arg| arg.kind == ArgKind::Steps);
    let fixed = spec.args.len() - usize::from(open_ended);
    if rest.len() < fixed {
        return Err(ParseError::TooFew(table::usage(spec)));
    }
    if !open_ended && rest.len() > fixed {
        return Err(ParseError::TooMany(table::usage(spec)));
    }
    let mut values = spec
        .args
        .iter()
        .zip(&rest)
        .take(fixed)
        .map(|(arg, word)| value_of(arg, word))
        .collect::<Result<Vec<_>, _>>()?;
    if open_ended {
        values.push(steps_of(&rest[fixed..])?);
    }
    (spec.build)(&values).ok_or(ParseError::Broken(spec.name))
}

/// Reads `take <item> <count>` and `make <item> <count>` steps, three words each, into a
/// checked recipe. No words at all is no recipe yet.
fn steps_of(words: &[&str]) -> Result<Value, ParseError> {
    if words.is_empty() {
        return Ok(Value::Steps(None));
    }
    let mut steps = Vec::new();
    for group in words.chunks(3) {
        let [verb, item, count] = group else {
            return Err(ParseError::BadStep(clip(&group.join(" "))));
        };
        let item = items::ids()
            .find(|&id| items::get(id).is_some_and(|d| d.name.eq_ignore_ascii_case(item)))
            .ok_or_else(|| ParseError::UnknownItem(clip(item)))?;
        let count: u8 =
            count
                .parse()
                .ok()
                .filter(|&n| n >= 1)
                .ok_or_else(|| ParseError::OutOfRange {
                    arg: "count",
                    min: 1,
                    max: i32::from(u8::MAX),
                })?;
        steps.push(match verb.to_ascii_lowercase().as_str() {
            "take" => Step::Take { item, count },
            "make" => Step::Make { item, count },
            _ => return Err(ParseError::BadStep(clip(verb))),
        });
    }
    RecipeProgram::try_new(steps)
        .map(|program| Value::Steps(Some(program)))
        .map_err(|error| ParseError::Recipe(error.to_string().replace(';', ",")))
}

fn value_of(arg: &ArgSpec, word: &str) -> Result<Value, ParseError> {
    let name = arg.name;
    let number = |min: i32, max: i32| -> Result<i32, ParseError> {
        let value: i32 = word.parse().map_err(|_| ParseError::NotANumber {
            arg: name,
            word: clip(word),
        })?;
        if (min..=max).contains(&value) {
            Ok(value)
        } else {
            Err(ParseError::OutOfRange {
                arg: name,
                min,
                max,
            })
        }
    };
    Ok(match arg.kind {
        ArgKind::Item => Value::Item(
            items::ids()
                .find(|&id| items::get(id).is_some_and(|d| d.name.eq_ignore_ascii_case(word)))
                .ok_or_else(|| ParseError::UnknownItem(clip(word)))?,
        ),
        ArgKind::Block => Value::Block(
            (0..)
                .map(BlockId)
                .map_while(|id| blocks::get(id).map(|def| (id, def)))
                .find(|(_, def)| def.name.eq_ignore_ascii_case(word))
                .map(|(id, _)| id)
                .ok_or_else(|| ParseError::UnknownBlock(clip(word)))?,
        ),
        ArgKind::Mob if word.eq_ignore_ascii_case("zombie") => Value::Mob,
        ArgKind::Mob => return Err(ParseError::UnknownMob(clip(word))),
        ArgKind::Count { max } => {
            // `max` is at most u16::MAX, which fits an i32, and the range check keeps the
            // result in 1..=max, which fits a u16.
            let value = number(1, i32::from(max))?;
            Value::Count(u16::try_from(value).unwrap_or(max))
        }
        ArgKind::Offset => Value::Offset(number(-OFFSET_MAX, OFFSET_MAX)?),
        ArgKind::Name if name_is_valid(word) => Value::Name(word.to_owned()),
        ArgKind::Name => return Err(ParseError::BadRecipeName),
        // `parse` reads the open-ended last argument itself.
        ArgKind::Steps => steps_of(&[word])?,
        ArgKind::Hack => Value::Hack(
            library::find(word)
                .map(|(id, _)| id)
                .ok_or_else(|| ParseError::UnknownHack(clip(word)))?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::items::registry::ItemId;

    #[test]
    fn a_good_line_becomes_its_request() {
        assert_eq!(parse("help"), Ok(Request::Help));
        assert_eq!(
            parse("  GIVE fuel   5 "),
            Ok(Request::Give {
                item: ItemId::FUEL,
                count: 5
            })
        );
        assert_eq!(parse("go 1 -2 3"), Ok(Request::Go { by: [1, -2, 3] }));
        assert_eq!(
            parse("set stone 0 1 0"),
            Ok(Request::Set {
                block: BlockId::STONE,
                by: [0, 1, 0]
            })
        );
        assert_eq!(
            parse("spawn zombie 2 0 0"),
            Ok(Request::Spawn { by: [2, 0, 0] })
        );
        assert_eq!(parse("zone 10"), Ok(Request::Zone { back: 10 }));
        assert_eq!(
            parse("copy build"),
            Ok(Request::Copy {
                what: "build".to_owned()
            })
        );
        assert_eq!(parse("paste"), Ok(Request::Paste));
        assert_eq!(parse("rewind"), Ok(Request::Rewind));
        assert_eq!(parse("redo"), Ok(Request::Redo));
        assert_eq!(
            parse("hack Pillar"),
            Ok(Request::Hack {
                script: library::find("pillar").map(|(id, _)| id).unwrap()
            })
        );
    }

    #[test]
    fn a_recipe_line_makes_a_recipe_or_asks_for_the_editor() {
        let Ok(Request::Recipe { name, program }) = parse("recipe loaf take grass 3 make bread 1")
        else {
            panic!("expected a recipe");
        };
        assert_eq!(name, "loaf");
        assert_eq!(program.map(|p| p.steps().len()), Some(2));
        assert_eq!(
            parse("recipe loaf"),
            Ok(Request::Recipe {
                name: "loaf".into(),
                program: None
            })
        );
    }

    #[test]
    fn a_bad_recipe_line_says_what_to_type_instead() {
        assert_eq!(
            parse("recipe"),
            Err(ParseError::TooFew("recipe name steps".into()))
        );
        assert_eq!(parse("recipe Bad!"), Err(ParseError::BadRecipeName));
        assert!(matches!(
            parse("recipe loaf take grass"),
            Err(ParseError::BadStep(_))
        ));
        assert!(matches!(
            parse("recipe loaf toss grass 3"),
            Err(ParseError::BadStep(_))
        ));
        assert_eq!(
            parse("recipe loaf take gold 3"),
            Err(ParseError::UnknownItem("gold".into()))
        );
        assert!(matches!(
            parse("recipe loaf take grass 0"),
            Err(ParseError::OutOfRange { .. })
        ));
        // Fair recipes only: this one makes more worth than it takes.
        let Err(ParseError::Recipe(why)) = parse("recipe free take grass 1 make bread 1") else {
            panic!("expected an unfair recipe");
        };
        assert!(why.contains("more worth"));
    }

    #[test]
    fn a_bad_line_says_what_to_type_instead() {
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert_eq!(parse("   "), Err(ParseError::Empty));
        assert_eq!(parse("fly"), Err(ParseError::UnknownCommand("fly".into())));
        assert_eq!(
            parse("give fuel"),
            Err(ParseError::TooFew("give item count".into()))
        );
        assert_eq!(
            parse("rewind now"),
            Err(ParseError::TooMany("rewind".into()))
        );
        assert_eq!(
            parse("give fuel x"),
            Err(ParseError::NotANumber {
                arg: "count",
                word: "x".into()
            })
        );
        assert_eq!(
            parse("give fuel 0"),
            Err(ParseError::OutOfRange {
                arg: "count",
                min: 1,
                max: 999
            })
        );
        assert_eq!(
            parse("give gold 1"),
            Err(ParseError::UnknownItem("gold".into()))
        );
        assert_eq!(
            parse("set gold 0 0 0"),
            Err(ParseError::UnknownBlock("gold".into()))
        );
        assert_eq!(
            parse("spawn cow 0 0 0"),
            Err(ParseError::UnknownMob("cow".into()))
        );
        assert_eq!(
            parse("hack fly"),
            Err(ParseError::UnknownHack("fly".into()))
        );
        assert!(matches!(
            parse("go 1001 0 0"),
            Err(ParseError::OutOfRange { .. })
        ));
    }

    #[test]
    fn a_long_word_is_cut_short_in_the_error() {
        let Err(ParseError::UnknownCommand(word)) = parse(&"q".repeat(500)) else {
            panic!("expected an unknown command");
        };
        assert_eq!(word.chars().count(), ECHO);
    }

    proptest! {
        // Whatever is typed, the parser answers and never panics, and every message fits the
        // font so the status line shows it whole.
        #[test]
        fn any_text_is_answered_without_a_panic(line in "\\PC{0,80}") {
            let _ = parse(&line);
        }

        #[test]
        fn offsets_in_range_always_parse(x in -1000i32..=1000, y in -1000i32..=1000, z in -1000i32..=1000) {
            prop_assert_eq!(
                parse(&format!("go {x} {y} {z}")),
                Ok(Request::Go { by: [x, y, z] })
            );
        }

        #[test]
        fn errors_only_use_characters_the_font_has(line in "[a-z0-9 -]{0,40}") {
            if let Err(error) = parse(&line) {
                prop_assert!(error.to_string().chars().all(|c| crate::ui::font::glyph(c).is_some()));
            }
        }
    }
}
