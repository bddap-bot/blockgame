//! Voxel raycasting — what block is the player looking at, and which face of it.
//!
//! Amanatides & Woo grid traversal: step to the next voxel boundary along whichever axis
//! is nearest, so every voxel the ray passes through is visited exactly once, in order.

use crate::world::{BlockPos, World};
use bevy::math::{IVec3, Vec3};

/// The first solid block a ray meets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RayHit {
    /// The block that was hit.
    pub block: BlockPos,
    /// The face it was hit on, as an outward unit normal. `block + normal` is the empty
    /// cell in front of that face — where a placed block goes.
    pub normal: IVec3,
}

impl RayHit {
    /// The empty cell against the hit face.
    pub fn adjacent(self) -> BlockPos {
        self.block.offset(self.normal)
    }
}

/// Walks `dir` from `origin` for at most `max_dist` blocks, returning the first solid
/// block hit. `dir` need not be normalized; `max_dist` is measured in world units.
pub fn cast(world: &World, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }

    let mut voxel = BlockPos::containing(origin);
    if world.solid(voxel) {
        // Inside a block already (a peer built into us) — nothing sensible to target.
        return None;
    }

    let step = IVec3::new(
        dir.x.signum() as i32,
        dir.y.signum() as i32,
        dir.z.signum() as i32,
    );
    // Distance along the ray between successive crossings of each axis' grid planes.
    let delta = Vec3::new(
        if dir.x == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dir.x).abs()
        },
        if dir.y == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dir.y).abs()
        },
        if dir.z == 0.0 {
            f32::INFINITY
        } else {
            (1.0 / dir.z).abs()
        },
    );
    // Distance along the ray to the first crossing on each axis.
    let mut next = Vec3::new(
        axis_first_crossing(origin.x, dir.x, delta.x),
        axis_first_crossing(origin.y, dir.y, delta.y),
        axis_first_crossing(origin.z, dir.z, delta.z),
    );

    loop {
        // Advance along whichever axis crosses a boundary soonest.
        let normal = if next.x <= next.y && next.x <= next.z {
            if next.x > max_dist {
                return None;
            }
            voxel.x += step.x;
            next.x += delta.x;
            IVec3::new(-step.x, 0, 0)
        } else if next.y <= next.z {
            if next.y > max_dist {
                return None;
            }
            voxel.y += step.y;
            next.y += delta.y;
            IVec3::new(0, -step.y, 0)
        } else {
            if next.z > max_dist {
                return None;
            }
            voxel.z += step.z;
            next.z += delta.z;
            IVec3::new(0, 0, -step.z)
        };

        if world.solid(voxel) {
            return Some(RayHit {
                block: voxel,
                normal,
            });
        }
    }
}

fn axis_first_crossing(origin: f32, dir: f32, delta: f32) -> f32 {
    if dir == 0.0 {
        return f32::INFINITY;
    }
    let frac = origin - origin.floor();
    let to_boundary = if dir > 0.0 { 1.0 - frac } else { frac };
    to_boundary * delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Block;
    use crate::world::{CHUNK_SIZE, ChunkPos, WORLD_HEIGHT};

    fn empty_world() -> World {
        let mut w = World::new(1, []);
        w.load_chunk(ChunkPos::new(0, 0));
        for y in 0..WORLD_HEIGHT {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    w.set_block(BlockPos::new(x, y, z), Block::Air);
                }
            }
        }
        w
    }

    #[test]
    fn hits_the_near_face_of_a_block() {
        let mut w = empty_world();
        w.set_block(BlockPos::new(8, 10, 8), Block::Stone);
        let hit = cast(&w, Vec3::new(4.5, 10.5, 8.5), Vec3::X, 20.0).expect("should hit");
        assert_eq!(hit.block, BlockPos::new(8, 10, 8));
        assert_eq!(hit.normal, IVec3::new(-1, 0, 0));
        assert_eq!(hit.adjacent(), BlockPos::new(7, 10, 8));
    }

    #[test]
    fn respects_max_distance() {
        let mut w = empty_world();
        w.set_block(BlockPos::new(12, 10, 8), Block::Stone);
        assert!(cast(&w, Vec3::new(4.5, 10.5, 8.5), Vec3::X, 3.0).is_none());
        assert!(cast(&w, Vec3::new(4.5, 10.5, 8.5), Vec3::X, 20.0).is_some());
    }

    #[test]
    fn hits_a_floor_looking_down() {
        let mut w = empty_world();
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                w.set_block(BlockPos::new(x, 5, z), Block::Grass);
            }
        }
        let hit = cast(
            &w,
            Vec3::new(8.5, 9.0, 8.5),
            Vec3::new(0.0, -1.0, 0.0),
            10.0,
        )
        .unwrap();
        assert_eq!(hit.block, BlockPos::new(8, 5, 8));
        assert_eq!(hit.normal, IVec3::new(0, 1, 0));
        assert_eq!(hit.adjacent(), BlockPos::new(8, 6, 8));
    }

    /// A diagonal ray must still report the face it actually entered through, or placed
    /// blocks land inside walls.
    #[test]
    fn diagonal_rays_report_the_entry_face() {
        let mut w = empty_world();
        w.set_block(BlockPos::new(8, 10, 8), Block::Stone);
        let hit = cast(
            &w,
            Vec3::new(6.1, 10.5, 6.1),
            Vec3::new(1.0, 0.0, 1.0),
            20.0,
        )
        .unwrap();
        assert_eq!(hit.block, BlockPos::new(8, 10, 8));
        assert!(
            hit.normal == IVec3::new(-1, 0, 0) || hit.normal == IVec3::new(0, 0, -1),
            "entry face was {:?}",
            hit.normal
        );
        assert!(!w.solid(hit.adjacent()), "the place target must be empty");
    }

    #[test]
    fn empty_space_hits_nothing() {
        let w = empty_world();
        assert!(cast(&w, Vec3::new(8.5, 40.0, 8.5), Vec3::X, 50.0).is_none());
    }

    #[test]
    fn a_zero_direction_hits_nothing() {
        let w = empty_world();
        assert!(cast(&w, Vec3::new(8.5, 40.0, 8.5), Vec3::ZERO, 50.0).is_none());
    }
}
