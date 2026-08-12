//! What each item looks like when it is not in the world: the silhouettes the crafting
//! rig and the constellation both draw.
//!
//! One table, two surfaces. A nail read as a nail on the rig and as something else on the
//! hotbar would be two things to learn instead of one, and a child who cannot read has
//! only the shape to go on — so the shape is written once, here, and both surfaces spend
//! it.
//!
//! Every silhouette is a table of boxes in the item's own colour, like every other model
//! in this game, so a new item is drawn by whichever row of [`icon`] its class lands on
//! with no asset to make.

use crate::avatar::{self, Part, Skin, part};
use crate::registry::{Class, Item};

/// A blank cube, for anything whose whole identity is its colour: the blocks, which the
/// player has already been digging up all afternoon and knows on sight.
const BLOCK: &[Part] = &[part(
    Skin::Paint(Item::Grass),
    [1.0, 1.0, 1.0],
    [0.0, 0.0, 0.0],
)];

const NAIL: &[Part] = &[
    part(Skin::Paint(Item::Nail), [0.5, 0.13, 0.5], [0.0, 0.44, 0.0]),
    part(Skin::Paint(Item::Nail), [0.16, 0.7, 0.16], [0.0, 0.04, 0.0]),
    part(
        Skin::Paint(Item::Nail),
        [0.07, 0.2, 0.07],
        [0.0, -0.38, 0.0],
    ),
];

const HAMMER: &[Part] = &[
    part(
        Skin::Paint(Item::Hammer),
        [0.17, 0.95, 0.17],
        [0.0, -0.13, 0.0],
    ),
    part(Skin::Gear, [0.78, 0.28, 0.30], [0.0, 0.42, 0.0]),
    part(Skin::Gear, [0.22, 0.20, 0.34], [-0.44, 0.30, 0.0]),
];

const DRILL: &[Part] = &[
    part(
        Skin::Paint(Item::Drill),
        [0.56, 0.52, 0.44],
        [0.0, 0.28, 0.0],
    ),
    part(
        Skin::Paint(Item::Drill),
        [0.20, 0.30, 0.20],
        [0.0, 0.62, 0.0],
    ),
    part(Skin::Gear, [0.30, 0.26, 0.26], [0.0, -0.06, 0.0]),
    part(Skin::Gear, [0.19, 0.24, 0.19], [0.0, -0.28, 0.0]),
    part(Skin::Dark, [0.10, 0.22, 0.10], [0.0, -0.48, 0.0]),
];

const HANDGUN: &[Part] = &[
    part(
        Skin::Paint(Item::Handgun),
        [0.92, 0.22, 0.16],
        [0.06, 0.24, 0.0],
    ),
    part(
        Skin::Paint(Item::Handgun),
        [0.24, 0.52, 0.16],
        [-0.26, -0.16, 0.0],
    ),
    part(Skin::Gear, [0.14, 0.12, 0.10], [-0.05, 0.06, 0.0]),
];

const RIFLE: &[Part] = &[
    part(
        Skin::Paint(Item::Rifle),
        [1.30, 0.14, 0.13],
        [0.10, 0.12, 0.0],
    ),
    part(
        Skin::Paint(Item::Rifle),
        [0.40, 0.30, 0.15],
        [-0.48, -0.04, 0.0],
    ),
    part(
        Skin::Paint(Item::Rifle),
        [0.20, 0.34, 0.14],
        [-0.16, -0.18, 0.0],
    ),
    part(Skin::Dark, [0.34, 0.11, 0.11], [0.16, 0.28, 0.0]),
    part(Skin::Gear, [0.08, 0.10, 0.08], [0.30, 0.30, 0.0]),
];

const PARACHUTE: &[Part] = &[
    part(
        Skin::Paint(Item::Parachute),
        [0.92, 0.20, 0.50],
        [0.0, 0.40, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.60, 0.18, 0.40],
        [0.0, 0.56, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.24, 0.14, 0.28],
        [0.0, 0.68, 0.0],
    ),
    part(Skin::Dark, [0.04, 0.52, 0.04], [-0.36, 0.02, 0.0]),
    part(Skin::Dark, [0.04, 0.52, 0.04], [0.36, 0.02, 0.0]),
    part(Skin::Gear, [0.32, 0.24, 0.24], [0.0, -0.34, 0.0]),
];

/// The player's own mitten, which is what an empty hand looks like — the thing at the
/// middle of the constellation, and the only thing there that is not an item.
///
/// The spaceman's, in the spaceman's teal, because the hand at the centre of the chart is
/// meant to read as *your* hand and not as a fifteenth thing to make.
pub const MITTEN: &[Part] = &[
    part(Skin::Trim, [0.62, 0.54, 0.44], [0.0, 0.0, 0.0]),
    part(Skin::Trim, [0.26, 0.30, 0.34], [-0.40, -0.10, 0.0]),
    part(Skin::Suit, [0.36, 0.22, 0.40], [0.0, 0.38, 0.0]),
];

/// What one item looks like off the ground, and how big to draw the table.
///
/// The car is the game's own car — the one the player drives — shrunk onto a shelf,
/// because the strongest possible label for "this makes a car" is a car.
pub fn icon(item: Item) -> (&'static [Part], f32, f32) {
    match item.class() {
        Class::Block(_) => (BLOCK, 1.0, 0.0),
        Class::Component { .. } => (NAIL, 1.0, 0.0),
        Class::Equippable { .. } => (PARACHUTE, 1.0, 0.0),
        // Every part table is written about its own feet, and the car's is the game's:
        // it is lifted by half its height so it hangs on the string like everything else.
        Class::Vehicle { .. } => (avatar::CAR, 0.54, -0.42),
        Class::Tool { .. } => match item {
            Item::Drill => (DRILL, 1.0, 0.0),
            Item::Handgun => (HANDGUN, 1.0, 0.0),
            Item::Rifle => (RIFLE, 1.0, 0.0),
            _ => (HAMMER, 1.0, 0.0),
        },
    }
}

/// The silhouette tables are written for one item each, so the colour in them is that
/// item's. Re-skinning them per node is what lets one table draw every block.
pub fn repaint(p: &Part, as_item: Item) -> Part {
    Part {
        skin: match p.skin {
            Skin::Paint(_) => Skin::Paint(as_item),
            other => other,
        },
        size: p.size,
        at: p.at,
    }
}
