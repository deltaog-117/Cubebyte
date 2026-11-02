//! The command line: typed words that become the game's own actions.
//!
//! A line is never a new kind of history. [`parse::parse`] turns it into a [`model::Request`],
//! [`plan::plan`] turns that into an action the game already records (or a script command
//! it already checks and prices), and the table in [`table`] is the one place a command is
//! defined, so the parser, the dictionary and the error messages cannot drift apart.

pub mod line;
pub mod model;
pub mod parse;
pub mod plan;
pub mod table;
