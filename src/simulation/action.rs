//! What the player chose to do besides walking: the actions a replay must perform again.
//!
//! An action names its target, not the click that aimed at it. The camera's look direction is
//! not recorded, so "break the block at this spot" is what can be played back. Replaying one
//! works the result out from the world it finds, so it is refused, like any click at empty
//! air, if the world no longer fits.

use crate::shared::voxel::{BlockId, WorldPos};

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
}
