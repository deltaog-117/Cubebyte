//! Playing a script out over several ticks.
//!
//! Commands of one kind must wait for each other, so a script of three builds cannot all
//! land on one tick. A [`ScriptRun`] holds the commands still to do and offers the next one to
//! [`service::run`] on every tick. A command that is only cooling down is waited for; any
//! other refusal ends the script, since later steps usually rest on the one that failed.

use std::collections::VecDeque;

use crate::events::apply::Stage;
use crate::events::log::EventLog;
use crate::scripting::model::{Command, Denied, Limits, Pace, Permissions};
use crate::scripting::service::run;
use crate::shared::event::MobId;

/// A script in progress. It always has at least one command left: finishing it is what
/// [`step`] reports, and the run is gone with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRun {
    name: &'static str,
    pending: VecDeque<Command>,
}

impl ScriptRun {
    /// A run of `commands` called `name`, or `None` when there is nothing to do.
    pub fn new(name: &'static str, commands: Vec<Command>) -> Option<Self> {
        (!commands.is_empty()).then(|| Self {
            name,
            pending: commands.into(),
        })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// How many commands are still to do; at least one.
    pub fn left(&self) -> usize {
        self.pending.len()
    }
}

/// What one tick did to a script.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// `command` was accepted. `rest` is what remains, or `None` when the script is done.
    Ran {
        command: Command,
        rest: Option<ScriptRun>,
    },
    /// The next command's kind is still cooling down. Nothing changed.
    Waiting { run: ScriptRun, ticks_left: u64 },
    /// The next command was refused for a reason waiting will not cure, so the script ends.
    Stopped { name: &'static str, reason: Denied },
}

/// Offers `script`'s next command to the game on this tick.
pub fn step(
    mut script: ScriptRun,
    granted: Permissions,
    limits: Limits,
    pace: &mut Pace<'_>,
    stage: &mut Stage<'_>,
    log: &mut EventLog,
    next_mob: &mut MobId,
) -> Step {
    // A run is never empty, so there is always a first command.
    let Some(&command) = script.pending.front() else {
        return Step::Stopped {
            name: script.name,
            reason: Denied::NothingToDo,
        };
    };
    let result = run(&[command], granted, limits, pace, stage, log, next_mob)
        .into_iter()
        .next()
        .unwrap_or(Err(Denied::NothingToDo));
    match result {
        Ok(()) => {
            script.pending.pop_front();
            let rest = (!script.pending.is_empty()).then_some(script);
            Step::Ran { command, rest }
        }
        Err(Denied::CoolingDown { ticks_left }) => Step::Waiting {
            run: script,
            ticks_left,
        },
        Err(reason) => Step::Stopped {
            name: script.name,
            reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use proptest::prelude::*;

    use super::*;
    use crate::inventory::model::Inventory;
    use crate::items::registry::ItemId;
    use crate::mobs::model::Mobs;
    use crate::player::model::Player;
    use crate::scripting::model::Cooldowns;
    use crate::shared::clock::Tick;
    use crate::shared::event::Event;
    use crate::shared::voxel::{BlockId, WorldPos};
    use crate::survival::model::Stats;
    use crate::world::model::World;

    struct Game {
        world: World,
        player: Player,
        inventory: Inventory,
        stats: Stats,
        mobs: Mobs,
        log: EventLog,
        next_mob: MobId,
        cooldowns: Cooldowns,
    }

    fn cell(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    fn build(x: i32) -> Command {
        Command::SetBlock {
            pos: cell(x, 1, 3),
            block: BlockId::DIRT,
        }
    }

    impl Game {
        fn new(fuel: u32) -> Self {
            let mut world = World::default();
            for x in -20..20 {
                for z in -20..20 {
                    let event = Event::BlockSet {
                        pos: cell(x, 0, z),
                        old: BlockId::AIR,
                        new: BlockId::STONE,
                    };
                    world.apply(&event).unwrap();
                }
            }
            let mut inventory = Inventory::default();
            inventory.add(ItemId::FUEL, fuel).unwrap();
            Self {
                world,
                player: Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap(),
                inventory,
                stats: Stats::default(),
                mobs: Mobs::default(),
                log: EventLog::new(),
                next_mob: MobId::FIRST,
                cooldowns: Cooldowns::default(),
            }
        }

        fn step(&mut self, script: ScriptRun, tick: u64) -> Step {
            let mut stage = Stage {
                world: &mut self.world,
                player: &mut self.player,
                inventory: &mut self.inventory,
                stats: &mut self.stats,
                mobs: &mut self.mobs,
            };
            let mut pace = Pace {
                tick: Tick(tick),
                cooldowns: &mut self.cooldowns,
            };
            step(
                script,
                Permissions::ALL,
                Limits::standard(),
                &mut pace,
                &mut stage,
                &mut self.log,
                &mut self.next_mob,
            )
        }

        /// Steps `script` once per tick from tick 0 until it ends, up to `ticks` ticks.
        fn play(&mut self, mut script: Option<ScriptRun>, ticks: u64) -> Vec<Step> {
            let mut steps = Vec::new();
            for tick in 0..ticks {
                let Some(run) = script.take() else { break };
                let step = self.step(run, tick);
                script = match &step {
                    Step::Ran { rest, .. } => rest.clone(),
                    Step::Waiting { run, .. } => Some(run.clone()),
                    Step::Stopped { .. } => None,
                };
                steps.push(step);
            }
            steps
        }
    }

    #[test]
    fn an_empty_script_is_not_a_run() {
        assert_eq!(ScriptRun::new("nothing", Vec::new()), None);
    }

    #[test]
    fn a_script_of_builds_waits_out_each_cooldown_and_then_finishes() {
        let mut game = Game::new(20);
        let run = ScriptRun::new("row", vec![build(1), build(2), build(3)]);

        let steps = game.play(run, 100);

        let ran = steps
            .iter()
            .filter(|s| matches!(s, Step::Ran { .. }))
            .count();
        assert_eq!(ran, 3);
        assert!(matches!(steps.last(), Some(Step::Ran { rest: None, .. })));
        for x in 1..=3 {
            assert_eq!(game.world.block_at(cell(x, 1, 3)), BlockId::DIRT);
        }
        assert_eq!(
            game.inventory.count_of(ItemId::FUEL),
            20 - 3 * u32::from(build(0).fuel_cost())
        );
    }

    #[test]
    fn a_cooling_command_is_waited_for_and_changes_nothing() {
        let mut game = Game::new(20);
        let run = ScriptRun::new("row", vec![build(1), build(2)]).unwrap();

        let Step::Ran {
            rest: Some(run), ..
        } = game.step(run, 0)
        else {
            panic!("the first build is accepted");
        };
        let before = (game.world.clone(), game.inventory.clone(), game.log.clone());
        let Step::Waiting { run, ticks_left } = game.step(run, 1) else {
            panic!("the second build must wait");
        };

        assert_eq!(ticks_left, build(0).cooldown() - 1);
        assert_eq!(run.left(), 1);
        assert_eq!(
            (game.world.clone(), game.inventory.clone(), game.log.clone()),
            before
        );
    }

    #[test]
    fn a_refusal_that_waiting_cannot_cure_ends_the_script() {
        let mut game = Game::new(20);
        let far = Command::SetBlock {
            pos: cell(33, 1, 0),
            block: BlockId::DIRT,
        };
        let run = ScriptRun::new("far", vec![build(1), far, build(2)]);

        let steps = game.play(run, 100);

        assert!(matches!(
            steps.last(),
            Some(Step::Stopped {
                reason: Denied::OutOfRange,
                ..
            })
        ));
        assert_eq!(game.world.block_at(cell(1, 1, 3)), BlockId::DIRT);
        assert_eq!(game.world.block_at(cell(2, 1, 3)), BlockId::AIR);
    }

    #[test]
    fn a_pack_that_runs_dry_ends_the_script_where_it_stands() {
        let enough_for_one = u32::from(build(0).fuel_cost());
        let mut game = Game::new(enough_for_one);
        let run = ScriptRun::new("row", vec![build(1), build(2)]);

        let steps = game.play(run, 100);

        assert!(matches!(
            steps.last(),
            Some(Step::Stopped {
                reason: Denied::NoFuel { .. },
                ..
            })
        ));
        assert_eq!(game.inventory.count_of(ItemId::FUEL), 0);
    }

    proptest! {
        // However a script goes, fuel burned equals the cost of the commands that ran, the
        // run never ends with commands skipped silently, and every accepted command is one
        // logged edit.
        #[test]
        fn fuel_burned_is_the_cost_of_what_ran(
            xs in prop::collection::vec(-25i32..25, 1..8),
            fuel in 0u32..30,
        ) {
            let mut game = Game::new(fuel);
            let commands: Vec<_> = xs.iter().map(|&x| build(x)).collect();
            let run = ScriptRun::new("row", commands.clone());

            let steps = game.play(run, 400);

            let ran: Vec<_> = steps
                .iter()
                .filter_map(|s| match s {
                    Step::Ran { command, .. } => Some(*command),
                    _ => None,
                })
                .collect();
            prop_assert_eq!(&ran[..], &commands[..ran.len()]);
            let burned: u32 = ran.iter().map(|c| u32::from(c.fuel_cost())).sum();
            prop_assert_eq!(game.inventory.count_of(ItemId::FUEL), fuel - burned);
            prop_assert_eq!(game.log.actions(), ran.len());
            let ended = matches!(
                steps.last(),
                Some(Step::Ran { rest: None, .. } | Step::Stopped { .. })
            );
            prop_assert!(ended);
        }
    }
}
