//! The types a typed command is made of.

use crate::items::registry::ItemId;
use crate::scripting::library::ScriptId;
use crate::shared::voxel::BlockId;

/// Position of a command in the table. Like block and recipe IDs, a new command is only ever
/// appended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandId(pub u8);

/// What a word of a command means, and so which words it accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// The name of an item.
    Item,
    /// The name of a block.
    Block,
    /// The name of a kind of mob.
    Mob,
    /// A whole number from 1 to `max`.
    Count { max: u16 },
    /// A whole number of blocks from the player's feet, either way.
    Offset,
    /// The name of a ready-made script or quick-hack.
    Hack,
}

/// One word a command takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgSpec {
    pub name: &'static str,
    pub kind: ArgKind,
}

/// A word after it was checked against its [`ArgKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Item(ItemId),
    Block(BlockId),
    Mob,
    Count(u16),
    Offset(i32),
    Hack(ScriptId),
}

/// What a typed line asks for, with every word already checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open the table of commands.
    Help,
    Give {
        item: ItemId,
        count: u16,
    },
    /// Move by this many blocks from the feet.
    Go {
        by: [i32; 3],
    },
    Set {
        block: BlockId,
        by: [i32; 3],
    },
    Spawn {
        by: [i32; 3],
    },
    Rewind,
    Redo,
    /// Zone rewind around the feet, this many block-changing actions back.
    Zone {
        back: u16,
    },
    /// Run this script or quick-hack, anchored on what it needs.
    Hack {
        script: ScriptId,
    },
}

/// One command: how it is written, what it does, and how its checked words become a request.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub name: &'static str,
    /// One short line for the table. Only characters the interface font has.
    pub summary: &'static str,
    pub args: &'static [ArgSpec],
    /// Builds the request from one value per argument, or `None` if the values do not fit the
    /// arguments, which the parser cannot produce.
    pub build: fn(&[Value]) -> Option<Request>,
}
