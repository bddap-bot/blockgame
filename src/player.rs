//! Player state and voxel collision.
//!
//! The player is an axis-aligned box that slides along blocks. Collision is resolved one
//! axis at a time — the cheapest scheme that still lets you walk up to a wall, strafe
//! along it, and not tunnel through the floor. The collision half is pure functions over
//! [`World`], so it is unit-testable without a window; `game::apply_intent` is what drives
//! it each frame, and what turns input into the `delta` handed to [`move_and_slide`].

use crate::registry::{Block, Fall};
use crate::vehicle::Ride;
use crate::world::{BlockPos, WORLD_HEIGHT, World};
use bevy::math::{BVec3, Vec3};
use bevy::prelude::Component;

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

/// Hearts a whole player has.
pub const MAX_HEALTH: f32 = 10.0;

/// Fastest a player may hit the ground at for free, in blocks per second.
///
/// Comfortably past what a jump lands at, so hopping about never costs a heart, and a hair
/// over what a four-block drop reaches — which is roughly the height a child builds to
/// before they think of it as a height at all.
pub const SAFE_LANDING_SPEED: f32 = 15.0;
const _: () = assert!(SAFE_LANDING_SPEED > JUMP_SPEED);

/// Hearts a landing costs per block-per-second it is over [`SAFE_LANDING_SPEED`].
///
/// Derived rather than chosen, so the worst fall there is costs exactly one player: nothing
/// slower than terminal can empty the bar, and terminal empties it with none to spare,
/// whatever either number is later changed to.
pub const HEARTS_PER_SPEED: f32 = MAX_HEALTH / (Fall::UNAIDED.max_speed - SAFE_LANDING_SPEED);

/// Hearts a player gets back per second just by carrying on.
///
/// Slow enough that a bad landing is felt for a while, fast enough that a bruised child is
/// never told to go and stand somewhere. Nothing else in the game heals, and nothing needs
/// to: this is the whole of it.
pub const HEAL_RATE: f32 = 0.5;

/// Seconds flat on your back after a fall that took the last heart.
///
/// The whole of what running out costs — no respawn, no lost pockets. A child who fell off
/// their own tower gets their breath back where they landed, which is also the only story
/// the host can tell: health is the falling player's own business, so a death that
/// teleported them would be a jump across the map that no peer could vouch for.
pub const WINDED_TIME: f32 = 2.0;

/// Slowest bounce worth having, in blocks per second.
///
/// Below it a landing simply stops, because a bounce of a few centimetres is not a bounce —
/// it is a player resting on a cushion twitching once per frame as gravity and the spring
/// trade the same sliver back and forth.
pub const BOUNCE_FLOOR: f32 = 2.0;

/// The column the world spawns everybody over. One pair of coordinates, so "where is spawn"
/// has a single answer.
pub const SPAWN_COLUMN: (i32, i32) = (8, 8);

/// Nudge used when snapping out of a block, so the player rests *just* clear of the
/// surface instead of exactly on it (where the next frame's floor test is a coin flip).
const SKIN: f32 = 1e-3;

/// Longest distance moved between collision tests. Must stay under one block so nothing
/// can be crossed unnoticed, and under the player's half-width so a wall is never skipped.
const MAX_STEP: f32 = 0.25;
const _: () = assert!(MAX_STEP < HALF_WIDTH && HALF_WIDTH < 1.0);

/// How the player is moving. Walking falls, jumps, and can be standing on something;
/// flying does none of those — so "grounded while flying" cannot be written down.
#[derive(Debug, Clone, Copy)]
pub enum Motion {
    Walking {
        /// Horizontal speed comes straight from input every frame, with no acceleration or
        /// friction to remember, so gravity and jumping are all this carries.
        velocity: Vec3,
        grounded: bool,
    },
    Flying,
}

/// How the player's body is doing.
///
/// Two states rather than a heart count and a `winded` flag beside it: "winded with seven
/// hearts left" and "up and about with none" are both nonsense, and neither can be written
/// down. Running out is not death — there is nothing to lose and nowhere to wake up — it is
/// a couple of seconds flat on your back, and then you are whole again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Condition {
    /// On their feet, with this many of [`MAX_HEALTH`] hearts left. Always above zero.
    Well { health: f32 },
    /// Flat on their back, with this many seconds of it left. Gravity still has them;
    /// nothing else does.
    Winded { left: f32 },
}

impl Condition {
    pub const WHOLE: Condition = Condition::Well { health: MAX_HEALTH };

    /// Can they walk, jump, or take off? The one question the movement code asks.
    pub fn on_their_feet(self) -> bool {
        matches!(self, Condition::Well { .. })
    }

    /// Takes `hearts`. Losing the last one puts them down, and being put down again while
    /// already down does nothing — a player bouncing off the floor of a pit must not be
    /// pinned there by the fall that started it.
    pub fn hurt(&mut self, hearts: f32) {
        if let Condition::Well { health } = self {
            *health -= hearts;
            if *health <= 0.0 {
                *self = Condition::Winded { left: WINDED_TIME };
            }
        }
    }

    /// A moment of getting better: breath back while down, hearts back once up. They come
    /// up whole, because half a bar and no way to see why is not something to explain to a
    /// six-year-old.
    pub fn recover(&mut self, dt: f32) {
        match self {
            Condition::Well { health } => *health = (*health + HEAL_RATE * dt).min(MAX_HEALTH),
            Condition::Winded { left } => {
                *left -= dt;
                if *left <= 0.0 {
                    *self = Condition::WHOLE;
                }
            }
        }
    }
}

/// What hitting the ground did.
pub struct Landing {
    /// Hearts it cost.
    pub hurt: f32,
    /// How fast it throws them back up, or zero for a landing that simply stops them.
    pub rebound: f32,
}

/// Coming down at `speed` blocks per second onto `ground`.
///
/// `ground` is what they came down on, or `None` for a stop against nothing to stand on —
/// clipping the corner of a ledge on the way past it. Both halves are decided here together
/// because they are the same fact about the block: what breaks a fall is what throws you
/// back up, so nothing can be springy and punishing at once.
pub fn land(speed: f32, ground: Option<Block>) -> Landing {
    let up = speed * ground.map_or(0.0, Block::bounce);
    Landing {
        // Soft ground costs nothing however far the fall was — that is the whole of what a
        // cushion is for.
        hurt: if ground.is_some_and(Block::soft) || speed <= SAFE_LANDING_SPEED {
            0.0
        } else {
            (speed - SAFE_LANDING_SPEED) * HEARTS_PER_SPEED
        },
        rebound: if up < BOUNCE_FLOOR { 0.0 } else { up },
    }
}

/// The local player. Position is the centre of the feet; the camera hangs at
/// [`EYE_HEIGHT`] above it.
#[derive(Component, Debug, Clone)]
pub struct Player {
    pub pos: Vec3,
    /// Radians, left-handed about +Y. 0 looks down -Z.
    pub yaw: f32,
    /// Radians. `input::PITCH_LIMIT` is what keeps it just shy of straight up/down, where
    /// the camera basis degenerates.
    pub pitch: f32,
    pub motion: Motion,
    /// Their car: pocketed, parked, or under them. While they are driving, `motion` is not
    /// consulted at all — the car is what moves, and `pos` is the seat it puts them in.
    pub ride: Ride,
    /// How the fall went. Local, and never sent: nobody else's game reads it, and the host
    /// has no way to check a claim about somebody else's knees.
    pub condition: Condition,
}

impl Player {
    pub fn spawn_at(pos: Vec3) -> Self {
        Self {
            pos,
            yaw: 0.0,
            pitch: 0.0,
            // Spawning in flight means a peer whose terrain hasn't generated yet doesn't
            // fall through the hole and out of the world.
            motion: Motion::Flying,
            // You start with nothing, a car included: it has to be dug up and built first.
            ride: Ride::Pocketed,
            condition: Condition::WHOLE,
        }
    }

    pub fn is_flying(&self) -> bool {
        matches!(self.motion, Motion::Flying)
    }

    pub fn is_driving(&self) -> bool {
        self.ride.is_driving()
    }

    /// Puts them on their feet, at rest. Getting into a car and getting out of one both go
    /// through here: a driver who took off flying would step out into the sky, and one who
    /// got in mid-fall would step out still carrying the speed of it.
    pub fn stand(&mut self) {
        self.motion = Motion::Walking {
            velocity: Vec3::ZERO,
            grounded: false,
        };
    }

    /// Switches between walking and flying. Landing starts from rest: keeping the velocity
    /// you had when you took off would drop you at whatever speed gravity last left there.
    pub fn toggle_fly(&mut self) {
        match self.motion {
            Motion::Flying => self.stand(),
            Motion::Walking { .. } => self.motion = Motion::Flying,
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

/// The player's box, standing with feet at `pos`, as `(min, max)`. The one description of
/// the player's geometry: collision and the "may a block go here" rule read the same box.
pub fn aabb(pos: Vec3) -> (Vec3, Vec3) {
    (
        pos - Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH),
        pos + Vec3::new(HALF_WIDTH, HEIGHT, HALF_WIDTH),
    )
}

/// Does a block at `pos` overlap the box of a player standing at `actor`? Placing there
/// would trap them inside the world.
pub fn would_trap(actor: Vec3, pos: BlockPos) -> bool {
    let (min, max) = aabb(actor);
    let b = pos.corner();
    (min.x < b.x + 1.0 && max.x > b.x)
        && (min.y < b.y + 1.0 && max.y > b.y)
        && (min.z < b.z + 1.0 && max.z > b.z)
}

/// What the player's box shares space with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    /// Free space.
    None,
    /// A block, in a chunk that is loaded — so it is really there.
    Solid,
    /// Terrain nobody has generated yet. It stops movement exactly like [`Overlap::Solid`]
    /// does, because [`World::block`] reads an unloaded chunk as air and walking into that
    /// is how you fall out of the world. It is not a block to be pushed out of, though:
    /// there is nothing there, only an answer we don't have yet.
    Unknown,
}

impl Overlap {
    /// Does this stop the player? Ungenerated terrain does, same as a block.
    pub fn blocks(self) -> bool {
        self != Overlap::None
    }
}

/// What the player's box, standing with feet at `pos`, is sharing space with.
pub fn overlap(world: &World, pos: Vec3) -> Overlap {
    let (min, max) = aabb(pos);
    let mut unknown = false;
    // `- SKIN` on the max: a box whose face lies exactly on a block boundary is touching,
    // not overlapping, and must not count as a collision.
    for y in min.y.floor() as i32..=(max.y - SKIN).floor() as i32 {
        for z in min.z.floor() as i32..=(max.z - SKIN).floor() as i32 {
            for x in min.x.floor() as i32..=(max.x - SKIN).floor() as i32 {
                let p = BlockPos::new(x, y, z);
                // Chunks are full-height columns, so above and below the world is air
                // whatever is loaded — there is no chunk there to be waiting for.
                if (0..WORLD_HEIGHT).contains(&y) && !world.is_loaded(p.chunk()) {
                    unknown = true;
                } else if world.solid(p) {
                    return Overlap::Solid;
                }
            }
        }
    }
    if unknown {
        Overlap::Unknown
    } else {
        Overlap::None
    }
}

/// What one move did.
pub struct Move {
    pub pos: Vec3,
    /// The axes the move was cut short on. The caller kills velocity on exactly these,
    /// rather than inferring a collision from how far the player got.
    pub blocked: BVec3,
    /// What they came to rest on, or `None` for standing on nothing.
    ///
    /// The block rather than a bare "grounded" flag, because a landing has to ask *what*
    /// it landed on and being able to answer that separately is how the two drift: a
    /// landing that hurt on ground the jump code thought was a cushion.
    pub ground: Option<Block>,
}

impl Move {
    /// Standing on something — and the only way to ask, so it cannot disagree with
    /// [`Move::ground`].
    pub fn grounded(&self) -> bool {
        self.ground.is_some()
    }
}

/// Moves the player box by `delta`, sliding along whatever it hits.
///
/// `pos` must not already be inside a block. A player who is anyway — a peer can place one
/// on you, since the placement rules only check the *placer's* own box — is pushed one
/// step toward the nearest way out instead of moving: the X/Z snap below resolves an
/// overlap by moving *backwards* into the block, and from inside one you cannot break your
/// way out either ([`crate::raycast::cast`] declines to target anything with the eye
/// inside a block). Being wedged there is otherwise permanent.
pub fn move_and_slide(world: &World, mut pos: Vec3, delta: Vec3) -> Move {
    match overlap(world, pos) {
        Overlap::None => {}
        // Not an assertion: the host checks placements against poses up to a round trip
        // old, so a player who steps into the block somebody is placing gets here through
        // no fault of anyone's code. Recovering beats crashing whoever it happened to.
        Overlap::Solid => {
            return Move {
                pos: push_out(world, pos),
                blocked: BVec3::TRUE,
                ground: None,
            };
        }
        // Ungenerated terrain is nothing to be pushed out of, and there is nowhere known
        // to move to. Hold still until the chunk arrives — this is every spawn's and every
        // joiner's first frames, and being flung by a snap against a block that may not
        // even be there is worse than waiting.
        Overlap::Unknown => {
            return Move {
                pos,
                blocked: BVec3::TRUE,
                ground: None,
            };
        }
    }

    // A step longer than a block can pass clean through one — the collision test only
    // looks at where you land, never at what you crossed. Splitting long moves into
    // sub-block steps is the whole tunnelling defence, and it is also what makes the
    // snap arithmetic below correct: after a step this short you are at most one block
    // deep into whatever you hit.
    let steps = (delta.length() / MAX_STEP).ceil().max(1.0) as u32;
    let step = delta / steps as f32;

    let mut blocked = BVec3::FALSE;
    for _ in 0..steps {
        let (moved, hit) = step_once(world, pos, step);
        pos = moved;
        blocked |= hit;
    }

    // Asked of where they ended up, not of where a sub-step snapped them: a fall that
    // clipped the corner of a roof and slid off it landed on nothing. Standing still on a
    // floor is grounded too, otherwise you couldn't jump twice from the same spot — and the
    // `delta.y` guard is what stops a rise too small to leave the skin reading as one.
    let ground = (delta.y <= 0.0).then(|| ground_under(world, pos)).flatten();
    Move {
        pos,
        blocked,
        ground,
    }
}

/// The block the player's box is resting on, or `None` if it is resting on nothing.
///
/// The *softest* of them when the box straddles several, which is the generous reading on
/// purpose: a child who got one boot onto the cushion aimed for the cushion.
fn ground_under(world: &World, pos: Vec3) -> Option<Block> {
    let (min, max) = aabb(pos - Vec3::Y * (SKIN * 2.0));
    let y = min.y.floor() as i32;
    let mut best: Option<Block> = None;
    for z in min.z.floor() as i32..=(max.z - SKIN).floor() as i32 {
        for x in min.x.floor() as i32..=(max.x - SKIN).floor() as i32 {
            let block = world.block(BlockPos::new(x, y, z));
            if block.solid() && best.is_none_or(|b: Block| block.bounce() > b.bounce()) {
                best = Some(block);
            }
        }
    }
    best
}

fn step_once(world: &World, mut pos: Vec3, delta: Vec3) -> (Vec3, BVec3) {
    let mut blocked = BVec3::FALSE;

    // Y first: landing on a floor before resolving X/Z means walking into a wall can't
    // shove you into the ground.
    if delta.y != 0.0 {
        let want = pos + Vec3::Y * delta.y;
        if overlap(world, want).blocks() {
            blocked.y = true;
            if delta.y < 0.0 {
                pos.y = want.y.floor() + 1.0 + SKIN;
            } else {
                pos.y = (want.y + HEIGHT).floor() - HEIGHT - SKIN;
            }
        } else {
            pos = want;
        }
    }

    // Then X and Z, each on its own: a wall stops the axis that runs into it and leaves
    // the other free, which is what sliding along a wall is.
    let slide = |pos: &mut Vec3, axis: Vec3, amount: f32| {
        if amount == 0.0 {
            return false;
        }
        let want = *pos + axis * amount;
        if !overlap(world, want).blocks() {
            *pos = want;
            return false;
        }
        let along = want.dot(axis);
        let snapped = if amount > 0.0 {
            (along + HALF_WIDTH).floor() - HALF_WIDTH - SKIN
        } else {
            (along - HALF_WIDTH).floor() + 1.0 + HALF_WIDTH + SKIN
        };
        *pos += axis * (snapped - along);
        true
    };
    blocked.x = slide(&mut pos, Vec3::X, delta.x);
    blocked.z = slide(&mut pos, Vec3::Z, delta.z);

    (pos, blocked)
}

/// One step toward the shallowest way out of the blocks a player is standing inside.
///
/// Only ever called when [`overlap`] just answered [`Overlap::Solid`], so there is at
/// least one block here to measure against — and never against [`Overlap::Unknown`], which
/// is not a thing to escape from.
fn push_out(world: &World, pos: Vec3) -> Vec3 {
    let (min, max) = aabb(pos);
    // The union of the blocks we are inside: measuring the way out against all of them at
    // once is what gets a player out of a two-block-thick wall rather than one hopeless
    // step deeper into it.
    let mut hull_min = Vec3::splat(f32::INFINITY);
    let mut hull_max = Vec3::splat(f32::NEG_INFINITY);
    for y in min.y.floor() as i32..=(max.y - SKIN).floor() as i32 {
        for z in min.z.floor() as i32..=(max.z - SKIN).floor() as i32 {
            for x in min.x.floor() as i32..=(max.x - SKIN).floor() as i32 {
                let p = BlockPos::new(x, y, z);
                if world.solid(p) {
                    hull_min = hull_min.min(p.corner());
                    hull_max = hull_max.max(p.corner() + Vec3::ONE);
                }
            }
        }
    }

    let mut ways = [
        Vec3::X * (hull_max.x - min.x + SKIN),
        Vec3::X * (hull_min.x - max.x - SKIN),
        Vec3::Y * (hull_max.y - min.y + SKIN),
        Vec3::Y * (hull_min.y - max.y - SKIN),
        Vec3::Z * (hull_max.z - min.z + SKIN),
        Vec3::Z * (hull_min.z - max.z - SKIN),
    ];
    ways.sort_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
    // Shallowest first, and out into space the player can actually stand in.
    if let Some(escape) = ways
        .into_iter()
        .find(|w| overlap(world, pos + *w) == Overlap::None)
    {
        return pos + escape;
    }
    // Buried, with no single step that reaches open space. Climbing is the one direction
    // that always ends somewhere standable, and it is monotone: taking the shallowest way
    // out instead swaps which side is shallowest each frame, and the player oscillates
    // between two points for ever — which is being stuck, with jitter.
    pos + Vec3::Y
}

/// Where everybody starts, above the terrain over [`SPAWN_COLUMN`].
pub fn spawn_point(world: &World) -> Vec3 {
    let (x, z) = SPAWN_COLUMN;
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
        let m = move_and_slide(&w, Vec3::new(8.5, 14.0, 8.5), Vec3::new(0.0, -5.0, 0.0));
        assert!(m.grounded() && m.blocked.y);
        assert!((m.pos.y - 11.0).abs() < 0.01, "landed at {}", m.pos.y);
        assert_eq!(overlap(&w, m.pos), Overlap::None);
    }

    #[test]
    fn standing_still_stays_grounded() {
        let w = floor_world();
        let m = move_and_slide(&w, Vec3::new(8.5, 11.0 + SKIN, 8.5), Vec3::ZERO);
        assert!(
            m.grounded(),
            "resting on the floor should read as grounded at y={}",
            m.pos.y
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
        let m = move_and_slide(&w, start, Vec3::new(3.0, 0.0, 2.0));
        assert_eq!(overlap(&w, m.pos), Overlap::None);
        assert_eq!(
            (m.blocked.x, m.blocked.z),
            (true, false),
            "the wall stops x and nothing else"
        );
        assert!(
            m.pos.x < 6.0 - HALF_WIDTH + 0.01,
            "walked into the wall: x={}",
            m.pos.x
        );
        assert!(
            (m.pos.z - (start.z + 2.0)).abs() < 0.01,
            "should have slid along z: {}",
            m.pos.z
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
        let m = move_and_slide(&w, Vec3::new(8.5, 11.0, 8.5), Vec3::new(0.0, 5.0, 0.0));
        assert!(m.blocked.y, "the head hit something");
        assert!(
            m.pos.y + HEIGHT <= 14.0,
            "head poked through the ceiling: {}",
            m.pos.y
        );
        assert_eq!(overlap(&w, m.pos), Overlap::None);
    }

    /// The classic voxel bug: a fast frame must not step the player through a thin floor.
    #[test]
    fn a_fast_fall_does_not_tunnel() {
        let w = floor_world();
        let m = move_and_slide(&w, Vec3::new(8.5, 12.0, 8.5), Vec3::new(0.0, -40.0, 0.0));
        assert!(m.grounded() && m.pos.y > 10.0, "tunnelled to y={}", m.pos.y);
    }

    /// Which axes a move was stopped on is the caller's cue to kill velocity. Deriving it
    /// instead from "did I end up where I asked" needs an epsilon, and flight has no
    /// altitude clamp: high enough, f32's own step across a dozen sub-steps exceeds any
    /// fixed epsilon, and gravity gets cancelled every frame by a floor that isn't there.
    #[test]
    fn a_high_move_that_hits_nothing_is_blocked_on_nothing() {
        let w = floor_world();
        let start = Vec3::new(8.5, 12_345.6, 8.5);
        let delta = Vec3::new(1.2, -2.9, 0.7);
        let m = move_and_slide(&w, start, delta);
        assert!(!m.blocked.any(), "nothing up here to hit");
        assert!(!m.grounded());
        assert!(
            (m.pos.y - (start.y + delta.y)).abs() > 1e-4,
            "the sub-steps really do lose more than an epsilon: {}",
            m.pos.y
        );
    }

    /// An unloaded chunk reads as air, so a collision test that trusts it walks the player
    /// off the edge of the generated world.
    #[test]
    fn ungenerated_terrain_is_not_a_hole_to_walk_into() {
        let w = floor_world();
        let m = move_and_slide(&w, Vec3::new(14.0, 11.0, 8.5), Vec3::new(4.0, 0.0, 0.0));
        assert!(m.blocked.x);
        assert!(
            m.pos.x + HALF_WIDTH <= 16.0,
            "left the generated world at x={}",
            m.pos.x
        );
    }

    /// A fresh spawn — and every joining peer — exists for a few frames before its chunk
    /// does. Unloaded terrain blocks movement, but it must not be *snapped* against: there
    /// is no block there to land on or be shoved out of.
    #[test]
    fn a_player_over_ungenerated_terrain_waits_where_it_is() {
        let w = World::new(1, []);
        let start = Vec3::new(8.5, 50.0, 8.5);
        let m = move_and_slide(&w, start, Vec3::new(0.0, -3.0, 0.0));
        assert_eq!(m.pos, start, "neither fell nor teleported");
        assert!(m.blocked.y, "so the caller stops piling up gravity");
        assert!(!m.grounded(), "there is nothing known to stand on");
    }

    /// `would_trap` only guards the box of the player doing the placing, so a peer can
    /// build a block into you. From inside one, the X/Z snap moves you further in and
    /// `raycast::cast` refuses to target anything — without a push-out, that is a restart.
    #[test]
    fn a_block_placed_inside_you_pushes_you_out() {
        let mut w = floor_world();
        let feet = Vec3::new(8.5, 11.0, 8.5);
        assert_eq!(overlap(&w, feet), Overlap::None);

        w.set_block(BlockPos::new(8, 11, 8), Block::Stone);
        assert_eq!(overlap(&w, feet), Overlap::Solid);

        let out = push_out(&w, feet);
        assert_eq!(overlap(&w, out), Overlap::None, "still stuck at {out}");
        assert!(
            (out - feet).length() < 1.5,
            "a step out, not a teleport: {out}"
        );
    }

    /// Buried, with no single step to open space — a joiner spawning inside somebody's
    /// build. Pushing out the shallowest way swaps sides every frame and leaves them
    /// oscillating between two points for ever, which is stuck with jitter; climbing gets
    /// them out. Nothing else can: from inside a block the raycast has nothing to target,
    /// so they cannot dig their way free either.
    #[test]
    fn a_buried_player_climbs_out() {
        let mut w = floor_world();
        for y in 11..20 {
            for z in 6..11 {
                for x in 6..11 {
                    w.set_block(BlockPos::new(x, y, z), Block::Stone);
                }
            }
        }
        let mut pos = Vec3::new(8.5, 14.0, 8.5);
        assert_eq!(
            overlap(&w, pos),
            Overlap::Solid,
            "the test needs them stuck"
        );

        // One frame of standing still, repeatedly — the game's own loop.
        for _ in 0..40 {
            pos = move_and_slide(&w, pos, Vec3::ZERO).pos;
            if overlap(&w, pos) == Overlap::None {
                return;
            }
        }
        panic!("never got out; ended at {pos}");
    }

    #[test]
    fn placing_a_block_inside_yourself_is_refused() {
        let feet = Vec3::new(8.5, 10.0, 8.5);
        assert!(would_trap(feet, BlockPos::new(8, 10, 8)), "feet");
        assert!(would_trap(feet, BlockPos::new(8, 11, 8)), "head");
        assert!(
            !would_trap(feet, BlockPos::new(8, 9, 8)),
            "the floor is fine"
        );
        assert!(
            !would_trap(feet, BlockPos::new(8, 12, 8)),
            "above the head is fine"
        );
        assert!(!would_trap(feet, BlockPos::new(7, 10, 8)), "beside is fine");
    }

    /// How far you can drop before it costs anything, and what the drops past that cost.
    ///
    /// Heights rather than speeds, because a height is the thing a child actually chooses:
    /// they build a tower and jump off it. `sqrt(2gh)` is what a drop of `h` arrives at.
    fn dropping(blocks: f32) -> f32 {
        (2.0 * GRAVITY * blocks).sqrt()
    }

    #[test]
    fn a_short_drop_costs_nothing_and_a_long_one_costs_the_lot() {
        let onto_grass = |speed| land(speed, Some(Block::Grass)).hurt;
        assert_eq!(onto_grass(JUMP_SPEED), 0.0, "a hop");
        assert_eq!(onto_grass(dropping(4.0)), 0.0);
        assert!(onto_grass(dropping(5.0)) > 0.0, "five blocks is a fall");

        // Gentle: the height a child builds a tower to costs a fifth of them.
        let ten = onto_grass(dropping(10.0));
        assert!(
            (1.0..3.0).contains(&ten),
            "a ten-block drop cost {ten} hearts"
        );

        // And the worst fall there is costs exactly one player, with none to spare.
        let worst = onto_grass(Fall::UNAIDED.max_speed);
        assert!((worst - MAX_HEALTH).abs() < 1e-4, "terminal cost {worst}");
    }

    /// The cushion, in one assertion: however far you fell, landing on one costs nothing,
    /// and it throws you back up by less than it took.
    #[test]
    fn a_cushion_takes_the_whole_of_a_landing_and_gives_some_back() {
        for blocks in [5.0, 20.0, 100.0] {
            let it = land(dropping(blocks), Some(Block::Cushion));
            assert_eq!(it.hurt, 0.0, "{blocks} blocks onto a cushion");
            assert!(
                it.rebound > BOUNCE_FLOOR && it.rebound < dropping(blocks),
                "{blocks} blocks bounced back at {}",
                it.rebound
            );
        }

        let stone = land(dropping(20.0), Some(Block::Stone));
        assert!(stone.hurt > 0.0 && stone.rebound == 0.0, "stone is stone");
        assert_eq!(land(dropping(20.0), None).rebound, 0.0, "nothing to hit");
        assert_eq!(
            land(BOUNCE_FLOOR, Some(Block::Cushion)).rebound,
            0.0,
            "resting on a cushion is resting, not twitching"
        );
    }

    /// Running out is a couple of seconds on your back and then a whole player again — no
    /// respawn, nothing dropped. A second bad landing while down must not extend it, or a
    /// child who fell into a pit never gets up.
    #[test]
    fn running_out_of_hearts_is_a_breather_not_a_death() {
        let mut c = Condition::WHOLE;
        c.hurt(MAX_HEALTH - 1.0);
        assert!(c.on_their_feet(), "one heart is still standing");

        c.hurt(1.0);
        assert_eq!(c, Condition::Winded { left: WINDED_TIME });
        assert!(!c.on_their_feet());

        c.recover(WINDED_TIME / 2.0);
        c.hurt(MAX_HEALTH);
        assert_eq!(
            c,
            Condition::Winded {
                left: WINDED_TIME / 2.0
            },
            "hitting somebody who is already down does nothing"
        );

        c.recover(WINDED_TIME);
        assert_eq!(c, Condition::WHOLE, "up again, and whole");
    }

    /// Hearts come back on their own, and stop at full.
    #[test]
    fn a_bruise_heals_by_itself() {
        let mut c = Condition::WHOLE;
        c.hurt(4.0);
        c.recover(1.0);
        assert_eq!(c, Condition::Well { health: 6.5 });
        c.recover(1_000.0);
        assert_eq!(c, Condition::WHOLE, "and no further");
    }

    /// The whole point of the parachute: with one in hand there is no drop left in the
    /// world that costs a heart. Pinned against the damage rule rather than against the
    /// number in the table, so lowering `SAFE_LANDING_SPEED` cannot quietly make the
    /// parachute stop working.
    #[test]
    fn a_parachute_makes_every_fall_survivable() {
        let canopy = crate::registry::Item::Parachute.falling();
        assert!(
            canopy.max_speed < Fall::UNAIDED.max_speed,
            "it slows a fall"
        );
        assert_eq!(land(canopy.max_speed, Some(Block::Stone)).hurt, 0.0);
        assert!(canopy.drift > Fall::UNAIDED.drift, "and it steers");
    }

    /// A fall that clipped the corner of a roof and slid off it landed on nothing: asking
    /// where they *ended up* is what keeps "grounded" and "what am I standing on" one
    /// question with one answer.
    #[test]
    fn sliding_off_the_edge_of_what_you_hit_is_not_standing_on_it() {
        // A single block of ledge in an otherwise empty sky.
        let mut w = floor_world();
        for z in 0..crate::world::CHUNK_SIZE {
            for x in 0..crate::world::CHUNK_SIZE {
                w.set_block(BlockPos::new(x, 10, z), Block::Air);
            }
        }
        w.set_block(BlockPos::new(8, 10, 8), Block::Stone);

        let straight = move_and_slide(&w, Vec3::new(8.5, 11.6, 8.5), Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(straight.ground, Some(Block::Stone), "onto the ledge");

        let sideways = move_and_slide(&w, Vec3::new(8.5, 11.6, 8.5), Vec3::new(-1.0, -1.0, 0.0));
        assert!(sideways.blocked.y, "it clipped the ledge on the way past");
        assert_eq!(sideways.ground, None, "and then slid off it");
    }

    /// Which block a landing is measured against, when the box straddles more than one.
    #[test]
    fn one_boot_on_the_cushion_is_a_soft_landing() {
        let mut w = floor_world();
        w.set_block(BlockPos::new(8, 10, 8), Block::Cushion);
        // Feet on the seam: the box spans the cushion and the stone beside it.
        let m = move_and_slide(&w, Vec3::new(9.0, 12.0, 8.5), Vec3::new(0.0, -2.0, 0.0));
        assert_eq!(
            m.ground,
            Some(Block::Cushion),
            "the softest thing under you"
        );
    }

    #[test]
    fn flying_and_grounded_are_not_both_a_thing() {
        let mut p = Player::spawn_at(Vec3::ZERO);
        assert!(p.is_flying());
        p.toggle_fly();
        let Motion::Walking { velocity, grounded } = p.motion else {
            panic!("toggling flight off should leave the player walking");
        };
        assert_eq!(
            (velocity, grounded),
            (Vec3::ZERO, false),
            "landing from rest"
        );
        p.toggle_fly();
        assert!(p.is_flying());
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
