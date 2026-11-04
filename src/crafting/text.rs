//! The text form of a player recipe: `take` and `make` lines, with `repeat` and `if` around
//! them, in the same style as a script (see [`crate::scripting::text`]).
//!
//! ```text
//! take grass 3
//! if count dirt above 4 {
//!   make stew 1
//! } else {
//!   make bread 1
//! }
//! ```
//!
//! Reading ends in [`RecipeProgram::try_new`], so a text can never hold a recipe that gives back
//! more worth than it takes.

use std::fmt::Write as _;

use crate::crafting::program::{RecipeProgram, Step};
use crate::items::registry::{self as items, ItemId};
use crate::scripting::text::{Node, TextError, item_named, number, test_of, test_text};

fn step_of(node: &Node<'_>) -> Result<Step, TextError> {
    let line = node.line;
    let rest = &node.words[1..];
    match node.words[0] {
        word @ ("take" | "make") => {
            if node.opens {
                return Err(TextError::new(
                    line,
                    format!("'{word}' has no steps inside it"),
                ));
            }
            let [item, count] = rest else {
                return Err(TextError::new(
                    line,
                    format!("write: {word} <item> <count>"),
                ));
            };
            let item = item_named(item, line)?;
            let count = number(count, line, "a count")?;
            Ok(if word == "take" {
                Step::Take { item, count }
            } else {
                Step::Make { item, count }
            })
        }
        "repeat" => {
            let ([times], true) = (rest, node.opens) else {
                return Err(TextError::new(line, "write: repeat <times> {"));
            };
            Ok(Step::Repeat {
                times: number(times, line, "the repeat count")?,
                body: steps_of(&node.then)?,
            })
        }
        "if" => {
            if !node.opens {
                return Err(TextError::new(line, "write: if <test> {"));
            }
            Ok(Step::If {
                test: test_of(rest, line)?,
                then: steps_of(&node.then)?,
                otherwise: steps_of(&node.otherwise)?,
            })
        }
        other => Err(TextError::new(
            line,
            format!("unknown step '{other}' (take, make, repeat, if)"),
        )),
    }
}

/// The steps that nodes stand for.
///
/// # Errors
///
/// Returns a [`TextError`] for the first line that is not a step.
pub fn steps_of(nodes: &[Node<'_>]) -> Result<Vec<Step>, TextError> {
    nodes.iter().map(step_of).collect()
}

/// Builds a recipe from nodes, reporting a rule of [`RecipeProgram`] on `line`.
///
/// # Errors
///
/// Returns a [`TextError`] for a line that is not a step, or the rule the steps break.
pub fn recipe_of(nodes: &[Node<'_>], line: usize) -> Result<RecipeProgram, TextError> {
    RecipeProgram::try_new(steps_of(nodes)?)
        .map_err(|error| TextError::new(line, error.to_string()))
}

fn item_text(item: ItemId) -> String {
    items::get(item).map_or_else(|| format!("item{}", item.0), |def| def.name.to_owned())
}

fn print_steps(list: &[Step], indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for step in list {
        // Writing to a String cannot fail.
        let _ = match step {
            Step::Take { item, count } => writeln!(out, "{pad}take {} {count}", item_text(*item)),
            Step::Make { item, count } => writeln!(out, "{pad}make {} {count}", item_text(*item)),
            Step::Repeat { times, body } => {
                let _ = writeln!(out, "{pad}repeat {times} {{");
                print_steps(body, indent + 1, out);
                writeln!(out, "{pad}}}")
            }
            Step::If {
                test,
                then,
                otherwise,
            } => {
                let _ = writeln!(out, "{pad}if {} {{", test_text(*test));
                print_steps(then, indent + 1, out);
                if otherwise.is_empty() {
                    writeln!(out, "{pad}}}")
                } else {
                    let _ = writeln!(out, "{pad}}} else {{");
                    print_steps(otherwise, indent + 1, out);
                    writeln!(out, "{pad}}}")
                }
            }
        };
    }
}

/// A recipe as text. Reading it back gives an equal recipe.
pub fn print(recipe: &RecipeProgram) -> String {
    let mut out = String::new();
    print_steps(recipe.steps(), 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::text::{Cursor, Stop};

    fn parse(text: &str) -> Result<RecipeProgram, TextError> {
        let mut cursor = Cursor::new(text);
        let (nodes, stop) = cursor.body(0)?;
        assert_eq!(stop, Stop::Eof);
        recipe_of(&nodes, 1)
    }

    #[test]
    fn a_recipe_reads_and_prints_back_the_same() {
        let text =
            "take grass 3\nif count dirt above 4 {\n  make stew 1\n} else {\n  make bread 1\n}\n";
        let recipe = parse(text).unwrap();
        assert_eq!(print(&recipe), text);
    }

    #[test]
    fn mistakes_name_their_line_and_say_what_to_write() {
        let error = parse("take grass 3\nmake gold 1\n").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.message.contains("no item called 'gold'"));
        assert_eq!(parse("take grass").unwrap_err().line, 1);
        assert_eq!(parse("hop 0 1 0").unwrap_err().line, 1);
    }

    #[test]
    fn an_unfair_recipe_is_refused_with_the_reason() {
        let error = parse("take grass 1\nmake bread 1\n").unwrap_err();
        assert!(error.message.contains("more worth"));
    }
}
