//! Events: the only way the world changes. Lives in `shared` so `world` and `events`
//! can both use it without depending on each other.

use glam::Vec3;

use super::voxel::{BlockId, ChunkPos, WorldPos};

/// A position kept as the bit patterns of its three `f32`s.
///
/// Comparing bits instead of floats makes events `Eq`, and makes undo restore the exact
/// position the player stood at, not one that merely prints the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot([u32; 3]);

impl Spot {
    pub fn from_vec3(position: Vec3) -> Self {
        Self([
            position.x.to_bits(),
            position.y.to_bits(),
            position.z.to_bits(),
        ])
    }

    /// Rebuilds a position from its saved bits. Any bits are accepted, so the result can be
    /// NaN or infinite; the code that applies it must check.
    pub const fn from_bits(bits: [u32; 3]) -> Self {
        Self(bits)
    }

    pub const fn to_bits(self) -> [u32; 3] {
        self.0
    }

    pub fn to_vec3(self) -> Vec3 {
        Vec3::new(
            f32::from_bits(self.0[0]),
            f32::from_bits(self.0[1]),
            f32::from_bits(self.0[2]),
        )
    }
}

/// What one inventory slot holds. Item 0 with count 0 is an empty slot.
///
/// The item is a plain `u16` because `ItemId` lives in the `items` feature, which `shared`
/// cannot depend on; the item and count are checked against the item table when an event
/// holding this is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub item: u16,
    pub count: u8,
}

impl Held {
    pub const EMPTY: Self = Self { item: 0, count: 0 };
}

/// One change to the game. Each carries what it replaced, so it can be undone without a
/// snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `old` is recorded so the edit can be undone without a snapshot.
    BlockSet {
        pos: WorldPos,
        old: BlockId,
        new: BlockId,
    },
    /// The player's feet moved from `old` to `new` in one jump, not by walking.
    PlayerMoved { old: Spot, new: Spot },
    /// Inventory slot `slot` went from holding `old` to holding `new`. A change to several
    /// slots is several events, so undo restores which slot held what, not just the totals.
    SlotSet { slot: u8, old: Held, new: Held },
}

impl Event {
    /// The chunk this event changes, or `None` for an event that changes no blocks, which the
    /// per-chunk replay of a streaming chunk must not see.
    pub fn chunk(self) -> Option<ChunkPos> {
        match self {
            Self::BlockSet { pos, .. } => Some(pos.split().0),
            Self::PlayerMoved { .. } | Self::SlotSet { .. } => None,
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
            Self::PlayerMoved { old, new } => Self::PlayerMoved { old: new, new: old },
            Self::SlotSet { slot, old, new } => Self::SlotSet {
                slot,
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

    fn any_event() -> impl Strategy<Value = Event> {
        prop_oneof![
            (
                any::<i32>(),
                any::<i32>(),
                any::<i32>(),
                any::<u16>(),
                any::<u16>()
            )
                .prop_map(|(x, y, z, old, new)| Event::BlockSet {
                    pos: WorldPos { x, y, z },
                    old: BlockId(old),
                    new: BlockId(new),
                }),
            (any::<[u32; 3]>(), any::<[u32; 3]>()).prop_map(|(old, new)| Event::PlayerMoved {
                old: Spot::from_bits(old),
                new: Spot::from_bits(new),
            }),
            (
                any::<u8>(),
                (any::<u16>(), any::<u8>()),
                (any::<u16>(), any::<u8>())
            )
                .prop_map(|(slot, old, new)| Event::SlotSet {
                    slot,
                    old: Held {
                        item: old.0,
                        count: old.1
                    },
                    new: Held {
                        item: new.0,
                        count: new.1
                    },
                }),
        ]
    }

    proptest! {
        #[test]
        fn inverting_twice_is_the_identity(event in any_event()) {
            prop_assert_eq!(event.inverse().inverse(), event);
        }

        #[test]
        fn an_event_and_its_inverse_change_the_same_chunk(event in any_event()) {
            prop_assert_eq!(event.inverse().chunk(), event.chunk());
        }

        #[test]
        fn a_block_event_belongs_to_the_chunk_holding_its_block(
            x in any::<i32>(), y in any::<i32>(), z in any::<i32>(),
        ) {
            let pos = WorldPos { x, y, z };
            let event = Event::BlockSet { pos, old: BlockId(0), new: BlockId(1) };
            prop_assert_eq!(event.chunk(), Some(pos.split().0));
        }

        #[test]
        fn a_spot_keeps_every_bit_of_its_position(bits in any::<[u32; 3]>()) {
            prop_assert_eq!(Spot::from_bits(bits).to_bits(), bits);
        }

        #[test]
        fn a_spot_returns_the_exact_position_it_was_made_from(
            x in -1.0e6f32..1.0e6, y in -1.0e6f32..1.0e6, z in -1.0e6f32..1.0e6,
        ) {
            let position = Vec3::new(x, y, z);
            prop_assert_eq!(Spot::from_vec3(position).to_vec3(), position);
        }
    }

    #[test]
    fn only_block_events_have_a_chunk() {
        let spot = Spot::from_vec3(Vec3::ZERO);
        assert_eq!(
            Event::PlayerMoved {
                old: spot,
                new: spot
            }
            .chunk(),
            None
        );
        let slot = Event::SlotSet {
            slot: 0,
            old: Held::EMPTY,
            new: Held { item: 1, count: 1 },
        };
        assert_eq!(slot.chunk(), None);
    }
}
