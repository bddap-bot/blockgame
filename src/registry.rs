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
//! takes its colour from the voxel, so a key's picture can never disagree with what it puts
//! in the world. Everything else — tools, components, equippables, vehicles — is an item
//! and nothing more.
//!
//! **Adding content is a localised diff.** A new block: a [`Block`] variant, a [`Block::def`]
//! arm, and — if a player gets to place it — an [`Item`] variant, an [`Item::def`] arm, and
//! an [`Item::ALL`] entry. A new craftable thing: the item half alone, with a recipe.
//! The pad, the rig and the wire all read these tables; a holdable item also takes a slot
//! in [`crate::code`]'s ROSETTE and a picture in [`crate::glyph`], and forgetting either
//! is a compile error, not a missing thing in somebody's hands.

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
    /// How much of the speed you land on this at you get back, upwards.
    ///
    /// One number rather than a springiness column and a softness column beside it:
    /// anything that throws a player back into the air is by the same token something they
    /// did not land hard on, so `bounce > 0` *is* [`Block::soft`] and the two cannot
    /// disagree. Ordinary ground is `0.0` — it stops you, and a hard enough stop hurts.
    ///
    /// Must stay under one: a block that gives back everything it took is a player bouncing
    /// for ever, and `nothing_bounces_higher_than_it_fell` refuses one.
    bounce: f32,
}

/// Plain ground: something you stand on and nothing more. Every row below is written as a
/// difference from it, so what a block row says is exactly what makes that block unusual.
const GROUND: BlockDef = BlockDef {
    color: [0.0, 0.0, 0.0],
    solid: true,
    bounce: 0.0,
};

impl Block {
    /// The table. Exhaustive on purpose: a variant with no arm is a compile error, where
    /// a lookup into a table of rows was a panic the first time a peer named the block.
    fn def(self) -> BlockDef {
        match self {
            Block::Air => BlockDef {
                solid: false,
                ..GROUND
            },
            Block::Grass => BlockDef {
                color: [0.36, 0.63, 0.19],
                ..GROUND
            },
            Block::Dirt => BlockDef {
                color: [0.42, 0.28, 0.16],
                ..GROUND
            },
            Block::Stone => BlockDef {
                color: [0.45, 0.45, 0.47],
                ..GROUND
            },
            Block::Sand => BlockDef {
                color: [0.80, 0.74, 0.48],
                ..GROUND
            },
            Block::Wood => BlockDef {
                color: [0.36, 0.25, 0.13],
                ..GROUND
            },
            Block::Leaves => BlockDef {
                // Distinctly darker and bluer than grass — at a distance the two greens
                // have to stay tellable apart, or a forest reads as a lumpy field.
                color: [0.08, 0.31, 0.12],
                ..GROUND
            },
            Block::Bedrock => BlockDef {
                color: [0.10, 0.10, 0.12],
                ..GROUND
            },
            Block::Cushion => BlockDef {
                // Nothing in the world is pink, so a cushion reads as made rather than
                // grown from across a valley — which is the point of the first thing a
                // player crafts.
                color: [0.86, 0.36, 0.60],
                // A bounce reaches a sixth of the height it came from, so a ten-block drop
                // throws a child a block and a half back up — visibly a trampoline, and not
                // so much that a fall from a tower relaunches them off the pad and onto the
                // stone beside it.
                bounce: 0.4,
                ..GROUND
            },
        }
    }

    pub fn color(self) -> [f32; 3] {
        self.def().color
    }

    pub fn solid(self) -> bool {
        self.def().solid
    }

    /// How much of the speed a player lands on this at it gives back, upwards.
    pub fn bounce(self) -> f32 {
        self.def().bounce
    }

    /// Does landing on this break a fall? Exactly the springy blocks — see
    /// [`BlockDef::bounce`] for why that is one question and not two.
    pub fn soft(self) -> bool {
        self.bounce() > 0.0
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

/// What holding the use button does — the behaviour dimension of the registry.
///
/// One row of numbers, not a family of kinds: every use is the same act — reach out along
/// the aim and chew through the block there — and an item is only ever *further*,
/// *faster*, or *closer up* than a bare hand. A hammer and a rifle differ in three floats,
/// so the game loop has one path through them and no per-item branch anywhere.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Use {
    /// How far along the aim it works, in blocks.
    pub reach: f32,
    /// Blocks broken per second of holding the button.
    pub speed: f32,
    /// How far the view narrows while the button is held. 1.0 is no scope at all.
    pub zoom: f32,
}

impl Use {
    /// A bare hand, and the baseline every tool is measured against.
    ///
    /// Anything that is not a tool uses this, so "what does the button do" always has an
    /// answer: holding a nail is not a special case, it is just a hand with a nail in it.
    pub const BARE_HAND: Use = Use {
        reach: 6.0,
        speed: 2.0,
        zoom: 1.0,
    };
}

/// What wearing a thing does to a fall — the behaviour dimension of the equippables, and
/// the exact counterpart of [`Use`] for the tools.
///
/// A row of numbers rather than a family of kinds, for the same reason [`Use`] is one:
/// every fall is the same act, and a parachute only makes it *slower* and *steerable*. The
/// one path through gravity reads these two floats and has no parachute-shaped branch in
/// it, which is also what makes the terminal speed of an ordinary fall a row here —
/// [`Fall::UNAIDED`] — rather than a constant beside the gravity that produces it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fall {
    /// Fastest a fall gets, in blocks per second. What you hit the ground at, and so —
    /// through [`crate::player::fall_damage`] — how much a landing hurts.
    pub max_speed: f32,
    /// What the stick is worth on the way down, as a multiple of the speed the same stick
    /// buys on the ground. One is ordinary air control; more is a canopy you can steer.
    pub drift: f32,
}

impl Fall {
    /// Falling with nothing on, and the baseline every equippable is measured against.
    ///
    /// Anything that is not an equippable falls like this, so "how do I fall" always has an
    /// answer and holding a hammer is not a special case.
    pub const UNAIDED: Fall = Fall {
        max_speed: 55.0,
        drift: 1.0,
    };
}

/// What an item *is*.
///
/// [`Class::Block`] says which voxel the item puts in the world; [`Class::Tool`] says what
/// its use button does. The rest are the classes the requirement sheets ask for, and carry
/// only a colour, because a colour is all there is to draw of a thing with no behaviour
/// yet.
///
/// A [`Use`] lives on the tool arm and a [`Fall`] on the equippable arm, rather than beside
/// the class, so a component cannot be given a rifle's range or a parachute's canopy by a
/// mistyped table row: "a tool is the thing you use, an equippable is the thing you fall
/// with" is a fact about the shape of this enum.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Class {
    /// Goes in the world as this voxel, and is drawn in its colour.
    Block(Block),
    /// Swung or fired at the world. Linear RGB, as everywhere else in this file.
    Tool { color: [f32; 3], using: Use },
    /// Not used on its own — spent making something else.
    Component { color: [f32; 3] },
    /// Held out, and changes how its holder falls.
    Equippable { color: [f32; 3], falling: Fall },
    /// Ridden.
    Vehicle { color: [f32; 3] },
}

/// Everything a player can hold, in wire order.
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

impl Item {
    /// Every item, in wire order. `all_lists_every_item` is what keeps it complete.
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
            // Swung, so it reaches no further than an arm — what a hammer buys is pace.
            Item::Hammer => ItemDef {
                name: "hammer",
                class: Tool {
                    color: [0.55, 0.35, 0.18],
                    using: Use {
                        speed: 5.0,
                        ..Use::BARE_HAND
                    },
                },
                recipe: &[(Item::Wood, 2), (Item::Nail, 1)],
            },
            // The fastest thing in the game, and the last word in digging: a block a
            // frame or two. Nothing digs faster, because nothing may outrun `EDIT_RATE`.
            Item::Drill => ItemDef {
                name: "drill",
                class: Tool {
                    color: [0.90, 0.45, 0.08],
                    using: Use {
                        speed: 8.0,
                        ..Use::BARE_HAND
                    },
                },
                recipe: &[(Item::Wood, 1), (Item::Stone, 2), (Item::Nail, 2)],
            },
            // The guns break blocks at a distance — there is nothing else in the world to
            // shoot at, and knocking the top off a hill from the next hill over is the
            // whole game. Quicker than a hand, slower than the tools you have to walk to.
            Item::Handgun => ItemDef {
                name: "handgun",
                class: Tool {
                    color: [0.22, 0.22, 0.26],
                    using: Use {
                        reach: 24.0,
                        speed: 4.0,
                        ..Use::BARE_HAND
                    },
                },
                recipe: &[(Item::Wood, 1), (Item::Nail, 2)],
            },
            // Further than anything else, and the only thing that narrows the view: at 64
            // blocks a target is a few pixels wide, so the scope is what makes the range
            // usable rather than a number in a table.
            Item::Rifle => ItemDef {
                name: "rifle",
                class: Tool {
                    color: [0.30, 0.23, 0.16],
                    using: Use {
                        reach: 64.0,
                        speed: 3.0,
                        zoom: 3.0,
                    },
                },
                recipe: &[(Item::Wood, 2), (Item::Nail, 3)],
            },
            // Held out, not buckled on: typing its code is the only equip there is, so
            // opening the canopy is the d-pad and needs no button of its own. Six blocks a
            // second is a walk downwards — slow enough that no drop hurts, which
            // `a_parachute_makes_every_fall_survivable` is what pins.
            Item::Parachute => ItemDef {
                name: "parachute",
                class: Equippable {
                    color: [0.92, 0.28, 0.34],
                    falling: Fall {
                        max_speed: 6.0,
                        drift: 1.6,
                    },
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

    /// What the use button does while this is in hand. Everything that is not a tool is a
    /// hand holding something, which is the only sense in which a nail has a behaviour.
    pub fn using(self) -> Use {
        match self.class() {
            Class::Tool { using, .. } => using,
            _ => Use::BARE_HAND,
        }
    }

    /// How its holder falls while this is in hand. Everything that is not an equippable
    /// falls like a person holding nothing, which is what makes gravity one path.
    pub fn falling(self) -> Fall {
        match self.class() {
            Class::Equippable { falling, .. } => falling,
            _ => Fall::UNAIDED,
        }
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

    /// Linear RGB: the pad's key, and the cube in its owner's hand. A block item is
    /// drawn in the colour of the block it places, so the two cannot drift.
    pub fn color(self) -> [f32; 3] {
        match self.class() {
            Class::Block(b) => b.color(),
            Class::Tool { color, .. }
            | Class::Component { color }
            | Class::Equippable { color, .. }
            | Class::Vehicle { color } => color,
        }
    }

    /// The item with this name, for a person typing one on a command line. Read off the
    /// same table the name came from, so there is no list of spellings to keep in step.
    pub fn named(name: &str) -> Option<Item> {
        Item::ALL.iter().copied().find(|i| i.name() == name)
    }

    /// Position in [`Item::ALL`] — the wire id, and the slot an inventory keeps it in.
    pub fn index(self) -> usize {
        self as usize
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

    /// [`Item::ALL`] is the array an inventory is and the list every surface draws from —
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

    /// A code landing on a non-solid block would let a player place a hole, or place
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
            assert!(Item::ALL.contains(&item), "{item:?} is not holdable");
            assert!(!item.recipe().is_empty(), "{item:?} cannot be made");
        }
        assert_eq!(Item::Cushion.places(), Some(Block::Cushion), "placeable");
    }

    /// A tool is exactly the thing that does something, and the only thing that does. A
    /// tool no better than a hand is a recipe a child pays for and gets nothing from; a
    /// component that digs is a table row that wandered.
    #[test]
    fn only_tools_do_anything() {
        for item in Item::ALL {
            let is_tool = matches!(item.class(), Class::Tool { .. });
            assert_eq!(
                item.using() != Use::BARE_HAND,
                is_tool,
                "{item:?} is a {:?} that {}",
                item.class(),
                if is_tool { "does nothing" } else { "acts" }
            );
        }
    }

    /// The requirement sheet's tools, in one assertion: the hammer beats a hand, the drill
    /// beats the hammer, both are swung at arm's length, and the guns are the ones that
    /// work at a distance — the rifle furthest, and alone in having a scope.
    #[test]
    fn the_tools_behave_as_the_sheet_asks() {
        let (hand, hammer, drill) = (Use::BARE_HAND, Item::Hammer.using(), Item::Drill.using());
        assert!(hand.speed < hammer.speed && hammer.speed < drill.speed);
        assert_eq!(
            (hammer.reach, drill.reach),
            (hand.reach, hand.reach),
            "melee: a hammer buys pace, not range"
        );

        let (handgun, rifle) = (Item::Handgun.using(), Item::Rifle.using());
        assert!(hand.reach < handgun.reach && handgun.reach < rifle.reach);
        for item in Item::ALL {
            assert_eq!(
                item.using().zoom > 1.0,
                *item == Item::Rifle,
                "{item:?} and the scope"
            );
        }
        assert!(
            matches!(Item::Nail.class(), Class::Component { .. }),
            "a nail is spent in recipes and never swung"
        );
    }

    /// The equippables' half of `only_tools_do_anything`: a thing changes a fall exactly
    /// when it is worn. A parachute in the component column would be a nail that floats.
    #[test]
    fn only_equippables_change_a_fall() {
        for item in Item::ALL {
            let is_worn = matches!(item.class(), Class::Equippable { .. });
            assert_eq!(
                item.falling() != Fall::UNAIDED,
                is_worn,
                "{item:?} is a {:?} that {}",
                item.class(),
                if is_worn {
                    "falls like a stone"
                } else {
                    "floats"
                }
            );
        }
    }

    /// Every voxel kind there is — what the rules that must hold of *all* of them are checked
    /// against. The `match` is what keeps it complete: a new variant stops this compiling, so
    /// it cannot be added without joining those rules.
    fn every_block() -> [Block; 9] {
        let all = [
            Block::Air,
            Block::Grass,
            Block::Dirt,
            Block::Stone,
            Block::Sand,
            Block::Wood,
            Block::Leaves,
            Block::Bedrock,
            Block::Cushion,
        ];
        for block in all {
            match block {
                Block::Air
                | Block::Grass
                | Block::Dirt
                | Block::Stone
                | Block::Sand
                | Block::Wood
                | Block::Leaves
                | Block::Bedrock
                | Block::Cushion => {}
            }
        }
        all
    }

    /// A block that gave back everything it took would be a child bouncing on a cushion for
    /// ever, higher each time. A fraction of what it took is a trampoline; all of it is a
    /// bug — checked over every block, so the tenth cannot slip past.
    #[test]
    fn nothing_bounces_higher_than_it_fell() {
        for block in every_block() {
            assert!(
                (0.0..1.0).contains(&block.bounce()),
                "{block:?} bounces {}",
                block.bounce()
            );
            assert_eq!(block.soft(), block.bounce() > 0.0, "{block:?}");
        }
        assert!(Block::Cushion.soft(), "the one thing that is");
    }

    /// The two things the boyos asked for that change the physics, and the one assertion
    /// that says they are the only ones: everything else falls and lands like the world.
    #[test]
    fn the_cushion_and_the_parachute_are_the_only_soft_things() {
        let soft: Vec<Item> = Item::ALL
            .iter()
            .copied()
            .filter(|i| i.places().is_some_and(Block::soft) || i.falling() != Fall::UNAIDED)
            .collect();
        assert_eq!(soft, vec![Item::Cushion, Item::Parachute]);
    }

    /// The scope has to survive the struct-update the tool rows are written with: a
    /// `..BARE_HAND` under the wrong field is a rifle that reaches 64 blocks and shows you
    /// none of it.
    #[test]
    fn the_scope_narrows_the_view() {
        assert!(Item::Rifle.using().zoom > 2.0);
        assert_eq!(Use::BARE_HAND.zoom, 1.0, "a hand does not zoom");
    }
}
