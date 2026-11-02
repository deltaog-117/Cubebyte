//! What a script may ask for, what it is allowed to do, and how much of it per tick.

use glam::Vec3;
use thiserror::Error;

use crate::events::apply::StageError;
use crate::items::registry::ItemId;
use crate::shared::clock::Tick;
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

/// Ticks per second the simulation runs at, which the cooldowns below are written against.
const TICKS_PER_SECOND: u64 = 20;

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

/// Which of the six block directions the player is looking along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    /// Toward -z.
    North,
    /// Toward +x.
    East,
    /// Toward +z.
    South,
    /// Toward -x.
    West,
    Up,
    Down,
}

impl Facing {
    /// The direction `view` points most along, or `None` for a vector that is zero or not
    /// finite, which points nowhere. On a tie the x axis wins over y, and y over z, so the
    /// same view always gives the same answer.
    pub fn from_view(view: Vec3) -> Option<Self> {
        if !view.is_finite() || view == Vec3::ZERO {
            return None;
        }
        let (x, y, z) = (view.x.abs(), view.y.abs(), view.z.abs());
        Some(if x >= y && x >= z {
            if view.x > 0.0 { Self::East } else { Self::West }
        } else if y >= z {
            if view.y > 0.0 { Self::Up } else { Self::Down }
        } else if view.z > 0.0 {
            Self::South
        } else {
            Self::North
        })
    }

    /// The step of one block along this direction.
    pub const fn offset(self) -> [i32; 3] {
        match self {
            Self::North => [0, 0, -1],
            Self::East => [1, 0, 0],
            Self::South => [0, 0, 1],
            Self::West => [-1, 0, 0],
            Self::Up => [0, 1, 0],
            Self::Down => [0, -1, 0],
        }
    }

    /// The whole number a script reads: 0 north, 1 east, 2 south, 3 west, 4 up, 5 down.
    pub const fn code(self) -> u8 {
        match self {
            Self::North => 0,
            Self::East => 1,
            Self::South => 2,
            Self::West => 3,
            Self::Up => 4,
            Self::Down => 5,
        }
    }
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
    /// The direction the player is looking along.
    Facing,
    /// The block cell the nearest mob within range stands in.
    NearestMob,
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
    /// The direction a [`Query::Facing`] saw.
    Direction(Facing),
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
    /// Where this power's cooldown is kept in [`Cooldowns`].
    const fn index(self) -> usize {
        match self {
            Self::Move => 0,
            Self::Give => 1,
            Self::Build => 2,
            Self::Spawn => 3,
            Self::Look => 4,
        }
    }

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
    /// Fuel items one accepted command burns. A refused command burns none.
    pub const fn fuel_cost(self) -> u8 {
        match self {
            Self::Teleport { .. } => 2,
            Self::GiveItem { .. } => 4,
            Self::SetBlock { .. } => 1,
            Self::SpawnMob { .. } => 5,
        }
    }

    /// Ticks the same kind of command must wait after one was accepted.
    pub const fn cooldown(self) -> u64 {
        match self {
            Self::Teleport { .. } => 2 * TICKS_PER_SECOND,
            Self::GiveItem { .. } => 5 * TICKS_PER_SECOND,
            Self::SetBlock { .. } => TICKS_PER_SECOND / 5,
            Self::SpawnMob { .. } => 10 * TICKS_PER_SECOND,
        }
    }

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

    /// Every power a script can be granted.
    pub const ALL: Self = Self::NONE
        .with(Permission::Move)
        .with(Permission::Give)
        .with(Permission::Build)
        .with(Permission::Spawn)
        .with(Permission::Look);

    pub const fn with(self, permission: Permission) -> Self {
        Self(self.0 | permission.bit())
    }

    pub const fn allows(self, permission: Permission) -> bool {
        self.0 & permission.bit() != 0
    }
}

/// When each kind of command may next run. Commands of one kind share a wait, so a script
/// cannot dodge it by varying the target. It is kept in memory only: a restart clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cooldowns {
    ready_at: [Tick; 5],
}

impl Cooldowns {
    /// Ticks left before `command`'s kind may run on `now`; zero when it may run.
    pub fn wait_left(&self, command: Command, now: Tick) -> u64 {
        let ready = self.ready_at[command.permission().index()];
        ready.0.saturating_sub(now.0)
    }

    /// Starts the wait for `command`'s kind, from the tick it was accepted on.
    pub fn start(&mut self, command: Command, now: Tick) {
        self.ready_at[command.permission().index()] =
            Tick(now.0.saturating_add(command.cooldown()));
    }
}

/// The moment a batch of commands runs in, and the waits it is held to.
#[derive(Debug)]
pub struct Pace<'a> {
    pub tick: Tick,
    pub cooldowns: &'a mut Cooldowns,
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
    #[error("the command needs {needed} fuel and the pack holds {held}")]
    NoFuel { needed: u8, held: u32 },
    #[error("the command is cooling down for {ticks_left} more ticks")]
    CoolingDown { ticks_left: u64 },
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
    /// A program visited more blocks than one run may.
    #[error("the program ran for too long and was stopped")]
    TooLong,
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
    fn a_view_along_an_axis_faces_that_way_and_nowhere_faces_nothing() {
        let faced = |x, y, z| Facing::from_view(Vec3::new(x, y, z));
        assert_eq!(faced(0.0, 0.0, -1.0), Some(Facing::North));
        assert_eq!(faced(1.0, 0.0, 0.0), Some(Facing::East));
        assert_eq!(faced(0.0, 0.0, 1.0), Some(Facing::South));
        assert_eq!(faced(-1.0, 0.0, 0.0), Some(Facing::West));
        assert_eq!(faced(0.0, 1.0, 0.0), Some(Facing::Up));
        assert_eq!(faced(0.0, -1.0, 0.0), Some(Facing::Down));
        assert_eq!(faced(0.0, 0.0, 0.0), None);
        assert_eq!(faced(f32::NAN, 0.0, 1.0), None);
        assert_eq!(faced(f32::INFINITY, 0.0, 1.0), None);
        // Ties go to x, then y, then z.
        assert_eq!(faced(-1.0, 1.0, 1.0), Some(Facing::West));
        assert_eq!(faced(0.0, 1.0, 1.0), Some(Facing::Up));
    }

    proptest! {
        // The facing is the axis direction that lines up best with the view, and its code is
        // one of six distinct whole numbers.
        #[test]
        fn a_facing_is_the_axis_the_view_points_most_along(
            x in -100.0f32..100.0, y in -100.0f32..100.0, z in -100.0f32..100.0,
        ) {
            let view = Vec3::new(x, y, z);
            prop_assume!(view != Vec3::ZERO);
            let facing = Facing::from_view(view).expect("a finite non-zero view faces somewhere");
            let [dx, dy, dz] = facing.offset();
            let along = dx as f32 * x + dy as f32 * y + dz as f32 * z;
            prop_assert_eq!(along, x.abs().max(y.abs()).max(z.abs()));
            prop_assert!(facing.code() < 6);
        }
    }

    #[test]
    fn facing_codes_are_distinct() {
        let all = [
            Facing::North,
            Facing::East,
            Facing::South,
            Facing::West,
            Facing::Up,
            Facing::Down,
        ];
        let mut codes: Vec<_> = all.iter().map(|f| f.code()).collect();
        codes.sort_unstable();
        assert_eq!(codes, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn every_command_costs_fuel_and_waits() {
        let pos = WorldPos { x: 0, y: 0, z: 0 };
        let commands = [
            Command::Teleport { to: pos },
            Command::GiveItem {
                item: ItemId::STONE,
                count: 1,
            },
            Command::SetBlock {
                pos,
                block: BlockId::STONE,
            },
            Command::SpawnMob { at: pos },
        ];
        for command in commands {
            assert!(command.fuel_cost() > 0, "{command:?} is free");
            assert!(command.cooldown() > 0, "{command:?} never waits");
        }
    }

    proptest! {
        #[test]
        fn a_started_wait_counts_down_to_zero_and_stays_per_kind(
            start in 0u64..1_000_000,
            later in 0u64..1_000,
        ) {
            let pos = WorldPos { x: 0, y: 0, z: 0 };
            let teleport = Command::Teleport { to: pos };
            let build = Command::SetBlock { pos, block: BlockId::STONE };
            let mut cooldowns = Cooldowns::default();
            cooldowns.start(teleport, Tick(start));

            let left = cooldowns.wait_left(teleport, Tick(start + later));
            prop_assert_eq!(left, teleport.cooldown().saturating_sub(later));
            prop_assert_eq!(cooldowns.wait_left(build, Tick(start)), 0);
        }

        #[test]
        fn a_wait_started_at_the_end_of_time_does_not_overflow(now in u64::MAX - 5..=u64::MAX) {
            let pos = WorldPos { x: 0, y: 0, z: 0 };
            let mut cooldowns = Cooldowns::default();
            let command = Command::SpawnMob { at: pos };
            cooldowns.start(command, Tick(now));
            prop_assert!(cooldowns.wait_left(command, Tick(now)) <= command.cooldown());
        }
    }

    #[test]
    fn all_permissions_allow_every_power() {
        for permission in ALL {
            assert!(Permissions::ALL.allows(permission));
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
