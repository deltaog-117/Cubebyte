//! Chiselling: chipping a tiny cube off a block, or putting one back.
//!
//! A block is cut into 4×4×4 tiny cubes (see [`crate::blocks::mould`]). A chisel edit changes
//! one tiny cube and is one ordinary block change, from the block's ID to the ID of the new
//! cut, so it is logged, rewound and replayed like any other edit. Planning an edit may add the
//! new cut to the world's table of moulded blocks, which only ever grows.

use glam::Vec3;
use thiserror::Error;

use crate::blocks::mould::{MouldError, SIDE, bit};
use crate::building::raycast::cast_fine;
use crate::building::service::REACH;
use crate::shared::event::Event;
use crate::shared::voxel::WorldPos;
use crate::world::model::World;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChiselError {
    #[error("there is no block there to chisel")]
    NothingThere,
    #[error("that tiny cube is already gone")]
    AlreadyGone,
    #[error("that tiny cube is already there")]
    AlreadyThere,
    #[error(transparent)]
    Mould(#[from] MouldError),
}

/// Where the viewer's chisel points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aim {
    /// The solid tiny cube the ray hits: the one a chip removes.
    pub chip: WorldPos,
    /// The empty tiny cube in front of it, which a fill puts back. `None` when it lies in
    /// another block, since a chisel only reshapes the block it is cutting.
    pub fill: Option<WorldPos>,
}

/// The block a tiny cube belongs to and where it sits inside it, each in `0..SIDE`.
pub fn split(cell: WorldPos) -> (WorldPos, [i32; 3]) {
    let block = WorldPos {
        x: cell.x.div_euclid(SIDE),
        y: cell.y.div_euclid(SIDE),
        z: cell.z.div_euclid(SIDE),
    };
    (
        block,
        [cell.x, cell.y, cell.z].map(|axis| axis.rem_euclid(SIDE)),
    )
}

/// The tiny cubes the viewer's ray reaches, or `None` when it reaches none.
pub fn aim(world: &World, origin: Vec3, direction: Vec3) -> Option<Aim> {
    let hit = cast_fine(world, origin, direction, REACH)?;
    let block = split(hit.cell).0;
    Some(Aim {
        chip: hit.cell,
        fill: hit.before.filter(|&before| split(before).0 == block),
    })
}

/// The one event that chips the tiny cube at `cell` off (`add` false) or puts it back (`add`
/// true). Nothing is applied.
///
/// # Errors
///
/// - [`ChiselError::NothingThere`] when the block is air or unknown.
/// - [`ChiselError::AlreadyGone`] or [`ChiselError::AlreadyThere`] when the tiny cube is
///   already as asked.
/// - [`ChiselError::Mould`] when the new cut cannot be added to the table.
pub fn chisel_events(
    world: &mut World,
    cell: WorldPos,
    add: bool,
) -> Result<Vec<Event>, ChiselError> {
    let (pos, [x, y, z]) = split(cell);
    let old = world.block_at(pos);
    let bits = world.moulds().bits_of(old);
    if bits == 0 {
        return Err(ChiselError::NothingThere);
    }
    // `split` keeps every axis inside the block, so the tiny cube always has a bit.
    let tiny = bit(x, y, z).ok_or(ChiselError::NothingThere)?;
    let new_bits = match (add, bits & tiny != 0) {
        (true, true) => return Err(ChiselError::AlreadyThere),
        (false, false) => return Err(ChiselError::AlreadyGone),
        (true, false) => bits | tiny,
        (false, true) => bits & !tiny,
    };
    let base = world.moulds().base_of(old);
    let new = world.moulds_mut().id_for(base, new_bits)?;
    Ok(vec![Event::BlockSet { pos, old, new }])
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::voxel::BlockId;

    fn world_with_stone() -> World {
        let mut world = World::default();
        let pos = WorldPos { x: 2, y: 3, z: 4 };
        let event = Event::BlockSet {
            pos,
            old: BlockId::AIR,
            new: BlockId::STONE,
        };
        world.apply(&event).unwrap();
        world
    }

    /// The first tiny cube of the block at (2, 3, 4).
    const CORNER: WorldPos = WorldPos { x: 8, y: 12, z: 16 };

    fn apply(world: &mut World, events: &[Event]) {
        for event in events {
            world.apply(event).unwrap();
        }
    }

    #[test]
    fn a_tiny_cube_belongs_to_the_block_around_it() {
        assert_eq!(
            split(WorldPos { x: -1, y: 0, z: 7 }),
            (WorldPos { x: -1, y: 0, z: 1 }, [3, 0, 3])
        );
        assert_eq!(split(CORNER), (WorldPos { x: 2, y: 3, z: 4 }, [0, 0, 0]));
    }

    #[test]
    fn chipping_one_cube_moulds_the_block_and_filling_it_back_makes_it_whole() {
        let mut world = world_with_stone();
        let pos = WorldPos { x: 2, y: 3, z: 4 };
        let chip = chisel_events(&mut world, CORNER, false).unwrap();
        apply(&mut world, &chip);
        assert!(world.block_at(pos).0 >= crate::blocks::mould::MOULD_FIRST);
        assert!(!world.fine_solid([8, 12, 16]));
        assert!(world.fine_solid([9, 12, 16]));
        assert_eq!(world.base_block_at(pos), BlockId::STONE);
        let fill = chisel_events(&mut world, CORNER, true).unwrap();
        apply(&mut world, &fill);
        assert_eq!(world.block_at(pos), BlockId::STONE);
    }

    #[test]
    fn chipping_every_cube_leaves_air() {
        let mut world = world_with_stone();
        for x in 8..12 {
            for y in 12..16 {
                for z in 16..20 {
                    let cell = WorldPos { x, y, z };
                    let events = chisel_events(&mut world, cell, false).unwrap();
                    apply(&mut world, &events);
                }
            }
        }
        assert_eq!(world.block_at(WorldPos { x: 2, y: 3, z: 4 }), BlockId::AIR);
    }

    #[test]
    fn refused_edits_say_why() {
        let mut world = world_with_stone();
        assert_eq!(
            chisel_events(&mut world, CORNER, true),
            Err(ChiselError::AlreadyThere)
        );
        let far = WorldPos { x: 100, y: 0, z: 0 };
        assert_eq!(
            chisel_events(&mut world, far, false),
            Err(ChiselError::NothingThere)
        );
        let chip = chisel_events(&mut world, CORNER, false).unwrap();
        apply(&mut world, &chip);
        assert_eq!(
            chisel_events(&mut world, CORNER, false),
            Err(ChiselError::AlreadyGone)
        );
    }

    #[test]
    fn the_aim_chips_what_it_hits_and_fills_only_inside_the_block() {
        let mut world = world_with_stone();
        // Stand west of the block and look east through its middle.
        let origin = Vec3::new(0.5, 3.6, 4.6);
        let found = aim(&world, origin, Vec3::X).unwrap();
        assert_eq!(split(found.chip).0, WorldPos { x: 2, y: 3, z: 4 });
        assert_eq!(
            found.fill, None,
            "the empty cube in front is in another block"
        );
        let chip = chisel_events(&mut world, found.chip, false).unwrap();
        apply(&mut world, &chip);
        let deeper = aim(&world, origin, Vec3::X).unwrap();
        assert_eq!(deeper.chip.x, found.chip.x + 1);
        assert_eq!(deeper.fill, Some(found.chip));
    }

    proptest! {
        #[test]
        fn any_run_of_chisel_edits_undoes_exactly(
            edits in proptest::collection::vec((0i32..4, 0i32..4, 0i32..4, any::<bool>()), 1..40),
        ) {
            let mut world = world_with_stone();
            let pos = WorldPos { x: 2, y: 3, z: 4 };
            let start = world.block_at(pos);
            let mut applied = Vec::new();
            for (x, y, z, add) in edits {
                let cell = WorldPos { x: 8 + x, y: 12 + y, z: 16 + z };
                if let Ok(events) = chisel_events(&mut world, cell, add) {
                    apply(&mut world, &events);
                    applied.extend(events);
                }
            }
            for event in applied.into_iter().rev() {
                world.apply(&event.inverse()).unwrap();
            }
            prop_assert_eq!(world.block_at(pos), start);
        }
    }
}
