//! Events: the only way the world changes. Lives in `shared` so `world` and `events`
//! can both use it without depending on each other.

use super::voxel::{BlockId, ChunkPos, WorldPos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `old` is recorded so the edit can be undone without a snapshot.
    BlockSet {
        pos: WorldPos,
        old: BlockId,
        new: BlockId,
    },
}

impl Event {
    /// The chunk this event changes.
    pub fn chunk(self) -> ChunkPos {
        match self {
            Self::BlockSet { pos, .. } => pos.split().0,
        }
    }

    /// The event that undoes this one.
    pub fn inverse(self) -> Self {
        match self {
            Self::BlockSet { pos, old, new } => Self::BlockSet {
                pos,
                old: new,
                new: old,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn inverting_twice_is_the_identity(
            x in any::<i32>(), y in any::<i32>(), z in any::<i32>(),
            old in any::<u16>(), new in any::<u16>(),
        ) {
            let event = Event::BlockSet {
                pos: WorldPos { x, y, z },
                old: BlockId(old),
                new: BlockId(new),
            };
            prop_assert_eq!(event.inverse().inverse(), event);
        }

        #[test]
        fn an_event_and_its_inverse_change_the_same_chunk(
            x in any::<i32>(), y in any::<i32>(), z in any::<i32>(),
        ) {
            let pos = WorldPos { x, y, z };
            let event = Event::BlockSet { pos, old: BlockId(0), new: BlockId(1) };
            prop_assert_eq!(event.chunk(), pos.split().0);
            prop_assert_eq!(event.inverse().chunk(), event.chunk());
        }
    }
}
