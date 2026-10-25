//! Block, chunk and coordinate types.

use thiserror::Error;

/// Edge length of a cubic chunk, in blocks.
pub const CHUNK_SIZE: u8 = 16;

/// Number of blocks in one chunk.
pub const CHUNK_VOLUME: usize = (CHUNK_SIZE as usize).pow(3);

/// Identifies a block type. ID 0 is air, so a zeroed chunk is an empty chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockId(pub u16);

impl BlockId {
    pub const AIR: Self = Self(0);
    // Fixed IDs until the block registry exists; saved worlds depend on them.
    pub const STONE: Self = Self(1);
    pub const DIRT: Self = Self(2);
    pub const GRASS: Self = Self(3);

    pub const fn is_air(self) -> bool {
        self.0 == 0
    }
}

/// Position of a chunk in chunk units. Every coordinate triple is valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Position of a block in the world. Every coordinate triple is valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Position of a block inside one chunk. Each axis is always in `0..CHUNK_SIZE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalPos {
    x: u8,
    y: u8,
    z: u8,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("local position ({x}, {y}, {z}) is outside 0..{}", CHUNK_SIZE)]
pub struct LocalPosError {
    x: u8,
    y: u8,
    z: u8,
}

impl LocalPos {
    /// # Errors
    ///
    /// Returns [`LocalPosError`] if any axis is not below [`CHUNK_SIZE`].
    pub fn try_new(x: u8, y: u8, z: u8) -> Result<Self, LocalPosError> {
        if x < CHUNK_SIZE && y < CHUNK_SIZE && z < CHUNK_SIZE {
            Ok(Self { x, y, z })
        } else {
            Err(LocalPosError { x, y, z })
        }
    }

    pub const fn x(self) -> u8 {
        self.x
    }

    pub const fn y(self) -> u8 {
        self.y
    }

    pub const fn z(self) -> u8 {
        self.z
    }

    /// Every position in a chunk, in increasing [`LocalPos::index`] order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..CHUNK_SIZE).flat_map(|y| {
            (0..CHUNK_SIZE).flat_map(move |z| {
                // Each axis is below `CHUNK_SIZE` by construction, so `ok()` never drops one.
                (0..CHUNK_SIZE).filter_map(move |x| Self::try_new(x, y, z).ok())
            })
        })
    }

    /// Index into a chunk's flat block array. Always below [`CHUNK_VOLUME`].
    pub fn index(self) -> usize {
        let size = usize::from(CHUNK_SIZE);
        (usize::from(self.y) * size + usize::from(self.z)) * size + usize::from(self.x)
    }
}

impl WorldPos {
    /// Splits into the owning chunk and the position inside it.
    ///
    /// Uses floor division so negative coordinates land in the correct chunk
    /// (`-1` is the last block of chunk `-1`, not the first block of chunk `0`).
    pub fn split(self) -> (ChunkPos, LocalPos) {
        let size = i32::from(CHUNK_SIZE);
        let chunk = ChunkPos {
            x: self.x.div_euclid(size),
            y: self.y.div_euclid(size),
            z: self.z.div_euclid(size),
        };
        let local = LocalPos {
            x: local_axis(self.x),
            y: local_axis(self.y),
            z: local_axis(self.z),
        };
        (chunk, local)
    }
}

fn local_axis(value: i32) -> u8 {
    // rem_euclid by CHUNK_SIZE is always in 0..CHUNK_SIZE, so the cast cannot truncate.
    value.rem_euclid(i32::from(CHUNK_SIZE)) as u8
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use proptest::prelude::*;

    use super::*;

    fn world_pos() -> impl Strategy<Value = WorldPos> {
        (any::<i32>(), any::<i32>(), any::<i32>()).prop_map(|(x, y, z)| WorldPos { x, y, z })
    }

    proptest! {
        #[test]
        fn local_pos_accepts_exactly_the_in_range_coordinates(
            x in any::<u8>(), y in any::<u8>(), z in any::<u8>(),
        ) {
            let in_range = x < CHUNK_SIZE && y < CHUNK_SIZE && z < CHUNK_SIZE;
            prop_assert_eq!(LocalPos::try_new(x, y, z).is_ok(), in_range);
        }

        #[test]
        fn split_recombines_to_the_original_position(pos in world_pos()) {
            let (chunk, local) = pos.split();
            let size = i64::from(CHUNK_SIZE);
            prop_assert_eq!(i64::from(chunk.x) * size + i64::from(local.x()), i64::from(pos.x));
            prop_assert_eq!(i64::from(chunk.y) * size + i64::from(local.y()), i64::from(pos.y));
            prop_assert_eq!(i64::from(chunk.z) * size + i64::from(local.z()), i64::from(pos.z));
        }

        #[test]
        fn index_stays_inside_the_chunk(pos in world_pos()) {
            prop_assert!(pos.split().1.index() < CHUNK_VOLUME);
        }
    }

    #[test]
    fn index_is_a_bijection_over_the_chunk() {
        let mut seen = HashSet::new();
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let local = LocalPos::try_new(x, y, z).unwrap();
                    assert!(seen.insert(local.index()));
                }
            }
        }
        assert_eq!(seen.len(), CHUNK_VOLUME);
    }

    #[test]
    fn all_yields_every_position_in_index_order() {
        let indices: Vec<_> = LocalPos::all().map(LocalPos::index).collect();
        assert!(indices.iter().copied().eq(0..CHUNK_VOLUME));
    }

    #[test]
    fn negative_coordinates_floor_into_the_previous_chunk() {
        let (chunk, local) = WorldPos {
            x: -1,
            y: -1,
            z: -1,
        }
        .split();
        assert_eq!(
            chunk,
            ChunkPos {
                x: -1,
                y: -1,
                z: -1
            }
        );
        assert_eq!(local, LocalPos::try_new(15, 15, 15).unwrap());
    }
}
