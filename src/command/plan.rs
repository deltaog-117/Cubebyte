//! Turning a checked request into something the game already knows how to do.

use crate::command::model::Request;
use crate::crafting::program::RecipeProgram;
use crate::scripting::library::ScriptId;
use crate::scripting::model::Command;
use crate::shared::voxel::WorldPos;
use crate::simulation::action::Action;

/// What the game does with a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Show the table of commands.
    Table,
    /// Do this action now, as a key press would.
    Do(Action),
    /// Run this script command at the next tick, with the permission, range, fuel and wait
    /// checks a script gets. It waits for the tick so a replay, which runs script commands at
    /// that point of the tick, reproduces it.
    Queue(Command),
    /// Start this script or quick-hack, anchored on what it needs. The game finds the anchor,
    /// since the aim is not known here.
    Hack(ScriptId),
    /// Keep this recipe under this name, or open the recipe editor under the name when there
    /// is no program yet.
    Recipe {
        name: String,
        program: Option<RecipeProgram>,
    },
}

fn at(feet: WorldPos, by: [i32; 3]) -> WorldPos {
    WorldPos {
        x: feet.x.saturating_add(by[0]),
        y: feet.y.saturating_add(by[1]),
        z: feet.z.saturating_add(by[2]),
    }
}

/// What `request` means for a player standing in the cell `feet`.
pub fn plan(request: Request, feet: WorldPos) -> Plan {
    match request {
        Request::Help => Plan::Table,
        Request::Give { item, count } => Plan::Queue(Command::GiveItem { item, count }),
        Request::Go { by } => Plan::Queue(Command::Teleport { to: at(feet, by) }),
        Request::Set { block, by } => Plan::Queue(Command::SetBlock {
            pos: at(feet, by),
            block,
        }),
        Request::Spawn { by } => Plan::Queue(Command::SpawnMob { at: at(feet, by) }),
        Request::Rewind => Plan::Do(Action::Rewind),
        Request::Redo => Plan::Do(Action::Redo),
        Request::Zone { back } => Plan::Do(Action::ZoneRewind { centre: feet, back }),
        Request::Hack { script } => Plan::Hack(script),
        Request::Recipe { name, program } => Plan::Recipe { name, program },
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::voxel::BlockId;

    fn cell(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    #[test]
    fn positions_are_offsets_from_the_feet() {
        assert_eq!(
            plan(Request::Go { by: [1, 2, 3] }, cell(10, 20, 30)),
            Plan::Queue(Command::Teleport {
                to: cell(11, 22, 33)
            })
        );
        assert_eq!(
            plan(
                Request::Set {
                    block: BlockId::STONE,
                    by: [0, -1, 0]
                },
                cell(0, 5, 0)
            ),
            Plan::Queue(Command::SetBlock {
                pos: cell(0, 4, 0),
                block: BlockId::STONE
            })
        );
    }

    #[test]
    fn history_commands_act_now_and_help_opens_the_table() {
        let feet = cell(1, 2, 3);
        assert_eq!(plan(Request::Rewind, feet), Plan::Do(Action::Rewind));
        assert_eq!(plan(Request::Redo, feet), Plan::Do(Action::Redo));
        assert_eq!(
            plan(Request::Zone { back: 4 }, feet),
            Plan::Do(Action::ZoneRewind {
                centre: feet,
                back: 4
            })
        );
        assert_eq!(plan(Request::Help, feet), Plan::Table);
    }

    #[test]
    fn a_hack_is_left_to_the_game_to_anchor() {
        let script = ScriptId(5);
        assert_eq!(
            plan(Request::Hack { script }, cell(1, 2, 3)),
            Plan::Hack(script)
        );
    }

    #[test]
    fn a_recipe_is_left_to_the_game_to_keep_or_edit() {
        let plan_of = |program| {
            plan(
                Request::Recipe {
                    name: "loaf".into(),
                    program,
                },
                cell(0, 0, 0),
            )
        };
        assert_eq!(
            plan_of(None),
            Plan::Recipe {
                name: "loaf".into(),
                program: None
            }
        );
    }

    proptest! {
        // Even at the edge of the coordinate range an offset never overflows.
        #[test]
        fn offsets_at_the_edge_of_the_range_do_not_overflow(x in any::<i32>(), by in -1000i32..=1000) {
            let Plan::Queue(Command::Teleport { to }) =
                plan(Request::Go { by: [by, 0, 0] }, cell(x, 0, 0))
            else {
                panic!("go queues a teleport");
            };
            prop_assert_eq!(to.x, x.saturating_add(by));
        }
    }
}
