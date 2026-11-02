//! The reads the window shows on demand: where the player faces, what is under the crosshair
//! and which mob is near.
//!
//! The readout is made of ordinary [`Query`]s sent through [`ask`], so what the player sees is
//! exactly what a script would be told. It changes and logs nothing, which is also why it is
//! safe to show during a replay.

use crate::blocks::registry as blocks;
use crate::events::apply::Stage;
use crate::scripting::model::{Answer, Denied, Facing, Limits, Permissions, Query};
use crate::scripting::service::ask;
use crate::shared::voxel::WorldPos;

/// One line describing what the player can read right now. `aimed` is the block under the
/// crosshair, if any; `facing` is the direction the camera points along.
pub fn readout(aimed: Option<WorldPos>, facing: Facing, stage: &Stage<'_>) -> String {
    let mut queries = vec![Query::Facing];
    queries.extend(aimed.map(|pos| Query::Look { pos }));
    queries.extend([Query::NearestMob, Query::MobsNear]);
    let answers = ask(
        &queries,
        Permissions::ALL,
        Limits::standard(),
        facing,
        stage,
    );
    queries
        .iter()
        .zip(&answers)
        .map(|(&query, answer)| phrase(query, answer))
        .collect::<Vec<_>>()
        .join(", ")
}

fn phrase(query: Query, answer: &Result<Answer, Denied>) -> String {
    match (query, answer) {
        (_, Err(denied)) => format!("{query:?} unavailable ({denied})"),
        (_, Ok(Answer::Direction(facing))) => format!("facing {}", direction_name(*facing)),
        (_, Ok(Answer::Block(block))) => format!(
            "looking at {}",
            blocks::get(*block).map_or("an unknown block", |def| def.name)
        ),
        (_, Ok(Answer::Found(Some(at)))) => format!("nearest mob at {} {} {}", at.x, at.y, at.z),
        (_, Ok(Answer::Found(None))) => "no mob in range".to_owned(),
        (_, Ok(Answer::Number(count))) => format!("{count} mobs near"),
    }
}

const fn direction_name(facing: Facing) -> &'static str {
    match facing {
        Facing::North => "north",
        Facing::East => "east",
        Facing::South => "south",
        Facing::West => "west",
        Facing::Up => "up",
        Facing::Down => "down",
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use proptest::prelude::*;

    use super::*;
    use crate::inventory::model::Inventory;
    use crate::mobs::model::Mobs;
    use crate::player::model::Player;
    use crate::shared::event::Event;
    use crate::shared::voxel::BlockId;
    use crate::survival::model::Stats;
    use crate::world::model::World;

    fn cell(x: i32, y: i32, z: i32) -> WorldPos {
        WorldPos { x, y, z }
    }

    /// Reads from a player at the origin of a world holding one stone block at (2, 1, 0).
    fn read(aimed: Option<WorldPos>, facing: Facing) -> String {
        let mut world = World::default();
        let stone = Event::BlockSet {
            pos: cell(2, 1, 0),
            old: BlockId::AIR,
            new: BlockId::STONE,
        };
        assert_eq!(world.apply(&stone), Ok(()));
        let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).expect("a valid spawn");
        let (mut inventory, mut stats, mut mobs) =
            (Inventory::default(), Stats::default(), Mobs::default());
        let stage = Stage {
            world: &mut world,
            player: &mut player,
            inventory: &mut inventory,
            stats: &mut stats,
            mobs: &mut mobs,
        };
        readout(aimed, facing, &stage)
    }

    #[test]
    fn the_readout_names_the_facing_the_block_and_the_mobs() {
        assert_eq!(
            read(Some(cell(2, 1, 0)), Facing::East),
            "facing east, looking at stone, no mob in range, 0 mobs near"
        );
    }

    #[test]
    fn nothing_under_the_crosshair_leaves_that_part_out() {
        assert_eq!(
            read(None, Facing::Up),
            "facing up, no mob in range, 0 mobs near"
        );
    }

    #[test]
    fn a_block_beyond_range_is_reported_as_unavailable() {
        let line = read(Some(cell(500, 1, 0)), Facing::North);
        assert!(line.contains("unavailable"), "{line}");
    }

    proptest! {
        #[test]
        fn the_readout_always_starts_with_the_facing_and_never_panics(
            x in -100i32..100, y in -100i32..100, z in -100i32..100,
            aimed in any::<bool>(),
            view in (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0),
        ) {
            let facing = Facing::from_view(Vec3::new(view.0, view.1, view.2))
                .unwrap_or(Facing::North);
            let line = read(aimed.then(|| cell(x, y, z)), facing);
            prop_assert!(line.starts_with("facing "), "{}", line);
        }
    }
}
