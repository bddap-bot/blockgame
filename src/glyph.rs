//! What every item looks like, drawn out of coloured rectangles.
//!
//! The players cannot read. A cell that says "drill" says nothing to a four-year-old, so
//! every item here has a *picture* instead: a handful of rectangles laid out in a unit
//! square, in the item's own colour from [`crate::registry`]. There is no font, no icon
//! atlas and no artist in the loop — a new item is a new row here, exactly as it is a new
//! row in the registry.
//!
//! One colour per glyph, not a palette. Each part carries a [`Part::tone`] instead: the
//! same colour lifted toward white or dropped toward black. That is what gives a cube a lit
//! top and a shaded side without any row here being able to draw a green nail.

use bevy::prelude::Color;

use crate::registry::Item;

/// One rectangle of a picture, in fractions of the tile it is drawn in. `x` runs right and
/// `y` runs *down*, matching the screen and the UI layout that eventually places it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Part {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Corner radius, as a fraction of the part's shorter side. `0.0` is a square corner
    /// and `1.0` is as round as the shape goes — a circle, on a square part.
    pub round: f32,
    /// What to do to the item's colour. `1.0` is the colour itself; below darkens toward
    /// black and above lifts toward white. Shading is a number rather than a second colour
    /// column so a glyph cannot end up drawn in a colour the item is not.
    pub tone: f32,
}

const fn p(x: f32, y: f32, w: f32, h: f32, round: f32, tone: f32) -> Part {
    Part {
        x,
        y,
        w,
        h,
        round,
        tone,
    }
}

/// The shaded top, front and side of a cube, in that order — the base every ground block
/// is drawn as, with its own markings stacked on top.
///
/// Written once because seven items are a cube with something on it, and a cube whose lit
/// face wandered between them would read as seven different shapes.
const CUBE: [Part; 3] = [
    p(0.12, 0.14, 0.76, 0.20, 0.04, 1.42),
    p(0.12, 0.32, 0.52, 0.54, 0.04, 1.0),
    p(0.64, 0.32, 0.24, 0.54, 0.04, 0.62),
];

/// Plain ground, and the thing a child digs first.
const DIRT: &[Part] = &[
    CUBE[0],
    CUBE[1],
    CUBE[2],
    p(0.20, 0.48, 0.12, 0.12, 1.0, 0.70),
    p(0.42, 0.66, 0.10, 0.10, 1.0, 0.70),
];

/// Dirt with a fringe of blades standing up off the lit face.
const GRASS: &[Part] = &[
    CUBE[0],
    CUBE[1],
    CUBE[2],
    p(0.20, 0.04, 0.09, 0.12, 0.5, 1.70),
    p(0.44, 0.01, 0.09, 0.15, 0.5, 1.70),
    p(0.68, 0.05, 0.09, 0.11, 0.5, 1.70),
];

/// Chips of a lighter grey, where dirt has darker specks — the two are the same shape and
/// nearly the same brightness, so the marks are what tells them apart at arm's length.
const STONE: &[Part] = &[
    CUBE[0],
    CUBE[1],
    CUBE[2],
    p(0.18, 0.42, 0.16, 0.14, 0.4, 1.45),
    p(0.38, 0.62, 0.14, 0.12, 0.4, 1.45),
    p(0.20, 0.70, 0.10, 0.09, 0.4, 1.45),
];

/// Grains: many small dots rather than a few big ones.
const SAND: &[Part] = &[
    CUBE[0],
    CUBE[1],
    CUBE[2],
    p(0.20, 0.44, 0.07, 0.07, 1.0, 0.72),
    p(0.36, 0.56, 0.06, 0.06, 1.0, 0.72),
    p(0.22, 0.66, 0.06, 0.06, 1.0, 0.72),
    p(0.44, 0.72, 0.07, 0.07, 1.0, 0.72),
    p(0.48, 0.42, 0.06, 0.06, 1.0, 0.72),
];

/// A log, end-on: the one block that is not a cube, because wood and dirt are the same
/// brown and a child picking the wrong one gets no hammer.
const WOOD: &[Part] = &[
    p(0.26, 0.20, 0.48, 0.68, 0.16, 0.72),
    p(0.24, 0.12, 0.52, 0.24, 1.0, 1.30),
    p(0.34, 0.16, 0.32, 0.16, 1.0, 0.85),
    p(0.44, 0.19, 0.12, 0.10, 1.0, 1.30),
];

/// A cluster of round leaves — nothing else in the game is drawn out of overlapping
/// circles, so a forest reads as a forest even to somebody who has not learnt the colour.
const LEAVES: &[Part] = &[
    p(0.08, 0.30, 0.44, 0.44, 1.0, 0.85),
    p(0.44, 0.22, 0.46, 0.46, 1.0, 1.35),
    p(0.26, 0.48, 0.46, 0.46, 1.0, 1.0),
];

/// A pillow: the only soft-cornered block, which is the whole of what a cushion is.
const CUSHION: &[Part] = &[
    p(0.06, 0.24, 0.88, 0.54, 0.55, 1.0),
    p(0.18, 0.32, 0.30, 0.12, 0.5, 1.45),
    p(0.45, 0.46, 0.10, 0.10, 1.0, 0.65),
];

/// A spike, not a tee. The round head is barely wider than the shank and the point is
/// long, so that at the size the code pad draws it — grey, beside the hammer, with no
/// colour to tell them apart — it is unmistakably the smaller and pointier of the two.
const NAIL: &[Part] = &[
    p(0.38, 0.09, 0.24, 0.13, 1.0, 1.40),
    p(0.44, 0.20, 0.12, 0.38, 0.1, 1.0),
    p(0.46, 0.56, 0.08, 0.14, 0.0, 0.85),
    p(0.47, 0.68, 0.06, 0.12, 0.0, 0.70),
    p(0.48, 0.78, 0.04, 0.10, 0.0, 0.55),
];

/// A claw hammer, and deliberately **asymmetric**: the head hangs further to one side than
/// the other and the claw splits. A symmetric head over a centred handle is the same
/// silhouette as a nail, and those two are keys two and three of the code.
const HAMMER: &[Part] = &[
    p(0.30, 0.12, 0.44, 0.22, 0.06, 1.55),
    p(0.16, 0.13, 0.16, 0.10, 0.4, 1.35),
    p(0.16, 0.25, 0.16, 0.09, 0.4, 1.35),
    p(0.46, 0.32, 0.13, 0.56, 0.08, 0.62),
];

/// A power drill facing right: body, grip, chuck, and a bit that steps down to a point.
const DRILL: &[Part] = &[
    p(0.16, 0.26, 0.40, 0.34, 0.14, 1.0),
    p(0.22, 0.56, 0.20, 0.30, 0.12, 0.65),
    p(0.56, 0.34, 0.12, 0.18, 0.05, 1.45),
    p(0.68, 0.38, 0.12, 0.10, 0.0, 1.60),
    p(0.80, 0.40, 0.10, 0.06, 0.0, 1.60),
];

const HANDGUN: &[Part] = &[
    p(0.18, 0.28, 0.58, 0.16, 0.04, 1.35),
    p(0.24, 0.42, 0.18, 0.32, 0.06, 0.70),
    p(0.44, 0.44, 0.14, 0.06, 0.0, 0.50),
];

/// Longer than anything else in the game, and the only thing with a scope on it.
const RIFLE: &[Part] = &[
    p(0.05, 0.34, 0.82, 0.11, 0.04, 1.35),
    p(0.05, 0.42, 0.30, 0.19, 0.09, 0.72),
    p(0.36, 0.18, 0.32, 0.11, 0.05, 1.60),
    p(0.46, 0.28, 0.08, 0.07, 0.0, 0.55),
    p(0.38, 0.44, 0.11, 0.22, 0.05, 0.72),
];

/// A canopy on two lines, with somebody hanging under it.
///
/// Stacked bands rather than the obvious ring of circles: a scalloped canopy is the same
/// picture as [`LEAVES`], and a child who has to tell a parachute from a bush by its colour
/// has been told nothing. A dome that widens downward is a shape nothing else here has.
const PARACHUTE: &[Part] = &[
    p(0.30, 0.08, 0.40, 0.14, 0.5, 1.35),
    p(0.16, 0.18, 0.68, 0.14, 0.35, 1.15),
    p(0.04, 0.28, 0.92, 0.16, 0.30, 0.90),
    p(0.16, 0.44, 0.05, 0.24, 0.4, 0.55),
    p(0.79, 0.44, 0.05, 0.24, 0.4, 0.55),
    p(0.38, 0.66, 0.24, 0.22, 0.35, 0.62),
];

const CAR: &[Part] = &[
    p(0.06, 0.42, 0.88, 0.26, 0.13, 1.0),
    p(0.26, 0.20, 0.46, 0.26, 0.14, 1.25),
    p(0.32, 0.26, 0.34, 0.13, 0.06, 1.75),
    p(0.16, 0.60, 0.22, 0.22, 1.0, 0.30),
    p(0.62, 0.60, 0.22, 0.22, 1.0, 0.30),
];

/// The picture of one item. Exhaustive: a new item cannot reach a player without a face.
pub fn of(item: Item) -> &'static [Part] {
    match item {
        Item::Grass => GRASS,
        Item::Dirt => DIRT,
        Item::Stone => STONE,
        Item::Sand => SAND,
        Item::Wood => WOOD,
        Item::Leaves => LEAVES,
        Item::Cushion => CUSHION,
        Item::Nail => NAIL,
        Item::Hammer => HAMMER,
        Item::Drill => DRILL,
        Item::Handgun => HANDGUN,
        Item::Rifle => RIFLE,
        Item::Parachute => PARACHUTE,
        Item::Car => CAR,
    }
}

/// An item's colour, shaded by a part's [`Part::tone`]. Linear throughout, as the registry
/// is.
pub fn shaded(base: [f32; 3], tone: f32) -> Color {
    let mix = |c: f32| {
        if tone <= 1.0 {
            c * tone
        } else {
            let lift = (tone - 1.0).min(1.0);
            c + (1.0 - c) * lift
        }
    };
    Color::linear_rgb(mix(base[0]), mix(base[1]), mix(base[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing may spill outside the tile it is drawn in: a part that does draws over the
    /// item beside it, and in a grid of fourteen that is somebody else's picture.
    #[test]
    fn every_part_stays_inside_its_tile() {
        for item in Item::ALL {
            for part in of(*item) {
                let inside = part.x >= 0.0
                    && part.y >= 0.0
                    && part.x + part.w <= 1.0
                    && part.y + part.h <= 1.0
                    && part.w > 0.0
                    && part.h > 0.0;
                assert!(inside, "{item:?} has a part outside its tile: {part:?}");
                assert!((0.0..=1.0).contains(&part.round), "{item:?}: {part:?}");
                assert!((0.0..=2.0).contains(&part.tone), "{item:?}: {part:?}");
            }
        }
    }

    /// A picture, not a swatch. One rectangle is a coloured square, which is what the
    /// hotbar already had and what the players could not tell apart.
    #[test]
    fn every_item_has_a_picture() {
        for item in Item::ALL {
            assert!(of(*item).len() >= 3, "{item:?} is barely drawn");
        }
    }

    /// Two items drawn identically are one item as far as a non-reader is concerned. The
    /// pair this is really about is wood and dirt: near enough the same brown that colour
    /// alone cannot separate them, so one of them had to stop being a cube.
    #[test]
    fn no_two_items_are_drawn_the_same() {
        for a in Item::ALL {
            for b in Item::ALL {
                if a == b {
                    continue;
                }
                let same_shape = of(*a) == of(*b);
                let d: f32 = a
                    .color()
                    .iter()
                    .zip(b.color())
                    .map(|(x, y)| (x - y) * (x - y))
                    .sum();
                assert!(
                    !same_shape || d.sqrt() > 0.25,
                    "{a:?} and {b:?} are the same picture in the same colour"
                );
            }
        }
        assert_ne!(of(Item::Wood), of(Item::Dirt), "the brown pair");
    }

    #[test]
    fn tone_darkens_below_one_and_lifts_above() {
        let grey = [0.5, 0.5, 0.5];
        assert_eq!(shaded(grey, 1.0), Color::linear_rgb(0.5, 0.5, 0.5));
        assert_eq!(shaded(grey, 0.5), Color::linear_rgb(0.25, 0.25, 0.25));
        assert_eq!(shaded(grey, 2.0), Color::linear_rgb(1.0, 1.0, 1.0));
    }
}
