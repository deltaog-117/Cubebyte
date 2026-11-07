//! A blueprint as text: the programs file grammar plus one `blueprint` section.
//!
//! ```text
//! nib 3
//!
//! blueprint tower
//! build stone 0 0 0
//! build stone 0 1 0
//! end
//!
//! recipe loaf
//! take grass 3
//! make bread 1
//! end
//! ```
//!
//! Rules and recipes are written exactly as in `programs.nib`, and are read by the same
//! reader, so every check on them still applies. The `blueprint <name>` section is a build: a
//! program placed with its working cell on the player's feet. A blueprint cannot change a
//! ready-made script. Reading is all or nothing and an error names the line to fix. Text
//! longer than [`MAX_TEXT`] is refused before it is read, since it came from outside.

use std::fmt::Write as _;

use crate::blueprints::model::{Blueprint, BlueprintError, Build};
use crate::programs::file as programs_file;
use crate::programs::scripts::ScriptEdits;
use crate::scripting::text::{TextError, parse as parse_program, print};

/// Longest text that is read, in bytes. A full set of rules, recipes and a 64-block build
/// stays far below it.
pub const MAX_TEXT: usize = 32 * 1024;

/// The text of `blueprint`, which [`parse`] reads back to an equal one.
pub fn write(blueprint: &Blueprint) -> String {
    let shared = programs_file::write(
        blueprint.rules(),
        blueprint.recipes(),
        &ScriptEdits::default(),
    );
    // The programs file starts with a comment about itself, which does not fit here.
    let body = shared.split_once('\n').map_or("", |(_, rest)| rest);
    let mut out =
        String::from("# A blueprint. Copy all of it, then use the paste command in the game.\n");
    out.push_str(body);
    if let Some(build) = blueprint.build() {
        // Writing to a String cannot fail.
        let _ = write!(
            out,
            "\nblueprint {}\n{}end\n",
            build.name(),
            print(build.program())
        );
    }
    out
}

/// The words of `raw` before any `#` comment.
fn words_of(raw: &str) -> Vec<&str> {
    raw.split('#')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect()
}

/// Reads a blueprint.
///
/// # Errors
///
/// Returns a [`TextError`] for text that is too long, a line that is not part of the
/// grammar, a rule, recipe or program that breaks its checks, a second `blueprint` section, a
/// `script` section, or a text with nothing in it.
///
/// # Examples
///
/// ```
/// use minecraft_clone::blueprints::text::parse;
///
/// let blueprint = parse("nib 3\nblueprint tower\nbuild stone 0 0 0\nend\n").unwrap();
/// assert_eq!(blueprint.build().map(|build| build.name()), Some("tower"));
/// assert!(parse("nib 3\n").is_err());
/// ```
pub fn parse(text: &str) -> Result<Blueprint, TextError> {
    if text.len() > MAX_TEXT {
        return Err(TextError::new(
            1,
            format!("this text is longer than {MAX_TEXT} bytes, which is too long for a blueprint"),
        ));
    }
    let mut rest = String::with_capacity(text.len());
    let mut section: Option<(usize, String, String)> = None;
    let mut build: Option<(String, String)> = None;
    let mut open_line = 0;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let words = words_of(raw);
        if let Some((line, name, body)) = section.as_mut() {
            if words == ["end"] {
                build = Some((std::mem::take(name), std::mem::take(body)));
                open_line = *line;
                section = None;
            } else {
                body.push_str(raw);
                body.push('\n');
            }
            // Blank lines keep every later line number the same as in the pasted text.
            rest.push('\n');
            continue;
        }
        match words.as_slice() {
            ["blueprint", name] => {
                if build.is_some() {
                    return Err(TextError::new(number, "a blueprint holds one build only"));
                }
                // The body is read with its first line numbered as in the pasted text.
                let padding = "\n".repeat(number);
                section = Some((number, (*name).to_owned(), padding));
                rest.push('\n');
            }
            ["blueprint", ..] => {
                return Err(TextError::new(
                    number,
                    "write the build as: blueprint <name>",
                ));
            }
            _ => {
                rest.push_str(raw);
                rest.push('\n');
            }
        }
    }
    if let Some((line, ..)) = section {
        return Err(TextError::new(line, "this blueprint has no 'end'"));
    }
    let programs = programs_file::parse(&rest)?;
    if !programs.scripts.is_empty() {
        return Err(TextError::new(
            1,
            "a blueprint cannot change a ready-made script",
        ));
    }
    let build = build
        .map(|(name, body)| {
            let program = parse_program(&body)?;
            Build::try_new(&name, program)
                .map_err(|error| TextError::new(open_line, error.to_string()))
        })
        .transpose()?;
    Blueprint::try_new(programs.rules, programs.recipes, build).map_err(|error| match error {
        BlueprintError::Empty => TextError::new(1, error.to_string()),
        BlueprintError::BadName => TextError::new(open_line, error.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::scripting::program::{Block, Plan};
    use crate::shared::voxel::BlockId;

    const SAMPLE: &str = "nib 3\n\n\
        rule on feet\nwhen hunger below 8\ngive bread 1\nend\n\n\
        recipe loaf\ntake grass 3\nmake bread 1\nend\n\n\
        blueprint tower\nbuild stone 0 0 0\nbuild stone 0 1 0\nend\n";

    #[test]
    fn a_blueprint_round_trips_through_its_text() {
        let blueprint = parse(SAMPLE).unwrap();
        assert_eq!(parse(&write(&blueprint)), Ok(blueprint));
    }

    #[test]
    fn rules_recipes_and_the_build_are_all_kept() {
        let blueprint = parse(SAMPLE).unwrap();
        assert_eq!(blueprint.rules().len(), 1);
        assert_eq!(blueprint.recipes().len(), 1);
        assert_eq!(blueprint.build().map(Build::name), Some("tower"));
    }

    #[test]
    fn a_bad_line_in_the_build_is_reported_at_its_place_in_the_text() {
        let error = parse("nib 3\n\nblueprint tower\nbuild stone 0 0 0\nbuild lava 0 1 0\nend\n")
            .unwrap_err();
        assert_eq!(error.line, 5);
    }

    #[test]
    fn a_bad_line_in_a_rule_is_reported_at_its_place_in_the_text() {
        let error = parse("nib 3\n\nblueprint tower\nbuild stone 0 0 0\nend\n\nrule on feet\nwhen nonsense\nend\n")
            .unwrap_err();
        assert_eq!(error.line, 8);
    }

    #[test]
    fn nothing_in_it_is_refused() {
        assert!(parse("nib 3\n").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn a_second_build_an_open_build_and_a_script_are_refused() {
        let one = "blueprint a\nbuild stone 0 0 0\nend\n";
        assert!(parse(&format!("nib 3\n{one}{one}")).is_err());
        assert!(parse("nib 3\nblueprint a\nbuild stone 0 0 0\n").is_err());
        assert!(parse("nib 3\nscript tower\nbuild stone 0 0 0\nend\n").is_err());
    }

    #[test]
    fn text_over_the_limit_is_refused_before_reading() {
        let long = format!("nib 3\n# {}\n", "x".repeat(MAX_TEXT));
        assert!(parse(&long).is_err());
    }

    proptest! {
        #[test]
        fn any_build_of_placed_blocks_round_trips(
            cells in proptest::collection::vec((-4i32..=4, 0i32..8, -4i32..=4, 1u16..=3), 1..=64),
        ) {
            let blocks = cells
                .into_iter()
                .map(|(x, y, z, block)| {
                    Block::Do(Plan::Build { by: [x, y, z], block: BlockId(block) })
                })
                .collect();
            let program = crate::scripting::program::Program::try_new(blocks).unwrap();
            let build = Build::try_new("shape", program).unwrap();
            let blueprint = Blueprint::try_new(
                Default::default(),
                Default::default(),
                Some(build),
            )
            .unwrap();
            prop_assert_eq!(parse(&write(&blueprint)), Ok(blueprint));
        }
    }
}
