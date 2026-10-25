//! Chunk meshing.
//!
//! Turns a chunk into plain vertex and index data with one quad per exposed block face.
//! The output holds no `wgpu` types, so meshing runs and is tested without a window or a
//! GPU; the pipeline decides how to upload it.

use crate::shared::voxel::{BlockId, CHUNK_SIZE, ChunkPos, LocalPos, WorldPos};
use crate::world::model::{Chunk, World};

/// One of the six sides of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Face {
    pub const ALL: [Self; 6] = [
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
        Self::PosZ,
        Self::NegZ,
    ];

    /// Step from a block to the block this face looks at; also the face's outward normal.
    pub const fn normal(self) -> [i8; 3] {
        match self {
            Self::PosX => [1, 0, 0],
            Self::NegX => [-1, 0, 0],
            Self::PosY => [0, 1, 0],
            Self::NegY => [0, -1, 0],
            Self::PosZ => [0, 0, 1],
            Self::NegZ => [0, 0, -1],
        }
    }

    /// Corners of the face on a unit cube, counter-clockwise seen from outside, so the
    /// triangles `0 1 2` and `0 2 3` both point along [`Face::normal`].
    const fn corners(self) -> [[u8; 3]; 4] {
        match self {
            Self::PosX => [[1, 0, 1], [1, 0, 0], [1, 1, 0], [1, 1, 1]],
            Self::NegX => [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]],
            Self::PosY => [[0, 1, 1], [1, 1, 1], [1, 1, 0], [0, 1, 0]],
            Self::NegY => [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]],
            Self::PosZ => [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]],
            Self::NegZ => [[1, 0, 0], [0, 0, 0], [0, 1, 0], [1, 1, 0]],
        }
    }
}

/// A corner of a face. `position` is relative to the chunk's origin, in `0.0..=16.0`, so
/// `f32` stays exact far from the world origin; the renderer adds the chunk offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub face: Face,
    /// Lets the renderer pick a texture or colour per block type.
    pub block: BlockId,
}

/// Vertices and triangle indices for one chunk.
///
/// Always a whole number of quads: 4 vertices and 6 indices each, with every index below
/// the vertex count. Only [`mesh_chunk`] builds one, so these hold by construction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl MeshData {
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn quad_count(&self) -> usize {
        self.vertices.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    fn push_quad(&mut self, local: LocalPos, face: Face, block: BlockId) {
        // A chunk holds at most 6 * 16^3 quads, so the vertex count always fits in `u32`.
        let first = self.vertices.len() as u32;
        let origin = [local.x(), local.y(), local.z()];
        self.vertices.extend(face.corners().map(|corner| Vertex {
            position: std::array::from_fn(|axis| f32::from(origin[axis] + corner[axis])),
            face,
            block,
        }));
        self.indices
            .extend([0, 1, 2, 0, 2, 3].map(|offset| first + offset));
    }
}

/// Meshes the chunk at `pos`: one quad for every block face that touches air.
///
/// A face on the chunk border looks into the neighbouring chunk through `world`. An
/// unloaded neighbour reads as air, so those faces are drawn until it loads.
///
/// An unloaded chunk, and any chunk with no blocks, gives an empty mesh.
pub fn mesh_chunk(world: &World, pos: ChunkPos) -> MeshData {
    let mut mesh = MeshData::default();
    if let Some(chunk) = world.chunk(pos) {
        LocalPos::all()
            .map(|local| (local, chunk.get(local)))
            .filter(|&(_, block)| !block.is_air())
            .flat_map(|(local, block)| Face::ALL.map(|face| (local, face, block)))
            .filter(|&(local, face, _)| neighbor(world, pos, chunk, local, face).is_air())
            .for_each(|(local, face, block)| mesh.push_quad(local, face, block));
    }
    tracing::debug!(?pos, quads = mesh.quad_count(), "meshed chunk");
    mesh
}

/// The block that `face` of the block at `local` looks at.
fn neighbor(world: &World, pos: ChunkPos, chunk: &Chunk, local: LocalPos, face: Face) -> BlockId {
    let step = face.normal();
    let shifted = (
        shift(local.x(), step[0]),
        shift(local.y(), step[1]),
        shift(local.z(), step[2]),
    );
    // Inside the chunk the array is read directly; only border faces pay for a map lookup.
    if let (Some(x), Some(y), Some(z)) = shifted
        && let Ok(inside) = LocalPos::try_new(x, y, z)
    {
        return chunk.get(inside);
    }
    let world_axis = |chunk_axis: i32, local_axis: u8, delta: i8| {
        chunk_axis
            .checked_mul(i32::from(CHUNK_SIZE))?
            .checked_add(i32::from(local_axis) + i32::from(delta))
    };
    match (
        world_axis(pos.x, local.x(), step[0]),
        world_axis(pos.y, local.y(), step[1]),
        world_axis(pos.z, local.z(), step[2]),
    ) {
        (Some(x), Some(y), Some(z)) => world.block_at(WorldPos { x, y, z }),
        // Past the edge of the representable world there is nothing to hide a face.
        _ => BlockId::AIR,
    }
}

/// `value + delta`, or `None` when that would leave `0..CHUNK_SIZE`.
fn shift(value: u8, delta: i8) -> Option<u8> {
    value
        .checked_add_signed(delta)
        .filter(|&shifted| shifted < CHUNK_SIZE)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shared::event::Event;

    const ORIGIN: ChunkPos = ChunkPos { x: 0, y: 0, z: 0 };

    fn place(world: &mut World, x: i32, y: i32, z: i32, block: BlockId) {
        let pos = WorldPos { x, y, z };
        let event = Event::BlockSet {
            pos,
            old: world.block_at(pos),
            new: block,
        };
        world.apply(&event).unwrap();
    }

    fn world_with(blocks: &[(i32, i32, i32)]) -> World {
        let mut world = World::default();
        for &(x, y, z) in blocks {
            place(&mut world, x, y, z, BlockId::STONE);
        }
        world
    }

    /// Exposed faces of the chunk at `ORIGIN`, counted straight from `World::block_at`
    /// so it shares no code with the mesher's neighbour lookup.
    fn exposed_faces(world: &World) -> usize {
        LocalPos::all()
            .filter(|local| {
                let pos = WorldPos {
                    x: i32::from(local.x()),
                    y: i32::from(local.y()),
                    z: i32::from(local.z()),
                };
                !world.block_at(pos).is_air()
            })
            .map(|local| {
                Face::ALL
                    .iter()
                    .filter(|face| {
                        let step = face.normal();
                        let neighbor = WorldPos {
                            x: i32::from(local.x()) + i32::from(step[0]),
                            y: i32::from(local.y()) + i32::from(step[1]),
                            z: i32::from(local.z()) + i32::from(step[2]),
                        };
                        world.block_at(neighbor).is_air()
                    })
                    .count()
            })
            .sum()
    }

    #[test]
    fn an_unloaded_chunk_has_an_empty_mesh() {
        assert!(mesh_chunk(&World::default(), ORIGIN).is_empty());
    }

    #[test]
    fn a_lone_block_shows_all_six_faces() {
        let mesh = mesh_chunk(&world_with(&[(5, 5, 5)]), ORIGIN);
        assert_eq!(mesh.quad_count(), 6);
        assert_eq!(mesh.indices().len(), 36);
    }

    #[test]
    fn two_touching_blocks_hide_the_face_between_them() {
        let mesh = mesh_chunk(&world_with(&[(5, 5, 5), (6, 5, 5)]), ORIGIN);
        assert_eq!(mesh.quad_count(), 10);
    }

    #[test]
    fn a_loaded_neighbour_chunk_hides_the_border_face() {
        let alone = mesh_chunk(&world_with(&[(15, 3, 3)]), ORIGIN);
        let touching = mesh_chunk(&world_with(&[(15, 3, 3), (16, 3, 3)]), ORIGIN);
        assert_eq!(alone.quad_count(), 6);
        assert_eq!(touching.quad_count(), 5);
        assert!(touching.vertices().iter().all(|v| v.face != Face::PosX));
    }

    #[test]
    fn border_faces_look_into_negative_chunks_too() {
        let mesh = mesh_chunk(&world_with(&[(0, 0, 0), (-1, 0, 0)]), ORIGIN);
        assert_eq!(mesh.quad_count(), 5);
    }

    #[test]
    fn a_chunk_at_the_edge_of_the_representable_world_does_not_overflow() {
        let top = ChunkPos {
            x: i32::MAX.div_euclid(16),
            y: i32::MAX.div_euclid(16),
            z: i32::MAX.div_euclid(16),
        };
        let mut world = World::default();
        let mut chunk = Chunk::default();
        chunk.set(LocalPos::try_new(15, 15, 15).unwrap(), BlockId::STONE);
        world.insert_generated(top, chunk).unwrap();
        assert_eq!(mesh_chunk(&world, top).quad_count(), 6);
    }

    #[test]
    fn vertices_carry_the_block_type() {
        let mut world = World::default();
        place(&mut world, 1, 1, 1, BlockId::GRASS);
        let mesh = mesh_chunk(&world, ORIGIN);
        assert!(mesh.vertices().iter().all(|v| v.block == BlockId::GRASS));
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    proptest! {
        #[test]
        fn mesh_matches_an_independent_face_count_and_is_well_formed(
            cells in prop::collection::vec((-16i32..32, -16i32..32, -16i32..32), 0..300),
        ) {
            let world = world_with(&cells);
            let mesh = mesh_chunk(&world, ORIGIN);

            prop_assert_eq!(mesh.quad_count(), exposed_faces(&world));
            prop_assert_eq!(mesh.vertices().len(), mesh.quad_count() * 4);
            prop_assert_eq!(mesh.indices().len(), mesh.quad_count() * 6);
            let vertex_count = mesh.vertices().len();
            prop_assert!(mesh.indices().iter().all(|&i| (i as usize) < vertex_count));
            let in_chunk = mesh
                .vertices()
                .iter()
                .all(|v| v.position.iter().all(|c| (0.0..=16.0).contains(c)));
            prop_assert!(in_chunk);

        }

        #[test]
        fn every_triangle_faces_outward_along_its_face_normal(
            cells in prop::collection::vec((0i32..16, 0i32..16, 0i32..16), 1..60),
        ) {
            let mesh = mesh_chunk(&world_with(&cells), ORIGIN);
            for triangle in mesh.indices().chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices()[triangle[k] as usize]);
                let area = cross(sub(b.position, a.position), sub(c.position, a.position));
                let normal = a.face.normal().map(f32::from);
                // A unit triangle has half the area of its quad, so the cross product has
                // length 1 and must equal the normal exactly.
                prop_assert_eq!(area, normal);
            }
        }
    }
}
