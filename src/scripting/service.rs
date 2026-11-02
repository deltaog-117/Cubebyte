//! Running a script's commands against the game.
//!
//! Each command is checked, turned into an [`Event`], applied to the game state and then
//! logged, in that order, so the log never holds an event the game refused. A denied command
//! changes nothing and logs nothing.
//!
//! An accepted command also burns fuel from the pack, as slot events in the same all-or-nothing
//! batch as the command's own events, and starts a wait before its kind may run again. A denied
//! command costs no fuel and starts no wait.
//!
//! A [`Query`] only reads. [`ask`] takes the stage by shared reference, so it cannot change the
//! game, and it logs nothing.

use glam::Vec3;

use crate::blocks::registry as blocks;
use crate::events::apply::{Stage, StageError, apply_all, slot_events};
use crate::events::log::{ActionKind, EventLog};
use crate::events::rewind::log_action;
use crate::inventory::model::{Inventory, NotEnough, UnknownItem};
use crate::items::registry::{self as items, ItemId};
use crate::mobs::model::{MobError, Zombie};
use crate::mobs::service as mobs;
use crate::player::model::Player;
use crate::player::service::collides;
use crate::scripting::model::{
    Answer, Command, Denied, FIND_RADIUS, Facing, Limits, MAX_MOBS, Pace, Permission, Permissions,
    Query,
};
use crate::shared::event::{Event, MobId, Spot};
use crate::shared::voxel::{BlockId, WorldPos};

/// Runs one tick's `commands` in order and reports what became of each.
///
/// Only the first [`Limits::max_effects`] commands are tried; the rest are
/// [`Denied::OverLimit`]. Denied commands count toward that limit.
///
/// `pace` says which tick this is and holds the waits; a command whose kind is still cooling
/// down is [`Denied::CoolingDown`], and one the pack cannot pay for is [`Denied::NoFuel`].
///
/// `next_mob` is the id the next spawned mob gets. It moves on only when a spawn is accepted,
/// so ids are never reused.
pub fn run(
    commands: &[Command],
    granted: Permissions,
    limits: Limits,
    pace: &mut Pace<'_>,
    stage: &mut Stage<'_>,
    log: &mut EventLog,
    next_mob: &mut MobId,
) -> Vec<Result<(), Denied>> {
    let allowed = usize::from(limits.max_effects());
    commands
        .iter()
        .enumerate()
        .map(|(index, &command)| {
            let result = if index >= allowed {
                Err(Denied::OverLimit)
            } else {
                run_one(command, granted, limits, pace, stage, log, next_mob)
            };
            match &result {
                Ok(()) => tracing::debug!(?command, logged = log.len(), "script command applied"),
                Err(denied) => tracing::debug!(?command, %denied, "script command denied"),
            }
            result
        })
        .collect()
}

fn run_one(
    command: Command,
    granted: Permissions,
    limits: Limits,
    pace: &mut Pace<'_>,
    stage: &mut Stage<'_>,
    log: &mut EventLog,
    next_mob: &mut MobId,
) -> Result<(), Denied> {
    let needed = command.permission();
    if !granted.allows(needed) {
        return Err(Denied::NoPermission(needed));
    }
    let ticks_left = pace.cooldowns.wait_left(command, pace.tick);
    if ticks_left > 0 {
        return Err(Denied::CoolingDown { ticks_left });
    }
    // The fuel goes first and the command is worked out on what is left, so a grant that
    // lands in the slot the fuel came out of still gets slot events that match the state.
    let cost = command.fuel_cost();
    let mut paid = stage.inventory.clone();
    paid.take(ItemId::FUEL, u32::from(cost))
        .map_err(|NotEnough { held, .. }| Denied::NoFuel { needed: cost, held })?;
    let mut events = slot_events(stage.inventory, &paid);
    match command {
        Command::Teleport { to } => events.push(move_event(to, limits, stage)?),
        Command::GiveItem { item, count } => events.extend(give_events(item, count, &paid)?),
        Command::SetBlock { pos, block } => events.push(set_event(pos, block, limits, stage)?),
        Command::SpawnMob { at } => events.push(spawn_event(at, *next_mob, limits, stage)?),
    }
    apply_all(stage, &events)?;
    // One accepted command is one edit, with the fuel it burned, so `Z` takes it back whole.
    log_action(log, &events, ActionKind::Edit);
    pace.cooldowns.start(command, pace.tick);
    if matches!(command, Command::SpawnMob { .. }) {
        *next_mob = next_mob.next();
    }
    Ok(())
}

/// Answers each of `queries` in order, or says why it could not. `facing` is the direction the
/// player looks along, which the game works out from the camera.
///
/// Reading needs [`Permission::Look`]. Only the first [`Limits::max_effects`] queries are
/// tried; the rest are [`Denied::OverLimit`], and denied queries count toward that limit, so
/// searches cannot be spammed. Blocks in chunks that are not loaded read as air, which is why
/// [`Query::Look`] stays within [`Limits::range`].
pub fn ask(
    queries: &[Query],
    granted: Permissions,
    limits: Limits,
    facing: Facing,
    stage: &Stage<'_>,
) -> Vec<Result<Answer, Denied>> {
    let allowed = usize::from(limits.max_effects());
    queries
        .iter()
        .enumerate()
        .map(|(index, &query)| {
            let result = if index >= allowed {
                Err(Denied::OverLimit)
            } else {
                ask_one(query, granted, limits, facing, stage)
            };
            match &result {
                Ok(answer) => tracing::debug!(?query, ?answer, "script query answered"),
                Err(denied) => tracing::debug!(?query, %denied, "script query denied"),
            }
            result
        })
        .collect()
}

fn ask_one(
    query: Query,
    granted: Permissions,
    limits: Limits,
    facing: Facing,
    stage: &Stage<'_>,
) -> Result<Answer, Denied> {
    if !granted.allows(Permission::Look) {
        return Err(Denied::NoPermission(Permission::Look));
    }
    let here = stage.player.position();
    match query {
        Query::Look { pos } => {
            if !within_range(here, middle_of(pos), limits) {
                return Err(Denied::OutOfRange);
            }
            Ok(Answer::Block(stage.world.block_at(pos)))
        }
        Query::Find { block } => {
            if blocks::get(block).is_none() {
                return Err(Denied::UnknownBlock(block.0));
            }
            Ok(Answer::Found(nearest(block, limits, stage)))
        }
        Query::Count { item } => {
            if items::get(item).is_none() {
                return Err(Denied::UnknownItem(item.0));
            }
            Ok(Answer::Number(stage.inventory.count_of(item)))
        }
        Query::Health => Ok(Answer::Number(u32::from(stage.stats.health().value()))),
        Query::Hunger => Ok(Answer::Number(u32::from(stage.stats.hunger().value()))),
        Query::MobsNear => {
            let near = stage
                .mobs
                .iter()
                .filter(|(_, mob)| within_range(here, mob.position(), limits))
                .count();
            Ok(Answer::Number(u32::try_from(near).unwrap_or(u32::MAX)))
        }
        Query::Facing => Ok(Answer::Direction(facing)),
        Query::NearestMob => Ok(Answer::Found(nearest_mob(limits, stage))),
    }
}

/// The block cell of the mob closest to the player among those within range. Ties go to the
/// lowest mob id, so the same world always gives the same answer.
fn nearest_mob(limits: Limits, stage: &Stage<'_>) -> Option<WorldPos> {
    let here = stage.player.position();
    stage
        .mobs
        .iter()
        .filter(|(_, mob)| within_range(here, mob.position(), limits))
        .min_by(|(a_id, a), (b_id, b)| {
            let (a_dist, b_dist) = (
                here.distance_squared(a.position()),
                here.distance_squared(b.position()),
            );
            a_dist.total_cmp(&b_dist).then(a_id.cmp(b_id))
        })
        .map(|(_, mob)| {
            let cell = mob.position().floor().as_ivec3();
            WorldPos {
                x: cell.x,
                y: cell.y,
                z: cell.z,
            }
        })
}

/// The block of kind `block` closest to the player, searching a cube of [`FIND_RADIUS`] (or
/// the range, if that is smaller) around the block cell they stand in. Ties go to the lowest
/// x, then y, then z, so the same world always gives the same answer.
fn nearest(block: BlockId, limits: Limits, stage: &Stage<'_>) -> Option<WorldPos> {
    let here = stage.player.position();
    let centre = here.floor().as_ivec3();
    // Truncation is the intent: only whole blocks of reach count.
    let radius = FIND_RADIUS.min(limits.range() as i32);
    let offsets = -radius..=radius;
    offsets
        .clone()
        .flat_map(|dx| {
            let offsets = offsets.clone();
            offsets.flat_map(move |dy| (-radius..=radius).map(move |dz| (dx, dy, dz)))
        })
        .map(|(dx, dy, dz)| {
            let distance = i64::from(dx).pow(2) + i64::from(dy).pow(2) + i64::from(dz).pow(2);
            let pos = WorldPos {
                x: centre.x.saturating_add(dx),
                y: centre.y.saturating_add(dy),
                z: centre.z.saturating_add(dz),
            };
            (distance, pos)
        })
        .filter(|&(_, pos)| stage.world.block_at(pos) == block)
        .filter(|&(_, pos)| within_range(here, middle_of(pos), limits))
        .min_by_key(|&(distance, pos)| (distance, pos.x, pos.y, pos.z))
        .map(|(_, pos)| pos)
}

/// The bottom middle of block cell `cell`, where a player's feet go.
fn feet_of(cell: WorldPos) -> Vec3 {
    Vec3::new(cell.x as f32 + 0.5, cell.y as f32, cell.z as f32 + 0.5)
}

/// The middle of block cell `cell`.
fn middle_of(cell: WorldPos) -> Vec3 {
    Vec3::new(
        cell.x as f32 + 0.5,
        cell.y as f32 + 0.5,
        cell.z as f32 + 0.5,
    )
}

fn within_range(from: Vec3, to: Vec3, limits: Limits) -> bool {
    from.distance_squared(to) <= limits.range() * limits.range()
}

fn move_event(to: WorldPos, limits: Limits, stage: &Stage<'_>) -> Result<Event, Denied> {
    let feet = feet_of(to);
    let here = stage.player.position();
    if !within_range(here, feet, limits) {
        return Err(Denied::OutOfRange);
    }
    // The destination is checked with a body of the player's size, as it would stand there.
    let arrived = Player::try_new(feet).map_err(StageError::Player)?;
    if collides(&arrived, stage.world) {
        return Err(Denied::Blocked);
    }
    let (old, new) = (Spot::from_vec3(here), Spot::from_vec3(feet));
    if old == new {
        return Err(Denied::NothingToDo);
    }
    Ok(Event::PlayerMoved { old, new })
}

/// A new zombie `id` standing in cell `at`, if that is in range, in the open, and under the
/// cap on mobs.
fn spawn_event(
    at: WorldPos,
    id: MobId,
    limits: Limits,
    stage: &Stage<'_>,
) -> Result<Event, Denied> {
    let feet = feet_of(at);
    if !within_range(stage.player.position(), feet, limits) {
        return Err(Denied::OutOfRange);
    }
    if stage.mobs.len() >= MAX_MOBS {
        return Err(Denied::TooManyMobs);
    }
    // The mob is checked as it would stand there, like a teleporting player.
    let arrived = Zombie::try_new(feet).map_err(|error| StageError::Mob(MobError::Body(error)))?;
    if mobs::blocked(&arrived, stage.world) {
        return Err(Denied::Blocked);
    }
    Ok(mobs::spawn_event(id, feet))
}

/// One slot event for each slot the grant changes, so undo puts every stack back where it was.
fn give_events(item: ItemId, count: u16, inventory: &Inventory) -> Result<Vec<Event>, Denied> {
    let room = inventory
        .room_for(item)
        .ok_or(Denied::UnknownItem(item.0))?;
    if count == 0 {
        return Err(Denied::NothingToDo);
    }
    if room < u32::from(count) {
        return Err(Denied::NoRoom { room });
    }
    let mut after = inventory.clone();
    after
        .add(item, u32::from(count))
        .map_err(|UnknownItem(raw)| Denied::UnknownItem(raw))?;
    Ok(slot_events(inventory, &after))
}

fn set_event(
    pos: WorldPos,
    block: BlockId,
    limits: Limits,
    stage: &Stage<'_>,
) -> Result<Event, Denied> {
    if blocks::get(block).is_none() {
        return Err(Denied::UnknownBlock(block.0));
    }
    if !within_range(stage.player.position(), middle_of(pos), limits) {
        return Err(Denied::OutOfRange);
    }
    let old = stage.world.block_at(pos);
    if old == block {
        return Err(Denied::NothingToDo);
    }
    Ok(Event::BlockSet {
        pos,
        old,
        new: block,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::events::apply::apply;
    use crate::inventory::model::Inventory;
    use crate::mobs::model::Mobs;
    use crate::scripting::model::{Cooldowns, MAX_EFFECTS};
    use crate::shared::clock::Tick;
    use crate::survival::model::Stats;
    use crate::world::model::World;

    const EVERYTHING: Permissions = Permissions::NONE
        .with(Permission::Move)
        .with(Permission::Give)
        .with(Permission::Build)
        .with(Permission::Spawn)
        .with(Permission::Look);

    struct State {
        world: World,
        player: Player,
        inventory: Inventory,
        stats: Stats,
        mobs: Mobs,
        next_mob: MobId,
        log: EventLog,
        tick: Tick,
        cooldowns: Cooldowns,
    }

    impl State {
        /// A stone floor whose top face is at y = 1, with the player standing on it and enough
        /// fuel in the pack for any run these tests make.
        fn new() -> Self {
            let mut state = Self::broke();
            state.inventory.add(ItemId::FUEL, FUEL_HELD).unwrap();
            state
        }

        /// The same floor, with an empty pack.
        fn broke() -> Self {
            let mut world = World::default();
            for x in -40..40 {
                for z in -40..40 {
                    let pos = WorldPos { x, y: 0, z };
                    let event = Event::BlockSet {
                        pos,
                        old: BlockId::AIR,
                        new: BlockId::STONE,
                    };
                    world.apply(&event).unwrap();
                }
            }
            Self {
                world,
                player: Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap(),
                inventory: Inventory::default(),
                stats: Stats::default(),
                mobs: Mobs::default(),
                next_mob: MobId::FIRST,
                log: EventLog::new(),
                tick: Tick(0),
                cooldowns: Cooldowns::default(),
            }
        }

        fn ask(&self, queries: &[Query], granted: Permissions) -> Vec<Result<Answer, Denied>> {
            // `ask` only reads, so the borrow it needs is made from clones.
            let (mut world, mut player) = (self.world.clone(), self.player);
            let (mut inventory, mut stats) = (self.inventory.clone(), self.stats);
            let mut mobs = self.mobs.clone();
            let stage = Stage {
                world: &mut world,
                player: &mut player,
                inventory: &mut inventory,
                stats: &mut stats,
                mobs: &mut mobs,
            };
            ask(queries, granted, Limits::standard(), Facing::East, &stage)
        }

        /// Runs each command alone, long after the one before, so no wait is ever in the way.
        fn run(&mut self, commands: &[Command], granted: Permissions) -> Vec<Result<(), Denied>> {
            self.run_spaced_with(commands, granted, Limits::standard())
        }

        fn run_spaced_with(
            &mut self,
            commands: &[Command],
            granted: Permissions,
            limits: Limits,
        ) -> Vec<Result<(), Denied>> {
            commands
                .iter()
                .flat_map(|command| {
                    self.tick = Tick(self.tick.0 + QUIET);
                    self.run_with(std::slice::from_ref(command), granted, limits)
                })
                .collect()
        }

        /// Runs all of `commands` on the current tick.
        fn run_with(
            &mut self,
            commands: &[Command],
            granted: Permissions,
            limits: Limits,
        ) -> Vec<Result<(), Denied>> {
            let mut stage = Stage {
                world: &mut self.world,
                player: &mut self.player,
                inventory: &mut self.inventory,
                stats: &mut self.stats,
                mobs: &mut self.mobs,
            };
            let mut pace = Pace {
                tick: self.tick,
                cooldowns: &mut self.cooldowns,
            };
            run(
                commands,
                granted,
                limits,
                &mut pace,
                &mut stage,
                &mut self.log,
                &mut self.next_mob,
            )
        }
    }

    /// Fuel the test pack starts with: more than any run here burns.
    const FUEL_HELD: u32 = 640;

    /// Ticks between spaced runs, longer than any cooldown.
    const QUIET: u64 = 1_000;

    fn fuel_of(state: &State) -> u32 {
        state.inventory.count_of(ItemId::FUEL)
    }

    fn cell(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    fn give(item: ItemId, count: u16) -> Command {
        Command::GiveItem { item, count }
    }

    #[test]
    fn a_teleport_moves_the_player_and_logs_the_jump() {
        let mut state = State::new();
        let from = Spot::from_vec3(state.player.position());

        let results = state.run(&[Command::Teleport { to: cell(5, 1, -3) }], EVERYTHING);

        assert_eq!(results, [Ok(())]);
        assert_eq!(state.player.position(), Vec3::new(5.5, 1.0, -2.5));
        let to = Spot::from_vec3(state.player.position());
        assert_eq!(
            state.log.iter().last(),
            Some(&Event::PlayerMoved { old: from, new: to })
        );
    }

    #[test]
    fn a_teleport_into_solid_blocks_is_blocked() {
        let mut state = State::new();
        let before = state.player;

        let results = state.run(&[Command::Teleport { to: cell(5, 0, 5) }], EVERYTHING);

        assert_eq!(results, [Err(Denied::Blocked)]);
        assert_eq!(state.player, before);
        assert!(state.log.is_empty());
    }

    #[test]
    fn a_teleport_to_where_the_player_already_stands_does_nothing() {
        let mut state = State::new();

        let results = state.run(&[Command::Teleport { to: cell(0, 1, 0) }], EVERYTHING);

        assert_eq!(results, [Err(Denied::NothingToDo)]);
        assert!(state.log.is_empty());
    }

    #[test]
    fn a_target_beyond_range_is_out_of_range_for_moves_and_edits() {
        let mut state = State::new();
        let far = cell(33, 1, 0);

        let results = state.run(
            &[
                Command::Teleport { to: far },
                Command::SetBlock {
                    pos: far,
                    block: BlockId::DIRT,
                },
            ],
            EVERYTHING,
        );

        assert_eq!(results, [Err(Denied::OutOfRange), Err(Denied::OutOfRange)]);
        assert!(state.log.is_empty());
    }

    #[test]
    fn a_target_inside_range_is_reached() {
        let mut state = State::new();

        let results = state.run(
            &[Command::SetBlock {
                pos: cell(31, 1, 0),
                block: BlockId::DIRT,
            }],
            EVERYTHING,
        );

        assert_eq!(results, [Ok(())]);
        assert_eq!(state.world.block_at(cell(31, 1, 0)), BlockId::DIRT);
    }

    #[test]
    fn without_a_permission_a_command_is_refused_and_changes_nothing() {
        let mut state = State::new();
        let (world, player) = (state.world.clone(), state.player);
        let inventory = state.inventory.clone();

        let results = state.run(
            &[
                Command::Teleport { to: cell(5, 1, 5) },
                give(ItemId::STONE, 1),
                Command::SetBlock {
                    pos: cell(2, 1, 2),
                    block: BlockId::DIRT,
                },
            ],
            Permissions::NONE,
        );

        assert_eq!(
            results,
            [
                Err(Denied::NoPermission(Permission::Move)),
                Err(Denied::NoPermission(Permission::Give)),
                Err(Denied::NoPermission(Permission::Build)),
            ]
        );
        assert_eq!((state.world, state.player), (world, player));
        assert_eq!(state.inventory, inventory);
        assert!(state.log.is_empty());
    }

    #[test]
    fn one_permission_does_not_open_the_others() {
        let mut state = State::new();

        let results = state.run(
            &[
                give(ItemId::STONE, 1),
                Command::Teleport { to: cell(5, 1, 5) },
            ],
            Permissions::NONE.with(Permission::Give),
        );

        assert_eq!(
            results,
            [Ok(()), Err(Denied::NoPermission(Permission::Move))]
        );
    }

    #[test]
    fn commands_past_the_limit_are_not_tried_and_denied_ones_count() {
        let mut state = State::new();
        let limits = Limits::try_new(32.0, 2).unwrap();

        let results = state.run_with(
            &[
                give(ItemId(999), 1),
                give(ItemId::STONE, 1),
                give(ItemId::STONE, 1),
                give(ItemId::STONE, 1),
            ],
            EVERYTHING,
            limits,
        );

        assert_eq!(
            results,
            [
                Err(Denied::UnknownItem(999)),
                Ok(()),
                Err(Denied::OverLimit),
                Err(Denied::OverLimit),
            ]
        );
        assert_eq!(state.inventory.count_of(ItemId::STONE), 1);
        // The grant's own slot event, and one for the fuel it burned.
        assert_eq!(state.log.len(), 2);
    }

    #[test]
    fn a_grant_that_does_not_fit_whole_gives_nothing() {
        let mut state = State::new();
        let room = state.inventory.room_for(ItemId::DIRT).unwrap();
        let too_many = u16::try_from(room + 1).unwrap();
        let inventory = state.inventory.clone();

        let results = state.run(&[give(ItemId::DIRT, too_many)], EVERYTHING);

        assert!(matches!(results[..], [Err(Denied::NoRoom { .. })]));
        assert_eq!(state.inventory, inventory);
        assert!(state.log.is_empty());
    }

    #[test]
    fn undefined_items_and_blocks_and_empty_changes_are_refused() {
        let mut state = State::new();

        let results = state.run(
            &[
                give(ItemId(0), 1),
                give(ItemId::STONE, 0),
                Command::SetBlock {
                    pos: cell(1, 1, 1),
                    block: BlockId(999),
                },
                Command::SetBlock {
                    pos: cell(1, 0, 1),
                    block: BlockId::STONE,
                },
            ],
            EVERYTHING,
        );

        assert_eq!(
            results,
            [
                Err(Denied::UnknownItem(0)),
                Err(Denied::NothingToDo),
                Err(Denied::UnknownBlock(999)),
                Err(Denied::NothingToDo),
            ]
        );
        assert!(state.log.is_empty());
    }

    fn spawn(x: i32, y: i32, z: i32) -> Command {
        Command::SpawnMob { at: cell(x, y, z) }
    }

    #[test]
    fn a_spawn_puts_a_zombie_in_the_open_and_logs_it() {
        let mut state = State::new();

        let results = state.run(&[spawn(5, 1, 5), spawn(-4, 1, 3)], EVERYTHING);

        assert_eq!(results, [Ok(()), Ok(())]);
        assert_eq!(state.mobs.len(), 2);
        let first = state.mobs.get(MobId::FIRST).unwrap();
        assert_eq!(first.position(), Vec3::new(5.5, 1.0, 5.5));
        assert!(state.mobs.get(MobId::FIRST.next()).is_some());
        assert_eq!(state.next_mob, MobId::FIRST.next().next());
        let spawned = state
            .log
            .iter()
            .filter(|event| matches!(event, Event::MobSpawned { .. }))
            .count();
        assert_eq!(spawned, 2);
    }

    #[test]
    fn a_refused_spawn_changes_nothing_and_keeps_the_id() {
        let mut state = State::new();

        let results = state.run(&[spawn(5, 0, 5), spawn(33, 1, 0)], EVERYTHING);
        let denied = state.run(&[spawn(5, 1, 5)], Permissions::NONE);

        assert_eq!(results, [Err(Denied::Blocked), Err(Denied::OutOfRange)]);
        assert_eq!(denied, [Err(Denied::NoPermission(Permission::Spawn))]);
        assert!(state.mobs.is_empty());
        assert!(state.log.is_empty());
        assert_eq!(state.next_mob, MobId::FIRST);
    }

    #[test]
    fn spawns_stop_at_the_cap_on_mobs() {
        let mut state = State::new();
        let limits = Limits::try_new(32.0, u8::MAX).unwrap();
        let commands: Vec<_> = (0..=MAX_MOBS as i32).map(|x| spawn(x, 1, 5)).collect();

        let results = state.run_spaced_with(&commands, EVERYTHING, limits);

        assert!(results[..MAX_MOBS].iter().all(Result::is_ok));
        assert_eq!(results[MAX_MOBS], Err(Denied::TooManyMobs));
        assert_eq!(state.mobs.len(), MAX_MOBS);
    }

    #[test]
    fn a_look_reads_a_block_in_range_and_refuses_one_beyond_it() {
        let state = State::new();

        let answers = state.ask(
            &[
                Query::Look { pos: cell(3, 0, 3) },
                Query::Look { pos: cell(3, 1, 3) },
                Query::Look {
                    pos: cell(33, 0, 0),
                },
            ],
            EVERYTHING,
        );

        assert_eq!(
            answers,
            [
                Ok(Answer::Block(BlockId::STONE)),
                Ok(Answer::Block(BlockId::AIR)),
                Err(Denied::OutOfRange),
            ]
        );
    }

    #[test]
    fn a_find_gives_the_nearest_block_with_ties_to_the_lowest_position() {
        let mut state = State::new();
        // The player stands in cell (0, 1, 0). Two dirt blocks are equally far; (-2, ..) sorts
        // first. A third, farther one must lose.
        let events: Vec<_> = [cell(2, 1, 0), cell(-2, 1, 0), cell(6, 1, 6)]
            .into_iter()
            .map(|pos| Event::BlockSet {
                pos,
                old: BlockId::AIR,
                new: BlockId::DIRT,
            })
            .collect();
        events.iter().for_each(|e| state.world.apply(e).unwrap());

        let answers = state.ask(
            &[
                Query::Find {
                    block: BlockId::DIRT,
                },
                Query::Find {
                    block: BlockId::GRASS,
                },
                Query::Find {
                    block: BlockId(999),
                },
            ],
            EVERYTHING,
        );

        assert_eq!(
            answers,
            [
                Ok(Answer::Found(Some(cell(-2, 1, 0)))),
                Ok(Answer::Found(None)),
                Err(Denied::UnknownBlock(999)),
            ]
        );
    }

    #[test]
    fn a_find_does_not_see_past_its_radius() {
        let mut state = State::new();
        let far = Event::BlockSet {
            pos: cell(FIND_RADIUS + 1, 1, 0),
            old: BlockId::AIR,
            new: BlockId::DIRT,
        };
        state.world.apply(&far).unwrap();

        let answers = state.ask(
            &[Query::Find {
                block: BlockId::DIRT,
            }],
            EVERYTHING,
        );

        assert_eq!(answers, [Ok(Answer::Found(None))]);
    }

    #[test]
    fn queries_about_the_player_report_the_game_as_it_is() {
        let mut state = State::new();
        state.run(
            &[give(ItemId::STONE, 5), spawn(5, 1, 5), spawn(-3, 1, 0)],
            EVERYTHING,
        );

        let answers = state.ask(
            &[
                Query::Count {
                    item: ItemId::STONE,
                },
                Query::Count { item: ItemId(999) },
                Query::Health,
                Query::Hunger,
                Query::MobsNear,
            ],
            EVERYTHING,
        );

        assert_eq!(
            answers,
            [
                Ok(Answer::Number(5)),
                Err(Denied::UnknownItem(999)),
                Ok(Answer::Number(u32::from(state.stats.health().value()))),
                Ok(Answer::Number(u32::from(state.stats.hunger().value()))),
                Ok(Answer::Number(2)),
            ]
        );
    }

    #[test]
    fn a_facing_read_gives_the_direction_the_game_passed_in() {
        let state = State::new();

        let answers = state.ask(&[Query::Facing], EVERYTHING);

        assert_eq!(answers, [Ok(Answer::Direction(Facing::East))]);
    }

    #[test]
    fn the_nearest_mob_is_the_closest_one_in_range_with_ties_to_the_lowest_id() {
        let mut state = State::new();
        let put = |state: &mut State, id: u32, at: Vec3| {
            state.mobs.spawn(MobId(id), at, 10).unwrap();
        };
        // The player is at (0.5, 1, 0.5). Ids 1 and 2 are equally far, so 1 wins; 3 is
        // farther; 5 is beyond the range and must not count. Id 4, added later, is the closest.
        put(&mut state, 1, Vec3::new(6.5, 1.0, 0.5));
        put(&mut state, 2, Vec3::new(-5.5, 1.0, 0.5));
        put(&mut state, 3, Vec3::new(20.5, 1.0, 0.5));
        put(&mut state, 5, Vec3::new(60.5, 1.0, 0.5));

        let answers = state.ask(&[Query::NearestMob], EVERYTHING);
        assert_eq!(answers, [Ok(Answer::Found(Some(cell(6, 1, 0))))]);

        put(&mut state, 4, Vec3::new(2.5, 1.0, 3.5));
        let answers = state.ask(&[Query::NearestMob], EVERYTHING);
        assert_eq!(answers, [Ok(Answer::Found(Some(cell(2, 1, 3))))]);
    }

    #[test]
    fn with_no_mob_in_range_there_is_no_nearest_mob() {
        let mut state = State::new();
        assert_eq!(
            state.ask(&[Query::NearestMob], EVERYTHING),
            [Ok(Answer::Found(None))]
        );

        state
            .mobs
            .spawn(MobId(1), Vec3::new(60.5, 1.0, 0.5), 10)
            .unwrap();
        assert_eq!(
            state.ask(&[Query::NearestMob], EVERYTHING),
            [Ok(Answer::Found(None))]
        );
    }

    #[test]
    fn reading_needs_the_look_permission_and_counts_against_the_limit() {
        let state = State::new();
        let queries = [Query::Health; 3];

        let refused = state.ask(&queries, Permissions::NONE.with(Permission::Give));
        assert!(
            refused
                .iter()
                .all(|r| *r == Err(Denied::NoPermission(Permission::Look)))
        );

        let limits = Limits::try_new(32.0, 2).unwrap();
        let (mut world, mut player) = (state.world.clone(), state.player);
        let (mut inventory, mut stats) = (state.inventory.clone(), state.stats);
        let mut mobs = Mobs::default();
        let stage = Stage {
            world: &mut world,
            player: &mut player,
            inventory: &mut inventory,
            stats: &mut stats,
            mobs: &mut mobs,
        };
        let answers = ask(&queries, EVERYTHING, limits, Facing::East, &stage);
        assert_eq!(answers[2], Err(Denied::OverLimit));
        assert!(answers[..2].iter().all(Result::is_ok));
    }

    #[test]
    fn an_accepted_command_burns_its_fuel_and_undo_gives_it_back() {
        let mut state = State::new();
        let build = Command::SetBlock {
            pos: cell(2, 1, 2),
            block: BlockId::DIRT,
        };
        let start = state.inventory.clone();

        let results = state.run(&[build], EVERYTHING);

        assert_eq!(results, [Ok(())]);
        assert_eq!(fuel_of(&state), FUEL_HELD - u32::from(build.fuel_cost()));
        let events: Vec<_> = state.log.iter().copied().collect();
        assert_eq!(events.len(), 2);
        for event in events.iter().rev() {
            let mut stage = Stage {
                world: &mut state.world,
                player: &mut state.player,
                inventory: &mut state.inventory,
                stats: &mut state.stats,
                mobs: &mut state.mobs,
            };
            assert_eq!(apply(&mut stage, &event.inverse()), Ok(()));
        }
        assert_eq!(state.inventory, start);
        assert_eq!(state.world.block_at(cell(2, 1, 2)), BlockId::AIR);
    }

    #[test]
    fn a_pack_that_cannot_pay_gets_nothing_done_and_burns_nothing() {
        let mut state = State::broke();
        state.inventory.add(ItemId::FUEL, 3).unwrap();
        let (world, mob_id) = (state.world.clone(), state.next_mob);

        let results = state.run(&[spawn(5, 1, 5)], EVERYTHING);

        assert_eq!(results, [Err(Denied::NoFuel { needed: 5, held: 3 })]);
        assert_eq!(fuel_of(&state), 3);
        assert_eq!((&state.world, state.next_mob), (&world, mob_id));
        assert!(state.mobs.is_empty());
        assert!(state.log.is_empty());
    }

    #[test]
    fn a_refused_command_burns_no_fuel_and_starts_no_wait() {
        let mut state = State::new();
        let far = Command::SetBlock {
            pos: cell(33, 1, 0),
            block: BlockId::DIRT,
        };
        let near = Command::SetBlock {
            pos: cell(1, 1, 0),
            block: BlockId::DIRT,
        };

        let results = state.run_with(&[far, near], EVERYTHING, Limits::standard());

        assert_eq!(results, [Err(Denied::OutOfRange), Ok(())]);
        assert_eq!(fuel_of(&state), FUEL_HELD - u32::from(near.fuel_cost()));
    }

    #[test]
    fn a_kind_of_command_waits_but_other_kinds_do_not() {
        let mut state = State::new();
        let build = |x| Command::SetBlock {
            pos: cell(x, 1, 3),
            block: BlockId::DIRT,
        };
        let wait = build(0).cooldown();

        let together = state.run_with(
            &[build(1), build(2), Command::Teleport { to: cell(5, 1, 5) }],
            EVERYTHING,
            Limits::standard(),
        );
        assert_eq!(
            together,
            [
                Ok(()),
                Err(Denied::CoolingDown { ticks_left: wait }),
                Ok(())
            ]
        );
        assert_eq!(state.world.block_at(cell(2, 1, 3)), BlockId::AIR);

        state.tick = Tick(state.tick.0 + wait - 1);
        let almost = state.run_with(&[build(2)], EVERYTHING, Limits::standard());
        assert_eq!(almost, [Err(Denied::CoolingDown { ticks_left: 1 })]);

        state.tick = Tick(state.tick.0 + 1);
        let ready = state.run_with(&[build(2)], EVERYTHING, Limits::standard());
        assert_eq!(ready, [Ok(())]);
    }

    #[test]
    fn a_command_cooling_down_burns_no_fuel() {
        let mut state = State::new();
        let build = Command::SetBlock {
            pos: cell(1, 1, 1),
            block: BlockId::DIRT,
        };

        state.run_with(&[build, build], EVERYTHING, Limits::standard());

        assert_eq!(fuel_of(&state), FUEL_HELD - u32::from(build.fuel_cost()));
    }

    #[test]
    fn a_grant_can_land_in_the_slot_the_fuel_came_out_of() {
        let mut state = State::broke();
        // Exactly the fuel a grant costs, so paying empties the only slot the grant could use.
        let cost = give(ItemId::STONE, 1).fuel_cost();
        state.inventory.add(ItemId::FUEL, u32::from(cost)).unwrap();

        let results = state.run(&[give(ItemId::STONE, 1)], EVERYTHING);

        assert_eq!(results, [Ok(())]);
        assert_eq!(state.inventory.count_of(ItemId::STONE), 1);
        assert_eq!(fuel_of(&state), 0);
    }

    proptest! {
        #[test]
        fn queries_never_change_the_game_or_the_log(
            commands in prop::collection::vec(command(), 0..10),
            queries in prop::collection::vec(query(), 0..12),
        ) {
            let mut state = State::new();
            state.run(&commands, EVERYTHING);
            let before = (
                state.world.clone(),
                state.player,
                state.inventory.clone(),
                state.stats,
                state.mobs.clone(),
                state.log.clone(),
            );

            let answers = state.ask(&queries, EVERYTHING);

            prop_assert_eq!(answers.len(), queries.len());
            prop_assert_eq!(
                (state.world, state.player, state.inventory, state.stats, state.mobs, state.log),
                before
            );
        }

        #[test]
        fn a_find_answers_with_a_block_of_that_kind_no_farther_than_any_other(
            blocks in prop::collection::vec((-18i32..18, 0i32..3, -18i32..18), 0..12),
        ) {
            let mut state = State::new();
            for (x, y, z) in blocks {
                let pos = cell(x, y, z);
                let old = state.world.block_at(pos);
                let event = Event::BlockSet { pos, old, new: BlockId::DIRT };
                state.world.apply(&event).unwrap();
            }
            let Ok(Answer::Found(found)) = state.ask(
                &[Query::Find { block: BlockId::DIRT }],
                EVERYTHING,
            )[0] else {
                return Err(TestCaseError::fail("a find must be answered"));
            };

            let here = (0, 1, 0);
            let distance = |p: WorldPos| {
                (p.x - here.0).pow(2) + (p.y - here.1).pow(2) + (p.z - here.2).pow(2)
            };
            let within = |p: &WorldPos| {
                let reach = FIND_RADIUS;
                (p.x - here.0).abs() <= reach
                    && (p.y - here.1).abs() <= reach
                    && (p.z - here.2).abs() <= reach
            };
            let all: Vec<WorldPos> = (-20..20)
                .flat_map(|x| (-1..4).flat_map(move |y| (-20..20).map(move |z| cell(x, y, z))))
                .filter(|&p| state.world.block_at(p) == BlockId::DIRT && within(&p))
                .collect();
            match found {
                None => prop_assert!(all.is_empty()),
                Some(pos) => {
                    prop_assert_eq!(state.world.block_at(pos), BlockId::DIRT);
                    prop_assert!(all.iter().all(|&other| distance(pos) <= distance(other)));
                }
            }
        }
    }

    fn command() -> impl Strategy<Value = Command> {
        prop_oneof![
            (-45i32..45, -1i32..4, -45i32..45)
                .prop_map(|(x, y, z)| Command::Teleport { to: cell(x, y, z) }),
            (0u16..7, 0u16..120).prop_map(|(item, count)| give(ItemId(item), count)),
            ((-45i32..45, -1i32..4, -45i32..45), 0u16..6).prop_map(|((x, y, z), id)| {
                Command::SetBlock {
                    pos: cell(x, y, z),
                    block: BlockId(id),
                }
            }),
            (-45i32..45, -1i32..4, -45i32..45)
                .prop_map(|(x, y, z)| Command::SpawnMob { at: cell(x, y, z) }),
        ]
    }

    fn query() -> impl Strategy<Value = Query> {
        prop_oneof![
            (-45i32..45, -1i32..4, -45i32..45)
                .prop_map(|(x, y, z)| Query::Look { pos: cell(x, y, z) }),
            (0u16..6).prop_map(|id| Query::Find { block: BlockId(id) }),
            (0u16..8).prop_map(|item| Query::Count { item: ItemId(item) }),
            Just(Query::Health),
            Just(Query::Hunger),
            Just(Query::MobsNear),
            Just(Query::Facing),
            Just(Query::NearestMob),
        ]
    }

    proptest! {
        #[test]
        fn every_accepted_command_is_logged_and_the_log_replays_and_undoes_exactly(
            ticks in prop::collection::vec(prop::collection::vec(command(), 0..14), 0..6),
        ) {
            let mut live = State::new();
            let (world, player, inventory) = (live.world.clone(), live.player, live.inventory.clone());
            let mut accepted = 0;
            let mut burned = 0;
            for commands in &ticks {
                let results = live.run_with(commands, EVERYTHING, Limits::standard());
                prop_assert_eq!(results.len(), commands.len());
                prop_assert!(
                    results.iter().filter(|r| r.is_ok()).count() <= usize::from(MAX_EFFECTS)
                );
                accepted += results.iter().filter(|r| r.is_ok()).count();
                burned += commands
                    .iter()
                    .zip(&results)
                    .filter(|(_, result)| result.is_ok())
                    .map(|(command, _)| u32::from(command.fuel_cost()))
                    .sum::<u32>();
                // Some waits are still running at the next batch, and some are over.
                live.tick = Tick(live.tick.0 + 50);
            }
            prop_assert_eq!(fuel_of(&live), FUEL_HELD - burned);
            // A grant that spans several slots logs one event per slot.
            prop_assert!(live.log.len() >= accepted);
            prop_assert_eq!(live.log.is_empty(), accepted == 0);

            let mut replayed = State::new();
            for event in live.log.iter() {
                let mut stage = Stage {
                    world: &mut replayed.world,
                    player: &mut replayed.player,
                    inventory: &mut replayed.inventory,
                    stats: &mut replayed.stats,
                    mobs: &mut replayed.mobs,
                };
                prop_assert_eq!(apply(&mut stage, event), Ok(()));
            }
            prop_assert_eq!(&replayed.world, &live.world);
            prop_assert_eq!(replayed.player, live.player);
            prop_assert_eq!(&replayed.inventory, &live.inventory);
            prop_assert_eq!(&replayed.mobs, &live.mobs);

            let events: Vec<_> = live.log.iter().copied().collect();
            for event in events.iter().rev() {
                let mut stage = Stage {
                    world: &mut live.world,
                    player: &mut live.player,
                    inventory: &mut live.inventory,
                    stats: &mut live.stats,
                    mobs: &mut live.mobs,
                };
                prop_assert_eq!(apply(&mut stage, &event.inverse()), Ok(()));
            }
            prop_assert_eq!(&live.world, &world);
            prop_assert_eq!(live.player, player);
            prop_assert_eq!(&live.inventory, &inventory);
            prop_assert!(live.mobs.is_empty());
        }
    }
}
