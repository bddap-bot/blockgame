//! The player model — **the one place a body is built.**
//!
//! Drawn from `design/spaceman-avatar.jpg`: a white suit with teal trim, a bubble helmet
//! whose dark visor is ringed in teal, a backpack, a chest panel with a rocket on it,
//! mitten hands, knee patches and boots. Every part is the same unit cube scaled and
//! placed, so the whole model is the [`SPACEMAN`] table and swapping it for a different
//! body is an edit to that table — no other file changes.
//!
//! Coordinates are relative to the player's feet, in blocks: `+Y` is up, `-Z` is the way
//! they are facing.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::registry::Item;

/// Which material a part is painted with. Add a colour here and to [`Palette`] together.
#[derive(Clone, Copy)]
pub enum Skin {
    /// The white suit.
    Suit,
    /// Teal: trim, boots, mittens, the visor ring.
    Trim,
    /// Grey hardware — the backpack.
    Gear,
    /// Near-black gloss: the visor, and the rocket inked on the chest panel.
    Dark,
}

pub struct Part {
    pub skin: Skin,
    /// Width, height, depth in blocks.
    pub size: [f32; 3],
    /// Centre of the part, relative to the feet.
    pub at: [f32; 3],
}

const fn part(skin: Skin, size: [f32; 3], at: [f32; 3]) -> Part {
    Part { skin, size, at }
}

/// Front face of the torso. Anything decorating the chest sits just proud of it, so the
/// panel and the rocket on it are stacked in `z` from this one number.
const CHEST_Z: f32 = -0.13;

/// The spaceman, one row per box.
pub const SPACEMAN: &[Part] = &[
    // legs, knee patches, boots
    part(Skin::Suit, [0.20, 0.62, 0.20], [-0.13, 0.46, 0.0]),
    part(Skin::Suit, [0.20, 0.62, 0.20], [0.13, 0.46, 0.0]),
    part(Skin::Trim, [0.22, 0.15, 0.22], [-0.13, 0.50, 0.0]),
    part(Skin::Trim, [0.22, 0.15, 0.22], [0.13, 0.50, 0.0]),
    part(Skin::Trim, [0.24, 0.16, 0.30], [-0.13, 0.08, -0.03]),
    part(Skin::Trim, [0.24, 0.16, 0.30], [0.13, 0.08, -0.03]),
    // torso, belt, and the collar ring the helmet sits in
    part(Skin::Suit, [0.46, 0.58, 0.26], [0.0, 1.06, 0.0]),
    part(Skin::Trim, [0.48, 0.09, 0.28], [0.0, 0.815, 0.0]),
    part(Skin::Trim, [0.40, 0.08, 0.28], [0.0, 1.36, 0.0]),
    // backpack, with a tank strapped either side of it
    part(Skin::Gear, [0.32, 0.40, 0.14], [0.0, 1.10, 0.19]),
    part(Skin::Trim, [0.07, 0.30, 0.07], [-0.10, 1.12, 0.28]),
    part(Skin::Trim, [0.07, 0.30, 0.07], [0.10, 1.12, 0.28]),
    // arms, held clear of the body like the drawing, with shoulder pads and mittens. The
    // gap to the torso is what makes them read as arms and not as a wider chest.
    part(Skin::Suit, [0.15, 0.60, 0.15], [-0.35, 1.02, 0.0]),
    part(Skin::Suit, [0.15, 0.60, 0.15], [0.35, 1.02, 0.0]),
    part(Skin::Trim, [0.20, 0.10, 0.20], [-0.35, 1.33, 0.0]),
    part(Skin::Trim, [0.20, 0.10, 0.20], [0.35, 1.33, 0.0]),
    // mittens hang below the belt, or the two teal bands merge into one from a distance
    part(Skin::Trim, [0.19, 0.16, 0.19], [-0.35, 0.68, 0.0]),
    part(Skin::Trim, [0.19, 0.16, 0.19], [0.35, 0.68, 0.0]),
    // chest panel: a teal frame around a white plate, with a rocket on it
    part(Skin::Trim, [0.26, 0.26, 0.02], [0.0, 1.12, CHEST_Z - 0.01]),
    part(Skin::Suit, [0.20, 0.20, 0.02], [0.0, 1.12, CHEST_Z - 0.02]),
    part(Skin::Dark, [0.06, 0.11, 0.02], [0.0, 1.13, CHEST_Z - 0.03]), // rocket body
    part(Skin::Dark, [0.03, 0.04, 0.02], [0.0, 1.20, CHEST_Z - 0.03]), // nose
    part(
        Skin::Dark,
        [0.03, 0.05, 0.02],
        [-0.05, 1.06, CHEST_Z - 0.03],
    ), // fins
    part(Skin::Dark, [0.03, 0.05, 0.02], [0.05, 1.06, CHEST_Z - 0.03]),
    part(Skin::Trim, [0.03, 0.03, 0.02], [0.0, 1.05, CHEST_Z - 0.04]), // exhaust
    part(Skin::Trim, [0.03, 0.03, 0.02], [0.0, 1.15, CHEST_Z - 0.04]), // porthole
    // helmet: a white bubble, a dark visor across the front, teal ringing it
    part(Skin::Suit, [0.42, 0.40, 0.42], [0.0, 1.57, 0.0]),
    part(Skin::Trim, [0.44, 0.06, 0.44], [0.0, 1.74, 0.0]), // crown band
    part(Skin::Dark, [0.26, 0.22, 0.04], [0.0, 1.58, -0.21]),
    part(Skin::Trim, [0.33, 0.04, 0.03], [0.0, 1.71, -0.215]), // visor ring
    part(Skin::Trim, [0.33, 0.04, 0.03], [0.0, 1.45, -0.215]),
    part(Skin::Trim, [0.04, 0.30, 0.03], [-0.145, 1.58, -0.215]),
    part(Skin::Trim, [0.04, 0.30, 0.03], [0.145, 1.58, -0.215]),
];

/// The cube in a player's right mitten: where it sits, and how big it is.
///
/// One cube whatever is being carried, coloured from the item table. A rifle that is
/// really a rifle is a model, and a model is a shape this file does not have yet — but a
/// coloured block in the hand is enough to see *that* somebody swapped what they are
/// holding, which is the thing the network has to prove it moved.
/// Held out in front of the mitten, not inside it: level with the hand and clear of it in
/// `z`, so the cube is its own silhouette from the front and from either side. Tucked
/// against the palm it is hidden by the arm from every angle but dead-on.
const HELD_AT: [f32; 3] = [0.35, 0.66, -0.20];
const HELD_SIZE: f32 = 0.18;

/// The materials [`SPACEMAN`] paints with. One per [`Skin`], plus one per [`Item`] for
/// whatever is in the hand.
#[derive(Resource)]
pub struct Palette {
    cube: Handle<Mesh>,
    suit: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
    gear: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    /// Built from the registry, so a new item is drawable in a hand the moment it exists.
    items: HashMap<Item, Handle<StandardMaterial>>,
}

impl Palette {
    pub fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let mut paint = |color, rough| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: rough,
                ..default()
            })
        };
        let mut items = HashMap::new();
        for item in Item::ALL {
            // Linear, because that is the space the registry's colours are in — the same
            // numbers the mesher bakes into a block's faces.
            let [r, g, b] = item.color();
            items.insert(*item, paint(Color::linear_rgb(r, g, b), 0.8));
        }
        Self {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            suit: paint(Color::srgb(0.93, 0.94, 0.95), 0.85),
            trim: paint(Color::srgb(0.10, 0.60, 0.60), 0.7),
            gear: paint(Color::srgb(0.55, 0.58, 0.62), 0.8),
            dark: paint(Color::srgb(0.06, 0.09, 0.12), 0.25),
            items,
        }
    }

    fn material(&self, skin: Skin) -> Handle<StandardMaterial> {
        match skin {
            Skin::Suit => self.suit.clone(),
            Skin::Trim => self.trim.clone(),
            Skin::Gear => self.gear.clone(),
            Skin::Dark => self.dark.clone(),
        }
    }
}

/// One player's model: the body, and the hand whose contents change as they play.
#[derive(Debug, Clone, Copy)]
pub struct Body {
    pub root: Entity,
    hand: Entity,
}

/// Spawns one player body at `transform` (its feet). The ONE site a player model is
/// created — remote players today, a visible local body tomorrow.
pub fn spawn(commands: &mut Commands, palette: &Palette, transform: Transform) -> Body {
    // Spawned empty-handed: a player who has picked nothing up yet is carrying nothing,
    // and their first pose says what they are really holding.
    let hand = commands
        .spawn((
            Transform {
                translation: Vec3::from(HELD_AT),
                scale: Vec3::splat(HELD_SIZE),
                ..default()
            },
            Visibility::Hidden,
        ))
        .id();
    let root = commands
        .spawn((transform, Visibility::Visible))
        .add_child(hand)
        .with_children(|body| {
            for p in SPACEMAN {
                body.spawn((
                    Mesh3d(palette.cube.clone()),
                    MeshMaterial3d(palette.material(p.skin)),
                    Transform {
                        translation: Vec3::from(p.at),
                        scale: Vec3::from(p.size),
                        ..default()
                    },
                ));
            }
        })
        .id();
    Body { root, hand }
}

/// Puts `held` in this body's hand, or empties it. Idempotent — the caller is a pose
/// stream, and re-stating what is already there costs a component write.
pub fn show_held(commands: &mut Commands, palette: &Palette, body: Body, held: Option<Item>) {
    match held.and_then(|item| palette.items.get(&item)) {
        Some(material) => {
            commands.entity(body.hand).insert((
                Mesh3d(palette.cube.clone()),
                MeshMaterial3d(material.clone()),
                Visibility::Visible,
            ));
        }
        None => {
            commands.entity(body.hand).insert(Visibility::Hidden);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How far any part may sit from the player's centre in X or Z. Wider than
    /// `player::HALF_WIDTH` on purpose — see the test below.
    const REACH_OUT: f32 = 0.45;

    /// The model stands on the ground and keeps its head under the ceiling: vertically it
    /// is exactly the collision box, so an avatar never sinks into the floor or pokes
    /// through the block above the player it draws.
    ///
    /// Sideways it is *wider* than the box, deliberately. The arms are held clear of the
    /// torso because that gap is what makes them read as arms, and a 0.6-wide box cannot
    /// hold a torso, two gaps and two arms without leaving a figure thinner than his own
    /// helmet. The cost is that an arm can dip into the wall its owner is standing flush
    /// against — the same trade Minecraft makes, and the same one the drawing asks for.
    /// [`REACH_OUT`] is the budget: it exists so a new part cannot quietly stick out a
    /// metre, not to pretend the model fits.
    #[test]
    fn the_model_stands_in_the_player_box() {
        // The held cube is a part like any other as far as the budget is concerned: it is
        // out at arm's length, which is exactly where a part is most able to stick out.
        let held = part(Skin::Gear, [HELD_SIZE; 3], HELD_AT);
        let (mut lowest, mut highest, mut widest) = (f32::MAX, f32::MIN, 0.0f32);
        for p in SPACEMAN.iter().chain(std::iter::once(&held)) {
            lowest = lowest.min(p.at[1] - p.size[1] / 2.0);
            highest = highest.max(p.at[1] + p.size[1] / 2.0);
            for axis in [0, 2] {
                widest = widest.max(p.at[axis].abs() + p.size[axis] / 2.0);
            }
        }
        assert!(lowest >= -0.01, "the model starts below the feet: {lowest}");
        assert!(
            highest <= crate::player::HEIGHT,
            "the model is taller than the player box: {highest}"
        );
        assert!(highest > 1.5, "the model is suspiciously short: {highest}");
        assert!(
            widest <= REACH_OUT,
            "the model reaches {widest} from the player's centre, past the {REACH_OUT} budget"
        );
    }

    #[test]
    fn every_part_has_volume() {
        for (i, p) in SPACEMAN.iter().enumerate() {
            assert!(
                p.size.iter().all(|d| *d > 0.0),
                "part {i} has a zero dimension"
            );
        }
    }

    /// The chest decoration is a stack of thin plates on the torso's front face: frame,
    /// then plate, then rocket. A plate level with the layer under it z-fights into a
    /// flicker — the sort of thing you only ever see on someone else's screen.
    #[test]
    fn the_chest_stack_is_layered() {
        let mut layers: Vec<f32> = SPACEMAN
            .iter()
            .filter(|p| p.at[2] < CHEST_Z && p.at[1] < 1.3)
            .map(|p| p.at[2] - p.size[2] / 2.0)
            .collect();
        assert!(layers.len() >= 6, "the chest panel lost its plates");
        assert!(
            layers.iter().all(|z| *z < CHEST_Z),
            "a chest plate is buried in the torso"
        );
        layers.sort_by(f32::total_cmp);
        layers.dedup();
        assert!(
            layers.len() >= 3,
            "frame, plate and rocket must sit at three different depths"
        );
    }
}
