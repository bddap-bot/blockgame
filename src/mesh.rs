//! Chunk meshing: turn a chunk of voxels into one `Mesh`.
//!
//! Naive culled faces — a quad is emitted only where a drawn block meets one that isn't
//! drawn ([`crate::registry::Block::visible`] decides both halves of that, once).
//! No greedy merging: it is ~40 lines instead of ~200 and holds a comfortable frame rate
//! at the bones' draw distance. If the world ever needs bigger view distances, this
//! function is the single place to make greedy.
//!
//! Block colour rides in vertex colours, so the entire world draws with ONE material and
//! adding a block type needs no asset work at all — just a colour in
//! [`crate::registry::Block`].

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

    // Chunk-local neighbour read. Everything but the four seam planes is already in the
    // chunk we hold; going through `world` for those would be a `div_euclid` pair and a
    // hashed chunk lookup per face — six per block, ~54k per chunk, on the main thread.
    let neighbor = |x: i32, y: i32, z: i32| -> Block {
        if !(0..CHUNK_SIZE).contains(&x) || !(0..CHUNK_SIZE).contains(&z) {
            return world.block(BlockPos::new(origin.x + x, y, origin.z + z));
        }
        if !(0..WORLD_HEIGHT).contains(&y) {
            return Block::Air;
        }
        chunk.get(x, y, z)
    };

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for y in 0..WORLD_HEIGHT {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let block = chunk.get(x, y, z);
                if !block.visible() {
                    continue;
                }
                for face in &FACES {
                    let n = face.normal;
                    // y == 0 is bedrock, which cannot be broken, so nothing ever gets
                    // under the world to look up at its underside.
                    if y == 0 && n.y < 0 {
                        continue;
                    }
                    if neighbor(x + n.x, y + n.y, z + n.z).visible() {
                        continue;
                    }
                    let base = positions.len() as u32;
                    let [r, g, b] = block.color();
                    let s = face.shade;
                    for c in &face.corners {
                        positions.push([x as f32 + c[0], y as f32 + c[1], z as f32 + c[2]]);
                        normals.push([n.x as f32, n.y as f32, n.z as f32]);
                        colors.push([r * s, g * s, b * s, 1.0]);
                    }
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }
    }

    if indices.is_empty() {
        return None;
    }

    // RENDER_WORLD only: nothing reads a chunk mesh back — collision and picking ask the
    // voxels — so keeping a second copy in RAM costs a Deck's shared memory for nothing.
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::ChunkPos;
    use bevy::math::Vec3;

    fn vertex_count(mesh: &Mesh) -> usize {
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("positions")
            .len()
    }

    /// A loaded chunk emptied of terrain, so a test can state the world it means to mesh
    /// instead of inheriting whatever worldgen happens to put there.
    fn air_chunk(cp: ChunkPos) -> World {
        let mut w = World::new(5, []);
        w.load_chunk(cp);
        let origin = cp.origin();
        for y in 0..WORLD_HEIGHT {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    w.set_block(BlockPos::new(origin.x + x, y, origin.z + z), Block::Air);
                }
            }
        }
        w
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
        let cp = ChunkPos::new(0, 0);
        assert!(build_chunk_mesh(&air_chunk(cp), cp).is_none());
    }

    /// Interior faces must be culled: a lump of blocks draws its shell and nothing else,
    /// so burying a block cannot cost six quads.
    ///
    /// This is also as close as a test can currently get to pinning the emit side and the
    /// occlude side to the *same* predicate: every non-`Air` block in the table is solid
    /// today, so `solid` and `visible` agree and no chunk can tell them apart. The first
    /// visible-but-not-solid block makes a real divergence test possible.
    #[test]
    fn only_the_shell_of_a_solid_lump_is_drawn() {
        let cp = ChunkPos::new(0, 0);
        let mut w = air_chunk(cp);
        for y in 5..8 {
            for z in 6..9 {
                for x in 6..9 {
                    w.set_block(BlockPos::new(x, y, z), Block::Stone);
                }
            }
        }
        let shell = build_chunk_mesh(&w, cp).map(|m| vertex_count(&m)).unwrap();
        assert_eq!(
            shell,
            6 * 9 * 4,
            "a 3x3x3 cube is six sides of nine quads — the buried centre draws nothing"
        );

        // Hollow it: the six blocks around the cavity now each face air.
        w.set_block(BlockPos::new(7, 6, 7), Block::Air);
        let hollow = build_chunk_mesh(&w, cp).map(|m| vertex_count(&m)).unwrap();
        assert_eq!(
            hollow,
            shell + 6 * 4,
            "a buried hole exposes exactly its own six faces"
        );
    }

    /// Bedrock at y == 0 is unbreakable, so the world's underside is unreachable — and
    /// 256 quads per chunk of it is ~74k quads over a full view distance.
    #[test]
    fn the_underside_of_the_world_is_not_drawn() {
        let cp = ChunkPos::new(0, 0);
        let mut w = air_chunk(cp);
        w.set_block(BlockPos::new(8, 0, 8), Block::Bedrock);
        let mesh = build_chunk_mesh(&w, cp).expect("the floor block has faces");
        assert_eq!(vertex_count(&mesh), 5 * 4, "five faces, never the sixth");
        let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().as_float3();
        assert!(
            !normals.unwrap().contains(&[0.0, -1.0, 0.0]),
            "a downward face escaped at y == 0"
        );
    }

    /// The 24 hand-typed corners are a table a transposed digit hides in forever: a
    /// mis-wound face is culled by the backface test and simply goes missing, and a corner
    /// off its face's plane bends the quad. Both are checked at every corner of every face.
    #[test]
    fn face_corners_are_wound_counter_clockwise() {
        for face in &FACES {
            let n = face.normal.as_vec3();
            let corners: Vec<Vec3> = face.corners.iter().copied().map(Vec3::from).collect();
            // A face is a side of the unit cube: the coordinate its normal names is 1 on
            // the positive faces and 0 on the negative ones, which is what `dot` reads.
            let plane = if face.normal.max_element() > 0 {
                1.0
            } else {
                0.0
            };
            for (i, &c) in corners.iter().enumerate() {
                assert_eq!(c.dot(n), plane, "corner {i} of face {n} is off its plane");
                let a = corners[(i + 1) % 4] - c;
                let b = corners[(i + 2) % 4] - c;
                assert!(
                    a.cross(b).dot(n) > 0.0,
                    "face {n} turns clockwise at corner {i}"
                );
            }
        }
    }

    #[test]
    fn unmeshed_chunk_is_none() {
        let w = World::new(5, []);
        assert!(build_chunk_mesh(&w, ChunkPos::new(9, 9)).is_none());
    }
}
