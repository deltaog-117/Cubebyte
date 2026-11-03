//! Chunk meshing.
//!
//! Turns a chunk into plain vertex and index data with one quad per exposed block face.
//! The output holds no `wgpu` types, so meshing runs and is tested without a window or a
//! GPU; the pipeline decides how to upload it.

use crate::blocks::registry;
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

    /// The two axes (0 = x, 1 = y, 2 = z) that lie in the plane of the face.
    const fn tangents(self) -> [usize; 2] {
        match self {
            Self::PosX | Self::NegX => [1, 2],
            Self::PosY | Self::NegY => [0, 2],
            Self::PosZ | Self::NegZ => [0, 1],
        }
    }
}

/// How much a corner of a face is shut in by the blocks around it: [`Occlusion::OPEN`] in
/// the open down to 0 in a tight nook. Only [`Occlusion::from_neighbours`] makes one, so
/// the level is always in range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occlusion(u8);

impl Occlusion {
    /// Nothing crowds the corner.
    pub const OPEN: Self = Self(3);

    /// The occlusion of a corner, given whether the two blocks beside it (along the face's
    /// two edges) and the block diagonal to it are solid. Both sides solid shuts the corner
    /// completely, whatever the diagonal holds, because no light reaches it past them.
    pub const fn from_neighbours(side_a: bool, side_b: bool, diagonal: bool) -> Self {
        if side_a && side_b {
            Self(0)
        } else {
            Self(3 - side_a as u8 - side_b as u8 - diagonal as u8)
        }
    }

    /// The level, from 0 (shut in) to [`Occlusion::OPEN`].
    pub const fn level(self) -> u8 {
        self.0
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
    /// How shut in this corner is, for the renderer to darken it.
    pub occlusion: Occlusion,
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

    fn push_quad(
        &mut self,
        local: LocalPos,
        face: Face,
        block: BlockId,
        occlusion: [Occlusion; 4],
    ) {
        // A chunk holds at most 6 * 16^3 quads, so the vertex count always fits in `u32`.
        let first = self.vertices.len() as u32;
        let origin = [local.x(), local.y(), local.z()];
        let corners = face.corners();
        self.vertices.extend((0..4).map(|k| Vertex {
            position: std::array::from_fn(|axis| f32::from(origin[axis] + corners[k][axis])),
            face,
            block,
            occlusion: occlusion[k],
        }));
        // The quad is two triangles, and shading is interpolated along them. Cutting along
        // the diagonal that joins the more shut-in corners keeps a single dark corner from
        // smearing into a streak. Starting the same loop one corner later turns that
        // diagonal without changing which way the triangles face.
        let [a, b, c, d] = occlusion.map(|corner| corner.level());
        let order = if a + c > b + d {
            [1, 2, 3, 1, 3, 0]
        } else {
            [0, 1, 2, 0, 2, 3]
        };
        self.indices.extend(order.map(|offset| first + offset));
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
            .filter(|&(local, face, _)| neighbor(world, pos, chunk, local, face.normal()).is_air())
            .for_each(|(local, face, block)| {
                let occlusion = face_occlusion(world, pos, chunk, local, face);
                mesh.push_quad(local, face, block, occlusion);
            });
    }
    tracing::debug!(?pos, quads = mesh.quad_count(), "meshed chunk");
    mesh
}

/// Chunks whose meshes depend on the block at `pos`: the chunk holding it, plus any
/// chunk that holds a block next to it, even one touching only at an edge or a corner.
///
/// Face visibility needs the six face neighbours, but the shading of a corner reads the
/// blocks diagonal to it, so an edit can darken or lighten a face on any of the 26 blocks
/// around it.
///
/// After an edit at `pos`, re-meshing exactly these chunks keeps every mesh up to date.
pub fn chunks_to_remesh(pos: WorldPos) -> Vec<ChunkPos> {
    let mut chunks = vec![pos.split().0];
    let around =
        (-1..=1).flat_map(|dx| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dz| [dx, dy, dz])));
    for step in around {
        // Saturating at the edge of the representable world only repeats `pos` itself.
        let touching = WorldPos {
            x: pos.x.saturating_add(step[0]),
            y: pos.y.saturating_add(step[1]),
            z: pos.z.saturating_add(step[2]),
        };
        let chunk = touching.split().0;
        if !chunks.contains(&chunk) {
            chunks.push(chunk);
        }
    }
    chunks
}

/// The occlusion at each corner of `face` on the block at `local`, in the order of the
/// face's corners. It reads the layer of blocks just in front of the face: the two beside
/// each corner along the face's edges, and the one diagonal to it.
fn face_occlusion(
    world: &World,
    pos: ChunkPos,
    chunk: &Chunk,
    local: LocalPos,
    face: Face,
) -> [Occlusion; 4] {
    let normal = face.normal();
    let [first, second] = face.tangents();
    let solid = |step: [i8; 3]| registry::is_solid(neighbor(world, pos, chunk, local, step));
    face.corners().map(|corner| {
        // One step from the block's centre toward this corner along an axis.
        let toward = |axis: usize| if corner[axis] == 1 { 1 } else { -1 };
        let along = |axis: usize| {
            std::array::from_fn(|k| normal[k] + if k == axis { toward(k) } else { 0 })
        };
        let both = std::array::from_fn(|k| {
            normal[k]
                + if k == first || k == second {
                    toward(k)
                } else {
                    0
                }
        });
        Occlusion::from_neighbours(solid(along(first)), solid(along(second)), solid(both))
    })
}

/// The block `step` away from the block at `local`, one block at most along each axis.
fn neighbor(
    world: &World,
    pos: ChunkPos,
    chunk: &Chunk,
    local: LocalPos,
    step: [i8; 3],
) -> BlockId {
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

    /// The occlusion levels of the corners of the top face of the block at `(x, y, z)`,
    /// keyed by the corner's position inside the chunk.
    fn top_corners(world: &World, x: u8, y: u8, z: u8) -> Vec<([f32; 3], u8)> {
        let top = [x, y, z].map(f32::from);
        mesh_chunk(world, ORIGIN)
            .vertices()
            .iter()
            .filter(|v| v.face == Face::PosY && v.position[1] == top[1] + 1.0)
            .filter(|v| {
                (v.position[0] - top[0]).abs() <= 1.0 && (v.position[2] - top[2]).abs() <= 1.0
            })
            .map(|v| (v.position, v.occlusion.level()))
            .collect()
    }

    #[test]
    fn occlusion_follows_the_neighbours_around_a_corner() {
        let expected = [
            // (side, side, diagonal) -> level
            ((false, false, false), 3),
            ((true, false, false), 2),
            ((false, true, false), 2),
            ((false, false, true), 2),
            ((true, false, true), 1),
            ((false, true, true), 1),
            ((true, true, false), 0),
            ((true, true, true), 0),
        ];
        for ((a, b, d), level) in expected {
            assert_eq!(Occlusion::from_neighbours(a, b, d).level(), level);
        }
    }

    #[test]
    fn a_lone_block_has_open_corners_everywhere() {
        let mesh = mesh_chunk(&world_with(&[(5, 5, 5)]), ORIGIN);
        assert!(
            mesh.vertices()
                .iter()
                .all(|v| v.occlusion == Occlusion::OPEN)
        );
    }

    #[test]
    fn a_wall_darkens_the_floor_corners_beside_it() {
        // A floor block at (5,5,5) with a wall block rising on its +x side.
        let world = world_with(&[(5, 5, 5), (6, 6, 5)]);
        let corners = top_corners(&world, 5, 5, 5);
        assert_eq!(corners.len(), 4);
        for (position, level) in corners {
            let beside_wall = position[0] == 6.0;
            assert_eq!(level, if beside_wall { 2 } else { 3 }, "{position:?}");
        }
    }

    #[test]
    fn a_nook_between_two_walls_is_fully_shut() {
        let world = world_with(&[(5, 5, 5), (6, 6, 5), (5, 6, 6)]);
        let nook = top_corners(&world, 5, 5, 5)
            .into_iter()
            .find(|(position, _)| position[0] == 6.0 && position[2] == 6.0)
            .map(|(_, level)| level);
        assert_eq!(nook, Some(0));
    }

    #[test]
    fn a_block_next_door_in_another_chunk_darkens_the_corner() {
        // The wall stands in the next chunk over, so the lookup must cross the border.
        let world = world_with(&[(15, 5, 5), (16, 6, 5)]);
        let beside_wall = top_corners(&world, 15, 5, 5)
            .into_iter()
            .filter(|(position, _)| position[0] == 16.0)
            .all(|(_, level)| level == 2);
        assert!(beside_wall);
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

        #[test]
        fn quads_are_cut_along_the_more_shut_in_diagonal(
            cells in prop::collection::vec((0i32..16, 0i32..16, 0i32..16), 1..80),
        ) {
            let mesh = mesh_chunk(&world_with(&cells), ORIGIN);
            for (quad, indices) in mesh.vertices().chunks_exact(4).zip(mesh.indices().chunks_exact(6)) {
                // Both orders start on one end of the diagonal and put the other end third.
                let base = *indices.iter().min().expect("a quad has six indices");
                let diagonal = [indices[0], indices[2]].map(|i| (i - base) as usize);
                let levels = quad.iter().map(|v| u16::from(v.occlusion.level())).collect::<Vec<_>>();
                let cut = levels[diagonal[0]] + levels[diagonal[1]];
                let other = levels.iter().sum::<u16>() - cut;
                prop_assert!(cut <= other, "cut {cut} other {other}");
            }
        }

        #[test]
        fn an_edit_changes_only_the_meshes_it_asks_to_remesh(
            cells in prop::collection::vec((-16i32..32, -16i32..32, -16i32..32), 0..200),
            edit in (-16i32..32, -16i32..32, -16i32..32),
            block in 0u16..2,
        ) {
            let mut world = world_with(&cells);
            let pos = WorldPos { x: edit.0, y: edit.1, z: edit.2 };
            let touched = chunks_to_remesh(pos);
            let chunks: Vec<_> = (-1..=1)
                .flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| ChunkPos { x, y, z })))
                .collect();
            let before: Vec<_> = chunks.iter().map(|&chunk| mesh_chunk(&world, chunk)).collect();

            let event = Event::BlockSet { pos, old: world.block_at(pos), new: BlockId(block) };
            prop_assert_eq!(world.apply(&event), Ok(()));

            prop_assert!(touched.contains(&pos.split().0));
            for (chunk, old_mesh) in chunks.iter().zip(&before) {
                if !touched.contains(chunk) {
                    prop_assert_eq!(&mesh_chunk(&world, *chunk), old_mesh);
                }
            }
        }
    }
}
