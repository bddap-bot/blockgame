//! The content registry — **this is the seam you extend.**
//!
//! [`Block`] is every voxel kind and [`Block::def`] is the one table behind it: name,
//! colour, solidity. [`PLACEABLE`] is the hotbar — the blocks a player can hold and put
//! back — so "placeable" and "holdable" are the same list and cannot drift apart.
//!
//! Adding a block is a variant, a `def` arm, and a [`PLACEABLE`] entry if the player gets
//! to place it: the Cushion on `design/requirements-2.jpg` is exactly that. The Car,
//! Handgun and Parachute on those sheets are not blocks, and there is deliberately no
//! empty scaffolding here waiting for them — they get a shape when the behaviour that
//! makes them worth holding does.

use serde::{Deserialize, Serialize};

/// Every voxel kind. `Air` means "nothing here".
///
/// Serialization writes a variant's **declaration index**, not its `as` discriminant, so
/// **append new variants at the end**: reordering this list changes what a peer on an
/// older build decodes — grass arrives as dirt. `block_ids_are_declaration_order` pins the
/// current order to exact bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    Wood,
    Leaves,
    Bedrock,
}

/// What the rest of the game needs to know about a voxel kind. Private: everything asks
/// through [`Block`]'s accessors, so there is one way to ask.
struct BlockDef {
    /// Shown in the status line and debug text.
    name: &'static str,
    /// Linear RGB, baked into mesh vertex colours — the whole world uses one material.
    color: [f32; 3],
    /// Solid voxels stop the player, occlude neighbouring faces, and can be targeted.
    solid: bool,
}

/// The hotbar, in slot order: every block a player may hold, and therefore every block a
/// player may place. Air and bedrock are the world's to give, not the player's.
pub const PLACEABLE: &[Block] = &[
    Block::Grass,
    Block::Dirt,
    Block::Stone,
    Block::Sand,
    Block::Wood,
    Block::Leaves,
];

const _: () = assert!(
    !PLACEABLE.is_empty(),
    "an empty hotbar makes Item::from_slot divide by zero"
);

impl Block {
    /// The table. Exhaustive on purpose: a variant with no arm is a compile error, where
    /// a lookup into a table of rows was a panic the first time a peer named the block.
    fn def(self) -> BlockDef {
        match self {
            Block::Air => BlockDef {
                name: "air",
                color: [0.0, 0.0, 0.0],
                solid: false,
            },
            Block::Grass => BlockDef {
                name: "grass",
                color: [0.36, 0.63, 0.19],
                solid: true,
            },
            Block::Dirt => BlockDef {
                name: "dirt",
                color: [0.42, 0.28, 0.16],
                solid: true,
            },
            Block::Stone => BlockDef {
                name: "stone",
                color: [0.45, 0.45, 0.47],
                solid: true,
            },
            Block::Sand => BlockDef {
                name: "sand",
                color: [0.80, 0.74, 0.48],
                solid: true,
            },
            Block::Wood => BlockDef {
                name: "wood",
                color: [0.36, 0.25, 0.13],
                solid: true,
            },
            Block::Leaves => BlockDef {
                // Distinctly darker and bluer than grass — at a distance the two greens
                // have to stay tellable apart, or a forest reads as a lumpy field.
                name: "leaves",
                color: [0.08, 0.31, 0.12],
                solid: true,
            },
            Block::Bedrock => BlockDef {
                name: "bedrock",
                color: [0.10, 0.10, 0.12],
                solid: true,
            },
        }
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn color(self) -> [f32; 3] {
        self.def().color
    }

    pub fn solid(self) -> bool {
        self.def().solid
    }

    /// Can a player put this voxel in the world at all? The host's edit rules ask this
    /// instead of keeping a second list beside [`PLACEABLE`].
    pub fn placeable(self) -> bool {
        PLACEABLE.contains(&self)
    }
}

/// What a player is holding: a slot of the [`PLACEABLE`] hotbar. Every holdable thing is
/// a block you place, so the slot is the whole item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Item(usize);

impl Item {
    /// Wrapping, so cycling the hotbar is arithmetic with no bounds check — and the
    /// index inside an [`Item`] is always a real slot.
    pub fn from_slot(slot: usize) -> Item {
        Item(slot % PLACEABLE.len())
    }

    pub fn slot(self) -> usize {
        self.0
    }

    pub const fn count() -> usize {
        PLACEABLE.len()
    }

    /// The voxel this item puts in the world.
    pub fn places(self) -> Block {
        PLACEABLE[self.0]
    }

    pub fn name(self) -> &'static str {
        self.places().name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::{Msg, encode};
    use crate::world::BlockPos;

    /// The wire tag of a block is its declaration index, written little-endian as the
    /// last four bytes of an [`Msg::Edit`]. Reordering [`Block`] would silently repaint
    /// every world an older peer sends, so the ids are spelled out as literals here: a
    /// reorder has to fail this test before it can reach a player.
    #[test]
    fn block_ids_are_declaration_order() {
        let ids = [
            (Block::Air, 0u8),
            (Block::Grass, 1),
            (Block::Dirt, 2),
            (Block::Stone, 3),
            (Block::Sand, 4),
            (Block::Wood, 5),
            (Block::Leaves, 6),
            (Block::Bedrock, 7),
        ];
        for (block, id) in ids {
            let bytes = encode(&Msg::Edit {
                pos: BlockPos::new(0, 0, 0),
                block,
            })
            .unwrap();
            assert_eq!(bytes[bytes.len() - 4..], [id, 0, 0, 0], "{block:?} wire id");
        }
    }

    /// A hotbar slot holding a non-solid block would let a player place a hole, or place
    /// nothing at all and wonder why the button is broken.
    #[test]
    fn every_placeable_block_is_solid() {
        for &block in PLACEABLE {
            assert!(block.solid(), "{} is placeable but not solid", block.name());
        }
    }

    #[test]
    fn the_hotbar_wraps() {
        assert_eq!(Item::from_slot(0).places(), PLACEABLE[0]);
        assert_eq!(Item::from_slot(2).slot(), 2);
        assert_eq!(Item::from_slot(Item::count()), Item::from_slot(0));
    }
}
