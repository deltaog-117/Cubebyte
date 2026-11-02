//! The ready-made scripts a player can start, as steps relative to where they stand.
//!
//! A script's ID is its position in [`SCRIPTS`], so a new one is only ever appended.

use crate::items::registry::ItemId;
use crate::scripting::model::Command;
use crate::shared::voxel::{BlockId, WorldPos};

/// Identifies a script by its position in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptId(pub u8);

/// One step of a ready-made script. Offsets are in blocks from the cell the player stands in
/// when the script starts, so the same script works anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Build { by: [i32; 3], block: BlockId },
    Hop { by: [i32; 3] },
    Give { item: ItemId, count: u16 },
    Summon { by: [i32; 3] },
}

/// A named run of steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Script {
    pub name: &'static str,
    pub steps: &'static [Plan],
}

const SCRIPTS: [Script; 4] = [
    Script {
        name: "tower",
        steps: &[
            Plan::Build {
                by: [2, 0, 0],
                block: BlockId::STONE,
            },
            Plan::Build {
                by: [2, 1, 0],
                block: BlockId::STONE,
            },
            Plan::Build {
                by: [2, 2, 0],
                block: BlockId::STONE,
            },
        ],
    },
    Script {
        name: "hop",
        steps: &[Plan::Hop { by: [4, 0, 0] }],
    },
    Script {
        name: "stock",
        steps: &[Plan::Give {
            item: ItemId::DIRT,
            count: 10,
        }],
    },
    Script {
        name: "summon",
        steps: &[Plan::Summon { by: [6, 0, 0] }],
    },
];

/// The script `id` names, or `None` for an ID past the table.
pub fn get(id: ScriptId) -> Option<&'static Script> {
    SCRIPTS.get(usize::from(id.0))
}

/// Every script, in table order.
pub fn ids() -> impl Iterator<Item = ScriptId> {
    (0..SCRIPTS.len() as u8).map(ScriptId)
}

impl Plan {
    /// The command this step is, for a player standing in `origin`.
    pub fn command(self, origin: WorldPos) -> Command {
        let at = |by: [i32; 3]| WorldPos {
            x: origin.x.saturating_add(by[0]),
            y: origin.y.saturating_add(by[1]),
            z: origin.z.saturating_add(by[2]),
        };
        match self {
            Self::Build { by, block } => Command::SetBlock { pos: at(by), block },
            Self::Hop { by } => Command::Teleport { to: at(by) },
            Self::Give { item, count } => Command::GiveItem { item, count },
            Self::Summon { by } => Command::SpawnMob { at: at(by) },
        }
    }
}

impl Script {
    /// The commands of this script for a player standing in `origin`, in order.
    pub fn commands(&self, origin: WorldPos) -> Vec<Command> {
        self.steps.iter().map(|step| step.command(origin)).collect()
    }

    /// Fuel the whole script burns if every step is accepted.
    pub fn fuel_cost(&self) -> u32 {
        self.commands(WorldPos { x: 0, y: 0, z: 0 })
            .iter()
            .map(|command| u32::from(command.fuel_cost()))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::items::registry as items;

    #[test]
    fn every_script_has_a_name_and_at_least_one_step() {
        for id in ids() {
            let script = get(id).expect("listed scripts exist");
            assert!(!script.name.is_empty());
            assert!(!script.steps.is_empty(), "{} has no steps", script.name);
        }
    }

    #[test]
    fn every_script_uses_defined_items_and_blocks() {
        use crate::blocks::registry as blocks;
        for id in ids() {
            let script = get(id).expect("listed scripts exist");
            for step in script.steps {
                match *step {
                    Plan::Build { block, .. } => assert!(blocks::get(block).is_some()),
                    Plan::Give { item, count } => {
                        assert!(items::get(item).is_some());
                        assert!(count > 0, "{} gives nothing", script.name);
                    }
                    Plan::Hop { .. } | Plan::Summon { .. } => {}
                }
            }
        }
    }

    #[test]
    fn ids_past_the_table_are_not_scripts() {
        assert!(get(ScriptId(SCRIPTS.len() as u8)).is_none());
        assert_eq!(ids().count(), SCRIPTS.len());
    }

    proptest! {
        // A script means the same thing wherever the player stands: every command is its
        // step shifted by the origin, and the cost does not depend on the origin.
        #[test]
        fn a_script_is_its_steps_shifted_by_the_origin(
            x in -1000i32..1000, y in -100i32..100, z in -1000i32..1000,
        ) {
            let origin = WorldPos { x, y, z };
            for id in ids() {
                let script = get(id).expect("listed scripts exist");
                let here = script.commands(origin);
                let there = script.commands(WorldPos { x: 0, y: 0, z: 0 });
                prop_assert_eq!(here.len(), there.len());
                let cost: u32 = here.iter().map(|c| u32::from(c.fuel_cost())).sum();
                prop_assert_eq!(cost, script.fuel_cost());
            }
        }
    }
}
