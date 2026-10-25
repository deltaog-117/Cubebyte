//! Building rules; routes edits through the event log.
//!
//! Every edit becomes an [`Event::BlockSet`] that is applied to the world and then logged,
//! in that order, so the log never holds an edit the world rejected.

use glam::Vec3;

use crate::building::model::{EditError, SolidBlock};
use crate::building::raycast::cast;
use crate::events::log::EventLog;
use crate::shared::event::Event;
use crate::shared::voxel::{BlockId, WorldPos};
use crate::world::model::World;

/// How far the viewer can reach, in blocks.
pub const REACH: f32 = 5.0;

/// Breaks the block the viewer looks at and returns where it was.
///
/// # Errors
///
/// Returns [`EditError::NothingInReach`] when no block is within [`REACH`]. The world and
/// the log are left unchanged on any error.
pub fn break_block(
    world: &mut World,
    log: &mut EventLog,
    origin: Vec3,
    direction: Vec3,
) -> Result<WorldPos, EditError> {
    let hit = cast(world, origin, direction, REACH).ok_or(EditError::NothingInReach)?;
    set_block(world, log, hit.block, BlockId::AIR)?;
    Ok(hit.block)
}

/// Places `block` against the face the viewer looks at and returns where it went.
///
/// # Errors
///
/// - [`EditError::NothingInReach`] when no block is within [`REACH`].
/// - [`EditError::NoFace`] when the viewer is inside the block it would build against.
/// - [`EditError::Occupied`] when the new block would fill the cell holding the viewer's
///   eye.
///
/// The world and the log are left unchanged on any error.
pub fn place_block(
    world: &mut World,
    log: &mut EventLog,
    origin: Vec3,
    direction: Vec3,
    block: SolidBlock,
) -> Result<WorldPos, EditError> {
    let hit = cast(world, origin, direction, REACH).ok_or(EditError::NothingInReach)?;
    let target = hit.before.ok_or(EditError::NoFace)?;
    // A cast that returned a hit has a finite origin within the cast's coordinate limit.
    let eye = origin.floor().as_ivec3();
    if (target.x, target.y, target.z) == (eye.x, eye.y, eye.z) {
        return Err(EditError::Occupied);
    }
    set_block(world, log, target, block.id())?;
    Ok(target)
}

fn set_block(
    world: &mut World,
    log: &mut EventLog,
    pos: WorldPos,
    new: BlockId,
) -> Result<(), EditError> {
    let event = Event::BlockSet {
        pos,
        old: world.block_at(pos),
        new,
    };
    world.apply(&event)?;
    log.push(event);
    tracing::debug!(?event, logged = log.len(), "block edit applied");
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::building::model::NotSolid;

    const EYE: Vec3 = Vec3::new(0.5, 0.5, 0.5);

    fn stone_at(world: &mut World, x: i32, y: i32, z: i32) {
        let pos = WorldPos { x, y, z };
        let event = Event::BlockSet {
            pos,
            old: world.block_at(pos),
            new: BlockId::STONE,
        };
        world.apply(&event).unwrap();
    }

    #[test]
    fn breaking_clears_the_block_and_logs_how_to_undo_it() {
        let mut world = World::default();
        let mut log = EventLog::new();
        stone_at(&mut world, 3, 0, 0);

        let broken = break_block(&mut world, &mut log, EYE, Vec3::X).unwrap();

        let pos = WorldPos { x: 3, y: 0, z: 0 };
        assert_eq!(broken, pos);
        assert_eq!(world.block_at(pos), BlockId::AIR);
        let logged: Vec<_> = log.iter().copied().collect();
        assert_eq!(
            logged,
            [Event::BlockSet {
                pos,
                old: BlockId::STONE,
                new: BlockId::AIR
            }]
        );
    }

    #[test]
    fn placing_builds_on_the_face_that_was_hit() {
        let mut world = World::default();
        let mut log = EventLog::new();
        stone_at(&mut world, 3, 0, 0);

        let placed = place_block(&mut world, &mut log, EYE, Vec3::X, SolidBlock::DIRT).unwrap();

        let pos = WorldPos { x: 2, y: 0, z: 0 };
        assert_eq!(placed, pos);
        assert_eq!(world.block_at(pos), BlockId::DIRT);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn looking_at_nothing_changes_nothing() {
        let mut world = World::default();
        let mut log = EventLog::new();
        stone_at(&mut world, 30, 0, 0);
        let before = world.clone();

        assert_eq!(
            break_block(&mut world, &mut log, EYE, Vec3::X),
            Err(EditError::NothingInReach)
        );
        assert_eq!(
            place_block(&mut world, &mut log, EYE, Vec3::X, SolidBlock::STONE),
            Err(EditError::NothingInReach)
        );
        assert_eq!(world, before);
        assert!(log.is_empty());
    }

    #[test]
    fn a_block_cannot_be_built_inside_the_viewers_own_cell() {
        let mut world = World::default();
        let mut log = EventLog::new();
        stone_at(&mut world, 1, 0, 0);

        // The eye is in cell 0, directly against the block, so the free cell is the eye's.
        let result = place_block(
            &mut world,
            &mut log,
            Vec3::new(0.9, 0.5, 0.5),
            Vec3::X,
            SolidBlock::STONE,
        );

        assert_eq!(result, Err(EditError::Occupied));
        assert!(log.is_empty());
    }

    #[test]
    fn nothing_can_be_built_from_inside_a_solid_block() {
        let mut world = World::default();
        let mut log = EventLog::new();
        stone_at(&mut world, 0, 0, 0);

        let result = place_block(&mut world, &mut log, EYE, Vec3::X, SolidBlock::STONE);

        assert_eq!(result, Err(EditError::NoFace));
        assert!(log.is_empty());
    }

    #[test]
    fn air_and_undefined_ids_are_not_solid_blocks() {
        assert_eq!(SolidBlock::try_new(BlockId::AIR), Err(NotSolid));
        assert_eq!(SolidBlock::try_new(BlockId(999)), Err(NotSolid));
        assert_eq!(SolidBlock::try_new(BlockId::STONE), Ok(SolidBlock::STONE));
    }

    proptest! {
        #[test]
        fn any_run_of_edits_replays_and_undoes_exactly(
            edits in prop::collection::vec(
                (any::<bool>(), (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0)),
                0..60,
            ),
        ) {
            let mut start = World::default();
            for x in -3..=3 {
                for y in -3..=3 {
                    for z in -3..=3 {
                        stone_at(&mut start, x, y, z);
                    }
                }
            }
            // The eye starts inside the block, so the first breaks open room to build in.
            let mut log = EventLog::new();
            let mut live = start.clone();
            for (placing, (x, y, z)) in edits {
                let direction = Vec3::new(x, y, z);
                let _ = if placing {
                    place_block(&mut live, &mut log, EYE, direction, SolidBlock::GRASS)
                } else {
                    break_block(&mut live, &mut log, EYE, direction)
                };
            }

            let mut replayed = start.clone();
            for event in log.iter() {
                prop_assert_eq!(replayed.apply(event), Ok(()));
            }
            prop_assert_eq!(&replayed, &live);

            let events: Vec<_> = log.iter().copied().collect();
            for event in events.iter().rev() {
                prop_assert_eq!(live.apply(&event.inverse()), Ok(()));
            }
            prop_assert_eq!(live, start);
        }
    }
}
