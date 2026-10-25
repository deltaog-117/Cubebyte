//! Seeded terrain generation.
//!
//! Terrain is a pure function of the seed and the position, so a chunk can be dropped and
//! regenerated, or generated in any order, and always comes out the same. Heights come from
//! Perlin noise written out here, not taken from a crate, for the same reason as [`Rng`]:
//! the output for a seed must never change under saved worlds. It uses only `+`, `-`, `*`,
//! `/` and `round` on `f64`, which are exactly specified, and no `sin`, `cos` or `powf`,
//! which vary between platforms.

use crate::shared::rng::{Rng, splitmix64};
use crate::shared::voxel::{BlockId, CHUNK_SIZE, ChunkPos, LocalPos};
use crate::world::model::Chunk;

/// Height of the surface where the noise is zero, in blocks.
const BASE_HEIGHT: i32 = 16;

/// Most the noise can raise or lower the surface from [`BASE_HEIGHT`], in blocks.
const AMPLITUDE: i32 = 24;

/// Lowest surface height any column can have.
pub const MIN_HEIGHT: i32 = BASE_HEIGHT - AMPLITUDE;

/// Highest surface height any column can have.
pub const MAX_HEIGHT: i32 = BASE_HEIGHT + AMPLITUDE;

/// Dirt blocks between the grass and the stone.
const DIRT_DEPTH: i64 = 3;

/// `(wavelength in blocks, weight)` per noise layer. Wavelengths are powers of two so the
/// position inside a cell is an exact fraction.
const OCTAVES: [(i64, f64); 4] = [(64, 1.0), (32, 0.5), (16, 0.25), (8, 0.125)];

const TOTAL_WEIGHT: f64 = 1.0 + 0.5 + 0.25 + 0.125;

/// Builds the terrain for one world seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainGenerator {
    /// One hash salt per octave, so the layers are independent of each other.
    salts: [u64; OCTAVES.len()],
}

impl TerrainGenerator {
    pub fn new(seed: u64) -> Self {
        // Never drawn from, only forked, so the salts do not depend on call order.
        let root = Rng::from_seed(seed);
        Self {
            salts: std::array::from_fn(|octave| root.fork(octave as u64).next_u64()),
        }
    }

    /// The `y` of the topmost block in the column at `(x, z)`.
    ///
    /// Always within [`MIN_HEIGHT`]`..=`[`MAX_HEIGHT`].
    pub fn height_at(&self, x: i64, z: i64) -> i32 {
        let sum: f64 = OCTAVES
            .iter()
            .zip(&self.salts)
            .map(|(&(wavelength, weight), &salt)| weight * perlin(salt, x, z, wavelength))
            .sum();
        // Perlin noise is not strictly bounded by 1 with these gradients; the clamp is what
        // guarantees the height range.
        let noise = (sum / TOTAL_WEIGHT).clamp(-1.0, 1.0);
        BASE_HEIGHT + (noise * f64::from(AMPLITUDE)).round() as i32
    }

    /// The chunk at `pos`: grass on top, [`DIRT_DEPTH`] blocks of dirt, then stone.
    pub fn generate_chunk(&self, pos: ChunkPos) -> Chunk {
        let _span = tracing::debug_span!("generate_chunk", ?pos).entered();

        let size = i64::from(CHUNK_SIZE);
        let (base_x, base_y, base_z) = (
            i64::from(pos.x) * size,
            i64::from(pos.y) * size,
            i64::from(pos.z) * size,
        );
        // Most chunks above the ground are skipped without sampling any noise.
        if base_y > i64::from(MAX_HEIGHT) {
            return Chunk::default();
        }

        // Row-major over (z, x), the same order `LocalPos` uses inside a layer.
        let heights: [i32; (CHUNK_SIZE as usize).pow(2)] = std::array::from_fn(|column| {
            let (lx, lz) = (column as i64 % size, column as i64 / size);
            self.height_at(base_x + lx, base_z + lz)
        });

        let mut chunk = Chunk::default();
        for local in LocalPos::all() {
            let height =
                heights[usize::from(local.z()) * usize::from(CHUNK_SIZE) + usize::from(local.x())];
            let depth = i64::from(height) - (base_y + i64::from(local.y()));
            let block = layer_block(depth);
            if !block.is_air() {
                chunk.set(local, block);
            }
        }
        chunk
    }
}

/// The block `depth` blocks below the surface; negative depths are above it.
fn layer_block(depth: i64) -> BlockId {
    match depth {
        ..0 => BlockId::AIR,
        0 => BlockId::GRASS,
        1..=DIRT_DEPTH => BlockId::DIRT,
        _ => BlockId::STONE,
    }
}

/// 2D Perlin noise at block `(x, z)` for one octave, roughly in `-1.0..=1.0`.
fn perlin(salt: u64, x: i64, z: i64, wavelength: i64) -> f64 {
    let (cell_x, cell_z) = (x.div_euclid(wavelength), z.div_euclid(wavelength));
    let span = wavelength as f64;
    let fx = x.rem_euclid(wavelength) as f64 / span;
    let fz = z.rem_euclid(wavelength) as f64 / span;

    let corner = |dx: i64, dz: i64| {
        gradient(
            lattice_hash(salt, cell_x + dx, cell_z + dz),
            fx - dx as f64,
            fz - dz as f64,
        )
    };
    let (u, v) = (fade(fx), fade(fz));
    let bottom = lerp(corner(0, 0), corner(1, 0), u);
    let top = lerp(corner(0, 1), corner(1, 1), u);
    lerp(bottom, top, v)
}

/// A well-mixed hash of one lattice point.
fn lattice_hash(salt: u64, cell_x: i64, cell_z: i64) -> u64 {
    let mut state = salt ^ cell_x as u64;
    let mut state = splitmix64(&mut state) ^ cell_z as u64;
    splitmix64(&mut state)
}

/// Dot product of `(dx, dz)` with one of eight fixed gradient directions chosen by `hash`.
fn gradient(hash: u64, dx: f64, dz: f64) -> f64 {
    match hash & 7 {
        0 => dx + dz,
        1 => dx - dz,
        2 => -dx + dz,
        3 => -dx - dz,
        4 => dx,
        5 => -dx,
        6 => dz,
        _ => -dz,
    }
}

/// Perlin's `6t^5 - 15t^4 + 10t^3`, which has zero slope at both ends so cells join smoothly.
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn chunk_pos() -> impl Strategy<Value = ChunkPos> {
        // Spans the surface (chunk y -1..=2 covers MIN_HEIGHT..=MAX_HEIGHT) and negative columns.
        (-8i32..8, -2i32..4, -8i32..8).prop_map(|(x, y, z)| ChunkPos { x, y, z })
    }

    // Regression pins, not independent reference values. If this fails, the terrain for
    // every existing seed changed and saved worlds no longer match their event logs.
    #[test]
    fn heights_are_pinned_for_known_seeds() {
        let points = [
            (0, 0),
            (1, 0),
            (0, 1),
            (100, -250),
            (-1000, 777),
            (12345, 54321),
        ];
        let heights = |seed| {
            let terrain = TerrainGenerator::new(seed);
            points.map(|(x, z)| terrain.height_at(x, z))
        };
        assert_eq!(heights(0), [16, 16, 16, 15, 15, 24]);
        assert_eq!(heights(42), [16, 16, 16, 10, 18, 15]);
    }

    #[test]
    fn neighbouring_columns_never_form_a_cliff() {
        let terrain = TerrainGenerator::new(0);
        // Spans the origin so the seams between chunks on negative and positive sides are covered.
        for x in -64..64 {
            for z in -64..64 {
                let here = terrain.height_at(x, z);
                assert!((here - terrain.height_at(x + 1, z)).abs() <= 4);
                assert!((here - terrain.height_at(x, z + 1)).abs() <= 4);
            }
        }
    }

    #[test]
    fn chunks_above_the_highest_surface_are_empty() {
        let terrain = TerrainGenerator::new(1);
        let above = MAX_HEIGHT / i32::from(CHUNK_SIZE) + 1;
        assert!(
            terrain
                .generate_chunk(ChunkPos {
                    x: 0,
                    y: above,
                    z: 0
                })
                .is_empty()
        );
    }

    #[test]
    fn chunks_far_below_the_lowest_surface_are_solid_stone() {
        let terrain = TerrainGenerator::new(1);
        let chunk = terrain.generate_chunk(ChunkPos {
            x: 3,
            y: -10,
            z: -3,
        });
        assert!(LocalPos::all().all(|local| chunk.get(local) == BlockId::STONE));
    }

    proptest! {
        #[test]
        fn height_stays_within_the_documented_range(
            seed in any::<u64>(), x in any::<i64>(), z in any::<i64>(),
        ) {
            let height = TerrainGenerator::new(seed).height_at(x, z);
            prop_assert!((MIN_HEIGHT..=MAX_HEIGHT).contains(&height));
        }

        #[test]
        fn same_seed_gives_the_same_chunk(seed in any::<u64>(), pos in chunk_pos()) {
            let a = TerrainGenerator::new(seed).generate_chunk(pos);
            let b = TerrainGenerator::new(seed).generate_chunk(pos);
            prop_assert_eq!(a, b);
        }

        #[test]
        fn different_seeds_give_different_terrain(a in any::<u64>(), b in any::<u64>()) {
            prop_assume!(a != b);
            let (a, b) = (TerrainGenerator::new(a), TerrainGenerator::new(b));
            let differs = (0..32).any(|i| {
                let (x, z) = (i * 37 - 500, i * 53 - 800);
                a.height_at(x, z) != b.height_at(x, z)
            });
            prop_assert!(differs);
        }

        #[test]
        fn chunk_matches_the_layers_implied_by_the_height(
            seed in any::<u64>(), pos in chunk_pos(),
        ) {
            let terrain = TerrainGenerator::new(seed);
            let chunk = terrain.generate_chunk(pos);
            let size = i64::from(CHUNK_SIZE);
            for local in LocalPos::all() {
                let x = i64::from(pos.x) * size + i64::from(local.x());
                let y = i64::from(pos.y) * size + i64::from(local.y());
                let z = i64::from(pos.z) * size + i64::from(local.z());
                let depth = i64::from(terrain.height_at(x, z)) - y;
                prop_assert_eq!(chunk.get(local), layer_block(depth));
            }
        }

        #[test]
        fn every_chunk_position_generates_without_panicking(
            seed in any::<u64>(), x in any::<i32>(), y in any::<i32>(), z in any::<i32>(),
        ) {
            let _ = TerrainGenerator::new(seed).generate_chunk(ChunkPos { x, y, z });
        }
    }
}
