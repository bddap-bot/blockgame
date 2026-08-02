//! Player state and voxel collision.
//!
//! The player is an axis-aligned box that slides along blocks. Collision is resolved one
//! axis at a time — the cheapest scheme that still lets you walk up to a wall, strafe
//! along it, and not tunnel through the floor. All of it is pure functions over
//! [`World`], so it is unit-testable without a window.

use crate::world::{BlockPos, World};
use bevy::math::Vec3;
use bevy::prelude::{Component, Resource};

/// Half the player's width and depth, in blocks.
pub const HALF_WIDTH: f32 = 0.3;
/// Player box height, in blocks.
pub const HEIGHT: f32 = 1.8;
/// Camera height above the player's feet.
pub const EYE_HEIGHT: f32 = 1.62;

pub const WALK_SPEED: f32 = 5.2;
pub const SPRINT_SPEED: f32 = 8.5;
pub const FLY_SPEED: f32 = 14.0;
pub const JUMP_SPEED: f32 = 8.4;
pub const GRAVITY: f32 = 26.0;
/// Terminal fall speed. Parachutes (see `design/requirements-2.jpg`) will want to scale
/// this while equipped — that is why it is a constant here and not inlined.
pub const MAX_FALL_SPEED: f32 = 55.0;

/// Nudge used when snapping out of a block, so the player rests *just* clear of the
/// surface instead of exactly on it (where the next frame's floor test is a coin flip).
const SKIN: f32 = 1e-3;

/// Longest distance moved between collision tests. Must stay under one block so nothing
/// can be crossed unnoticed, and under the player's half-width so a wall is never skipped.
const MAX_STEP: f32 = 0.25;

/// The local player. Position is the centre of the feet; the camera hangs at
/// [`EYE_HEIGHT`] above it.
#[derive(Component, Debug, Clone)]
pub struct Player {
    pub pos: Vec3,
    pub velocity: Vec3,
    /// Radians, left-handed about +Y. 0 looks down -Z.
    pub yaw: f32,
    /// Radians, clamped just shy of straight up/down so the camera can't gimbal-flip.
    pub pitch: f32,
    pub grounded: bool,
    pub flying: bool,
}

impl Player {
    pub fn spawn_at(pos: Vec3) -> Self {
        Self {
            pos,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            grounded: false,
            // Spawning in flight means a peer whose terrain hasn't meshed yet doesn't fall
            // through the hole and out of the world.
            flying: true,
        }
    }

    pub fn eye(&self) -> Vec3 {
        self.pos + Vec3::Y * EYE_HEIGHT
    }

    /// Unit vector the camera looks along.
    pub fn look_dir(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(-sy * cp, sp, -cy * cp)
    }

    /// Horizontal basis for movement: `(forward, right)`, both flat on the XZ plane.
    pub fn move_basis(&self) -> (Vec3, Vec3) {
        let (sy, cy) = self.yaw.sin_cos();
        (Vec3::new(-sy, 0.0, -cy), Vec3::new(cy, 0.0, -sy))
    }
}

/// Which hotbar slot the local player is holding. Separate from [`Player`] because it is
/// the seam the item registry grows into (an inventory replaces this resource, nothing
/// else).
#[derive(Resource, Debug, Clone, Copy)]
pub struct Held(pub crate::registry::Item);

impl Default for Held {
    fn default() -> Self {
        Held(crate::registry::Item::from_slot(0))
    }
}

/// Is the player's box, standing with feet at `pos`, inside any solid block?
pub fn intersects_world(world: &World, pos: Vec3) -> bool {
    let min = pos - Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH);
    let max = pos + Vec3::new(HALF_WIDTH, HEIGHT, HALF_WIDTH);
    // `- SKIN` on the max: a box whose face lies exactly on a block boundary is touching,
    // not overlapping, and must not count as a collision.
    for y in min.y.floor() as i32..=(max.y - SKIN).floor() as i32 {
        for z in min.z.floor() as i32..=(max.z - SKIN).floor() as i32 {
            for x in min.x.floor() as i32..=(max.x - SKIN).floor() as i32 {
                if world.solid(BlockPos::new(x, y, z)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Moves the player box by `delta`, sliding along whatever it hits.
///
/// Returns the new position and whether the player ended up standing on something.
pub fn move_and_slide(world: &World, mut pos: Vec3, delta: Vec3) -> (Vec3, bool) {
    // A step longer than a block can pass clean through one — the collision test only
    // looks at where you land, never at what you crossed. Splitting long moves into
    // sub-block steps is the whole tunnelling defence, and it is also what makes the
    // snap arithmetic below correct: after a step this short you are at most one block
    // deep into whatever you hit.
    let steps = (delta.length() / MAX_STEP).ceil().max(1.0) as u32;
    let step = delta / steps as f32;

    let mut landed = false;
    for _ in 0..steps {
        (pos, landed) = step_once(world, pos, step);
    }

    // Standing still on a floor still counts as grounded, otherwise you couldn't jump
    // twice from the same spot.
    let grounded =
        landed || (delta.y <= 0.0 && intersects_world(world, pos - Vec3::Y * (SKIN * 2.0)));
    (pos, grounded)
}

fn step_once(world: &World, mut pos: Vec3, delta: Vec3) -> (Vec3, bool) {
    let mut grounded = false;

    // Y first: landing on a floor before resolving X/Z means walking into a wall can't
    // shove you into the ground.
    if delta.y != 0.0 {
        let want = pos + Vec3::Y * delta.y;
        if intersects_world(world, want) {
            if delta.y < 0.0 {
                pos.y = want.y.floor() + 1.0 + SKIN;
                grounded = true;
            } else {
                pos.y = (want.y + HEIGHT).floor() - HEIGHT - SKIN;
            }
        } else {
            pos = want;
        }
    }

    for (axis, amount) in [(Vec3::X, delta.x), (Vec3::Z, delta.z)] {
        if amount == 0.0 {
            continue;
        }
        let want = pos + axis * amount;
        if intersects_world(world, want) {
            let along = want.dot(axis);
            let snapped = if amount > 0.0 {
                (along + HALF_WIDTH).floor() - HALF_WIDTH - SKIN
            } else {
                (along - HALF_WIDTH).floor() + 1.0 + HALF_WIDTH + SKIN
            };
            pos += axis * (snapped - along);
        } else {
            pos = want;
        }
    }

    (pos, grounded)
}

/// A spawn point above the terrain at `(x, z)`, in world space.
pub fn spawn_point(world: &World, x: i32, z: i32) -> Vec3 {
    let ground = world.ground_height(x, z);
    Vec3::new(x as f32 + 0.5, ground as f32 + 12.0, z as f32 + 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Block;
    use crate::world::ChunkPos;

    /// A world with one solid floor slab at y=10 across chunk (0,0), nothing else.
    fn floor_world() -> World {
        let mut w = World::new(1, []);
        let cp = ChunkPos::new(0, 0);
        w.load_chunk(cp);
        for y in 0..crate::world::WORLD_HEIGHT {
            for z in 0..crate::world::CHUNK_SIZE {
                for x in 0..crate::world::CHUNK_SIZE {
                    let b = if y == 10 { Block::Stone } else { Block::Air };
                    w.set_block(BlockPos::new(x, y, z), b);
                }
            }
        }
        w
    }

    #[test]
    fn falling_lands_on_the_floor() {
        let w = floor_world();
        let (pos, grounded) =
            move_and_slide(&w, Vec3::new(8.5, 14.0, 8.5), Vec3::new(0.0, -5.0, 0.0));
        assert!(grounded);
        assert!((pos.y - 11.0).abs() < 0.01, "landed at {}", pos.y);
        assert!(!intersects_world(&w, pos));
    }

    #[test]
    fn standing_still_stays_grounded() {
        let w = floor_world();
        let (pos, grounded) = move_and_slide(&w, Vec3::new(8.5, 11.0 + SKIN, 8.5), Vec3::ZERO);
        assert!(
            grounded,
            "resting on the floor should read as grounded at y={}",
            pos.y
        );
    }

    #[test]
    fn a_wall_stops_horizontal_motion_but_allows_sliding() {
        let mut w = floor_world();
        for y in 11..14 {
            for z in 0..crate::world::CHUNK_SIZE {
                w.set_block(BlockPos::new(6, y, z), Block::Stone);
            }
        }
        let start = Vec3::new(5.0, 11.0, 8.5);
        let (pos, _) = move_and_slide(&w, start, Vec3::new(3.0, 0.0, 2.0));
        assert!(!intersects_world(&w, pos));
        assert!(
            pos.x < 6.0 - HALF_WIDTH + 0.01,
            "walked into the wall: x={}",
            pos.x
        );
        assert!(
            (pos.z - (start.z + 2.0)).abs() < 0.01,
            "should have slid along z: {}",
            pos.z
        );
    }

    #[test]
    fn a_ceiling_stops_a_jump() {
        let mut w = floor_world();
        for z in 0..crate::world::CHUNK_SIZE {
            for x in 0..crate::world::CHUNK_SIZE {
                w.set_block(BlockPos::new(x, 14, z), Block::Stone);
            }
        }
        let (pos, _) = move_and_slide(&w, Vec3::new(8.5, 11.0, 8.5), Vec3::new(0.0, 5.0, 0.0));
        assert!(
            pos.y + HEIGHT <= 14.0,
            "head poked through the ceiling: {}",
            pos.y
        );
        assert!(!intersects_world(&w, pos));
    }

    /// The classic voxel bug: a fast frame must not step the player through a thin floor.
    #[test]
    fn a_fast_fall_does_not_tunnel() {
        let w = floor_world();
        let (pos, grounded) =
            move_and_slide(&w, Vec3::new(8.5, 12.0, 8.5), Vec3::new(0.0, -40.0, 0.0));
        assert!(grounded && pos.y > 10.0, "tunnelled to y={}", pos.y);
    }

    #[test]
    fn look_dir_is_unit_and_faces_negative_z_at_rest() {
        let p = Player::spawn_at(Vec3::ZERO);
        let d = p.look_dir();
        assert!((d.length() - 1.0).abs() < 1e-5);
        assert!((d - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
    }

    #[test]
    fn move_basis_is_orthonormal_and_flat() {
        let mut p = Player::spawn_at(Vec3::ZERO);
        p.yaw = 1.1;
        p.pitch = -0.7;
        let (f, r) = p.move_basis();
        assert!((f.length() - 1.0).abs() < 1e-5 && (r.length() - 1.0).abs() < 1e-5);
        assert!(
            f.dot(r).abs() < 1e-5,
            "forward and right should be perpendicular"
        );
        assert_eq!(
            (f.y, r.y),
            (0.0, 0.0),
            "movement must stay flat regardless of pitch"
        );
    }
}
