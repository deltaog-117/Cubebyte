//! The programs file (`programs.nib`): every rule and every player recipe, as readable text.
//!
//! ```text
//! nib 1
//!
//! rule on feet
//! when hunger below 8
//! give bread 1
//! end
//!
//! recipe loaf
//! take grass 3
//! make bread 1
//! end
//! ```
//!
//! The first line is the format version; a file with another version is refused instead of
//! guessed at. A rule's header is `rule on|off` and an anchor (`feet`, `mob` or `cell x y z`),
//! its `when` line is the test, and the lines up to `end` are its program. A recipe's header is
//! `recipe <name>`. Everything goes through the same checks as blocks in the game, so the file
//! can never hold a program the editor could not build. Reading is all or nothing: one bad line
//! refuses the file and names the line, and the game then leaves the file alone on exit so a
//! typo cannot cost the player their work.
//!
//! Comments (`#`) and blank lines are free to write but are not kept: the game rewrites the
//! file from what it holds.

use std::fmt::Write as _;

use crate::crafting::custom::CustomRecipes;
use crate::crafting::text::{print as print_recipe, recipe_of};
use crate::logic::model::{Anchor, Rules};
use crate::scripting::text::{
    Cursor, Stop, TextError, number, print, program_of, test_of, test_text,
};
use crate::shared::voxel::WorldPos;

/// The format version this game writes and reads.
pub const VERSION: u32 = 1;

/// What a programs file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Programs {
    pub rules: Rules,
    pub recipes: CustomRecipes,
}

fn anchor_text(anchor: Anchor) -> String {
    match anchor {
        Anchor::Feet => "feet".to_owned(),
        Anchor::Mob => "mob".to_owned(),
        Anchor::Cell(cell) => format!("cell {} {} {}", cell.x, cell.y, cell.z),
    }
}

/// The file's text for these rules and recipes. [`parse`] reads it back to equal ones (a rule's
/// rest timer is not kept: a rule starts ready).
pub fn write(rules: &Rules, recipes: &CustomRecipes) -> String {
    let mut out = format!(
        "# Your rules and recipes. The game rewrites this file when it closes, so comments are not kept.\nnib {VERSION}\n"
    );
    for (_, rule) in rules.iter() {
        // Writing to a String cannot fail.
        let _ = write!(
            out,
            "\nrule {} {}\nwhen {}\n{}end\n",
            if rule.is_on() { "on" } else { "off" },
            anchor_text(rule.anchor()),
            test_text(rule.test()),
            print(rule.program()),
        );
    }
    for (_, recipe) in recipes.iter() {
        let _ = write!(
            out,
            "\nrecipe {}\n{}end\n",
            recipe.name(),
            print_recipe(recipe.program()),
        );
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

/// Reads a programs file.
///
/// # Errors
///
/// Returns a [`TextError`] naming the first line to fix: a missing or unknown version, an
/// unknown section, a block or step that is not valid, a program that breaks a rule of the
/// language, or more rules or recipes than the game allows.
///
/// # Examples
///
/// ```
/// use minecraft_clone::programs::file::{parse, write};
///
/// let text = "nib 1\nrecipe loaf\ntake grass 3\nmake bread 1\nend\n";
/// let programs = parse(text).unwrap();
/// assert_eq!(programs.recipes.len(), 1);
/// assert_eq!(parse(&write(&programs.rules, &programs.recipes)), Ok(programs));
/// assert!(parse("nib 9\n").is_err());
/// ```
pub fn parse(text: &str) -> Result<Programs, TextError> {
    let mut cursor = Cursor::new(text);
    match cursor.take() {
        Some(line) if line.words.as_slice() == ["nib", "1"] => {}
        Some(line) if line.words.first() == Some(&"nib") => {
            return Err(TextError::new(
                line.number,
                format!("this game reads 'nib {VERSION}' files only"),
            ));
        }
        Some(line) => {
            return Err(TextError::new(
                line.number,
                format!("the first line must be 'nib {VERSION}'"),
            ));
        }
        None => {
            return Err(TextError::new(
                1,
                format!("empty file: start with 'nib {VERSION}'"),
            ));
        }
    }
    let mut programs = Programs::default();
    while let Some(header) = cursor.take() {
        let line = header.number;
        match header.words.as_slice() {
            ["rule", state @ ("on" | "off"), anchor @ ..] => {
                let anchor = anchor_of(anchor, line)?;
                let when = cursor
                    .take()
                    .filter(|when| when.words.first() == Some(&"when"))
                    .ok_or_else(|| TextError::new(line, "a rule's next line is: when <test>"))?;
                let test = test_of(&when.words[1..], when.number)?;
                let (nodes, stop) = cursor.body(0)?;
                if stop != Stop::End {
                    return Err(TextError::new(line, "this rule has no 'end'"));
                }
                let program = program_of(&nodes, when.number)?;
                let id = programs
                    .rules
                    .add(test, program, anchor)
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
                let recipe = recipe_of(&nodes, line)?;
                programs
                    .recipes
                    .add(name, recipe)
                    .map_err(|error| TextError::new(line, error.to_string()))?;
            }
            _ => {
                return Err(TextError::new(
                    line,
                    "expected 'rule on|off <anchor>' or 'recipe <name>'",
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
        let again = parse(&write(&programs.rules, &programs.recipes)).unwrap();
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
        let text = write(&Rules::default(), &CustomRecipes::default());
        assert_eq!(parse(&text), Ok(Programs::default()));
        assert!(text.contains("nib 1"));
    }

    #[test]
    fn the_version_line_is_required_and_checked() {
        assert_eq!(parse("").unwrap_err().line, 1);
        assert_eq!(parse("recipe a\n").unwrap_err().line, 1);
        let error = parse("nib 2\n").unwrap_err();
        assert!(error.message.contains("nib 1"));
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
        let text = write(&rules, &CustomRecipes::default());
        assert_eq!(parse(&text).unwrap().rules, rules);
    }
}
