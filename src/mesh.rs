//! Chunk meshing: turn a chunk of voxels into one `Mesh`.
//!
//! Naive culled faces — a quad is emitted only where a solid block meets a non-solid one.
//! No greedy merging: it is ~40 lines instead of ~200 and holds a comfortable frame rate
//! at the bones' draw distance. If the world ever needs bigger view distances, this
//! function is the single place to make greedy.
//!
//! Block colour rides in vertex colours, so the entire world draws with ONE material and
//! adding a block type needs no asset work at all — just a [`crate::registry::BLOCKS`] row.

use crate::registry::Block;
use crate::world::{BlockPos, CHUNK_SIZE, ChunkPos, WORLD_HEIGHT, World};
use bevy::asset::RenderAssetUsages;
use bevy::math::IVec3;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

/// One cube face: its outward normal, its four corners wound counter-clockwise as seen
/// from outside, and a flat shade factor that fakes directional lighting so faces stay
/// readable regardless of where the sun is.
struct Face {
    normal: IVec3,
    corners: [[f32; 3]; 4],
    shade: f32,
}

const FACES: [Face; 6] = [
    Face {
        normal: IVec3::new(0, 1, 0),
        corners: [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]],
        shade: 1.0,
    },
    Face {
        normal: IVec3::new(0, -1, 0),
        corners: [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]],
        shade: 0.45,
    },
    Face {
        normal: IVec3::new(1, 0, 0),
        corners: [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
        shade: 0.8,
    },
    Face {
        normal: IVec3::new(-1, 0, 0),
        corners: [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
        shade: 0.8,
    },
    Face {
        normal: IVec3::new(0, 0, 1),
        corners: [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
        shade: 0.65,
    },
    Face {
        normal: IVec3::new(0, 0, -1),
        corners: [[0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]],
        shade: 0.65,
    },
];

/// Builds the mesh for one chunk, in chunk-local coordinates (the entity carries the
/// chunk's world translation). `None` when the chunk has no visible faces at all — an
/// empty mesh is an entity and a draw call for nothing.
///
/// Neighbouring chunks must be loaded first: an unloaded neighbour reads as air, which
/// would draw a wall of faces along the seam.
pub fn build_chunk_mesh(world: &World, cp: ChunkPos) -> Option<Mesh> {
    let chunk = world.chunk(cp)?;
    let origin = cp.origin();

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for y in 0..WORLD_HEIGHT {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let block = chunk.get(x, y, z);
                if block == Block::Air {
                    continue;
                }
                let world_pos = BlockPos::new(origin.x + x, y, origin.z + z);
                for face in &FACES {
                    if world.solid(world_pos.offset(face.normal)) {
                        continue;
                    }
                    let base = positions.len() as u32;
                    let [r, g, b] = block.color();
                    let s = face.shade;
                    for (i, c) in face.corners.iter().enumerate() {
                        positions.push([x as f32 + c[0], y as f32 + c[1], z as f32 + c[2]]);
                        normals.push([
                            face.normal.x as f32,
                            face.normal.y as f32,
                            face.normal.z as f32,
                        ]);
                        colors.push([r * s, g * s, b * s, 1.0]);
                        uvs.push([
                            (i == 2 || i == 3) as u8 as f32,
                            (i == 1 || i == 2) as u8 as f32,
                        ]);
                    }
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }
    }

    if indices.is_empty() {
        return None;
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::ChunkPos;

    fn vertex_count(mesh: &Mesh) -> usize {
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("positions")
            .len()
    }

    #[test]
    fn a_terrain_chunk_meshes() {
        let mut w = World::new(5, []);
        for cx in -1..=1 {
            for cz in -1..=1 {
                w.load_chunk(ChunkPos::new(cx, cz));
            }
        }
        let mesh = build_chunk_mesh(&w, ChunkPos::new(0, 0)).expect("terrain has faces");
        assert!(vertex_count(&mesh) > 100);
        assert_eq!(vertex_count(&mesh) % 4, 0, "faces are quads");
    }

    #[test]
    fn an_all_air_chunk_meshes_to_nothing() {
        let mut w = World::new(5, []);
        let cp = ChunkPos::new(0, 0);
        w.load_chunk(cp);
        for y in 0..WORLD_HEIGHT {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    w.set_block(BlockPos::new(x, y, z), Block::Air);
                }
            }
        }
        assert!(build_chunk_mesh(&w, cp).is_none());
    }

    /// Interior faces must be culled: a solid block buried in solid neighbours emits
    /// nothing, so a 1-block change can't blow the vertex budget up by 6 quads.
    #[test]
    fn buried_faces_are_culled() {
        let mut w = World::new(5, []);
        let cp = ChunkPos::new(0, 0);
        w.load_chunk(cp);
        let below = build_chunk_mesh(&w, cp).map(|m| vertex_count(&m)).unwrap();

        // Fill the column interior solid; the surface area (and so the mesh) must shrink
        // relative to filling only the top, never grow per buried block.
        let deep = BlockPos::new(8, 5, 8);
        assert!(
            w.solid(deep),
            "test picked a point that should be underground"
        );
        w.set_block(deep, Block::Air);
        let with_hole = build_chunk_mesh(&w, cp).map(|m| vertex_count(&m)).unwrap();
        assert_eq!(
            with_hole,
            below + 6 * 4,
            "a hole underground should expose exactly its 6 faces"
        );
    }

    #[test]
    fn unmeshed_chunk_is_none() {
        let w = World::new(5, []);
        assert!(build_chunk_mesh(&w, ChunkPos::new(9, 9)).is_none());
    }
}
