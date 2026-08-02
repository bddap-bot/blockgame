//! The voxel world: chunk storage, seed-deterministic terrain, and the one block-edit
//! entry point.
//!
//! Terrain is a pure function of `(seed, x, z)`. That is what lets multiplayer ship a
//! `u64` instead of megabytes of chunks: every peer generates the identical world and only
//! *edits* travel the wire (see [`crate::net::wire`]). Nothing in here may read the clock,
//! a random generator, or anything else a peer can't reproduce.

use crate::registry::Block;
use bevy::math::{IVec3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Chunk footprint in blocks (square in X/Z). Chunks are full-height columns — one mesh
/// per column keeps the bones simple; vertical chunking is a later optimization.
pub const CHUNK_SIZE: i32 = 16;
/// World ceiling. Terrain tops out well below this so there is room to build.
pub const WORLD_HEIGHT: i32 = 96;

/// Blocks in one chunk. Also the ceiling on a chunk's edit overlay — one edit per block —
/// which is what bounds the size of a world transfer (see [`crate::net::wire`]).
pub const BLOCKS_PER_CHUNK: usize = (CHUNK_SIZE * WORLD_HEIGHT * CHUNK_SIZE) as usize;

/// A block coordinate in world space. `y` is up.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn offset(self, d: IVec3) -> Self {
        Self::new(self.x + d.x, self.y + d.y, self.z + d.z)
    }

    /// The block containing a world-space point.
    pub fn containing(p: Vec3) -> Self {
        Self::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }

    /// The block's minimum corner in world space.
    pub fn corner(self) -> Vec3 {
        Vec3::new(self.x as f32, self.y as f32, self.z as f32)
    }

    pub fn center(self) -> Vec3 {
        self.corner() + Vec3::splat(0.5)
    }

    pub fn chunk(self) -> ChunkPos {
        ChunkPos::new(self.x.div_euclid(CHUNK_SIZE), self.z.div_euclid(CHUNK_SIZE))
    }
}

/// A chunk column's coordinate, in chunks (multiply by [`CHUNK_SIZE`] for blocks).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// World-space block coordinate of this chunk's `(0, 0, 0)` corner.
    pub fn origin(self) -> BlockPos {
        BlockPos::new(self.x * CHUNK_SIZE, 0, self.z * CHUNK_SIZE)
    }
}

/// One full-height column of blocks, generated from the seed and then mutated by edits.
pub struct Chunk {
    blocks: Vec<Block>,
}

impl Chunk {
    fn index(x: i32, y: i32, z: i32) -> usize {
        debug_assert!((0..CHUNK_SIZE).contains(&x) && (0..CHUNK_SIZE).contains(&z));
        debug_assert!((0..WORLD_HEIGHT).contains(&y));
        ((y * CHUNK_SIZE + z) * CHUNK_SIZE + x) as usize
    }

    /// Local-coordinate read. `x`/`z` in `0..CHUNK_SIZE`, `y` in `0..WORLD_HEIGHT`.
    pub fn get(&self, x: i32, y: i32, z: i32) -> Block {
        self.blocks[Self::index(x, y, z)]
    }

    fn set(&mut self, x: i32, y: i32, z: i32, b: Block) {
        self.blocks[Self::index(x, y, z)] = b;
    }
}

/// How one block differs from what worldgen put there.
///
/// The generated block is remembered alongside the current one so that putting a block
/// back the way it was *removes* the edit instead of recording a second one: a player who
/// digs a hole and fills it in again leaves the world exactly as they found it, and the
/// log exactly as long. Without that, every no-op edit is permanent.
#[derive(Clone, Copy)]
struct Overlay {
    block: Block,
    /// `None` until this chunk has been generated once. A joining peer is handed edits
    /// for chunks it has never loaded, so the baseline is not knowable yet;
    /// [`World::load_chunk`] fills it in at the moment worldgen runs.
    generated: Option<Block>,
}

/// The whole world: loaded chunks plus the edits made to them.
///
/// The edits are the only *world* state multiplayer replicates — player poses travel too,
/// but they are not the world. A late joiner gets the seed and the edits and reconstructs
/// every block.
///
/// Edits are bucketed by chunk, which is what keeps two costs off the critical path:
/// loading a chunk reads only that chunk's overlay rather than replaying every edit ever
/// made, and the world travels one bounded frame per chunk instead of one frame that
/// grows until it cannot be sent at all.
pub struct World {
    seed: u64,
    chunks: HashMap<ChunkPos, Chunk>,
    edits: HashMap<ChunkPos, HashMap<BlockPos, Overlay>>,
}

impl World {
    pub fn new(seed: u64, edits: impl IntoIterator<Item = (BlockPos, Block)>) -> Self {
        let mut world = Self {
            seed,
            chunks: HashMap::new(),
            edits: HashMap::new(),
        };
        // Through the same door as every other edit: a peer's `Welcome` is untrusted
        // input, and `set_block` is where an out-of-bounds one is refused.
        for (pos, block) in edits {
            world.set_block(pos, block);
        }
        world
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The world's edits, one batch per chunk — the shape it travels in.
    ///
    /// A batch holds at most one edit per block of one chunk, so it has a size ceiling
    /// ([`BLOCKS_PER_CHUNK`]) that a single whole-world message does not.
    pub fn overlays(&self) -> Vec<Vec<(BlockPos, Block)>> {
        self.edits
            .values()
            .map(|chunk| chunk.iter().map(|(p, o)| (*p, o.block)).collect())
            .collect()
    }

    pub fn is_loaded(&self, cp: ChunkPos) -> bool {
        self.chunks.contains_key(&cp)
    }

    /// Every chunk currently in memory. This is the set unloading must be driven from:
    /// a chunk can be generated and never drawn, so the render side does not know them
    /// all.
    pub fn loaded_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks.keys().copied()
    }

    pub fn chunk(&self, cp: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&cp)
    }

    /// Generates the chunk if it isn't loaded yet. Idempotent.
    ///
    /// Costs one worldgen pass plus this chunk's own overlay — never the whole world's
    /// edits, which would make each load slower as the world was built in and make a
    /// well-used world stall on every step the player takes.
    pub fn load_chunk(&mut self, cp: ChunkPos) {
        if self.chunks.contains_key(&cp) {
            return;
        }
        let mut chunk = generate_chunk(self.seed, cp);
        if let Some(overlay) = self.edits.get_mut(&cp) {
            // First sight of what worldgen makes here: learn each baseline, and drop the
            // edits that turn out to ask for what is already there. That is where a
            // joiner's log gets pruned, since it arrives with no baselines at all.
            overlay.retain(|pos, o| {
                let (lx, lz) = (pos.x.rem_euclid(CHUNK_SIZE), pos.z.rem_euclid(CHUNK_SIZE));
                let generated = chunk.get(lx, pos.y, lz);
                o.generated = Some(generated);
                if o.block == generated {
                    return false;
                }
                chunk.set(lx, pos.y, lz, o.block);
                true
            });
            if overlay.is_empty() {
                self.edits.remove(&cp);
            }
        }
        self.chunks.insert(cp, chunk);
    }

    pub fn unload_chunk(&mut self, cp: ChunkPos) {
        self.chunks.remove(&cp);
    }

    /// Reads a block. Out of vertical bounds, or in an unloaded chunk, reads as `Air`.
    ///
    /// Meshing relies on that: a chunk is only meshed once its neighbours are loaded, so
    /// "unloaded reads as air" never leaks a hole into a visible mesh.
    pub fn block(&self, pos: BlockPos) -> Block {
        if !(0..WORLD_HEIGHT).contains(&pos.y) {
            return Block::Air;
        }
        match self.chunks.get(&pos.chunk()) {
            Some(c) => c.get(
                pos.x.rem_euclid(CHUNK_SIZE),
                pos.y,
                pos.z.rem_euclid(CHUNK_SIZE),
            ),
            None => Block::Air,
        }
    }

    pub fn solid(&self, pos: BlockPos) -> bool {
        self.block(pos).solid()
    }

    /// The one block-mutation path: local edits and replicated edits both land here.
    ///
    /// Bounds are all this enforces. Whether an edit is *allowed* — reach, bedrock,
    /// what may be placed where — is the host's call, made once in
    /// `game::edit_is_legal` before anything reaches this far.
    ///
    /// Returns `false` for an out-of-bounds write (a malformed or hostile peer message),
    /// which the caller drops rather than replicating.
    pub fn set_block(&mut self, pos: BlockPos, block: Block) -> bool {
        if !(0..WORLD_HEIGHT).contains(&pos.y) {
            return false;
        }
        let cp = pos.chunk();
        let (lx, lz) = (pos.x.rem_euclid(CHUNK_SIZE), pos.z.rem_euclid(CHUNK_SIZE));
        // What worldgen puts here, if that is known yet: an existing edit remembers it,
        // and otherwise an unedited loaded chunk still holds it.
        let generated = match self.edits.get(&cp).and_then(|c| c.get(&pos)) {
            Some(o) => o.generated,
            None => self.chunks.get(&cp).map(|c| c.get(lx, pos.y, lz)),
        };

        if let Some(c) = self.chunks.get_mut(&cp) {
            c.set(lx, pos.y, lz, block);
        }

        if generated == Some(block) {
            // Not an edit at all: the world is back the way it was generated.
            if let Some(overlay) = self.edits.get_mut(&cp) {
                overlay.remove(&pos);
                if overlay.is_empty() {
                    self.edits.remove(&cp);
                }
            }
        } else {
            self.edits
                .entry(cp)
                .or_default()
                .insert(pos, Overlay { block, generated });
        }
        true
    }

    /// Y of a column's terrain surface — ignoring edits, and ignoring anything standing
    /// on it such as a tree. Used to pick a spawn point before any chunk is loaded.
    pub fn ground_height(&self, x: i32, z: i32) -> i32 {
        terrain_height(self.seed, x, z)
    }
}

// ---------------------------------------------------------------------------------------
// Worldgen. Deterministic in (seed, x, z) — integer hashing plus f32 arithmetic, no RNG
// state carried between columns, so any peer can regenerate any chunk independently.
// ---------------------------------------------------------------------------------------

/// The height terrain varies around — the middle of the hills, not a sea level: there is
/// no water block for one to mean anything against.
const BASE_HEIGHT: i32 = 34;
/// Columns topping out at or below this are beach sand; everything higher is grass. A
/// look, not a shoreline.
const SAND_UP_TO: i32 = 26;

/// The canopy: one row per layer, as `(offset from the trunk top, horizontal reach)`.
///
/// The one description of a canopy's shape. The chunk overscan below and the seam test
/// both derive from it, so a bigger canopy cannot quietly start getting clipped at chunk
/// borders.
const CANOPY: [(i32, i32); 3] = [(-1, 2), (0, 2), (1, 1)];

/// How far a tree's leaves reach, in blocks — the overscan a chunk uses so trees crossing
/// a chunk border are still complete.
const TREE_REACH: i32 = {
    let (mut reach, mut i) = (0, 0);
    while i < CANOPY.len() {
        if CANOPY[i].1 > reach {
            reach = CANOPY[i].1;
        }
        i += 1;
    }
    reach
};

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn hash2(seed: u64, x: i32, z: i32) -> u64 {
    let mixed = seed
        ^ (x as i64 as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ (z as i64 as u64).wrapping_mul(0xC2B2AE3D27D4EB4F);
    splitmix64(mixed)
}

/// A stable pseudo-random value in `[0, 1)` for a lattice point.
fn lattice(seed: u64, x: i32, z: i32) -> f32 {
    (hash2(seed, x, z) >> 40) as f32 / (1u32 << 24) as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear value noise in `[0, 1)` at `(x, z)` blocks, one lattice cell per `period`.
fn value_noise(seed: u64, x: i32, z: i32, period: i32) -> f32 {
    let cx = x.div_euclid(period);
    let cz = z.div_euclid(period);
    let fx = smoothstep(x.rem_euclid(period) as f32 / period as f32);
    let fz = smoothstep(z.rem_euclid(period) as f32 / period as f32);
    let n00 = lattice(seed, cx, cz);
    let n10 = lattice(seed, cx + 1, cz);
    let n01 = lattice(seed, cx, cz + 1);
    let n11 = lattice(seed, cx + 1, cz + 1);
    let a = n00 + (n10 - n00) * fx;
    let b = n01 + (n11 - n01) * fx;
    a + (b - a) * fz
}

/// Surface height (the Y of the topmost terrain block) for a column.
pub fn terrain_height(seed: u64, x: i32, z: i32) -> i32 {
    let hills = value_noise(seed ^ 0xA1, x, z, 64) - 0.5;
    let bumps = value_noise(seed ^ 0xB2, x, z, 23) - 0.5;
    let grit = value_noise(seed ^ 0xC3, x, z, 7) - 0.5;
    let h = BASE_HEIGHT as f32 + hills * 34.0 + bumps * 10.0 + grit * 3.0;
    (h as i32).clamp(2, WORLD_HEIGHT - 24)
}

fn surface_block(height: i32) -> Block {
    if height <= SAND_UP_TO {
        Block::Sand
    } else {
        Block::Grass
    }
}

/// Does a tree grow from this column? Deterministic per column, independent of chunking.
fn tree_here(seed: u64, x: i32, z: i32) -> Option<i32> {
    let h = terrain_height(seed, x, z);
    if surface_block(h) != Block::Grass {
        return None;
    }
    // Roughly one column in 140. Denser than that and the canopies merge into a wall you
    // can't walk or see through.
    let r = hash2(seed ^ 0x7EE_5EED, x, z);
    if r % 1000 >= 7 {
        return None;
    }
    // Modulo in u64 and cast after: `(r >> 8) as i32` is negative for half of all hashes,
    // and Rust's `%` keeps the dividend's sign, which grew trunks of 2 and 3.
    Some(4 + ((r >> 8) % 3) as i32)
}

fn generate_chunk(seed: u64, cp: ChunkPos) -> Chunk {
    let mut chunk = Chunk {
        blocks: vec![Block::Air; BLOCKS_PER_CHUNK],
    };
    let origin = cp.origin();

    for lz in 0..CHUNK_SIZE {
        for lx in 0..CHUNK_SIZE {
            let (wx, wz) = (origin.x + lx, origin.z + lz);
            let h = terrain_height(seed, wx, wz);
            let top = surface_block(h);
            for y in 0..=h {
                let b = if y == 0 {
                    Block::Bedrock
                } else if y == h {
                    top
                } else if y > h - 4 {
                    if top == Block::Sand {
                        Block::Sand
                    } else {
                        Block::Dirt
                    }
                } else {
                    Block::Stone
                };
                chunk.set(lx, y, lz, b);
            }
        }
    }

    // Trees are planted from an overscanned column range so a canopy straddling a chunk
    // border is complete on BOTH sides — each chunk plants every tree that reaches it,
    // and writes only the blocks that land inside itself.
    for wz in (origin.z - TREE_REACH)..(origin.z + CHUNK_SIZE + TREE_REACH) {
        for wx in (origin.x - TREE_REACH)..(origin.x + CHUNK_SIZE + TREE_REACH) {
            let Some(trunk) = tree_here(seed, wx, wz) else {
                continue;
            };
            plant_tree(&mut chunk, cp, seed, wx, wz, trunk);
        }
    }

    chunk
}

fn plant_tree(chunk: &mut Chunk, cp: ChunkPos, seed: u64, wx: i32, wz: i32, trunk: i32) {
    let base = terrain_height(seed, wx, wz);
    let top = base + trunk;
    let origin = cp.origin();

    let mut put = |pos: BlockPos, b: Block, overwrite_solid: bool| {
        if !(0..WORLD_HEIGHT).contains(&pos.y) {
            return;
        }
        let (lx, lz) = (pos.x - origin.x, pos.z - origin.z);
        if !(0..CHUNK_SIZE).contains(&lx) || !(0..CHUNK_SIZE).contains(&lz) {
            return;
        }
        if !overwrite_solid && chunk.get(lx, pos.y, lz) != Block::Air {
            return;
        }
        chunk.set(lx, pos.y, lz, b);
    };

    for y in (base + 1)..=top {
        put(BlockPos::new(wx, y, wz), Block::Wood, true);
    }
    for (dy, reach) in CANOPY {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                // Clip the canopy corners so it reads as a blob, not a cube.
                if dx.abs() == reach && dz.abs() == reach {
                    continue;
                }
                put(
                    BlockPos::new(wx + dx, top + dy, wz + dz),
                    Block::Leaves,
                    false,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many blocks differ from worldgen — the world's whole replicated size.
    fn edit_count(w: &World) -> usize {
        w.overlays().iter().map(Vec::len).sum()
    }

    #[test]
    fn terrain_is_seed_deterministic() {
        for (x, z) in [(0, 0), (17, -93), (-1000, 4321)] {
            let a = terrain_height(42, x, z);
            let b = terrain_height(42, x, z);
            assert_eq!(a, b);
        }
        assert_ne!(
            (0..64).map(|x| terrain_height(1, x, 0)).collect::<Vec<_>>(),
            (0..64).map(|x| terrain_height(2, x, 0)).collect::<Vec<_>>(),
            "different seeds should give different terrain"
        );
    }

    /// Trunks are 4, 5 or 6 blocks. A signed-modulo bug once let the hash produce 2 and 3
    /// as well, and a 2-block "tree" is a bush the player walks into.
    #[test]
    fn tree_trunks_are_four_to_six_blocks() {
        let mut seen = [0usize; 3];
        for z in -200..200 {
            for x in -200..200 {
                let Some(trunk) = tree_here(9, x, z) else {
                    continue;
                };
                assert!((4..=6).contains(&trunk), "trunk {trunk} at ({x},{z})");
                seen[(trunk - 4) as usize] += 1;
            }
        }
        assert!(
            seen.iter().all(|&n| n > 0),
            "every trunk height should occur: {seen:?}"
        );
    }

    #[test]
    fn terrain_has_relief() {
        let heights: Vec<i32> = (0..256).map(|x| terrain_height(7, x, 0)).collect();
        let lo = *heights.iter().min().unwrap();
        let hi = *heights.iter().max().unwrap();
        assert!(hi - lo > 8, "terrain is too flat: {lo}..{hi}");
        assert!(
            lo >= 2 && hi < WORLD_HEIGHT - 20,
            "terrain escaped its clamp: {lo}..{hi}"
        );
    }

    #[test]
    fn columns_are_layered_grass_dirt_stone() {
        let mut w = World::new(7, []);
        let cp = ChunkPos::new(0, 0);
        w.load_chunk(cp);
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let h = terrain_height(7, lx, lz);
                let surface = w.block(BlockPos::new(lx, h, lz));
                // A tree may sit above the surface, never below it.
                assert!(
                    matches!(surface, Block::Grass | Block::Sand),
                    "surface at ({lx},{lz}) is {}",
                    surface.name()
                );
                assert!(w.solid(BlockPos::new(lx, h - 1, lz)));
                assert_eq!(w.block(BlockPos::new(lx, h - 6, lz)), Block::Stone);
                assert_eq!(w.block(BlockPos::new(lx, 0, lz)), Block::Bedrock);
                // Above the surface is open air, or a tree standing on it.
                assert!(matches!(
                    w.block(BlockPos::new(lx, h + 1, lz)),
                    Block::Air | Block::Wood | Block::Leaves
                ));
            }
        }
    }

    /// The property multiplayer rests on: seed + edit log fully reconstructs a world.
    #[test]
    fn edit_log_reconstructs_the_world() {
        let mut host = World::new(99, []);
        let cp = ChunkPos::new(1, -2);
        host.load_chunk(cp);
        let dug = BlockPos::new(20, terrain_height(99, 20, -20), -20);
        let built = BlockPos::new(21, 70, -21);
        assert!(host.set_block(dug, Block::Air));
        assert!(host.set_block(built, Block::Stone));

        let mut peer = World::new(99, host.overlays().concat());
        peer.load_chunk(dug.chunk());
        peer.load_chunk(built.chunk());
        assert_eq!(peer.block(dug), Block::Air);
        assert_eq!(peer.block(built), Block::Stone);
    }

    /// An edit that puts a block back the way worldgen made it is not an edit. Without
    /// this, digging a hole and filling it in leaves two entries behind forever — and the
    /// world's replicated size only ever grows.
    #[test]
    fn restoring_a_block_removes_the_edit() {
        let mut w = World::new(21, []);
        let cp = ChunkPos::new(0, 0);
        w.load_chunk(cp);
        let surface = BlockPos::new(8, terrain_height(21, 8, 8), 8);
        let was = w.block(surface);
        assert_eq!(edit_count(&w), 0);

        w.set_block(surface, Block::Air);
        assert_eq!(edit_count(&w), 1);
        w.set_block(surface, was);
        assert_eq!(edit_count(&w), 0, "the world is as generated again");
        assert_eq!(w.block(surface), was);
    }

    /// The same, for a peer's edits: they arrive with no idea what worldgen puts there, so
    /// the pruning has to happen when the chunk is first generated.
    #[test]
    fn a_joiners_redundant_edits_are_pruned_on_load() {
        let seed = 21;
        let surface = BlockPos::new(8, terrain_height(seed, 8, 8), 8);
        let mut reference = World::new(seed, []);
        reference.load_chunk(ChunkPos::new(0, 0));
        let generated = reference.block(surface);

        let mut w = World::new(
            seed,
            [
                (surface, generated),
                (BlockPos::new(8, 80, 8), Block::Stone),
            ],
        );
        assert_eq!(edit_count(&w), 2, "nothing is knowable before worldgen");
        w.load_chunk(ChunkPos::new(0, 0));
        assert_eq!(edit_count(&w), 1, "the redundant one is gone");
        assert_eq!(w.block(BlockPos::new(8, 80, 8)), Block::Stone);
    }

    /// Loading a chunk must not walk edits belonging to other chunks: that is O(all edits)
    /// per load, which turns a well-built world into a stall on every step the player takes.
    #[test]
    fn a_chunk_load_reads_only_its_own_chunks_edits() {
        let mut w = World::new(5, []);
        let far = BlockPos::new(500, 80, 500);
        w.set_block(far, Block::Stone);
        let near = BlockPos::new(3, 80, 3);
        w.set_block(near, Block::Wood);

        let batches = w.overlays();
        assert_eq!(batches.len(), 2, "one batch per edited chunk");
        assert!(
            batches.iter().all(|b| b.len() == 1),
            "each edit is in its own chunk's batch: {batches:?}"
        );

        w.load_chunk(near.chunk());
        assert_eq!(w.block(near), Block::Wood);
        assert_eq!(w.block(far), Block::Air, "the far chunk is not loaded");
    }

    /// Edits made before a chunk loads must still be there when it does — the late-joiner
    /// path, where the whole edit log arrives before any terrain exists.
    #[test]
    fn edits_survive_a_later_chunk_load() {
        let target = BlockPos::new(5, 60, 5);
        let mut w = World::new(3, [(target, Block::Wood)]);
        assert_eq!(w.block(target), Block::Air, "nothing is loaded yet");
        w.load_chunk(target.chunk());
        assert_eq!(w.block(target), Block::Wood);
    }

    #[test]
    fn out_of_bounds_edits_are_refused() {
        let mut w = World::new(1, []);
        assert!(!w.set_block(BlockPos::new(0, -1, 0), Block::Stone));
        assert!(!w.set_block(BlockPos::new(0, WORLD_HEIGHT, 0), Block::Stone));
        assert!(w.set_block(BlockPos::new(0, WORLD_HEIGHT - 1, 0), Block::Stone));
    }

    /// A canopy that straddles a chunk border must be identical from either chunk's view.
    #[test]
    fn trees_are_seamless_across_chunk_borders() {
        let seed = 12345;
        let mut w = World::new(seed, []);
        for cx in -1..=1 {
            for cz in -1..=1 {
                w.load_chunk(ChunkPos::new(cx, cz));
            }
        }
        // The loaded chunks cover [-16, 32) in x and z; scanning inside that by the
        // canopy's reach keeps every block this test reads inside a generated chunk.
        let mut leaves = 0;
        for wz in (-16 + TREE_REACH)..(32 - TREE_REACH) {
            for wx in (-16 + TREE_REACH)..(32 - TREE_REACH) {
                if let Some(trunk) = tree_here(seed, wx, wz) {
                    let top = terrain_height(seed, wx, wz) + trunk;
                    assert_eq!(w.block(BlockPos::new(wx, top, wz)), Block::Wood);
                    for (dy, reach) in CANOPY {
                        for dz in -reach..=reach {
                            for dx in -reach..=reach {
                                if dx.abs() == reach && dz.abs() == reach {
                                    continue;
                                }
                                let p = BlockPos::new(wx + dx, top + dy, wz + dz);
                                assert!(w.solid(p), "canopy hole at {p:?} for tree ({wx},{wz})");
                                leaves += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(leaves > 0, "seed {seed} grew no trees to check");
    }
}
