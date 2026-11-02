//! The ready-made scripts and quick-hacks a player can start.
//!
//! Each one is a [`Program`] anchored on something: the player's feet, the block under the
//! crosshair, or the nearest mob. A script's ID is its position in the table, so a new one is
//! only ever appended.

use std::sync::LazyLock;

use crate::items::registry::ItemId;
use crate::scripting::program::{Block, Compare, Program, Read, Test};
use crate::scripting::runner::ScriptRun;
use crate::shared::voxel::{BlockId, WorldPos};

pub use crate::scripting::program::Plan;

/// Identifies a script by its position in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptId(pub u8);

/// What a script is anchored on, which is what its offsets count from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The cell the player stands in. Needs no aiming.
    Feet,
    /// The block under the crosshair.
    Block,
    /// The mob nearest to the player, within the script range.
    Mob,
}

impl Target {
    /// What the player must do for this target to exist, for a message when it does not.
    pub const fn needs(self) -> &'static str {
        match self {
            Self::Feet => "nothing",
            Self::Block => "aim at a block",
            Self::Mob => "no mob in range",
        }
    }
}

/// A named program and what it is anchored on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub name: &'static str,
    pub target: Target,
    program: Program,
}

fn build(by: [i32; 3], block: BlockId) -> Block {
    Block::Do(Plan::Build { by, block })
}

fn shift(by: [i32; 3]) -> Block {
    Block::Shift { by }
}

fn repeat(times: u8, body: Vec<Block>) -> Block {
    Block::Repeat { times, body }
}

/// A built-in script, or `None` (and an error in the log) if its program is not valid. The slot
/// stays in the table so later IDs do not move; `every_slot_holds_a_valid_program` fails the
/// build before that can ship.
fn script(name: &'static str, target: Target, blocks: Vec<Block>) -> Option<Script> {
    match Program::try_new(blocks) {
        Ok(program) => Some(Script {
            name,
            target,
            program,
        }),
        Err(error) => {
            tracing::error!(%error, name, "a built-in script is not a valid program");
            None
        }
    }
}

/// The table, built once. Existing positions never move.
static SCRIPTS: LazyLock<Vec<Option<Script>>> = LazyLock::new(|| {
    let stone = BlockId::STONE;
    vec![
        script(
            "tower",
            Target::Feet,
            vec![
                shift([2, 0, 0]),
                repeat(3, vec![build([0, 0, 0], stone), shift([0, 1, 0])]),
            ],
        ),
        script(
            "hop",
            Target::Feet,
            vec![Block::Do(Plan::Hop { by: [4, 0, 0] })],
        ),
        script(
            "stock",
            Target::Feet,
            vec![Block::Do(Plan::Give {
                item: ItemId::DIRT,
                count: 10,
            })],
        ),
        script(
            "summon",
            Target::Feet,
            vec![Block::Do(Plan::Summon { by: [6, 0, 0] })],
        ),
        script(
            "pillar",
            Target::Block,
            vec![
                shift([0, 1, 0]),
                repeat(3, vec![build([0, 0, 0], stone), shift([0, 1, 0])]),
            ],
        ),
        script(
            "perch",
            Target::Block,
            vec![Block::Do(Plan::Hop { by: [0, 1, 0] })],
        ),
        script("dig", Target::Block, vec![build([0, 0, 0], BlockId::AIR)]),
        script(
            "cage",
            Target::Mob,
            vec![
                repeat(
                    2,
                    vec![
                        build([1, 0, 0], stone),
                        build([-1, 0, 0], stone),
                        build([0, 0, 1], stone),
                        build([0, 0, -1], stone),
                        shift([0, 1, 0]),
                    ],
                ),
                build([0, 0, 0], stone),
            ],
        ),
        script(
            "ward",
            Target::Feet,
            vec![Block::If {
                test: Test {
                    read: Read::MobsNear,
                    compare: Compare::Above,
                    than: 0,
                },
                then: vec![build([2, 0, 0], stone), build([2, 1, 0], stone)],
                otherwise: vec![Block::Do(Plan::Give {
                    item: ItemId::DIRT,
                    count: 5,
                })],
            }],
        ),
    ]
});

/// The script `id` names, or `None` for an ID past the table.
pub fn get(id: ScriptId) -> Option<&'static Script> {
    SCRIPTS.get(usize::from(id.0))?.as_ref()
}

/// Every script, in table order.
pub fn ids() -> impl Iterator<Item = ScriptId> {
    // The table is far shorter than 256 entries.
    (0..SCRIPTS.len() as u8)
        .map(ScriptId)
        .filter(|&id| get(id).is_some())
}

/// Every script anchored on `target`, in table order.
pub fn ids_for(target: Target) -> impl Iterator<Item = ScriptId> {
    ids().filter(move |&id| get(id).is_some_and(|script| script.target == target))
}

/// The script called `name`, in any letter case.
pub fn find(name: &str) -> Option<(ScriptId, &'static Script)> {
    ids()
        .filter_map(|id| Some((id, get(id)?)))
        .find(|(_, script)| script.name.eq_ignore_ascii_case(name))
}

/// The cell a script of kind `target` is anchored on, given what the player is standing in,
/// aiming at and near; `None` when that thing does not exist right now.
pub fn anchor(
    target: Target,
    feet: WorldPos,
    aimed_block: Option<WorldPos>,
    nearest_mob: Option<WorldPos>,
) -> Option<WorldPos> {
    match target {
        Target::Feet => Some(feet),
        Target::Block => aimed_block,
        Target::Mob => nearest_mob,
    }
}

impl Script {
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// A run of this script with its working cell on `anchor`.
    pub fn start(&self, anchor: WorldPos) -> ScriptRun {
        ScriptRun::from_program(self.name, &self.program, anchor)
    }

    /// The most fuel the whole script can burn (see [`Program::fuel_cost`]).
    pub fn fuel_cost(&self) -> u32 {
        self.program.fuel_cost()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::blocks::registry as blocks;
    use crate::items::registry as items;
    use crate::scripting::model::{Answer, Command, Denied};

    fn cell(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    /// The commands of script `name` anchored at `at`, with every read answered `answer`.
    fn commands(name: &str, at: WorldPos, answer: u32) -> Vec<Command> {
        let (_, script) = find(name).expect("a listed script");
        script
            .program()
            .preview(at, &mut |_| Ok::<_, Denied>(Answer::Number(answer)))
            .expect("a built-in program runs")
    }

    fn set(x: i32, y: i32, z: i32, block: BlockId) -> Command {
        Command::SetBlock {
            pos: cell(x, y, z),
            block,
        }
    }

    #[test]
    fn every_slot_holds_a_valid_program() {
        assert!(SCRIPTS.iter().all(Option::is_some));
        assert!(ids().count() >= 9);
        for id in ids() {
            let script = get(id).expect("listed scripts exist");
            assert!(!script.name.is_empty());
            assert!(
                script
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() && crate::ui::font::glyph(c).is_some())
            );
        }
    }

    #[test]
    fn script_names_are_unique() {
        let names: Vec<_> = ids().filter_map(get).map(|script| script.name).collect();
        for (index, name) in names.iter().enumerate() {
            assert!(!names[index + 1..].contains(name), "{name} twice");
        }
    }

    #[test]
    fn the_first_four_scripts_are_still_the_original_ones() {
        let stone = BlockId::STONE;
        assert_eq!(
            commands("tower", cell(10, 5, 10), 0),
            vec![
                set(12, 5, 10, stone),
                set(12, 6, 10, stone),
                set(12, 7, 10, stone)
            ]
        );
        assert_eq!(
            commands("hop", cell(1, 2, 3), 0),
            vec![Command::Teleport { to: cell(5, 2, 3) }]
        );
        assert_eq!(
            commands("stock", cell(0, 0, 0), 0),
            vec![Command::GiveItem {
                item: ItemId::DIRT,
                count: 10
            }]
        );
        assert_eq!(
            commands("summon", cell(1, 2, 3), 0),
            vec![Command::SpawnMob { at: cell(7, 2, 3) }]
        );
        assert_eq!(get(ScriptId(0)).map(|s| s.name), Some("tower"));
        assert_eq!(get(ScriptId(3)).map(|s| s.name), Some("summon"));
    }

    #[test]
    fn block_hacks_build_on_and_around_the_aimed_block() {
        let stone = BlockId::STONE;
        assert_eq!(
            commands("pillar", cell(4, 10, 4), 0),
            vec![
                set(4, 11, 4, stone),
                set(4, 12, 4, stone),
                set(4, 13, 4, stone)
            ]
        );
        assert_eq!(
            commands("perch", cell(4, 10, 4), 0),
            vec![Command::Teleport { to: cell(4, 11, 4) }]
        );
        assert_eq!(
            commands("dig", cell(4, 10, 4), 0),
            vec![set(4, 10, 4, BlockId::AIR)]
        );
    }

    #[test]
    fn the_cage_walls_the_mob_in_two_high_and_roofs_it() {
        let walls = commands("cage", cell(0, 5, 0), 0);

        assert_eq!(walls.len(), 9);
        for height in [5, 6] {
            for (x, z) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                assert!(walls.contains(&set(x, height, z, BlockId::STONE)));
            }
        }
        assert_eq!(walls.last(), Some(&set(0, 7, 0, BlockId::STONE)));
    }

    #[test]
    fn the_ward_walls_up_when_a_mob_is_near_and_stocks_up_otherwise() {
        assert_eq!(commands("ward", cell(0, 1, 0), 1).len(), 2);
        assert_eq!(
            commands("ward", cell(0, 1, 0), 0),
            vec![Command::GiveItem {
                item: ItemId::DIRT,
                count: 5
            }]
        );
    }

    #[test]
    fn every_script_uses_defined_items_and_blocks() {
        for id in ids() {
            let script = get(id).expect("listed scripts exist");
            for answer in [0, 1] {
                for command in commands(script.name, cell(0, 0, 0), answer) {
                    match command {
                        Command::SetBlock { block, .. } => {
                            assert!(blocks::get(block).is_some(), "{}", script.name);
                        }
                        Command::GiveItem { item, count } => {
                            assert!(items::get(item).is_some(), "{}", script.name);
                            assert!(count > 0, "{} gives nothing", script.name);
                        }
                        Command::Teleport { .. } | Command::SpawnMob { .. } => {}
                    }
                }
            }
        }
    }

    #[test]
    fn ids_past_the_table_are_not_scripts() {
        assert!(get(ScriptId(SCRIPTS.len() as u8)).is_none());
    }

    #[test]
    fn find_ignores_letter_case_and_lists_by_target() {
        assert_eq!(find("TOWER").map(|(id, _)| id), Some(ScriptId(0)));
        assert!(find("nope").is_none());
        let feet: Vec<_> = ids_for(Target::Feet).collect();
        assert!(feet.contains(&ScriptId(0)));
        assert!(!feet.contains(&find("pillar").map(|(id, _)| id).unwrap_or(ScriptId(0))));
        assert_eq!(
            ids_for(Target::Feet).count()
                + ids_for(Target::Block).count()
                + ids_for(Target::Mob).count(),
            ids().count()
        );
    }

    #[test]
    fn a_target_exists_only_when_there_is_something_to_anchor_on() {
        let (feet, block, mob) = (cell(0, 0, 0), cell(1, 1, 1), cell(2, 2, 2));
        assert_eq!(anchor(Target::Feet, feet, None, None), Some(feet));
        assert_eq!(anchor(Target::Block, feet, None, Some(mob)), None);
        assert_eq!(anchor(Target::Block, feet, Some(block), None), Some(block));
        assert_eq!(anchor(Target::Mob, feet, Some(block), None), None);
        assert_eq!(anchor(Target::Mob, feet, None, Some(mob)), Some(mob));
    }

    proptest! {
        // A script means the same thing wherever it is anchored: every command is the one at
        // the origin shifted by the anchor, and the cost does not depend on the anchor.
        #[test]
        fn a_script_is_its_steps_shifted_by_the_anchor(
            x in -1000i32..1000, y in -100i32..100, z in -1000i32..1000,
            answer in 0u32..3,
        ) {
            for id in ids() {
                let script = get(id).expect("listed scripts exist");
                let here = commands(script.name, cell(x, y, z), answer);
                let there = commands(script.name, cell(0, 0, 0), answer);
                prop_assert_eq!(here.len(), there.len());
                let cost: u32 = here.iter().map(|c| u32::from(c.fuel_cost())).sum();
                prop_assert!(cost <= script.fuel_cost());
            }
        }
    }
}
