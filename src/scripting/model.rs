//! What a script may ask for, what it is allowed to do, and how much of it per tick.

use thiserror::Error;

use crate::events::apply::StageError;
use crate::items::registry::ItemId;
use crate::shared::voxel::{BlockId, WorldPos};

/// How far from the player a script may act, in blocks. It stays inside the streamed area, so
/// a script never edits or lands in a chunk that is not loaded.
pub const RANGE: f32 = 32.0;

/// Commands a script may run in one tick, whether or not they succeed, so refused commands
/// cost as much as accepted ones and cannot be spammed for free.
pub const MAX_EFFECTS: u8 = 8;

/// Most mobs that may be alive when a script asks for one more, so a script cannot flood the
/// world.
pub const MAX_MOBS: usize = 8;

/// How far from the player, in blocks along each axis, a [`Query::Find`] searches. It stays
/// well under [`RANGE`] so a search costs a few tens of thousands of lookups, not hundreds of
/// thousands.
pub const FIND_RADIUS: i32 = 16;

/// One thing a script asks the world to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Stand at the bottom middle of the block cell `to`.
    Teleport { to: WorldPos },
    /// Put `count` of `item` in the inventory, all of them or none.
    GiveItem { item: ItemId, count: u16 },
    /// Make the block at `pos` a `block`.
    SetBlock { pos: WorldPos, block: BlockId },
    /// Put a new zombie at the bottom middle of the block cell `at`.
    SpawnMob { at: WorldPos },
}

/// One thing a script asks to know. A query changes nothing and logs nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    /// The block at `pos`, which must be within range.
    Look { pos: WorldPos },
    /// The nearest block of kind `block` within [`FIND_RADIUS`] of the player.
    Find { block: BlockId },
    /// How many of `item` the player carries.
    Count { item: ItemId },
    /// The player's health points.
    Health,
    /// The player's hunger points.
    Hunger,
    /// How many mobs are within range of the player.
    MobsNear,
}

/// What a [`Query`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The block a [`Query::Look`] saw.
    Block(BlockId),
    /// Where a [`Query::Find`] found its block, or `None` when there is none in reach.
    Found(Option<WorldPos>),
    /// A count or meter value.
    Number(u32),
}

/// A kind of power a script can be granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Move,
    Give,
    Build,
    Spawn,
    /// Reading the world with a [`Query`].
    Look,
}

impl Permission {
    const fn bit(self) -> u8 {
        match self {
            Self::Move => 1,
            Self::Give => 2,
            Self::Build => 4,
            Self::Spawn => 8,
            Self::Look => 16,
        }
    }
}

impl Command {
    /// The permission this command needs.
    pub const fn permission(self) -> Permission {
        match self {
            Self::Teleport { .. } => Permission::Move,
            Self::GiveItem { .. } => Permission::Give,
            Self::SetBlock { .. } => Permission::Build,
            Self::SpawnMob { .. } => Permission::Spawn,
        }
    }
}

/// The set of [`Permission`]s a script was granted. A script starts with none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Permissions(u8);

impl Permissions {
    pub const NONE: Self = Self(0);

    pub const fn with(self, permission: Permission) -> Self {
        Self(self.0 | permission.bit())
    }

    pub const fn allows(self, permission: Permission) -> bool {
        self.0 & permission.bit() != 0
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LimitsError {
    #[error("the range must be finite and above zero")]
    BadRange,
}

/// How far and how often a script may act.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    range: f32,
    max_effects: u8,
}

impl Limits {
    /// # Errors
    ///
    /// Returns [`LimitsError::BadRange`] if `range` is NaN, infinite, zero or negative.
    pub fn try_new(range: f32, max_effects: u8) -> Result<Self, LimitsError> {
        if !(range.is_finite() && range > 0.0) {
            return Err(LimitsError::BadRange);
        }
        Ok(Self { range, max_effects })
    }

    /// [`RANGE`] and [`MAX_EFFECTS`].
    pub const fn standard() -> Self {
        Self {
            range: RANGE,
            max_effects: MAX_EFFECTS,
        }
    }

    pub const fn range(self) -> f32 {
        self.range
    }

    pub const fn max_effects(self) -> u8 {
        self.max_effects
    }
}

/// Why a command did nothing. The state and the log are left as they were.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Denied {
    #[error("the script was not granted the {0:?} permission")]
    NoPermission(Permission),
    #[error("the target is beyond the script's range")]
    OutOfRange,
    #[error("the script already used its commands for this tick")]
    OverLimit,
    #[error("the destination overlaps solid blocks")]
    Blocked,
    #[error("item {0} is not defined")]
    UnknownItem(u16),
    #[error("block {0} is not defined")]
    UnknownBlock(u16),
    #[error("the world already holds the most mobs a script may add ({MAX_MOBS})")]
    TooManyMobs,
    /// A grant is all or nothing, so one that does not fit whole gives nothing.
    #[error("the inventory has room for only {room} of the item")]
    NoRoom { room: u32 },
    /// The command would change nothing, for example a zero count or a block set to itself.
    #[error("the command would change nothing")]
    NothingToDo,
    /// The command was fine but the game state did not accept its event.
    #[error(transparent)]
    Refused(#[from] StageError),
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const ALL: [Permission; 5] = [
        Permission::Move,
        Permission::Give,
        Permission::Build,
        Permission::Spawn,
        Permission::Look,
    ];

    proptest! {
        #[test]
        fn permissions_allow_exactly_what_was_granted(mask in 0u8..32) {
            let granted: Vec<_> = ALL
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, &p)| p)
                .collect();
            let permissions = granted
                .iter()
                .fold(Permissions::NONE, |set, &p| set.with(p));
            for permission in ALL {
                prop_assert_eq!(permissions.allows(permission), granted.contains(&permission));
            }
        }

        #[test]
        fn only_a_finite_positive_range_makes_limits(range in any::<f32>(), effects in any::<u8>()) {
            let made = Limits::try_new(range, effects);
            if range.is_finite() && range > 0.0 {
                prop_assert_eq!(made.map(|l| (l.range(), l.max_effects())), Ok((range, effects)));
            } else {
                prop_assert_eq!(made, Err(LimitsError::BadRange));
            }
        }
    }

    #[test]
    fn a_new_script_has_no_permissions() {
        for permission in ALL {
            assert!(!Permissions::NONE.allows(permission));
        }
    }

    #[test]
    fn each_command_needs_its_own_permission() {
        let pos = WorldPos { x: 0, y: 0, z: 0 };
        assert_eq!(Command::Teleport { to: pos }.permission(), Permission::Move);
        assert_eq!(
            Command::GiveItem {
                item: ItemId::STONE,
                count: 1
            }
            .permission(),
            Permission::Give
        );
        assert_eq!(
            Command::SetBlock {
                pos,
                block: BlockId::STONE
            }
            .permission(),
            Permission::Build
        );
        assert_eq!(
            Command::SpawnMob { at: pos }.permission(),
            Permission::Spawn
        );
    }
}
