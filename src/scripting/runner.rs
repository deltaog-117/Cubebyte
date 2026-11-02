//! Playing a script out over several ticks.
//!
//! Commands of one kind must wait for each other, so a script of three builds cannot all
//! land on one tick. A [`ScriptRun`] holds the commands still to do and offers the next one to
//! [`service::run`] on every tick. A command that is only cooling down is waited for; any
//! other refusal ends the script, since later steps usually rest on the one that failed.
//!
//! A run is either a flat list of commands or a [`Program`] being stepped. A program decides
//! its next command as it goes, so an `if` reads the world on the tick it is reached; the
//! command it picks goes through the same checks as any other.

use std::collections::VecDeque;

use crate::events::apply::Stage;
use crate::events::log::EventLog;
use crate::scripting::model::{Command, Denied, Facing, Limits, Pace, Permissions};
use crate::scripting::program::{Next, Program, ProgramRun};
use crate::scripting::service::{ask, run};
use crate::shared::event::MobId;
use crate::shared::voxel::WorldPos;

/// Where a run gets its commands from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Flat(VecDeque<Command>),
    Program(Box<ProgramRun>),
}

/// A script in progress. A flat run always has at least one command left: finishing it is what
/// [`step`] reports, and the run is gone with it. A program run ends when [`step`] reports
/// [`Step::Finished`], which can only be known once the stepper has looked past the last command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRun {
    name: &'static str,
    source: Source,
    ran: usize,
}

impl ScriptRun {
    /// A run of `commands` called `name`, or `None` when there is nothing to do.
    pub fn new(name: &'static str, commands: Vec<Command>) -> Option<Self> {
        (!commands.is_empty()).then(|| Self {
            name,
            source: Source::Flat(commands.into()),
            ran: 0,
        })
    }

    /// A run of `program` called `name`, with its working cell on `anchor`.
    pub fn from_program(name: &'static str, program: &Program, anchor: WorldPos) -> Self {
        Self {
            name,
            source: Source::Program(Box::new(ProgramRun::new(program, anchor))),
            ran: 0,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// How many commands are still to do; at least one. A program cannot say before it ends
    /// how many it has left, so it reports one.
    pub fn left(&self) -> usize {
        match &self.source {
            Source::Flat(pending) => pending.len(),
            Source::Program(_) => 1,
        }
    }

    /// How many commands this run has had accepted so far.
    pub fn ran(&self) -> usize {
        self.ran
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
    /// The next command's kind is still cooling down, or a program used its share of the tick
    /// looking for one. Nothing changed.
    Waiting { run: ScriptRun, ticks_left: u64 },
    /// The next command was refused for a reason waiting will not cure, so the script ends.
    Stopped { name: &'static str, reason: Denied },
    /// A program found nothing more to run.
    Finished { name: &'static str },
}

/// What the run's source offers on this tick.
enum Pick {
    Command(Command),
    Pause,
    Finished,
}

/// The next command of `source`. A program reads through [`ask`] with the same permissions and
/// limits as its commands, and a read counts toward the per-tick limit.
fn pick(
    source: &mut Source,
    granted: Permissions,
    limits: Limits,
    stage: &Stage<'_>,
) -> Result<Pick, Denied> {
    match source {
        Source::Flat(pending) => Ok(pending
            .front()
            .map_or(Pick::Finished, |&command| Pick::Command(command))),
        Source::Program(program) => {
            // No program read depends on which way the player faces, so any direction will do.
            let mut read = |query| {
                ask(&[query], granted, limits, Facing::North, stage)
                    .into_iter()
                    .next()
                    .unwrap_or(Err(Denied::NothingToDo))
            };
            Ok(match program.settle(&mut read)? {
                Next::Command(command) => Pick::Command(command),
                Next::Pause => Pick::Pause,
                Next::Finished => Pick::Finished,
            })
        }
    }
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
    let name = script.name;
    let command = match pick(&mut script.source, granted, limits, stage) {
        Ok(Pick::Command(command)) => command,
        Ok(Pick::Pause) => {
            return Step::Waiting {
                run: script,
                ticks_left: 1,
            };
        }
        // A flat run is never empty, so this is a program that ran out of blocks.
        Ok(Pick::Finished) => return Step::Finished { name },
        Err(reason) => return Step::Stopped { name, reason },
    };
    let result = run(&[command], granted, limits, pace, stage, log, next_mob)
        .into_iter()
        .next()
        .unwrap_or(Err(Denied::NothingToDo));
    match result {
        Ok(()) => {
            script.ran += 1;
            let rest = match &mut script.source {
                Source::Flat(pending) => {
                    pending.pop_front();
                    !pending.is_empty()
                }
                Source::Program(program) => {
                    program.accept();
                    true
                }
            };
            Step::Ran {
                command,
                rest: rest.then_some(script),
            }
        }
        Err(Denied::CoolingDown { ticks_left }) => Step::Waiting {
            run: script,
            ticks_left,
        },
        Err(reason) => Step::Stopped { name, reason },
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
    use crate::scripting::program::{Block, Compare, Plan, Read, Test};
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
                    Step::Stopped { .. } | Step::Finished { .. } => None,
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

    fn dirt(by: [i32; 3]) -> Block {
        Block::Do(Plan::Build {
            by,
            block: BlockId::DIRT,
        })
    }

    fn program(blocks: Vec<Block>) -> Program {
        Program::try_new(blocks).expect("a valid test program")
    }

    #[test]
    fn a_program_plays_out_over_ticks_and_then_reports_it_finished() {
        let mut game = Game::new(20);
        let column = program(vec![Block::Repeat {
            times: 3,
            body: vec![dirt([0; 3]), Block::Shift { by: [0, 1, 0] }],
        }]);
        let run = ScriptRun::from_program("column", &column, cell(2, 1, 3));

        let steps = game.play(Some(run), 200);

        let ran = steps
            .iter()
            .filter(|s| matches!(s, Step::Ran { .. }))
            .count();
        assert_eq!(ran, 3);
        assert!(matches!(
            steps.last(),
            Some(Step::Finished { name: "column" })
        ));
        for y in 1..=3 {
            assert_eq!(game.world.block_at(cell(2, y, 3)), BlockId::DIRT);
        }
        assert_eq!(game.inventory.count_of(ItemId::FUEL), 20 - 3);
        assert_eq!(game.log.actions(), 3);
    }

    #[test]
    fn an_if_reads_the_game_on_the_tick_it_is_reached() {
        let choose = program(vec![Block::If {
            test: Test {
                read: Read::MobsNear,
                compare: Compare::Above,
                than: 0,
            },
            then: vec![dirt([1, 0, 0])],
            otherwise: vec![dirt([2, 0, 0])],
        }]);
        let mut calm = Game::new(20);
        let mut busy = Game::new(20);
        assert_eq!(
            busy.mobs.spawn(MobId::FIRST, Vec3::new(3.5, 1.0, 3.5), 20),
            Ok(())
        );

        calm.play(
            Some(ScriptRun::from_program("choose", &choose, cell(0, 1, 3))),
            50,
        );
        busy.play(
            Some(ScriptRun::from_program("choose", &choose, cell(0, 1, 3))),
            50,
        );

        assert_eq!(calm.world.block_at(cell(2, 1, 3)), BlockId::DIRT);
        assert_eq!(calm.world.block_at(cell(1, 1, 3)), BlockId::AIR);
        assert_eq!(busy.world.block_at(cell(1, 1, 3)), BlockId::DIRT);
        assert_eq!(busy.world.block_at(cell(2, 1, 3)), BlockId::AIR);
    }

    #[test]
    fn a_program_command_that_is_cooling_down_is_waited_for_not_skipped() {
        let mut game = Game::new(20);
        let two = program(vec![dirt([1, 0, 0]), dirt([2, 0, 0])]);
        let run = ScriptRun::from_program("two", &two, cell(0, 1, 3));

        let Step::Ran {
            rest: Some(run), ..
        } = game.step(run, 0)
        else {
            panic!("the first build is accepted");
        };
        let Step::Waiting { run, .. } = game.step(run, 1) else {
            panic!("the second build must wait");
        };

        assert_eq!(game.world.block_at(cell(2, 1, 3)), BlockId::AIR);
        let steps = game.play(Some(run), 100);
        assert!(matches!(steps.last(), Some(Step::Finished { .. })));
        assert_eq!(game.world.block_at(cell(2, 1, 3)), BlockId::DIRT);
    }

    #[test]
    fn a_program_that_never_reaches_a_command_stops_with_a_reason() {
        let mut game = Game::new(20);
        let leaf = Block::Repeat {
            times: crate::scripting::program::MAX_REPEAT,
            body: vec![Block::Shift { by: [0, 0, 1] }],
        };
        let mid = Block::Repeat {
            times: crate::scripting::program::MAX_REPEAT,
            body: vec![leaf],
        };
        let spin = program(vec![Block::Repeat {
            times: crate::scripting::program::MAX_REPEAT,
            body: vec![mid],
        }]);

        let steps = game.play(
            Some(ScriptRun::from_program("spin", &spin, cell(0, 1, 3))),
            1000,
        );

        assert!(matches!(
            steps.last(),
            Some(Step::Stopped {
                reason: Denied::TooLong,
                ..
            })
        ));
        assert_eq!(game.log.actions(), 0);
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
