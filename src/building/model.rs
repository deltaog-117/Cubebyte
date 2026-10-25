//! What can be placed, and why an edit can fail.

use thiserror::Error;

use crate::blocks::registry;
use crate::shared::voxel::BlockId;
use crate::world::model::ApplyError;

/// A block that can be placed. Always a defined solid block, never air, so "placing air" cannot be written; clearing a
/// cell is breaking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolidBlock(BlockId);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("only defined solid blocks can be placed; break the block to clear it")]
pub struct NotSolid;

impl SolidBlock {
    pub const STONE: Self = Self(BlockId::STONE);
    pub const DIRT: Self = Self(BlockId::DIRT);
    pub const GRASS: Self = Self(BlockId::GRASS);

    /// # Errors
    ///
    /// Returns [`NotSolid`] for air and for IDs with no block definition.
    pub fn try_new(block: BlockId) -> Result<Self, NotSolid> {
        if registry::is_solid(block) {
            Ok(Self(block))
        } else {
            Err(NotSolid)
        }
    }

    pub const fn id(self) -> BlockId {
        self.0
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EditError {
    #[error("no block within reach")]
    NothingInReach,
    #[error("the viewer is inside the block, so there is no face to build against")]
    NoFace,
    #[error("the block would fill the cell the viewer occupies")]
    Occupied,
    #[error("the world changed under the edit")]
    Stale(#[from] ApplyError),
}
