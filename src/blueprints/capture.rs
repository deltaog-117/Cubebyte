//! Capturing the blocks around the player as a program that builds them again.

use thiserror::Error;

use crate::blocks::registry;
use crate::scripting::program::{Block, MAX_BLOCKS, Plan, Program};
use crate::shared::voxel::{BlockId, WorldPos};
use crate::world::model::World;

/// How far a capture reaches sideways from the feet, in blocks.
pub const RADIUS: i32 = 4;

/// How far a capture reaches up from the feet, in blocks. The block under the feet is the
/// ground and is never captured.
pub const HEIGHT: i32 = 8;

/// Why nothing was captured.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("no blocks within {RADIUS} blocks of you to copy")]
    Nothing,
    #[error("{found} blocks is too many to copy; a build holds at most {MAX_BLOCKS}")]
    TooMany { found: usize },
}

/// The blocks around `feet` (see [`RADIUS`] and [`HEIGHT`]) as a program that places them,
/// lowest layer first, each at its offset from the feet.
///
/// # Errors
///
/// Returns [`CaptureError::Nothing`] for an empty area and [`CaptureError::TooMany`] for more
/// than [`MAX_BLOCKS`] blocks.
pub fn capture(world: &World, feet: WorldPos) -> Result<Program, CaptureError> {
    let found: Vec<([i32; 3], BlockId)> = (0..HEIGHT)
        .flat_map(|y| {
            (-RADIUS..=RADIUS).flat_map(move |x| (-RADIUS..=RADIUS).map(move |z| [x, y, z]))
        })
        .filter_map(|by| {
            let block = world.block_at(WorldPos {
                x: feet.x + by[0],
                y: feet.y + by[1],
                z: feet.z + by[2],
            });
            (block != BlockId::AIR && registry::get(block).is_some()).then_some((by, block))
        })
        .collect();
    if found.len() > MAX_BLOCKS {
        return Err(CaptureError::TooMany { found: found.len() });
    }
    let blocks = found
        .into_iter()
        .map(|(by, block)| Block::Do(Plan::Build { by, block }))
        .collect();
    // The count was checked above, so only an empty area can fail here.
    Program::try_new(blocks).map_err(|_| CaptureError::Nothing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::event::Event;

    fn place(world: &mut World, x: i32, y: i32, z: i32, block: BlockId) {
        let pos = WorldPos { x, y, z };
        let event = Event::BlockSet {
            pos,
            old: world.block_at(pos),
            new: block,
        };
        world.apply(&event).unwrap();
    }

    const FEET: WorldPos = WorldPos { x: 0, y: 5, z: 0 };

    #[test]
    fn an_empty_area_captures_nothing() {
        assert_eq!(capture(&World::default(), FEET), Err(CaptureError::Nothing));
    }

    #[test]
    fn blocks_are_kept_at_their_offsets_lowest_first() {
        let mut world = World::default();
        place(&mut world, 1, 6, 2, BlockId::DIRT);
        place(&mut world, -1, 5, 0, BlockId::STONE);
        let program = capture(&world, FEET).unwrap();
        assert_eq!(
            program.blocks(),
            [
                Block::Do(Plan::Build {
                    by: [-1, 0, 0],
                    block: BlockId::STONE
                }),
                Block::Do(Plan::Build {
                    by: [1, 1, 2],
                    block: BlockId::DIRT
                }),
            ]
        );
    }

    #[test]
    fn the_ground_under_the_feet_is_left_out() {
        let mut world = World::default();
        place(&mut world, 0, 4, 0, BlockId::GRASS);
        assert_eq!(capture(&world, FEET), Err(CaptureError::Nothing));
    }

    #[test]
    fn more_than_the_limit_is_refused_with_the_count() {
        let mut world = World::default();
        for x in -RADIUS..=RADIUS {
            for z in -RADIUS..=RADIUS {
                place(&mut world, x, 5, z, BlockId::STONE);
            }
        }
        assert_eq!(
            capture(&world, FEET),
            Err(CaptureError::TooMany { found: 81 })
        );
    }
}
