//! The content registry — **this is the seam you extend.**
//!
//! Two tables, and every piece of content in the game is a row in one of them.
//!
//! [`Block`] is every voxel kind, and [`Block::def`] is the table behind it: colour and
//! solidity. [`Item`] is everything a player can *hold*, and [`Item::def`] is the table
//! behind that one: name, what class of thing it is, and what it costs to make.
//!
//! Only items have names, because only items are ever spoken about — a voxel is drawn, and
//! the ones a player can talk about are exactly the ones they can hold.
//!
//! The two meet in [`Class::Block`]: an item of that class is a voxel you place, and it
//! takes its colour from the voxel, so a hotbar cell can never disagree with what it puts
//! in the world. Everything else — tools, components, equippables, vehicles — is an item
//! and nothing more.
//!
//! **Adding content is a localised diff.** A new block: a [`Block`] variant, a [`Block::def`]
//! arm, and — if a player gets to place it — an [`Item`] variant, an [`Item::def`] arm, and
//! an [`Item::ALL`] entry. A new craftable thing: the item half alone, with a recipe.
//! Nothing outside this file has to change; the hotbar, the crafting UI and the wire all
//! read these tables.

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
    Cushion,
}

/// What the rest of the game needs to know about a voxel kind. Private: everything asks
/// through [`Block`]'s accessors, so there is one way to ask.
struct BlockDef {
    /// Linear RGB, baked into mesh vertex colours — the whole world uses one material.
    color: [f32; 3],
    /// Solid voxels stop the player and can be targeted. Whether a voxel is *drawn* is a
    /// separate question — see [`Block::visible`].
    solid: bool,
}

impl Block {
    /// The table. Exhaustive on purpose: a variant with no arm is a compile error, where
    /// a lookup into a table of rows was a panic the first time a peer named the block.
    fn def(self) -> BlockDef {
        match self {
            Block::Air => BlockDef {
                color: [0.0, 0.0, 0.0],
                solid: false,
            },
            Block::Grass => BlockDef {
                color: [0.36, 0.63, 0.19],
                solid: true,
            },
            Block::Dirt => BlockDef {
                color: [0.42, 0.28, 0.16],
                solid: true,
            },
            Block::Stone => BlockDef {
                color: [0.45, 0.45, 0.47],
                solid: true,
            },
            Block::Sand => BlockDef {
                color: [0.80, 0.74, 0.48],
                solid: true,
            },
            Block::Wood => BlockDef {
                color: [0.36, 0.25, 0.13],
                solid: true,
            },
            Block::Leaves => BlockDef {
                // Distinctly darker and bluer than grass — at a distance the two greens
                // have to stay tellable apart, or a forest reads as a lumpy field.
                color: [0.08, 0.31, 0.12],
                solid: true,
            },
            Block::Bedrock => BlockDef {
                color: [0.10, 0.10, 0.12],
                solid: true,
            },
            Block::Cushion => BlockDef {
                // Nothing in the world is pink, so a cushion reads as made rather than
                // grown from across a valley — which is the point of the first thing a
                // player crafts.
                color: [0.86, 0.36, 0.60],
                solid: true,
            },
        }
    }

    pub fn color(self) -> [f32; 3] {
        self.def().color
    }

    pub fn solid(self) -> bool {
        self.def().solid
    }

    /// Does this voxel draw? The mesher's only notion of presence — it emits faces for a
    /// visible voxel and hides the faces a visible voxel is pressed against.
    ///
    /// Deliberately *not* [`Block::solid`], which is physics: a walk-through block that
    /// still has to be seen would otherwise draw a full six-face cube and occlude nothing.
    pub fn visible(self) -> bool {
        self != Block::Air
    }

    /// Can a player put this voxel in the world at all? Asked of the item table rather
    /// than of a second list beside it, so "placeable" and "some item places it" are the
    /// same question and cannot drift apart.
    pub fn placeable(self) -> bool {
        Item::placing(self).is_some()
    }
}

/// What an item *is*.
///
/// [`Class::Block`] is the only one the game acts on today: it says which voxel the item
/// puts in the world. The rest are the classes the requirement sheets ask for, and the
/// whole of what distinguishes a rifle from a car until one of them does something — they
/// carry a colour because that is all there is to draw of a thing with no behaviour yet.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Class {
    /// Goes in the world as this voxel, and is drawn in its colour.
    Block(Block),
    /// Swung or fired at the world. Linear RGB, as everywhere else in this file.
    Tool { color: [f32; 3] },
    /// Not used on its own — spent making something else.
    Component { color: [f32; 3] },
    /// Worn, and changes how its wearer moves.
    Equippable { color: [f32; 3] },
    /// Ridden.
    Vehicle { color: [f32; 3] },
}

impl Class {
    /// The word the HUD shows under an item's name.
    pub fn word(self) -> &'static str {
        match self {
            Class::Block(_) => "block",
            Class::Tool { .. } => "tool",
            Class::Component { .. } => "part",
            Class::Equippable { .. } => "wearable",
            Class::Vehicle { .. } => "vehicle",
        }
    }
}

/// Everything a player can hold, in hotbar order.
///
/// Wire ids are declaration indices, exactly as for [`Block`] — **append new variants at
/// the end**, or an older peer decodes a rifle as a hammer. `items_are_declaration_order`
/// pins it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Item {
    // Gathered: break the block, keep the block.
    Grass,
    Dirt,
    Stone,
    Sand,
    Wood,
    Leaves,
    // Made from what you gathered.
    Cushion,
    Nail,
    Hammer,
    Drill,
    Handgun,
    Rifle,
    Parachute,
    Car,
}

/// What the game needs to know about a held thing. Private, like [`BlockDef`]: everything
/// asks through [`Item`]'s accessors.
struct ItemDef {
    name: &'static str,
    class: Class,
    /// What one of these costs to make. Empty means you cannot make it at all — you go and
    /// find it. One craft makes one; a yield column is the next thing this table grows,
    /// and nothing wants it yet.
    recipe: &'static [(Item, u32)],
}

/// Nothing: the recipe of a thing you gather rather than make.
const GATHERED: &[(Item, u32)] = &[];

/// Hotbar cells per row — the width the HUD lays the hotbar out in, and the step the
/// d-pad's up and down take through it. One number so the two cannot disagree.
pub const HOTBAR_COLUMNS: usize = 7;

impl Item {
    /// Every item, in hotbar order. `all_lists_every_item` is what keeps it complete.
    pub const ALL: &'static [Item] = &[
        Item::Grass,
        Item::Dirt,
        Item::Stone,
        Item::Sand,
        Item::Wood,
        Item::Leaves,
        Item::Cushion,
        Item::Nail,
        Item::Hammer,
        Item::Drill,
        Item::Handgun,
        Item::Rifle,
        Item::Parachute,
        Item::Car,
    ];

    pub const COUNT: usize = Item::ALL.len();

    /// The table.
    ///
    /// Recipes are deliberately short and made of things a child has already dug up: two
    /// wood and a nail is a hammer, and you can see why. Nothing here needs a grid, a
    /// workbench, or an order.
    fn def(self) -> ItemDef {
        use Class::*;
        match self {
            Item::Grass => ItemDef {
                name: "grass",
                class: Block(self::Block::Grass),
                recipe: GATHERED,
            },
            Item::Dirt => ItemDef {
                name: "dirt",
                class: Block(self::Block::Dirt),
                recipe: GATHERED,
            },
            Item::Stone => ItemDef {
                name: "stone",
                class: Block(self::Block::Stone),
                recipe: GATHERED,
            },
            Item::Sand => ItemDef {
                name: "sand",
                class: Block(self::Block::Sand),
                recipe: GATHERED,
            },
            Item::Wood => ItemDef {
                name: "wood",
                class: Block(self::Block::Wood),
                recipe: GATHERED,
            },
            Item::Leaves => ItemDef {
                name: "leaves",
                class: Block(self::Block::Leaves),
                recipe: GATHERED,
            },
            // The first thing worth making: leaves for stuffing, wood for the frame.
            Item::Cushion => ItemDef {
                name: "cushion",
                class: Block(self::Block::Cushion),
                recipe: &[(Item::Leaves, 4), (Item::Wood, 1)],
            },
            Item::Nail => ItemDef {
                name: "nail",
                class: Component {
                    color: [0.62, 0.63, 0.66],
                },
                recipe: &[(Item::Stone, 1)],
            },
            Item::Hammer => ItemDef {
                name: "hammer",
                class: Tool {
                    color: [0.55, 0.35, 0.18],
                },
                recipe: &[(Item::Wood, 2), (Item::Nail, 1)],
            },
            Item::Drill => ItemDef {
                name: "drill",
                class: Tool {
                    color: [0.90, 0.45, 0.08],
                },
                recipe: &[(Item::Wood, 1), (Item::Stone, 2), (Item::Nail, 2)],
            },
            Item::Handgun => ItemDef {
                name: "handgun",
                class: Tool {
                    color: [0.22, 0.22, 0.26],
                },
                recipe: &[(Item::Wood, 1), (Item::Nail, 2)],
            },
            Item::Rifle => ItemDef {
                name: "rifle",
                class: Tool {
                    color: [0.30, 0.23, 0.16],
                },
                recipe: &[(Item::Wood, 2), (Item::Nail, 3)],
            },
            Item::Parachute => ItemDef {
                name: "parachute",
                class: Equippable {
                    color: [0.92, 0.28, 0.34],
                },
                recipe: &[(Item::Leaves, 6), (Item::Nail, 2)],
            },
            // The big one, and the reason nails are worth making by the handful.
            Item::Car => ItemDef {
                name: "car",
                class: Vehicle {
                    color: [0.15, 0.45, 0.85],
                },
                recipe: &[(Item::Wood, 6), (Item::Stone, 2), (Item::Nail, 8)],
            },
        }
    }

    pub fn name(self) -> &'static str {
        self.def().name
    }

    pub fn class(self) -> Class {
        self.def().class
    }

    /// What one of these costs to make, or empty for something you gather.
    pub fn recipe(self) -> &'static [(Item, u32)] {
        self.def().recipe
    }

    /// The voxel this item puts in the world, if it is a block at all.
    pub fn places(self) -> Option<Block> {
        match self.class() {
            Class::Block(b) => Some(b),
            _ => None,
        }
    }

    /// The item that places `block`, if any. The reverse of [`Item::places`], read off the
    /// same table rather than kept as a second one — `one_item_per_block` is what keeps
    /// the answer unambiguous.
    pub fn placing(block: Block) -> Option<Item> {
        Item::ALL
            .iter()
            .copied()
            .find(|i| i.places() == Some(block))
    }

    /// Linear RGB: the hotbar cell, and the cube in its owner's hand. A block item is
    /// drawn in the colour of the block it places, so the two cannot drift.
    pub fn color(self) -> [f32; 3] {
        match self.class() {
            Class::Block(b) => b.color(),
            Class::Tool { color }
            | Class::Component { color }
            | Class::Equippable { color }
            | Class::Vehicle { color } => color,
        }
    }

    /// Position in [`Item::ALL`] — the hotbar cursor's coordinate, and the wire id.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The item `steps` along the hotbar from this one, wrapping. Moving by
    /// [`HOTBAR_COLUMNS`] is a row; moving by one is a cell.
    pub fn step(self, steps: i32) -> Item {
        let n = Item::COUNT as i32;
        Item::ALL[(self.index() as i32 + steps).rem_euclid(n) as usize]
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
            (Block::Cushion, 8),
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

    /// [`Item::ALL`] is the hotbar, the crafting menu, and the array an inventory is —
    /// indexed by [`Item::index`], which is the `as` discriminant. An item missing from
    /// the list is an item nobody can hold; one listed out of order indexes somebody
    /// else's pile.
    #[test]
    fn items_are_declaration_order() {
        for (i, item) in Item::ALL.iter().enumerate() {
            assert_eq!(item.index(), i, "{item:?} is out of place in ALL");
        }
    }

    /// The other half: every variant is *in* the list. The `match` is what enforces it —
    /// a new variant stops this compiling, and the count below then fails until the
    /// variant reaches [`Item::ALL`] too.
    #[test]
    fn all_lists_every_item() {
        for item in Item::ALL {
            match item {
                Item::Grass
                | Item::Dirt
                | Item::Stone
                | Item::Sand
                | Item::Wood
                | Item::Leaves
                | Item::Cushion
                | Item::Nail
                | Item::Hammer
                | Item::Drill
                | Item::Handgun
                | Item::Rifle
                | Item::Parachute
                | Item::Car => {}
            }
        }
        assert_eq!(Item::COUNT, 14, "add the new item to Item::ALL");
    }

    /// A hotbar slot holding a non-solid block would let a player place a hole, or place
    /// nothing at all and wonder why the button is broken.
    #[test]
    fn every_placeable_block_is_solid() {
        for item in Item::ALL {
            if let Some(block) = item.places() {
                assert!(block.solid(), "{block:?} is placeable but not solid");
            }
        }
    }

    /// [`Item::placing`] is a reverse lookup, so two items placing one block would make
    /// "what did breaking this give me" a coin toss.
    #[test]
    fn one_item_per_block() {
        let mut placed: Vec<Block> = Item::ALL.iter().filter_map(|i| i.places()).collect();
        let placed_count = placed.len();
        placed.sort_by_key(|b| format!("{b:?}"));
        placed.dedup();
        assert_eq!(placed.len(), placed_count, "two items place the same block");
        assert_eq!(Item::placing(Block::Air), None, "nothing places air");
        assert_eq!(
            Item::placing(Block::Bedrock),
            None,
            "the world's, not yours"
        );
        assert_eq!(Item::placing(Block::Cushion), Some(Item::Cushion));
    }

    /// A recipe naming something no recipe and no block produces is a dead end a player
    /// can walk into and never get out of.
    #[test]
    fn every_ingredient_is_obtainable() {
        for item in Item::ALL {
            for (ingredient, n) in item.recipe() {
                assert!(*n > 0, "{item:?} asks for zero {ingredient:?}");
                assert!(
                    !ingredient.recipe().is_empty() || ingredient.places().is_some(),
                    "{item:?} needs {ingredient:?}, which cannot be made or found"
                );
            }
        }
    }

    /// A recipe that depends on itself, however far around, is an item nobody can ever
    /// make. Resolved by repeatedly collecting what is reachable: anything left over
    /// after that fixed point is only reachable through itself.
    #[test]
    fn no_recipe_depends_on_itself() {
        let mut obtainable: Vec<Item> = Item::ALL
            .iter()
            .copied()
            .filter(|i| i.recipe().is_empty())
            .collect();
        loop {
            let next: Vec<Item> = Item::ALL
                .iter()
                .copied()
                .filter(|i| !obtainable.contains(i))
                .filter(|i| i.recipe().iter().all(|(g, _)| obtainable.contains(g)))
                .collect();
            if next.is_empty() {
                break;
            }
            obtainable.extend(next);
        }
        for item in Item::ALL {
            assert!(
                obtainable.contains(item),
                "{item:?} can only be made out of itself"
            );
        }
    }

    /// The requirement sheets, in one assertion: every craftable the boyos asked for
    /// exists, is craftable, and is holdable.
    #[test]
    fn the_requested_craftables_are_all_here() {
        for item in [
            Item::Car,
            Item::Handgun,
            Item::Rifle,
            Item::Cushion,
            Item::Hammer,
            Item::Nail,
            Item::Drill,
            Item::Parachute,
        ] {
            assert!(Item::ALL.contains(&item), "{item:?} is not in the hotbar");
            assert!(!item.recipe().is_empty(), "{item:?} cannot be made");
        }
        assert_eq!(Item::Cushion.places(), Some(Block::Cushion), "placeable");
    }

    #[test]
    fn the_hotbar_wraps_in_both_directions() {
        assert_eq!(Item::Grass.step(1), Item::Dirt);
        assert_eq!(
            Item::Grass.step(-1),
            Item::Car,
            "off the front, round the back"
        );
        assert_eq!(Item::Grass.step(Item::COUNT as i32), Item::Grass);
        assert_eq!(
            Item::Grass.step(HOTBAR_COLUMNS as i32),
            Item::ALL[HOTBAR_COLUMNS],
            "down is a row"
        );
    }
}
