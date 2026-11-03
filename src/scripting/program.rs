//! The scripting language's program: a tree of blocks, checked when it is built, and a stepper
//! that plays it out one command at a time.
//!
//! A program is the one form that blocks, a later typed view and ready-made quick-hacks all
//! share. It holds whole numbers only, and every loop has a fixed count, so a program always
//! ends: the stepper also stops it once it has spent [`RUN_BUDGET`] block visits, and never
//! visits more than [`WORK_PER_TICK`] in one tick, so no program can freeze the game.
//!
//! Positions in a program are offsets from a *working cell*. A run starts with the working cell
//! on its anchor (the player's feet, an aimed block, a mob), and [`Block::Shift`] moves it.
//! The stepper only decides *which* command comes next; the permission, range, fuel and wait
//! checks stay in [`crate::scripting::service::run`], so a program can do nothing a script
//! command could not.

use thiserror::Error;

use crate::items::registry::ItemId;
use crate::scripting::model::{Answer, Command, Denied, Query};
use crate::shared::voxel::{BlockId, WorldPos};

/// Deepest nesting of repeats and ifs, counting the top level of the program as level 1.
pub const MAX_DEPTH: usize = 4;

/// Most blocks one program may hold, nested ones included.
pub const MAX_BLOCKS: usize = 64;

/// Most times one repeat may run its body.
pub const MAX_REPEAT: u8 = 16;

/// Furthest one shift may move the working cell along an axis, in blocks.
pub const MAX_SHIFT: i32 = 1000;

/// Most blocks the stepper visits while looking for the next command in one tick.
pub const WORK_PER_TICK: u32 = 32;

/// Most blocks one run may visit in all, after which it is stopped.
pub const RUN_BUDGET: u32 = 4096;

/// One step that becomes a script command. Offsets are in blocks from the working cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Build { by: [i32; 3], block: BlockId },
    Hop { by: [i32; 3] },
    Give { item: ItemId, count: u16 },
    Summon { by: [i32; 3] },
}

fn offset_from(origin: WorldPos, by: [i32; 3]) -> WorldPos {
    WorldPos {
        x: origin.x.saturating_add(by[0]),
        y: origin.y.saturating_add(by[1]),
        z: origin.z.saturating_add(by[2]),
    }
}

impl Plan {
    /// The command this step is, for a working cell at `origin`.
    pub fn command(self, origin: WorldPos) -> Command {
        match self {
            Self::Build { by, block } => Command::SetBlock {
                pos: offset_from(origin, by),
                block,
            },
            Self::Hop { by } => Command::Teleport {
                to: offset_from(origin, by),
            },
            Self::Give { item, count } => Command::GiveItem { item, count },
            Self::Summon { by } => Command::SpawnMob {
                at: offset_from(origin, by),
            },
        }
    }
}

/// A thing an `if` can read, always as a whole number. A block reads as its ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    Health,
    Hunger,
    /// How many mobs are within range of the player.
    MobsNear,
    /// How many of `item` the pack holds.
    Count {
        item: ItemId,
    },
    /// The block at `by` from the working cell.
    Look {
        by: [i32; 3],
    },
}

/// How an `if` compares what it read with a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Below,
    Is,
    Above,
}

/// The question an `if` asks: is what was read below, equal to, or above a number?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Test {
    pub read: Read,
    pub compare: Compare,
    pub than: u32,
}

impl Test {
    /// Whether the test holds for a working cell at `cell`, reading through `ask`. Nothing in
    /// the game changes.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the read, or [`Denied::NothingToDo`] for an answer that cannot be
    /// compared.
    pub fn check(
        self,
        cell: WorldPos,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Result<bool, Denied> {
        let answer = ask(self.query(cell))?;
        self.holds(answer).ok_or(Denied::NothingToDo)
    }

    /// The query that answers this test for a working cell at `cell`.
    fn query(self, cell: WorldPos) -> Query {
        match self.read {
            Read::Health => Query::Health,
            Read::Hunger => Query::Hunger,
            Read::MobsNear => Query::MobsNear,
            Read::Count { item } => Query::Count { item },
            Read::Look { by } => Query::Look {
                pos: offset_from(cell, by),
            },
        }
    }

    /// Whether `answer` passes the test, or `None` when the answer is not a number or a block
    /// and so cannot be compared.
    fn holds(self, answer: Answer) -> Option<bool> {
        let number = match answer {
            Answer::Number(number) => number,
            Answer::Block(block) => u32::from(block.0),
            Answer::Found(_) | Answer::Direction(_) => return None,
        };
        Some(match self.compare {
            Compare::Below => number < self.than,
            Compare::Is => number == self.than,
            Compare::Above => number > self.than,
        })
    }
}

/// One block of a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Run one command.
    Do(Plan),
    /// Move the working cell by `by` blocks.
    Shift { by: [i32; 3] },
    /// Run `body` `times` times in a row.
    Repeat { times: u8, body: Vec<Block> },
    /// Run `then` if the test holds, otherwise `otherwise` (which may be empty).
    If {
        test: Test,
        then: Vec<Block>,
        otherwise: Vec<Block>,
    },
}

/// Why a list of blocks is not a program. Each message says what to change.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProgramError {
    #[error("a program needs at least one block")]
    Empty,
    #[error("a repeat or the then part of an if holds no blocks")]
    EmptyBody,
    #[error("blocks may sit at most {MAX_DEPTH} levels deep")]
    TooDeep,
    #[error("a program may hold at most {MAX_BLOCKS} blocks")]
    TooBig,
    #[error("a repeat must run from 1 to {MAX_REPEAT} times")]
    BadRepeat,
    #[error("a shift may move at most {MAX_SHIFT} blocks along an axis")]
    BadShift,
}

/// A checked list of blocks: the only way to get one is [`Program::try_new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    blocks: Vec<Block>,
}

impl Program {
    /// # Errors
    ///
    /// Returns a [`ProgramError`] when `blocks` is empty, nests deeper than [`MAX_DEPTH`],
    /// holds more than [`MAX_BLOCKS`] blocks, repeats a number of times outside
    /// `1..=`[`MAX_REPEAT`], shifts farther than [`MAX_SHIFT`], or has an empty repeat body or
    /// empty then part.
    pub fn try_new(blocks: Vec<Block>) -> Result<Self, ProgramError> {
        if blocks.is_empty() {
            return Err(ProgramError::Empty);
        }
        check(&blocks, 1, &mut 0)?;
        Ok(Self { blocks })
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The most fuel a run can burn: every command accepted, and for each `if` the dearer of
    /// its two parts. A run that takes the other part burns less, never more.
    pub fn fuel_cost(&self) -> u32 {
        cost(&self.blocks)
    }

    /// The commands a run anchored at `anchor` would produce, if every command were accepted
    /// and every read were answered by `ask`. Nothing in the game changes.
    ///
    /// # Errors
    ///
    /// Returns the reason a run would stop, such as [`Denied::TooLong`] or a refused read.
    pub fn preview(
        &self,
        anchor: WorldPos,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Result<Vec<Command>, Denied> {
        let mut run = ProgramRun::new(self, anchor);
        let mut commands = Vec::new();
        loop {
            match run.settle(ask)? {
                Next::Command(command) => {
                    commands.push(command);
                    run.accept();
                }
                Next::Pause => {}
                Next::Finished => return Ok(commands),
            }
        }
    }
}

fn check(blocks: &[Block], depth: usize, count: &mut usize) -> Result<(), ProgramError> {
    if blocks.is_empty() {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(ProgramError::TooDeep);
    }
    for block in blocks {
        *count += 1;
        if *count > MAX_BLOCKS {
            return Err(ProgramError::TooBig);
        }
        match block {
            Block::Do(_) => {}
            Block::Shift { by } => {
                if by
                    .iter()
                    .any(|axis| axis.unsigned_abs() > MAX_SHIFT.unsigned_abs())
                {
                    return Err(ProgramError::BadShift);
                }
            }
            Block::Repeat { times, body } => {
                if !(1..=MAX_REPEAT).contains(times) {
                    return Err(ProgramError::BadRepeat);
                }
                if body.is_empty() {
                    return Err(ProgramError::EmptyBody);
                }
                check(body, depth + 1, count)?;
            }
            Block::If {
                then, otherwise, ..
            } => {
                if then.is_empty() {
                    return Err(ProgramError::EmptyBody);
                }
                check(then, depth + 1, count)?;
                check(otherwise, depth + 1, count)?;
            }
        }
    }
    Ok(())
}

fn cost(blocks: &[Block]) -> u32 {
    let origin = WorldPos { x: 0, y: 0, z: 0 };
    blocks
        .iter()
        .map(|block| match block {
            Block::Do(plan) => u32::from(plan.command(origin).fuel_cost()),
            Block::Shift { .. } => 0,
            Block::Repeat { times, body } => u32::from(*times).saturating_mul(cost(body)),
            Block::If {
                then, otherwise, ..
            } => cost(then).max(cost(otherwise)),
        })
        .fold(0, u32::saturating_add)
}

/// Which list of blocks a [`Frame`] walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Branch {
    Top,
    Body,
    Then,
    Otherwise,
}

/// One list being walked: `index` is the block it is on, and `left` the repeats still to do,
/// this one included, for a repeat body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    branch: Branch,
    index: usize,
    left: u8,
}

/// What the stepper found on a block, copied out so the program can be left alone while the
/// frames change.
#[derive(Clone, Copy)]
enum Visit {
    End,
    Do(Plan),
    Shift([i32; 3]),
    Repeat(u8),
    If(Test),
}

/// What the stepper found after looking for the next command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// The next command to offer the game. It stays the next command until it is accepted.
    Command(Command),
    /// This tick's [`WORK_PER_TICK`] visits are used up; ask again next tick.
    Pause,
    /// There is nothing left to run.
    Finished,
}

/// A program being played out. It holds the place in the tree, the working cell and the
/// number of visits spent, and nothing about the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramRun {
    program: Program,
    anchor: WorldPos,
    shift: [i32; 3],
    frames: Vec<Frame>,
    ready: Option<Command>,
    spent: u32,
    accepted: usize,
}

impl ProgramRun {
    /// A run of `program` with its working cell on `anchor`.
    pub fn new(program: &Program, anchor: WorldPos) -> Self {
        Self {
            program: program.clone(),
            anchor,
            shift: [0; 3],
            frames: vec![Frame {
                branch: Branch::Top,
                index: 0,
                left: 0,
            }],
            ready: None,
            spent: 0,
            accepted: 0,
        }
    }

    /// How many commands were accepted so far.
    pub fn accepted(&self) -> usize {
        self.accepted
    }

    fn cell(&self) -> WorldPos {
        offset_from(self.anchor, self.shift)
    }

    /// The list the innermost frame walks. A frame that does not match the tree cannot be
    /// built by [`Self::settle`]; if one ever appeared, an empty list ends it harmlessly.
    fn current(&self) -> &[Block] {
        let mut list: &[Block] = &self.program.blocks;
        for pair in self.frames.windows(2) {
            let (parent, child) = (pair[0], pair[1]);
            list = match (list.get(parent.index), child.branch) {
                (Some(Block::Repeat { body, .. }), Branch::Body) => body,
                (Some(Block::If { then, .. }), Branch::Then) => then,
                (Some(Block::If { otherwise, .. }), Branch::Otherwise) => otherwise,
                _ => return &[],
            };
        }
        list
    }

    /// Moves the innermost frame to its next block.
    fn next_block(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.index += 1;
        }
    }

    /// Finds the next command, answering any `if` on the way with `ask`.
    ///
    /// Call it once per tick. It returns the same command again until [`Self::accept`] is called.
    ///
    /// # Errors
    ///
    /// Returns [`Denied::TooLong`] when the run has spent [`RUN_BUDGET`] visits, the refusal of
    /// a read that was denied, or [`Denied::NothingToDo`] for an answer that cannot be compared.
    /// The run should be dropped after an error.
    pub fn settle(
        &mut self,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Result<Next, Denied> {
        if let Some(command) = self.ready {
            return Ok(Next::Command(command));
        }
        let mut work = 0;
        loop {
            let Some(&frame) = self.frames.last() else {
                return Ok(Next::Finished);
            };
            if self.spent >= RUN_BUDGET {
                return Err(Denied::TooLong);
            }
            if work >= WORK_PER_TICK {
                return Ok(Next::Pause);
            }
            self.spent += 1;
            work += 1;
            let visit = match self.current().get(frame.index) {
                None => Visit::End,
                Some(Block::Do(plan)) => Visit::Do(*plan),
                Some(Block::Shift { by }) => Visit::Shift(*by),
                Some(Block::Repeat { times, .. }) => Visit::Repeat(*times),
                Some(Block::If { test, .. }) => Visit::If(*test),
            };
            match visit {
                Visit::End if frame.branch == Branch::Body && frame.left > 1 => {
                    if let Some(top) = self.frames.last_mut() {
                        top.index = 0;
                        top.left -= 1;
                    }
                }
                Visit::End => {
                    self.frames.pop();
                    self.next_block();
                }
                Visit::Do(plan) => {
                    let command = plan.command(self.cell());
                    self.ready = Some(command);
                    return Ok(Next::Command(command));
                }
                Visit::Shift(by) => {
                    for (axis, step) in self.shift.iter_mut().zip(by) {
                        *axis = axis.saturating_add(step);
                    }
                    self.next_block();
                }
                Visit::Repeat(times) => self.frames.push(Frame {
                    branch: Branch::Body,
                    index: 0,
                    left: times,
                }),
                Visit::If(test) => {
                    let answer = ask(test.query(self.cell()))?;
                    let holds = test.holds(answer).ok_or(Denied::NothingToDo)?;
                    self.frames.push(Frame {
                        branch: if holds {
                            Branch::Then
                        } else {
                            Branch::Otherwise
                        },
                        index: 0,
                        left: 0,
                    });
                }
            }
        }
    }

    /// Marks the command [`Self::settle`] offered as done, so the next call finds the one after
    /// it. Does nothing when no command is on offer.
    pub fn accept(&mut self) {
        if self.ready.take().is_some() {
            self.accepted += 1;
            self.next_block();
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn cell(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    fn build(by: [i32; 3]) -> Block {
        Block::Do(Plan::Build {
            by,
            block: BlockId::STONE,
        })
    }

    fn set(x: i32, y: i32, z: i32) -> Command {
        Command::SetBlock {
            pos: cell(x, y, z),
            block: BlockId::STONE,
        }
    }

    fn program(blocks: Vec<Block>) -> Program {
        Program::try_new(blocks).expect("a valid test program")
    }

    /// Answers every read with the same number.
    fn always(number: u32) -> impl FnMut(Query) -> Result<Answer, Denied> {
        move |_| Ok(Answer::Number(number))
    }

    fn is_mobs(compare: Compare, than: u32) -> Test {
        Test {
            read: Read::MobsNear,
            compare,
            than,
        }
    }

    #[test]
    fn a_program_is_checked_when_it_is_built() {
        assert_eq!(Program::try_new(vec![]), Err(ProgramError::Empty));
        let repeat = |times, body| Block::Repeat { times, body };
        assert_eq!(
            Program::try_new(vec![repeat(0, vec![build([0; 3])])]),
            Err(ProgramError::BadRepeat)
        );
        assert_eq!(
            Program::try_new(vec![repeat(MAX_REPEAT + 1, vec![build([0; 3])])]),
            Err(ProgramError::BadRepeat)
        );
        assert_eq!(
            Program::try_new(vec![repeat(2, vec![])]),
            Err(ProgramError::EmptyBody)
        );
        assert_eq!(
            Program::try_new(vec![Block::Shift {
                by: [0, MAX_SHIFT + 1, 0]
            }]),
            Err(ProgramError::BadShift)
        );
        assert_eq!(
            Program::try_new(vec![Block::Shift {
                by: [i32::MIN, 0, 0]
            }]),
            Err(ProgramError::BadShift)
        );
        let empty_then = Block::If {
            test: is_mobs(Compare::Above, 0),
            then: vec![],
            otherwise: vec![build([0; 3])],
        };
        assert_eq!(
            Program::try_new(vec![empty_then]),
            Err(ProgramError::EmptyBody)
        );
    }

    #[test]
    fn nesting_and_size_are_capped() {
        let mut nested = build([0; 3]);
        for _ in 0..MAX_DEPTH {
            nested = Block::Repeat {
                times: 1,
                body: vec![nested],
            };
        }
        assert_eq!(Program::try_new(vec![nested]), Err(ProgramError::TooDeep));
        let mut allowed = build([0; 3]);
        for _ in 0..MAX_DEPTH - 1 {
            allowed = Block::Repeat {
                times: 1,
                body: vec![allowed],
            };
        }
        assert!(Program::try_new(vec![allowed]).is_ok());
        let many = vec![build([0; 3]); MAX_BLOCKS + 1];
        assert_eq!(Program::try_new(many), Err(ProgramError::TooBig));
        assert!(Program::try_new(vec![build([0; 3]); MAX_BLOCKS]).is_ok());
    }

    #[test]
    fn a_shift_moves_the_working_cell_for_everything_after_it() {
        let tower = program(vec![
            Block::Shift { by: [2, 0, 0] },
            Block::Repeat {
                times: 3,
                body: vec![build([0; 3]), Block::Shift { by: [0, 1, 0] }],
            },
        ]);

        let commands = tower.preview(cell(10, 5, 10), &mut always(0));

        assert_eq!(
            commands,
            Ok(vec![set(12, 5, 10), set(12, 6, 10), set(12, 7, 10)])
        );
    }

    #[test]
    fn an_if_runs_exactly_one_of_its_parts() {
        let choose = program(vec![Block::If {
            test: is_mobs(Compare::Above, 0),
            then: vec![build([1, 0, 0])],
            otherwise: vec![build([2, 0, 0]), build([3, 0, 0])],
        }]);

        assert_eq!(
            choose.preview(cell(0, 0, 0), &mut always(1)),
            Ok(vec![set(1, 0, 0)])
        );
        assert_eq!(
            choose.preview(cell(0, 0, 0), &mut always(0)),
            Ok(vec![set(2, 0, 0), set(3, 0, 0)])
        );
    }

    #[test]
    fn an_if_without_an_else_part_can_do_nothing() {
        let maybe = program(vec![
            Block::If {
                test: is_mobs(Compare::Is, 5),
                then: vec![build([1, 0, 0])],
                otherwise: vec![],
            },
            build([9, 0, 0]),
        ]);

        assert_eq!(
            maybe.preview(cell(0, 0, 0), &mut always(0)),
            Ok(vec![set(9, 0, 0)])
        );
    }

    #[test]
    fn a_look_test_asks_about_the_cell_it_names_from_the_working_cell() {
        let probe = program(vec![
            Block::Shift { by: [0, 1, 0] },
            Block::If {
                test: Test {
                    read: Read::Look { by: [1, 0, 0] },
                    compare: Compare::Is,
                    than: u32::from(BlockId::STONE.0),
                },
                then: vec![build([0; 3])],
                otherwise: vec![],
            },
        ]);
        let mut asked = Vec::new();
        let mut ask = |query| {
            asked.push(query);
            Ok(Answer::Block(BlockId::STONE))
        };

        let commands = probe.preview(cell(4, 4, 4), &mut ask);

        assert_eq!(commands, Ok(vec![set(4, 5, 4)]));
        assert_eq!(asked, vec![Query::Look { pos: cell(5, 5, 4) }]);
    }

    #[test]
    fn a_refused_read_stops_the_run_with_its_reason() {
        let ask_far = program(vec![Block::If {
            test: is_mobs(Compare::Above, 0),
            then: vec![build([0; 3])],
            otherwise: vec![],
        }]);

        let result = ask_far.preview(cell(0, 0, 0), &mut |_| Err(Denied::OutOfRange));

        assert_eq!(result, Err(Denied::OutOfRange));
    }

    #[test]
    fn an_answer_that_is_not_a_number_stops_the_run() {
        let odd = program(vec![Block::If {
            test: is_mobs(Compare::Above, 0),
            then: vec![build([0; 3])],
            otherwise: vec![],
        }]);

        let result = odd.preview(cell(0, 0, 0), &mut |_| Ok(Answer::Found(None)));

        assert_eq!(result, Err(Denied::NothingToDo));
    }

    #[test]
    fn a_command_on_offer_stays_until_it_is_accepted() {
        let two = program(vec![build([1, 0, 0]), build([2, 0, 0])]);
        let mut run = ProgramRun::new(&two, cell(0, 0, 0));
        let mut ask = always(0);

        let first = run.settle(&mut ask);
        assert_eq!(first, Ok(Next::Command(set(1, 0, 0))));
        assert_eq!(run.settle(&mut ask), first);
        assert_eq!(run.accepted(), 0);

        run.accept();
        assert_eq!(run.settle(&mut ask), Ok(Next::Command(set(2, 0, 0))));
        run.accept();
        assert_eq!(run.settle(&mut ask), Ok(Next::Finished));
        assert_eq!(run.accepted(), 2);
    }

    #[test]
    fn a_long_stretch_of_shifts_pauses_instead_of_spending_the_whole_tick() {
        let walk = program(vec![
            Block::Repeat {
                times: 16,
                body: vec![Block::Repeat {
                    times: 16,
                    body: vec![Block::Shift { by: [1, 0, 0] }],
                }],
            },
            build([0; 3]),
        ]);
        let mut run = ProgramRun::new(&walk, cell(0, 0, 0));
        let mut ask = always(0);

        assert_eq!(run.settle(&mut ask), Ok(Next::Pause));
        let mut ticks = 1;
        while run.settle(&mut ask) == Ok(Next::Pause) {
            ticks += 1;
        }

        assert!(ticks > 1);
        assert_eq!(run.settle(&mut ask), Ok(Next::Command(set(256, 0, 0))));
    }

    #[test]
    fn a_run_that_never_reaches_a_command_is_stopped_by_its_budget() {
        let innermost = Block::Repeat {
            times: MAX_REPEAT,
            body: vec![Block::Shift { by: [0, 0, 1] }],
        };
        let middle = Block::Repeat {
            times: MAX_REPEAT,
            body: vec![innermost],
        };
        let spin = program(vec![Block::Repeat {
            times: MAX_REPEAT,
            body: vec![middle],
        }]);

        assert_eq!(
            spin.preview(cell(0, 0, 0), &mut always(0)),
            Err(Denied::TooLong)
        );
    }

    #[test]
    fn the_fuel_cost_is_the_dearest_way_through() {
        let mixed = program(vec![
            Block::Repeat {
                times: 3,
                body: vec![build([0; 3])],
            },
            Block::If {
                test: is_mobs(Compare::Above, 0),
                then: vec![build([0; 3])],
                otherwise: vec![Block::Do(Plan::Summon { by: [0; 3] })],
            },
        ]);

        let build_cost = u32::from(set(0, 0, 0).fuel_cost());
        let summon_cost = u32::from(Command::SpawnMob { at: cell(0, 0, 0) }.fuel_cost());
        assert_eq!(
            mixed.fuel_cost(),
            3 * build_cost + build_cost.max(summon_cost)
        );
    }

    fn offset() -> impl Strategy<Value = [i32; 3]> {
        prop::array::uniform3(-5i32..5)
    }

    fn plan() -> impl Strategy<Value = Plan> {
        prop_oneof![
            (offset(), 0u16..4).prop_map(|(by, id)| Plan::Build {
                by,
                block: BlockId(id)
            }),
            offset().prop_map(|by| Plan::Hop { by }),
            (1u16..8, 1u16..20).prop_map(|(id, count)| Plan::Give {
                item: ItemId(id),
                count
            }),
            offset().prop_map(|by| Plan::Summon { by }),
        ]
    }

    fn test() -> impl Strategy<Value = Test> {
        let read = prop_oneof![
            Just(Read::Health),
            Just(Read::Hunger),
            Just(Read::MobsNear),
            (1u16..8).prop_map(|id| Read::Count { item: ItemId(id) }),
            offset().prop_map(|by| Read::Look { by }),
        ];
        let compare = prop_oneof![
            Just(Compare::Below),
            Just(Compare::Is),
            Just(Compare::Above)
        ];
        (read, compare, 0u32..30).prop_map(|(read, compare, than)| Test {
            read,
            compare,
            than,
        })
    }

    fn block() -> BoxedStrategy<Block> {
        let leaf = prop_oneof![
            plan().prop_map(Block::Do),
            offset().prop_map(|by| Block::Shift { by }),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                (1u8..=MAX_REPEAT, prop::collection::vec(inner.clone(), 1..4))
                    .prop_map(|(times, body)| Block::Repeat { times, body }),
                (
                    test(),
                    prop::collection::vec(inner.clone(), 1..4),
                    prop::collection::vec(inner, 0..3)
                )
                    .prop_map(|(test, then, otherwise)| Block::If {
                        test,
                        then,
                        otherwise
                    }),
            ]
        })
        .boxed()
    }

    proptest! {
        // However a valid program is built, a preview comes back: it either finishes within
        // the visit budget or is stopped by it, so no program can run forever.
        #[test]
        fn every_valid_program_ends(
            blocks in prop::collection::vec(block(), 1..6),
            answer in 0u32..30,
        ) {
            prop_assume!(Program::try_new(blocks.clone()).is_ok());
            let program = Program::try_new(blocks).unwrap();

            match program.preview(cell(0, 0, 0), &mut always(answer)) {
                Ok(commands) => prop_assert!(commands.len() as u32 <= RUN_BUDGET),
                Err(reason) => prop_assert_eq!(reason, Denied::TooLong),
            }
        }

        // The same program and the same answers always give the same commands, wherever the
        // anchor is, shifted by it.
        #[test]
        fn a_preview_is_deterministic_and_follows_the_anchor(
            blocks in prop::collection::vec(block(), 1..6),
            answer in 0u32..30,
            anchor in offset(),
        ) {
            prop_assume!(Program::try_new(blocks.clone()).is_ok());
            let program = Program::try_new(blocks).unwrap();
            let at = cell(anchor[0], anchor[1], anchor[2]);

            let here = program.preview(at, &mut always(answer));
            let again = program.preview(at, &mut always(answer));
            let origin = program.preview(cell(0, 0, 0), &mut always(answer));

            prop_assert_eq!(&here, &again);
            if let (Ok(here), Ok(origin)) = (here, origin) {
                prop_assert_eq!(here.len(), origin.len());
                let shifted: Vec<_> = origin
                    .iter()
                    .map(|command| match *command {
                        Command::SetBlock { pos, block } => Command::SetBlock {
                            pos: offset_from(pos, anchor),
                            block,
                        },
                        Command::Teleport { to } => Command::Teleport {
                            to: offset_from(to, anchor),
                        },
                        Command::SpawnMob { at } => Command::SpawnMob {
                            at: offset_from(at, anchor),
                        },
                        give @ Command::GiveItem { .. } => give,
                    })
                    .collect();
                prop_assert_eq!(here, shifted);
            }
        }

        // The advertised fuel cost is a ceiling: no run, whichever way its ifs go, burns more.
        #[test]
        fn a_run_never_costs_more_than_the_advertised_fuel(
            blocks in prop::collection::vec(block(), 1..6),
            answer in 0u32..30,
        ) {
            prop_assume!(Program::try_new(blocks.clone()).is_ok());
            let program = Program::try_new(blocks).unwrap();

            if let Ok(commands) = program.preview(cell(0, 0, 0), &mut always(answer)) {
                let burned: u32 = commands.iter().map(|c| u32::from(c.fuel_cost())).sum();
                prop_assert!(burned <= program.fuel_cost());
            }
        }
    }
}
