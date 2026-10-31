//! The fixed-step simulation: tick counting, per-system random streams and the intent log (walking steps and the actions taken between them), with its file format, its session files and a replay that plays one back.

pub mod action;
pub mod intent_log;
pub mod model;
pub mod persist;
pub mod replay;
pub mod stream;
