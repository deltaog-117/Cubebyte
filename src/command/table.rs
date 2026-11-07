//! The one table of commands. The parser, the dictionary and the error messages all read it.
//!
//! A command's ID is its position in [`COMMANDS`], so a new command is only ever appended.
//! Positions are offsets in whole blocks from the cell the player stands in, as in the
//! ready-made scripts.

use crate::command::model::{ArgKind, ArgSpec, CommandId, Request, Spec, Value};

const fn arg(name: &'static str, kind: ArgKind) -> ArgSpec {
    ArgSpec { name, kind }
}

const ITEM: ArgSpec = arg("item", ArgKind::Item);
const BLOCK: ArgSpec = arg("block", ArgKind::Block);
const MOB: ArgSpec = arg("mob", ArgKind::Mob);
const X: ArgSpec = arg("x", ArgKind::Offset);
const Y: ArgSpec = arg("y", ArgKind::Offset);
const Z: ArgSpec = arg("z", ArgKind::Offset);

/// Most items one `give` may ask for.
pub const GIVE_MAX: u16 = 999;

/// Furthest back a `zone` may reach, as the log keeps that much history.
pub const ZONE_MAX: u16 = crate::events::rewind::HISTORY_WINDOW as u16;

/// Furthest an offset may be written, in blocks. Farther is out of any script's range anyway.
pub const OFFSET_MAX: i32 = 1000;

const COMMANDS: [Spec; 12] = [
    Spec {
        name: "help",
        summary: "open this table",
        args: &[],
        build: |values| matches!(values, []).then_some(Request::Help),
    },
    Spec {
        name: "give",
        summary: "add items to the pack, costs fuel",
        args: &[ITEM, arg("count", ArgKind::Count { max: GIVE_MAX })],
        build: |values| match *values {
            [Value::Item(item), Value::Count(count)] => Some(Request::Give { item, count }),
            _ => None,
        },
    },
    Spec {
        name: "go",
        summary: "move by this many blocks",
        args: &[X, Y, Z],
        build: |values| match *values {
            [Value::Offset(x), Value::Offset(y), Value::Offset(z)] => {
                Some(Request::Go { by: [x, y, z] })
            }
            _ => None,
        },
    },
    Spec {
        name: "set",
        summary: "change one block nearby",
        args: &[BLOCK, X, Y, Z],
        build: |values| match *values {
            [
                Value::Block(block),
                Value::Offset(x),
                Value::Offset(y),
                Value::Offset(z),
            ] => Some(Request::Set {
                block,
                by: [x, y, z],
            }),
            _ => None,
        },
    },
    Spec {
        name: "spawn",
        summary: "put a mob nearby",
        args: &[MOB, X, Y, Z],
        build: |values| match *values {
            [
                Value::Mob,
                Value::Offset(x),
                Value::Offset(y),
                Value::Offset(z),
            ] => Some(Request::Spawn { by: [x, y, z] }),
            _ => None,
        },
    },
    Spec {
        name: "rewind",
        summary: "undo your last edit, costs fuel",
        args: &[],
        build: |values| matches!(values, []).then_some(Request::Rewind),
    },
    Spec {
        name: "redo",
        summary: "bring back what rewind undid",
        args: &[],
        build: |values| matches!(values, []).then_some(Request::Redo),
    },
    Spec {
        name: "zone",
        summary: "restore nearby blocks, count back",
        args: &[arg("count", ArgKind::Count { max: ZONE_MAX })],
        build: |values| match *values {
            [Value::Count(back)] => Some(Request::Zone { back }),
            _ => None,
        },
    },
    Spec {
        name: "hack",
        summary: "run a script on what you aim at",
        args: &[arg("name", ArgKind::Hack)],
        build: |values| match *values {
            [Value::Hack(script)] => Some(Request::Hack { script }),
            _ => None,
        },
    },
    Spec {
        name: "recipe",
        summary: "make one, or open the editor",
        args: &[arg("name", ArgKind::Name), arg("steps", ArgKind::Steps)],
        build: |values| match values {
            [Value::Name(name), Value::Steps(program)] => Some(Request::Recipe {
                name: name.clone(),
                program: program.clone(),
            }),
            _ => None,
        },
    },
    Spec {
        name: "copy",
        summary: "copy rules, recipes or build",
        args: &[arg("what", ArgKind::Name)],
        build: |values| match values {
            [Value::Name(what)] => Some(Request::Copy { what: what.clone() }),
            _ => None,
        },
    },
    Spec {
        name: "paste",
        summary: "use the blueprint you copied",
        args: &[],
        build: |values| matches!(values, []).then_some(Request::Paste),
    },
];

/// The command at `id`, or `None` for a position no command uses.
pub fn get(id: CommandId) -> Option<&'static Spec> {
    COMMANDS.get(usize::from(id.0))
}

/// Every command, in table order. This is the dictionary.
pub fn ids() -> impl Iterator<Item = CommandId> {
    // The table is far shorter than 256 entries.
    (0..COMMANDS.len() as u8).map(CommandId)
}

/// The command called `name`, in any letter case.
pub fn find(name: &str) -> Option<(CommandId, &'static Spec)> {
    ids()
        .filter_map(|id| Some((id, get(id)?)))
        .find(|(_, spec)| spec.name.eq_ignore_ascii_case(name))
}

/// How the command is written: its name, then one word per argument, such as "give item count".
pub fn usage(spec: &Spec) -> String {
    let mut text = spec.name.to_string();
    for arg in spec.args {
        text.push(' ');
        text.push_str(arg.name);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::registry as items;

    /// A word that is valid for `kind`.
    fn sample(kind: ArgKind) -> String {
        match kind {
            ArgKind::Item => items::get(items::ids().next().unwrap())
                .unwrap()
                .name
                .into(),
            ArgKind::Block => "stone".into(),
            ArgKind::Mob => "zombie".into(),
            ArgKind::Count { .. } => "1".into(),
            ArgKind::Offset => "0".into(),
            ArgKind::Hack => "tower".into(),
            ArgKind::Name => "loaf".into(),
            ArgKind::Steps => String::new(),
        }
    }

    #[test]
    fn names_are_unique_lowercase_words() {
        let names: Vec<_> = ids().filter_map(get).map(|spec| spec.name).collect();
        for (index, name) in names.iter().enumerate() {
            assert!(name.chars().all(|c| c.is_ascii_lowercase()), "{name}");
            assert!(!names[index + 1..].contains(name), "{name} twice");
        }
    }

    #[test]
    fn everything_the_interface_shows_has_a_glyph() {
        for spec in ids().filter_map(get) {
            for c in usage(spec).chars().chain(spec.summary.chars()) {
                assert!(
                    crate::ui::font::glyph(c).is_some(),
                    "{c:?} in {}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn each_command_builds_a_request_from_sample_values() {
        for spec in ids().filter_map(get) {
            let line = std::iter::once(spec.name.to_string())
                .chain(spec.args.iter().map(|arg| sample(arg.kind)))
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                crate::command::parse::parse(&line).is_ok(),
                "{line} did not parse"
            );
        }
    }

    #[test]
    fn find_ignores_letter_case_and_the_usage_starts_with_the_name() {
        let (id, spec) = find("GiVe").unwrap();
        assert_eq!(get(id).unwrap().name, "give");
        assert_eq!(usage(spec), "give item count");
        assert!(find("nope").is_none());
    }
}
