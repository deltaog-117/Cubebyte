//! Reading a typed line.

use thiserror::Error;

use crate::blocks::registry as blocks;
use crate::command::model::{ArgKind, ArgSpec, Request, Value};
use crate::command::table::{self, OFFSET_MAX};
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
    if rest.len() < spec.args.len() {
        return Err(ParseError::TooFew(table::usage(spec)));
    }
    if rest.len() > spec.args.len() {
        return Err(ParseError::TooMany(table::usage(spec)));
    }
    let values = spec
        .args
        .iter()
        .zip(rest)
        .map(|(arg, word)| value_of(arg, word))
        .collect::<Result<Vec<_>, _>>()?;
    (spec.build)(&values).ok_or(ParseError::Broken(spec.name))
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
