//! What the player chose to do besides walking: the actions a replay must perform again.
//!
//! An action names its target, not the click that aimed at it. The camera's look direction is
//! not recorded, so "break the block at this spot" is what can be played back, and a hit on a mob names the mob, not the aim. Replaying one
//! works the result out from the world it finds, so it is refused, like any click at empty
//! air, if the world no longer fits.

use crate::scripting::model::Command;
use crate::shared::voxel::{BlockId, WorldPos};
use crate::simulation::intent_log::ActorId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Break the block at `pos` and keep what it yields.
    Break(WorldPos),
    /// Place `block` in the empty cell at `pos`, spending one from the pack.
    Place { pos: WorldPos, block: BlockId },
    /// Craft the recipe at this position of the recipe table once.
    Craft(u8),
    /// Eat one food item.
    Eat,
    /// Hit the mob with this id with the best weapon in the pack. Refused if that mob is gone.
    Strike(ActorId),
    /// Step back the latest edit still in effect (key `Z`).
    Rewind,
    /// Bring back the edit the last rewind undid (key `Y`).
    Redo,
    /// One command a script ran, saved as it ran: what a replay performs again. The script that
    /// produced it is not saved, so a recording does not depend on the script library.
    Script(Command),
}
