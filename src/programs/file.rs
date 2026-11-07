//! The programs file (`programs.nib`): every rule, every player recipe and every edited ready-made
//! script, as readable text.
//!
//! ```text
//! nib 3
//!
//! rule on feet
//! when hunger below 8
//! give bread 1
//! end
//!
//! rule on feet
//! on break
//! when count fuel below 3
//! give fuel 2
//! end
//!
//! recipe loaf
//! take grass 3
//! make bread 1
//! end
//!
//! recipe stairs
//! shape stone .
//! shape stone stone
//! make fuel 2
//! end
//!
//! script tower
//! build stone 0 1 0
//! end
//! ```
//!
//! The first line is the format version: this game writes `nib 4` and reads `nib 1` to
//! `nib 4`; a file with another version is refused instead of guessed at. A rule's header is
//! `rule on|off` and an anchor (`feet`, `mob` or `cell x y z`). What follows is either
//! `when <test>`, which makes a rule that looks at its test once a second, or `on <action>`
//! (`break`, `place`, `craft`, `eat` or `hit`, and from version 3 `hurt`, `respawn` or
//! `appear`), which makes a rule that fires right after the player does that or it happens, and may be followed by `when <test>` so it fires only if the test holds
//! then. An `on` rule without a `when` line has no test, so none is kept. The lines up to `end`
//! are the rule's program. A recipe's header is `recipe <name>`, and from version 4 its body may be
//! `shape` rows (one word per cell: an item, or `.` for an empty cell) and `make` lines, a recipe
//! crafted from the grid that spends one item per filled cell; a script's is `script <name>`,
//! the name of a ready-made script, and its lines replace that script's program. Version 1
//! files have rules that poll and recipes only. Everything goes through the same checks as
//! blocks in the game, so the file can never hold a program the editor could not build.
//! Reading is all or nothing: one bad line refuses the file and names the line, and the game
//! then leaves the file alone on exit so a typo cannot cost the player their work.
//!
//! Comments (`#`) and blank lines are free to write but are not kept: the game rewrites the
//! file from what it holds.

use std::fmt::Write as _;

use crate::crafting::custom::CustomRecipes;
use crate::crafting::text::{print as print_recipe, print_shaped, recipe_of, shaped_of};
use crate::logic::model::{Anchor, DEFAULT_TEST, PlayerAction, Rules, Trigger};
use crate::programs::scripts::ScriptEdits;
use crate::scripting::library;
use crate::scripting::program::Test;
use crate::scripting::text::{
    Cursor, Stop, TextError, number, print, program_of, test_of, test_text,
};
use crate::shared::voxel::WorldPos;

/// The format version this game writes.
pub const VERSION: u32 = 4;

/// The oldest format version this game still reads.
const OLDEST: u32 = 1;

/// What a programs file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Programs {
    pub rules: Rules,
    pub recipes: CustomRecipes,
    pub scripts: ScriptEdits,
}

fn anchor_text(anchor: Anchor) -> String {
    match anchor {
        Anchor::Feet => "feet".to_owned(),
        Anchor::Mob => "mob".to_owned(),
        Anchor::Cell(cell) => format!("cell {} {} {}", cell.x, cell.y, cell.z),
    }
}

/// The file's text for these rules, recipes and script edits. [`parse`] reads it back to equal
/// ones (a rule's rest timer is not kept: a rule starts ready; nor is the test of an action
/// rule that does not use it).
pub fn write(rules: &Rules, recipes: &CustomRecipes, scripts: &ScriptEdits) -> String {
    let mut out = format!(
        "# Your rules, recipes and script edits. The game rewrites this file when it closes, so comments are not kept.\nnib {VERSION}\n"
    );
    for (_, rule) in rules.iter() {
        // Writing to a String cannot fail.
        let _ = write!(
            out,
            "\nrule {} {}\n",
            if rule.is_on() { "on" } else { "off" },
            anchor_text(rule.anchor()),
        );
        if let Trigger::On { kind, .. } = rule.trigger() {
            let _ = writeln!(out, "on {}", kind.name());
        }
        if rule.trigger().uses_test() {
            let _ = writeln!(out, "when {}", test_text(rule.test()));
        }
        let _ = writeln!(out, "{}end", print(rule.program()));
    }
    for (_, recipe) in recipes.iter() {
        let body = match recipe.pattern() {
            Some(pattern) => print_shaped(pattern, recipe.program()),
            None => print_recipe(recipe.program()),
        };
        let _ = write!(out, "\nrecipe {}\n{body}end\n", recipe.name());
    }
    for (id, program) in scripts.iter() {
        let name = library::get(id).map_or("?", |script| script.name);
        let _ = write!(out, "\nscript {name}\n{}end\n", print(program));
    }
    out
}

fn anchor_of(words: &[&str], line: usize) -> Result<Anchor, TextError> {
    match words {
        ["feet"] => Ok(Anchor::Feet),
        ["mob"] => Ok(Anchor::Mob),
        ["cell", x, y, z] => Ok(Anchor::Cell(WorldPos {
            x: number(x, line, "a cell coordinate")?,
            y: number(y, line, "a cell coordinate")?,
            z: number(z, line, "a cell coordinate")?,
        })),
        _ => Err(TextError::new(
            line,
            "write the anchor as feet, mob or cell <x> <y> <z>",
        )),
    }
}

/// The first format version in which `on <kind>` is written.
const fn written_since(kind: PlayerAction) -> u32 {
    match kind {
        PlayerAction::Hurt | PlayerAction::Respawn | PlayerAction::MobAppears => 3,
        PlayerAction::Break
        | PlayerAction::Place
        | PlayerAction::Craft
        | PlayerAction::Eat
        | PlayerAction::Hit => 2,
    }
}

/// Reads a rule's trigger and test: `when <test>` for a polling rule, or `on <action>` and an
/// optional `when <test>` for an action rule (version 2 only). Returns the trigger, the test
/// and the number of the last line read.
fn trigger_of(
    cursor: &mut Cursor<'_>,
    version: u32,
    rule_line: usize,
) -> Result<(Trigger, Test, usize), TextError> {
    let first = cursor
        .take()
        .ok_or_else(|| TextError::new(rule_line, "a rule's next line is: when <test>"))?;
    match first.words.as_slice() {
        ["when", rest @ ..] => {
            let test = test_of(rest, first.number)?;
            Ok((Trigger::Poll, test, first.number))
        }
        ["on", word] if version >= 2 => {
            let kind = PlayerAction::from_name(word).ok_or_else(|| {
                TextError::new(
                    first.number,
                    "write: on break, place, craft, eat, hit, hurt, respawn or appear",
                )
            })?;
            if version < written_since(kind) {
                return Err(TextError::new(
                    first.number,
                    format!("on {word} needs a file that starts with 'nib 3'"),
                ));
            }
            match cursor
                .peek()
                .filter(|next| next.words.first() == Some(&"when"))
            {
                Some(_) => {
                    let when = cursor.take().ok_or_else(|| {
                        TextError::new(first.number, "a rule's next line is: when <test>")
                    })?;
                    let test = test_of(&when.words[1..], when.number)?;
                    Ok((Trigger::On { kind, gated: true }, test, when.number))
                }
                None => Ok((
                    Trigger::On { kind, gated: false },
                    DEFAULT_TEST,
                    first.number,
                )),
            }
        }
        _ => Err(TextError::new(
            rule_line,
            if version >= 2 {
                "a rule's next line is: when <test> or on <action>"
            } else {
                "a rule's next line is: when <test>"
            },
        )),
    }
}

/// Reads a programs file.
///
/// # Errors
///
/// Returns a [`TextError`] naming the first line to fix: a missing or unknown version, an
/// unknown section, a block or step that is not valid, a program that breaks a rule of the
/// language, a script that does not exist or is edited twice, or more rules or recipes than
/// the game allows.
///
/// # Examples
///
/// ```
/// use minecraft_clone::programs::file::{parse, write};
///
/// let text = "nib 2\nrecipe loaf\ntake grass 3\nmake bread 1\nend\n";
/// let programs = parse(text).unwrap();
/// assert_eq!(programs.recipes.len(), 1);
/// assert_eq!(
///     parse(&write(&programs.rules, &programs.recipes, &programs.scripts)),
///     Ok(programs)
/// );
/// assert!(parse("nib 9\n").is_err());
/// ```
pub fn parse(text: &str) -> Result<Programs, TextError> {
    let mut cursor = Cursor::new(text);
    let version = match cursor.take() {
        Some(line) => match line.words.as_slice() {
            ["nib", word] => match word.parse::<u32>() {
                Ok(version) if (OLDEST..=VERSION).contains(&version) => version,
                _ => {
                    return Err(TextError::new(
                        line.number,
                        format!("this game reads 'nib {OLDEST}' and 'nib {VERSION}' files only"),
                    ));
                }
            },
            ["nib", ..] => {
                return Err(TextError::new(
                    line.number,
                    format!("this game reads 'nib {OLDEST}' and 'nib {VERSION}' files only"),
                ));
            }
            _ => {
                return Err(TextError::new(
                    line.number,
                    format!("the first line must be 'nib {VERSION}'"),
                ));
            }
        },
        None => {
            return Err(TextError::new(
                1,
                format!("empty file: start with 'nib {VERSION}'"),
            ));
        }
    };
    let mut programs = Programs::default();
    while let Some(header) = cursor.take() {
        let line = header.number;
        match header.words.as_slice() {
            ["rule", state @ ("on" | "off"), anchor @ ..] => {
                let anchor = anchor_of(anchor, line)?;
                let (trigger, test, last) = trigger_of(&mut cursor, version, line)?;
                let (nodes, stop) = cursor.body(0)?;
                if stop != Stop::End {
                    return Err(TextError::new(line, "this rule has no 'end'"));
                }
                let program = program_of(&nodes, last)?;
                let id = programs
                    .rules
                    .add_with(trigger, test, program, anchor)
                    .map_err(|error| TextError::new(line, error.to_string()))?;
                if *state == "off" {
                    programs.rules.switch(id);
                }
            }
            ["recipe", name] => {
                let (nodes, stop) = cursor.body(0)?;
                if stop != Stop::End {
                    return Err(TextError::new(line, "this recipe has no 'end'"));
                }
                let shaped = nodes
                    .first()
                    .is_some_and(|node| node.words.first() == Some(&"shape"));
                if shaped && version < 4 {
                    return Err(TextError::new(line, "shape rows need 'nib 4'"));
                }
                let added = if shaped {
                    let (pattern, program) = shaped_of(&nodes, line)?;
                    programs.recipes.add_shaped(name, pattern, program)
                } else {
                    programs.recipes.add(name, recipe_of(&nodes, line)?)
                };
                added.map_err(|error| TextError::new(line, error.to_string()))?;
            }
            ["script", name] if version >= 2 => {
                let (id, _) = library::find(name).ok_or_else(|| {
                    TextError::new(line, format!("no ready-made script called '{name}'"))
                })?;
                if programs.scripts.edited(id).is_some() {
                    return Err(TextError::new(line, format!("'{name}' is edited twice")));
                }
                let (nodes, stop) = cursor.body(0)?;
                if stop != Stop::End {
                    return Err(TextError::new(line, "this script has no 'end'"));
                }
                let program = program_of(&nodes, line)?;
                programs
                    .scripts
                    .set(id, program)
                    .map_err(|error| TextError::new(line, error.to_string()))?;
            }
            _ => {
                return Err(TextError::new(
                    line,
                    if version >= 2 {
                        "expected 'rule on|off <anchor>', 'recipe <name>' or 'script <name>'"
                    } else {
                        "expected 'rule on|off <anchor>' or 'recipe <name>'"
                    },
                ));
            }
        }
    }
    Ok(programs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::registry::ItemId;
    use crate::scripting::program::{Block, Compare, Plan, Program, Read, Test};

    const SAMPLE: &str = "nib 1\n\n\
        rule on feet\nwhen hunger below 8\ngive bread 1\nend\n\n\
        rule off cell 3 -4 5\nwhen count dirt below 2\nrepeat 2 {\n  build stone 0 1 0\n}\nend\n\n\
        rule on mob\nwhen mobs above 0\nsummon 2 0 0\nend\n\n\
        recipe loaf\ntake grass 3\nmake bread 1\nend\n\n\
        recipe feast\ntake grass 4\ntake dirt 2\nif count stone above 5 {\n  make stew 2\n} else {\n  make bread 2\n}\nend\n";

    #[test]
    fn a_file_reads_and_writes_back_the_same() {
        let programs = parse(SAMPLE).unwrap();
        assert_eq!(programs.rules.len(), 3);
        assert_eq!(programs.recipes.len(), 2);
        let again = parse(&write(
            &programs.rules,
            &programs.recipes,
            &programs.scripts,
        ))
        .unwrap();
        assert_eq!(again, programs);
    }

    #[test]
    fn the_switch_and_the_anchor_survive() {
        let programs = parse(SAMPLE).unwrap();
        let rules: Vec<_> = programs.rules.iter().map(|(_, rule)| rule).collect();
        assert!(rules[0].is_on() && !rules[1].is_on());
        assert_eq!(rules[0].anchor(), Anchor::Feet);
        assert_eq!(
            rules[1].anchor(),
            Anchor::Cell(WorldPos { x: 3, y: -4, z: 5 })
        );
        assert_eq!(rules[2].anchor(), Anchor::Mob);
    }

    #[test]
    fn an_empty_world_writes_just_the_version() {
        let text = write(
            &Rules::default(),
            &CustomRecipes::default(),
            &ScriptEdits::default(),
        );
        assert_eq!(parse(&text), Ok(Programs::default()));
        assert!(text.contains("nib 4"));
    }

    #[test]
    fn the_version_line_is_required_and_checked() {
        assert_eq!(parse("").unwrap_err().line, 1);
        assert_eq!(parse("recipe a\n").unwrap_err().line, 1);
        let error = parse("nib 9\n").unwrap_err();
        assert!(error.message.contains("nib 1") && error.message.contains("nib 4"));
        assert!(parse("nib 1\n").is_ok() && parse("nib 2\n").is_ok() && parse("nib 3\n").is_ok());
        assert!(parse("nib 4\n").is_ok());
    }

    #[test]
    fn mistakes_name_their_line() {
        let bad = |text: &str| parse(text).unwrap_err();
        assert_eq!(bad("nib 1\nstuff\n").line, 2);
        assert_eq!(bad("nib 1\nrule on feet\nhop 0 1 0\nend\n").line, 2);
        assert_eq!(
            bad("nib 1\nrule on feet\nwhen hunger below 8\nhop 0 1\nend\n").line,
            4
        );
        assert_eq!(bad("nib 1\nrule on nowhere\n").line, 2);
        assert_eq!(
            bad("nib 1\nrecipe a\ntake grass 3\nmake gold 1\nend\n").line,
            4
        );
        assert_eq!(bad("nib 1\nrecipe a\ntake grass 3\nmake bread 1\n").line, 2);
        assert_eq!(
            bad("nib 1\nrecipe bread\ntake grass 3\nmake bread 1\nend\n").line,
            2
        );
    }

    #[test]
    fn an_unfair_recipe_in_a_file_is_refused() {
        let error = parse("nib 1\nrecipe free\nmake sword 1\nend\n").unwrap_err();
        assert!(error.message.contains("more worth"));
    }

    #[test]
    fn too_many_rules_are_refused_not_dropped() {
        let one = "rule on feet\nwhen hunger below 8\ngive bread 1\nend\n";
        let text = format!("nib 1\n{}", one.repeat(crate::logic::model::MAX_RULES + 1));
        assert!(parse(&text).is_err());
    }

    #[test]
    fn rules_built_in_the_game_are_written_and_read_back() {
        let mut rules = Rules::default();
        let program = Program::try_new(vec![Block::Do(Plan::Give {
            item: ItemId::BREAD,
            count: 2,
        })])
        .unwrap();
        let test = Test {
            read: Read::Health,
            compare: Compare::Below,
            than: 5,
        };
        rules.add(test, program, Anchor::Mob).unwrap();
        let text = write(&rules, &CustomRecipes::default(), &ScriptEdits::default());
        assert_eq!(parse(&text).unwrap().rules, rules);
    }

    const SAMPLE_2: &str = "nib 2\n\n\
        rule on feet\non break\ngive fuel 1\nend\n\n\
        rule off mob\non craft\nwhen count fuel below 3\ngive fuel 2\nend\n\n\
        rule on feet\nwhen hunger below 8\ngive bread 1\nend\n\n\
        recipe loaf\ntake grass 3\nmake bread 1\nend\n\n\
        script tower\nbuild stone 0 4 0\nend\n";

    #[test]
    fn action_rules_and_script_edits_read_and_write_back_the_same() {
        let programs = parse(SAMPLE_2).unwrap();
        let rules: Vec<_> = programs.rules.iter().map(|(_, rule)| rule).collect();
        assert_eq!(
            rules[0].trigger(),
            Trigger::On {
                kind: PlayerAction::Break,
                gated: false
            }
        );
        assert_eq!(
            rules[1].trigger(),
            Trigger::On {
                kind: PlayerAction::Craft,
                gated: true
            }
        );
        assert_eq!(rules[2].trigger(), Trigger::Poll);
        assert_eq!(programs.scripts.len(), 1);
        let again = parse(&write(
            &programs.rules,
            &programs.recipes,
            &programs.scripts,
        ))
        .unwrap();
        assert_eq!(again, programs);
    }

    #[test]
    fn version_1_files_still_load_and_are_rewritten_as_version_4() {
        let programs = parse(SAMPLE).unwrap();
        let text = write(&programs.rules, &programs.recipes, &programs.scripts);
        assert!(text.contains("\nnib 4\n"));
        assert!(
            programs
                .rules
                .iter()
                .all(|(_, rule)| rule.trigger() == Trigger::Poll)
        );
    }

    #[test]
    fn version_1_has_no_action_rules_or_script_sections() {
        let bad = |text: &str| parse(text).unwrap_err();
        assert_eq!(
            bad("nib 1\nrule on feet\non break\ngive fuel 1\nend\n").line,
            2
        );
        assert_eq!(bad("nib 1\nscript tower\nbuild stone 0 1 0\nend\n").line, 2);
    }

    #[test]
    fn action_rule_and_script_mistakes_name_their_line() {
        let bad = |text: &str| parse(text).unwrap_err();
        assert_eq!(
            bad("nib 2\nrule on feet\non jump\ngive fuel 1\nend\n").line,
            3
        );
        assert_eq!(bad("nib 2\nrule on feet\non break extra\nend\n").line, 2);
        assert_eq!(
            bad("nib 2\nscript nowhere\nbuild stone 0 1 0\nend\n").line,
            2
        );
        assert_eq!(
            bad("nib 2\nscript tower\nbuild stone 0 1 0\nend\nscript tower\nbuild stone 0 2 0\nend\n")
                .line,
            5
        );
        assert_eq!(bad("nib 2\nscript tower\nbuild stone 0 1 0\n").line, 2);
    }

    #[test]
    fn a_script_edit_equal_to_the_built_in_one_is_not_kept() {
        let (_, tower) = library::find("tower").unwrap();
        let text = format!("nib 2\nscript tower\n{}end\n", print(tower.program()));
        assert!(parse(&text).unwrap().scripts.is_empty());
    }

    #[test]
    fn action_rules_built_in_the_game_are_written_and_read_back() {
        let mut rules = Rules::default();
        let program = Program::try_new(vec![Block::Do(Plan::Give {
            item: ItemId::FUEL,
            count: 1,
        })])
        .unwrap();
        let gated = Test {
            read: Read::Count { item: ItemId::FUEL },
            compare: Compare::Below,
            than: 4,
        };
        for (kind, gated_on) in [(PlayerAction::Place, true), (PlayerAction::Eat, false)] {
            let test = if gated_on { gated } else { DEFAULT_TEST };
            rules
                .add_with(
                    Trigger::On {
                        kind,
                        gated: gated_on,
                    },
                    test,
                    program.clone(),
                    Anchor::Feet,
                )
                .unwrap();
        }
        let text = write(&rules, &CustomRecipes::default(), &ScriptEdits::default());
        assert_eq!(parse(&text).unwrap().rules, rules);
    }

    #[test]
    fn rules_that_react_to_what_happens_are_written_and_read_back() {
        let text = "nib 3\n\n\
            rule on feet\non hurt\nwhen health below 6\ngive fuel 1\nend\n\n\
            rule on mob\non appear\ngive fuel 1\nend\n\n\
            rule on feet\non respawn\ngive fuel 1\nend\n";
        let programs = parse(text).unwrap();
        let kinds: Vec<_> = programs
            .rules
            .iter()
            .map(|(_, rule)| rule.trigger())
            .collect();
        assert_eq!(
            kinds,
            [
                Trigger::On {
                    kind: PlayerAction::Hurt,
                    gated: true
                },
                Trigger::On {
                    kind: PlayerAction::MobAppears,
                    gated: false
                },
                Trigger::On {
                    kind: PlayerAction::Respawn,
                    gated: false
                },
            ]
        );
        let again = parse(&write(
            &programs.rules,
            &programs.recipes,
            &programs.scripts,
        ))
        .unwrap();
        assert_eq!(again, programs);
    }

    #[test]
    fn a_shaped_recipe_is_written_and_read_back() {
        let text = "nib 4\nrecipe stairs\nshape stone .\nshape stone stone\nmake fuel 2\nend\n";
        let programs = parse(text).unwrap();
        let (_, recipe) = programs.recipes.iter().next().unwrap();
        assert!(recipe.pattern().is_some());
        let again = write(&programs.rules, &programs.recipes, &programs.scripts);
        assert_eq!(parse(&again), Ok(programs));
    }

    #[test]
    fn shape_rows_need_a_version_4_file() {
        let error = parse("nib 3\nrecipe stairs\nshape stone\nmake fuel 1\nend\n").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.message.contains("nib 4"));
    }

    #[test]
    fn the_new_reactions_need_a_version_3_file() {
        for word in ["hurt", "respawn", "appear"] {
            let text = format!("nib 2\nrule on feet\non {word}\ngive fuel 1\nend\n");
            let error = parse(&text).unwrap_err();
            assert_eq!(error.line, 3, "{word}");
            assert!(error.message.contains("nib 3"), "{word}");
        }
    }
}
