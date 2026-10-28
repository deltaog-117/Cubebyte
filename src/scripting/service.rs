//! Running a script's commands against the game.
//!
//! Each command is checked, turned into an [`Event`], applied to the game state and then
//! logged, in that order, so the log never holds an event the game refused. A denied command
//! changes nothing and logs nothing.

use glam::Vec3;

use crate::blocks::registry as blocks;
use crate::events::apply::{Stage, StageError, apply_all, slot_events};
use crate::events::log::EventLog;
use crate::inventory::model::UnknownItem;
use crate::items::registry::ItemId;
use crate::player::model::Player;
use crate::player::service::collides;
use crate::scripting::model::{Command, Denied, Limits, Permissions};
use crate::shared::event::{Event, Spot};
use crate::shared::voxel::{BlockId, WorldPos};

/// Runs one tick's `commands` in order and reports what became of each.
///
/// Only the first [`Limits::max_effects`] commands are tried; the rest are
/// [`Denied::OverLimit`]. Denied commands count toward that limit.
pub fn run(
    commands: &[Command],
    granted: Permissions,
    limits: Limits,
    stage: &mut Stage<'_>,
    log: &mut EventLog,
) -> Vec<Result<(), Denied>> {
    let allowed = usize::from(limits.max_effects());
    commands
        .iter()
        .enumerate()
        .map(|(index, &command)| {
            let result = if index >= allowed {
                Err(Denied::OverLimit)
            } else {
                run_one(command, granted, limits, stage, log)
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
    stage: &mut Stage<'_>,
    log: &mut EventLog,
) -> Result<(), Denied> {
    let needed = command.permission();
    if !granted.allows(needed) {
        return Err(Denied::NoPermission(needed));
    }
    let events = match command {
        Command::Teleport { to } => vec![move_event(to, limits, stage)?],
        Command::GiveItem { item, count } => give_events(item, count, stage)?,
        Command::SetBlock { pos, block } => vec![set_event(pos, block, limits, stage)?],
    };
    apply_all(stage, &events)?;
    events.into_iter().for_each(|event| log.push(event));
    Ok(())
}

/// The bottom middle of block cell `cell`, where a player's feet go.
fn feet_of(cell: WorldPos) -> Vec3 {
    Vec3::new(cell.x as f32 + 0.5, cell.y as f32, cell.z as f32 + 0.5)
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

/// One slot event for each slot the grant changes, so undo puts every stack back where it was.
fn give_events(item: ItemId, count: u16, stage: &Stage<'_>) -> Result<Vec<Event>, Denied> {
    let room = stage
        .inventory
        .room_for(item)
        .ok_or(Denied::UnknownItem(item.0))?;
    if count == 0 {
        return Err(Denied::NothingToDo);
    }
    if room < u32::from(count) {
        return Err(Denied::NoRoom { room });
    }
    let mut after = stage.inventory.clone();
    after
        .add(item, u32::from(count))
        .map_err(|UnknownItem(raw)| Denied::UnknownItem(raw))?;
    Ok(slot_events(stage.inventory, &after))
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
    let middle = Vec3::new(pos.x as f32 + 0.5, pos.y as f32 + 0.5, pos.z as f32 + 0.5);
    if !within_range(stage.player.position(), middle, limits) {
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
    use crate::scripting::model::{MAX_EFFECTS, Permission};
    use crate::world::model::World;

    const EVERYTHING: Permissions = Permissions::NONE
        .with(Permission::Move)
        .with(Permission::Give)
        .with(Permission::Build);

    struct State {
        world: World,
        player: Player,
        inventory: Inventory,
        log: EventLog,
    }

    impl State {
        /// A stone floor whose top face is at y = 1, with the player standing on it.
        fn new() -> Self {
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
                log: EventLog::new(),
            }
        }

        fn run(&mut self, commands: &[Command], granted: Permissions) -> Vec<Result<(), Denied>> {
            self.run_with(commands, granted, Limits::standard())
        }

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
            };
            run(commands, granted, limits, &mut stage, &mut self.log)
        }
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
        assert!(
            state
                .log
                .iter()
                .eq([&Event::PlayerMoved { old: from, new: to }])
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
        assert!(state.inventory.is_empty());
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
        assert_eq!(state.log.len(), 1);
    }

    #[test]
    fn a_grant_that_does_not_fit_whole_gives_nothing() {
        let mut state = State::new();
        let room = state.inventory.room_for(ItemId::DIRT).unwrap();
        let too_many = u16::try_from(room + 1).unwrap();

        let results = state.run(&[give(ItemId::DIRT, too_many)], EVERYTHING);

        assert!(matches!(results[..], [Err(Denied::NoRoom { .. })]));
        assert!(state.inventory.is_empty());
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

    fn command() -> impl Strategy<Value = Command> {
        prop_oneof![
            (-45i32..45, -1i32..4, -45i32..45)
                .prop_map(|(x, y, z)| Command::Teleport { to: cell(x, y, z) }),
            (0u16..8, 0u16..120).prop_map(|(item, count)| give(ItemId(item), count)),
            ((-45i32..45, -1i32..4, -45i32..45), 0u16..6).prop_map(|((x, y, z), id)| {
                Command::SetBlock {
                    pos: cell(x, y, z),
                    block: BlockId(id),
                }
            }),
        ]
    }

    proptest! {
        #[test]
        fn every_accepted_command_is_logged_and_the_log_replays_and_undoes_exactly(
            ticks in prop::collection::vec(prop::collection::vec(command(), 0..14), 0..6),
        ) {
            let mut live = State::new();
            let (world, player) = (live.world.clone(), live.player);
            let mut accepted = 0;
            for commands in &ticks {
                let results = live.run(commands, EVERYTHING);
                prop_assert_eq!(results.len(), commands.len());
                prop_assert!(
                    results.iter().filter(|r| r.is_ok()).count() <= usize::from(MAX_EFFECTS)
                );
                accepted += results.iter().filter(|r| r.is_ok()).count();
            }
            // A grant that spans several slots logs one event per slot.
            prop_assert!(live.log.len() >= accepted);
            prop_assert_eq!(live.log.is_empty(), accepted == 0);

            let mut replayed = State::new();
            for event in live.log.iter() {
                let mut stage = Stage {
                    world: &mut replayed.world,
                    player: &mut replayed.player,
                    inventory: &mut replayed.inventory,
                };
                prop_assert_eq!(apply(&mut stage, event), Ok(()));
            }
            prop_assert_eq!(&replayed.world, &live.world);
            prop_assert_eq!(replayed.player, live.player);
            prop_assert_eq!(&replayed.inventory, &live.inventory);

            let events: Vec<_> = live.log.iter().copied().collect();
            for event in events.iter().rev() {
                let mut stage = Stage {
                    world: &mut live.world,
                    player: &mut live.player,
                    inventory: &mut live.inventory,
                };
                prop_assert_eq!(apply(&mut stage, &event.inverse()), Ok(()));
            }
            prop_assert_eq!(&live.world, &world);
            prop_assert_eq!(live.player, player);
            prop_assert!(live.inventory.is_empty());
        }
    }
}
