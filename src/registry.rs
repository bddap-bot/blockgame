//! The content registry — **this is the seam you extend.**
//!
//! Two tables, both plain `const` data:
//!
//! * [`BLOCKS`] — what a voxel *is* in the world (colour, solidity).
//! * [`ITEMS`] — what a player can *hold*, and what happens when they use it. An item's
//!   [`ItemKind`] is the behaviour dimension: place a block, use a tool, ride a vehicle.
//!
//! The kids' requirement sheets (`design/requirements-1.jpg`, `-2.jpg`) ask for Car,
//! Handgun, Rifle, Cushion, Hammer, Nail, Drill and Parachute. That spread is exactly why
//! items are not just blocks — every one of them is a small, localized diff:
//!
//! | want              | diff                                                                    |
//! |-------------------|-------------------------------------------------------------------------|
//! | Cushion           | a `Block` variant + a [`BLOCKS`] row + an [`ITEMS`] row `Place(Cushion)` |
//! | Handgun, Hammer   | an `Item` variant + an [`ITEMS`] row `Tool` (then a use-system)          |
//! | Car               | an `Item` variant + an [`ITEMS`] row `Vehicle` (then a ride-system)      |
//! | Nail              | an `Item` variant + an [`ITEMS`] row `Material`                          |
//! | Parachute         | an `Item` variant + an [`ITEMS`] row `Wearable` (then a physics hook)    |
//!
//! Only `Place` carries behaviour in the bones; the other kinds are no-ops. The variants
//! exist so adding the behaviour is a localized change with an obvious place to put it,
//! instead of a refactor of everything that assumed "held thing == block".

// The registry is an extension API, not just what the bones happen to call today: a row
// or accessor with no caller yet is the point of the file, not an oversight.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Every voxel kind. `Air` is index 0 and means "nothing here".
///
/// The discriminant is the wire format (a `u8` in every network message), so **append new
/// variants at the end** — renumbering an existing one desynchronizes peers on old builds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Block {
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Sand = 4,
    Wood = 5,
    Leaves = 6,
    Bedrock = 7,
}

/// What the rest of the game needs to know about a voxel kind.
pub struct BlockDef {
    pub block: Block,
    /// Shown in the hotbar and debug text.
    pub name: &'static str,
    /// Linear RGB, baked into mesh vertex colours — the whole world uses one material.
    pub color: [f32; 3],
    /// Solid voxels stop the player, occlude neighbouring faces, and can be targeted.
    pub solid: bool,
}

/// The voxel table. Row order MUST match the [`Block`] discriminants — see [`Block::def`].
pub const BLOCKS: &[BlockDef] = &[
    BlockDef {
        block: Block::Air,
        name: "air",
        color: [0.0, 0.0, 0.0],
        solid: false,
    },
    BlockDef {
        block: Block::Grass,
        name: "grass",
        color: [0.25, 0.55, 0.16],
        solid: true,
    },
    BlockDef {
        block: Block::Dirt,
        name: "dirt",
        color: [0.42, 0.28, 0.16],
        solid: true,
    },
    BlockDef {
        block: Block::Stone,
        name: "stone",
        color: [0.45, 0.45, 0.47],
        solid: true,
    },
    BlockDef {
        block: Block::Sand,
        name: "sand",
        color: [0.80, 0.74, 0.48],
        solid: true,
    },
    BlockDef {
        block: Block::Wood,
        name: "wood",
        color: [0.36, 0.25, 0.13],
        solid: true,
    },
    BlockDef {
        block: Block::Leaves,
        name: "leaves",
        color: [0.16, 0.42, 0.13],
        solid: true,
    },
    BlockDef {
        block: Block::Bedrock,
        name: "bedrock",
        color: [0.10, 0.10, 0.12],
        solid: true,
    },
];

impl Block {
    pub fn def(self) -> &'static BlockDef {
        &BLOCKS[self as usize]
    }

    pub fn solid(self) -> bool {
        self.def().solid
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn color(self) -> [f32; 3] {
        self.def().color
    }

    /// Wire decode. `None` for an id this build doesn't know, so a peer on a newer build
    /// can't inject an out-of-range index into the world array.
    pub fn from_id(id: u8) -> Option<Block> {
        BLOCKS.get(id as usize).map(|d| d.block)
    }
}

/// Everything a player can hold. Hotbar order is [`ITEMS`] order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Item {
    Grass = 0,
    Dirt = 1,
    Stone = 2,
    Sand = 3,
    Wood = 4,
    Leaves = 5,
}

/// What holding an item lets you do — the behaviour dimension of the registry.
///
/// Only `Place` has any rows in [`ITEMS`] yet, so the rest read as dead code. They are
/// the point of this enum: each is a shape the kids have already asked for, waiting for
/// its first row.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    /// Using it puts this voxel in the world. Every item in the bones is one of these.
    Place(Block),
    /// Swung or fired at what you are looking at (handgun, rifle, hammer, drill).
    Tool,
    /// Entered and driven (the car).
    Vehicle,
    /// A component with no direct use — it exists to be consumed by crafting or
    /// construction (nails).
    Material,
    /// Equipped, and changes how the player moves while worn (parachute).
    Wearable,
}

pub struct ItemDef {
    pub item: Item,
    pub name: &'static str,
    pub kind: ItemKind,
}

/// The item table. Row order MUST match the [`Item`] discriminants — see [`Item::def`].
pub const ITEMS: &[ItemDef] = &[
    ItemDef {
        item: Item::Grass,
        name: "grass",
        kind: ItemKind::Place(Block::Grass),
    },
    ItemDef {
        item: Item::Dirt,
        name: "dirt",
        kind: ItemKind::Place(Block::Dirt),
    },
    ItemDef {
        item: Item::Stone,
        name: "stone",
        kind: ItemKind::Place(Block::Stone),
    },
    ItemDef {
        item: Item::Sand,
        name: "sand",
        kind: ItemKind::Place(Block::Sand),
    },
    ItemDef {
        item: Item::Wood,
        name: "wood",
        kind: ItemKind::Place(Block::Wood),
    },
    ItemDef {
        item: Item::Leaves,
        name: "leaves",
        kind: ItemKind::Place(Block::Leaves),
    },
];

impl Item {
    pub fn def(self) -> &'static ItemDef {
        &ITEMS[self as usize]
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn kind(self) -> ItemKind {
        self.def().kind
    }

    /// The voxel this item places, if it places one.
    pub fn places(self) -> Option<Block> {
        match self.kind() {
            ItemKind::Place(b) => Some(b),
            ItemKind::Tool | ItemKind::Vehicle | ItemKind::Material | ItemKind::Wearable => None,
        }
    }

    /// The hotbar slot this item sits in (0-based).
    pub fn slot(self) -> usize {
        self as usize
    }

    /// The item in hotbar slot `slot`, wrapping — how the d-pad / scroll wheel cycles.
    pub fn from_slot(slot: usize) -> Item {
        ITEMS[slot % ITEMS.len()].item
    }

    pub fn count() -> usize {
        ITEMS.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_table_is_consistent() {
        for (i, def) in BLOCKS.iter().enumerate() {
            assert_eq!(
                def.block as usize, i,
                "BLOCKS row {i} is out of order ({})",
                def.name
            );
            assert_eq!(Block::from_id(i as u8), Some(def.block));
        }
        assert_eq!(Block::from_id(BLOCKS.len() as u8), None);
    }

    #[test]
    fn item_table_is_consistent() {
        for (i, def) in ITEMS.iter().enumerate() {
            assert_eq!(
                def.item as usize, i,
                "ITEMS row {i} is out of order ({})",
                def.name
            );
            assert_eq!(Item::from_slot(i), def.item);
        }
        assert_eq!(
            Item::from_slot(ITEMS.len()),
            ITEMS[0].item,
            "hotbar should wrap"
        );
    }

    #[test]
    fn air_is_the_only_non_solid_block() {
        for def in BLOCKS {
            assert_eq!(def.solid, def.block != Block::Air, "{} solidity", def.name);
        }
    }

    /// A `Place` item that places air (or an unplaceable block) would let a player delete
    /// the world from the hotbar.
    #[test]
    fn placed_blocks_are_solid() {
        for def in ITEMS {
            if let ItemKind::Place(b) = def.kind {
                assert!(b.solid(), "item {} places non-solid {}", def.name, b.name());
            }
        }
    }
}
