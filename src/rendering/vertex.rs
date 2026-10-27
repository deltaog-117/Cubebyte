//! The vertex layout the GPU reads.
//!
//! Kept apart from the pipeline so the conversion from [`MeshData`] is tested without a
//! device.

use glam::Vec3;

use crate::blocks::registry;
use crate::rendering::meshing::{Face, MeshData};
use crate::shared::voxel::{CHUNK_SIZE, ChunkPos};

/// A corner in world space with its final colour. Lighting is baked in here, so the shader
/// only interpolates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl GpuVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
}

/// Fixed light from above: tops are brightest, bottoms darkest, so faces stay readable
/// without a lighting pass.
fn face_shade(face: Face) -> f32 {
    match face {
        Face::PosY => 1.0,
        Face::PosZ | Face::NegZ => 0.8,
        Face::PosX | Face::NegX => 0.65,
        Face::NegY => 0.5,
    }
}

/// Vertices and indices of one chunk, ready to upload.
pub fn gpu_mesh(mesh: &MeshData, chunk: ChunkPos) -> (Vec<GpuVertex>, Vec<u32>) {
    let size = f32::from(CHUNK_SIZE);
    // Chunk coordinates are small enough that the product is exact in `f32`.
    let origin = [chunk.x, chunk.y, chunk.z].map(|axis| axis as f32 * size);
    let vertices = mesh
        .vertices()
        .iter()
        .map(|vertex| {
            let shade = face_shade(vertex.face);
            GpuVertex {
                position: std::array::from_fn(|axis| vertex.position[axis] + origin[axis]),
                color: registry::color(vertex.block).map(|channel| channel * shade),
            }
        })
        .collect();
    (vertices, mesh.indices().to_vec())
}

/// The six faces of an axis-aligned box as counter-clockwise quads seen from outside, with
/// the same fixed shading as terrain.
pub fn box_mesh(min: Vec3, max: Vec3, color: [f32; 3]) -> (Vec<GpuVertex>, Vec<u32>) {
    let corner = |x: bool, y: bool, z: bool| {
        Vec3::new(
            if x { max.x } else { min.x },
            if y { max.y } else { min.y },
            if z { max.z } else { min.z },
        )
    };
    let faces = [
        (Face::PosX, [(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)]),
        (Face::NegX, [(0, 0, 1), (0, 1, 1), (0, 1, 0), (0, 0, 0)]),
        (Face::PosY, [(0, 1, 0), (0, 1, 1), (1, 1, 1), (1, 1, 0)]),
        (Face::NegY, [(0, 0, 1), (0, 0, 0), (1, 0, 0), (1, 0, 1)]),
        (Face::PosZ, [(1, 0, 1), (1, 1, 1), (0, 1, 1), (0, 0, 1)]),
        (Face::NegZ, [(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)]),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face, corners) in faces {
        let first = vertices.len() as u32;
        let shaded = color.map(|channel| channel * face_shade(face));
        vertices.extend(corners.map(|(x, y, z)| GpuVertex {
            position: corner(x == 1, y == 1, z == 1).to_array(),
            color: shaded,
        }));
        indices.extend([0, 1, 2, 0, 2, 3].map(|offset| first + offset));
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::meshing::mesh_chunk;
    use crate::shared::voxel::{BlockId, LocalPos, WorldPos};
    use crate::world::model::{Chunk, World};

    fn single_block_world() -> World {
        let mut chunk = Chunk::default();
        let center = LocalPos::try_new(8, 8, 8).expect("in range");
        chunk.set(center, BlockId::GRASS);
        let mut world = World::default();
        let pos = ChunkPos { x: 1, y: -1, z: 2 };
        assert_eq!(world.insert_generated(pos, chunk), Ok(()));
        assert_ne!(
            world.block_at(WorldPos {
                x: 24,
                y: -8,
                z: 40
            }),
            BlockId::AIR
        );
        world
    }

    #[test]
    fn vertices_are_moved_to_their_chunk() {
        let pos = ChunkPos { x: 1, y: -1, z: 2 };
        let mesh = mesh_chunk(&single_block_world(), pos);
        let (vertices, indices) = gpu_mesh(&mesh, pos);
        assert_eq!(vertices.len(), mesh.vertices().len());
        assert_eq!(indices, mesh.indices());
        for vertex in vertices {
            let [x, y, z] = vertex.position;
            assert!((24.0..=25.0).contains(&x), "x = {x}");
            assert!((-8.0..=-7.0).contains(&y), "y = {y}");
            assert!((40.0..=41.0).contains(&z), "z = {z}");
        }
    }

    #[test]
    fn colors_stay_displayable_and_tops_are_brightest() {
        for block in [BlockId::GRASS, BlockId::DIRT, BlockId::STONE, BlockId(999)] {
            for face in Face::ALL {
                let shade = face_shade(face);
                assert!((0.0..=1.0).contains(&shade));
                assert!(
                    registry::color(block)
                        .iter()
                        .all(|c| (0.0..=1.0).contains(c))
                );
                assert!(shade <= face_shade(Face::PosY));
            }
        }
    }

    #[test]
    fn a_box_has_outward_facing_triangles_inside_its_bounds() {
        let (min, max) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 4.0, 5.0));
        let (vertices, indices) = box_mesh(min, max, [0.5, 0.5, 0.5]);
        assert_eq!((vertices.len(), indices.len()), (24, 36));
        let centre = (min + max) / 2.0;
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(vertices[triangle[i] as usize].position));
            let normal = (b - a).cross(c - a);
            assert!(normal.dot(a - centre) > 0.0, "triangle faces inwards");
            for point in [a, b, c] {
                assert!(point.cmpge(min).all() && point.cmple(max).all());
            }
        }
    }
}
