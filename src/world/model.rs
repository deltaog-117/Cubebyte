//! World and chunk data held by the world feature.

use std::collections::HashMap;

use thiserror::Error;

use crate::shared::event::Event;
use crate::shared::voxel::{BlockId, CHUNK_VOLUME, ChunkPos, LocalPos, WorldPos};

/// A cube of blocks. Stored flat; `LocalPos` guarantees every index is in bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    blocks: Box<[BlockId; CHUNK_VOLUME]>,
    // Kept in step with `blocks` so an all-air chunk is detected in O(1).
    solid_count: usize,
}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            blocks: Box::new([BlockId::AIR; CHUNK_VOLUME]),
            solid_count: 0,
        }
    }
}

impl Chunk {
    pub fn get(&self, pos: LocalPos) -> BlockId {
        self.blocks[pos.index()]
    }

    /// Sets a block and returns the one it replaced.
    pub fn set(&mut self, pos: LocalPos, block: BlockId) -> BlockId {
        let previous = std::mem::replace(&mut self.blocks[pos.index()], block);
        match (previous.is_air(), block.is_air()) {
            (true, false) => self.solid_count += 1,
            (false, true) => self.solid_count -= 1,
            _ => {}
        }
        previous
    }

    pub fn is_empty(&self) -> bool {
        self.solid_count == 0
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplyError {
    /// The world no longer matches what the event was recorded against.
    #[error("stale event at {pos:?}: expected {expected:?} but found {found:?}")]
    StaleEvent {
        pos: WorldPos,
        expected: BlockId,
        found: BlockId,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InsertError {
    /// Inserting would overwrite blocks that may carry player edits.
    #[error("chunk {pos:?} is already loaded")]
    AlreadyLoaded { pos: ChunkPos },
}

/// The world's blocks. Player edits change it only through [`World::apply`].
///
/// Generated terrain is the baseline those events are recorded against. It is a pure
/// function of the seed, so it enters through [`World::insert_generated`] and stays out
/// of the event log.
///
/// All-air chunks are never stored, so two worlds with the same blocks compare equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct World {
    chunks: HashMap<ChunkPos, Chunk>,
}

impl World {
    /// Air where no chunk is loaded.
    pub fn block_at(&self, pos: WorldPos) -> BlockId {
        let (chunk_pos, local) = pos.split();
        self.chunks
            .get(&chunk_pos)
            .map_or(BlockId::AIR, |chunk| chunk.get(local))
    }

    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Installs a freshly generated chunk as the baseline for `pos`.
    ///
    /// An all-air chunk is accepted but not stored.
    ///
    /// # Errors
    ///
    /// Returns [`InsertError::AlreadyLoaded`] if a chunk is stored at `pos`, so generation
    /// can never overwrite edited blocks. The world is left unchanged.
    pub fn insert_generated(&mut self, pos: ChunkPos, chunk: Chunk) -> Result<(), InsertError> {
        if self.chunks.contains_key(&pos) {
            return Err(InsertError::AlreadyLoaded { pos });
        }
        if !chunk.is_empty() {
            self.chunks.insert(pos, chunk);
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`ApplyError::StaleEvent`] if the block at the event's position is not the
    /// `old` block it was recorded against. The world is left unchanged.
    pub fn apply(&mut self, event: &Event) -> Result<(), ApplyError> {
        match *event {
            Event::BlockSet { pos, old, new } => {
                let found = self.block_at(pos);
                if found != old {
                    tracing::warn!(?pos, ?old, ?found, "stale block event rejected");
                    return Err(ApplyError::StaleEvent {
                        pos,
                        expected: old,
                        found,
                    });
                }
                if old == new {
                    return Ok(());
                }
                let (chunk_pos, local) = pos.split();
                let chunk = self.chunks.entry(chunk_pos).or_default();
                chunk.set(local, new);
                if chunk.is_empty() {
                    self.chunks.remove(&chunk_pos);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const STONE: BlockId = BlockId(1);

    fn local(index: usize) -> LocalPos {
        let pos = WorldPos {
            x: (index % 16) as i32,
            y: (index / 256) as i32,
            z: ((index / 16) % 16) as i32,
        };
        pos.split().1
    }

    fn block() -> impl Strategy<Value = BlockId> {
        (0u16..4).prop_map(BlockId)
    }

    fn near_origin() -> impl Strategy<Value = WorldPos> {
        // Spans several chunks on every axis, including negative ones.
        (-20i32..20, -20i32..20, -20i32..20).prop_map(|(x, y, z)| WorldPos { x, y, z })
    }

    fn set_event(world: &World, pos: WorldPos, new: BlockId) -> Event {
        Event::BlockSet {
            pos,
            old: world.block_at(pos),
            new,
        }
    }

    proptest! {
        #[test]
        fn chunk_set_then_get_returns_the_block(i in 0..CHUNK_VOLUME, block in block()) {
            let mut chunk = Chunk::default();
            chunk.set(local(i), block);
            prop_assert_eq!(chunk.get(local(i)), block);
        }

        #[test]
        fn chunk_set_leaves_every_other_cell_untouched(
            i in 0..CHUNK_VOLUME, j in 0..CHUNK_VOLUME,
        ) {
            prop_assume!(i != j);
            let mut chunk = Chunk::default();
            chunk.set(local(i), STONE);
            prop_assert_eq!(chunk.get(local(j)), BlockId::AIR);
        }

        #[test]
        fn chunk_solid_count_matches_its_contents(
            edits in prop::collection::vec((0..CHUNK_VOLUME, block()), 0..200),
        ) {
            let mut chunk = Chunk::default();
            for (i, block) in edits {
                chunk.set(local(i), block);
            }
            let actual = chunk.blocks.iter().filter(|b| !b.is_air()).count();
            prop_assert_eq!(chunk.solid_count, actual);
        }

        #[test]
        fn undoing_every_event_in_reverse_restores_the_empty_world(
            edits in prop::collection::vec((near_origin(), block()), 0..100),
        ) {
            let mut world = World::default();
            let mut applied = Vec::new();
            for (pos, block) in edits {
                let event = set_event(&world, pos, block);
                prop_assert_eq!(world.apply(&event), Ok(()));
                applied.push(event);
            }
            for event in applied.iter().rev() {
                prop_assert_eq!(world.apply(&event.inverse()), Ok(()));
            }
            prop_assert_eq!(world, World::default());
        }
    }

    #[test]
    fn applied_block_is_visible_across_a_negative_chunk_border() {
        let mut world = World::default();
        let pos = WorldPos { x: -1, y: 0, z: 16 };
        let event = set_event(&world, pos, STONE);
        assert_eq!(world.apply(&event), Ok(()));
        assert_eq!(world.block_at(pos), STONE);
        assert_eq!(world.block_at(WorldPos { x: 0, y: 0, z: 16 }), BlockId::AIR);
        assert_eq!(world.chunk_count(), 1);
    }

    #[test]
    fn stale_event_is_rejected_and_leaves_the_world_unchanged() {
        let mut world = World::default();
        let pos = WorldPos { x: 3, y: 4, z: 5 };
        let stale = Event::BlockSet {
            pos,
            old: STONE,
            new: BlockId(2),
        };
        let before = world.clone();
        assert_eq!(
            world.apply(&stale),
            Err(ApplyError::StaleEvent {
                pos,
                expected: STONE,
                found: BlockId::AIR,
            })
        );
        assert_eq!(world, before);
    }

    #[test]
    fn event_that_changes_nothing_creates_no_chunk() {
        let mut world = World::default();
        let event = set_event(&world, WorldPos { x: 0, y: 0, z: 0 }, BlockId::AIR);
        assert_eq!(world.apply(&event), Ok(()));
        assert_eq!(world.chunk_count(), 0);
    }

    fn stone_chunk() -> Chunk {
        let mut chunk = Chunk::default();
        chunk.set(local(0), STONE);
        chunk
    }

    #[test]
    fn generated_chunk_is_readable_and_editable_through_events() {
        let mut world = World::default();
        let chunk_pos = ChunkPos { x: -1, y: 0, z: 2 };
        assert_eq!(world.insert_generated(chunk_pos, stone_chunk()), Ok(()));

        let pos = WorldPos {
            x: -16,
            y: 0,
            z: 32,
        };
        assert_eq!(world.block_at(pos), STONE);

        let dig = set_event(&world, pos, BlockId::AIR);
        assert_eq!(world.apply(&dig), Ok(()));
        assert_eq!(world.block_at(pos), BlockId::AIR);
    }

    #[test]
    fn insert_generated_refuses_to_overwrite_a_loaded_chunk() {
        let mut world = World::default();
        let chunk_pos = ChunkPos { x: 0, y: 0, z: 0 };
        assert_eq!(world.insert_generated(chunk_pos, stone_chunk()), Ok(()));
        let before = world.clone();
        assert_eq!(
            world.insert_generated(chunk_pos, Chunk::default()),
            Err(InsertError::AlreadyLoaded { pos: chunk_pos })
        );
        assert_eq!(world, before);
    }

    #[test]
    fn insert_generated_does_not_store_an_all_air_chunk() {
        let mut world = World::default();
        let chunk_pos = ChunkPos { x: 3, y: 3, z: 3 };
        assert_eq!(world.insert_generated(chunk_pos, Chunk::default()), Ok(()));
        assert_eq!(world.chunk_count(), 0);
    }

    #[test]
    fn clearing_the_last_block_drops_the_chunk() {
        let mut world = World::default();
        let pos = WorldPos { x: 0, y: 0, z: 0 };
        let place = set_event(&world, pos, STONE);
        assert_eq!(world.apply(&place), Ok(()));
        assert!(world.chunk(pos.split().0).is_some());
        assert_eq!(world.apply(&place.inverse()), Ok(()));
        assert!(world.chunk(pos.split().0).is_none());
    }
}
