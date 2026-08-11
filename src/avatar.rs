//! The models — **the one place a body is built.**
//!
//! [`SPACEMAN`] is drawn from `design/spaceman-avatar.jpg`: a white suit with teal trim, a
//! bubble helmet whose dark visor is ringed in teal, a backpack, a chest panel with a
//! rocket on it, mitten hands, knee patches and boots. [`CAR`] is the buggy he drives.
//! Every part of both is the same unit cube scaled and placed, so a model *is* its table
//! and swapping one for a different shape is an edit to that table — no other file changes.
//!
//! Coordinates are relative to the thing's own feet, in blocks: `+Y` is up, `-Z` is the way
//! it faces. A player's feet and a car's underside are the same origin, which is what lets
//! [`crate::vehicle::SEAT`] be one offset between them.

use bevy::prelude::*;

use crate::registry::Item;

/// Which material a part is painted with. Add a colour here and to [`Palette`] together.
#[derive(Clone, Copy)]
pub enum Skin {
    /// The white suit.
    Suit,
    /// Teal: trim, boots, mittens, the visor ring, the car's lamps and hubs.
    Trim,
    /// Grey hardware — the backpack, the roll bar, the bumpers.
    Gear,
    /// Near-black gloss: the visor, the rocket inked on the chest panel, tyres and glass.
    Dark,
    /// Bodywork, painted whatever colour the registry gives [`Item::Car`] — so the car in
    /// the world and the cell in the hotbar are the same blue by construction.
    Paint(Item),
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

/// The car, one row per box: an open buggy, so the driver standing at the wheel is visible
/// from outside rather than sealed into a coloured box.
///
/// Open-topped for a second reason too — the body has no sitting pose, so a driver stands
/// on the deck. Under a roof that would be a spaceman with his helmet through the ceiling.
/// Big enough to be a car around him rather than a skateboard under him: the tub comes up
/// past his belt, which is what makes him read as being *in* it.
pub const CAR: &[Part] = &[
    // the floor pan, and the low deck inside it the driver stands on
    part(Skin::Paint(Item::Car), [1.52, 0.22, 2.30], [0.0, 0.11, 0.0]),
    part(Skin::Dark, [1.14, 0.06, 1.00], [0.0, 0.25, 0.32]),
    // the tub around him: two sides and a back
    part(
        Skin::Paint(Item::Car),
        [0.18, 0.72, 1.30],
        [-0.67, 0.58, 0.30],
    ),
    part(
        Skin::Paint(Item::Car),
        [0.18, 0.72, 1.30],
        [0.67, 0.58, 0.30],
    ),
    part(
        Skin::Paint(Item::Car),
        [1.52, 0.72, 0.18],
        [0.0, 0.58, 0.97],
    ),
    // bonnet, and the screen between it and the driver
    part(
        Skin::Paint(Item::Car),
        [1.46, 0.44, 0.95],
        [0.0, 0.44, -0.72],
    ),
    part(Skin::Dark, [1.32, 0.46, 0.06], [0.0, 0.87, -0.24]),
    part(Skin::Trim, [1.38, 0.07, 0.09], [0.0, 1.13, -0.24]),
    // wheels, at the corners of the footprint the physics drives on
    part(Skin::Dark, [0.24, 0.56, 0.56], [-0.76, 0.28, -0.90]),
    part(Skin::Dark, [0.24, 0.56, 0.56], [0.76, 0.28, -0.90]),
    part(Skin::Dark, [0.24, 0.56, 0.56], [-0.76, 0.28, 0.90]),
    part(Skin::Dark, [0.24, 0.56, 0.56], [0.76, 0.28, 0.90]),
    part(Skin::Trim, [0.28, 0.24, 0.24], [-0.76, 0.28, -0.90]),
    part(Skin::Trim, [0.28, 0.24, 0.24], [0.76, 0.28, -0.90]),
    part(Skin::Trim, [0.28, 0.24, 0.24], [-0.76, 0.28, 0.90]),
    part(Skin::Trim, [0.28, 0.24, 0.24], [0.76, 0.28, 0.90]),
    // bumpers and lamps — which end is the front, from a distance
    part(Skin::Gear, [1.44, 0.20, 0.14], [0.0, 0.24, -1.22]),
    part(Skin::Gear, [1.44, 0.20, 0.14], [0.0, 0.24, 1.22]),
    part(Skin::Trim, [0.24, 0.18, 0.08], [-0.46, 0.50, -1.17]),
    part(Skin::Trim, [0.24, 0.18, 0.08], [0.46, 0.50, -1.17]),
    part(Skin::Dark, [0.20, 0.14, 0.08], [-0.46, 0.42, 1.17]),
    part(Skin::Dark, [0.20, 0.14, 0.08], [0.46, 0.42, 1.17]),
];

/// **What each item looks like when it is being shown rather than held** — the silhouettes
/// [`crate::crafttree`] hangs on the recipe graph.
///
/// A child who cannot read tells a hammer from a rifle by its *shape*, so the graph cannot
/// be fourteen coloured cubes. Each row below is the same table of boxes every other model
/// in this file is, drawn face-on: long in `x`, tall in `y`, thin in `z`.
///
/// A block is a cube on purpose. That is what it is in the world, and a picture of grass
/// that is not the grass you dug is a second thing to learn.
fn icon(item: Item) -> &'static [Part] {
    /// A block, drawn as the block it is.
    const fn cube(item: Item) -> [Part; 1] {
        [part(Skin::Paint(item), [1.0, 1.0, 1.0], [0.0; 3])]
    }
    const GRASS: &[Part] = &cube(Item::Grass);
    const DIRT: &[Part] = &cube(Item::Dirt);
    const STONE: &[Part] = &cube(Item::Stone);
    const SAND: &[Part] = &cube(Item::Sand);
    const WOOD: &[Part] = &cube(Item::Wood);
    const LEAVES: &[Part] = &cube(Item::Leaves);
    const CUSHION: &[Part] = &cube(Item::Cushion);

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
        // The car you drive, shrunk — one table of boxes, so the thing in the graph and
        // the thing in the field can never be two different cars.
        Item::Car => CAR,
    }
}

const NAIL: &[Part] = &[
    part(Skin::Paint(Item::Nail), [0.10, 0.78, 0.10], [0.0, 0.0, 0.0]),
    part(
        Skin::Paint(Item::Nail),
        [0.38, 0.10, 0.38],
        [0.0, 0.44, 0.0],
    ),
    part(Skin::Gear, [0.05, 0.18, 0.05], [0.0, -0.48, 0.0]),
];

const HAMMER: &[Part] = &[
    part(
        Skin::Paint(Item::Hammer),
        [0.13, 0.92, 0.13],
        [0.0, -0.06, 0.0],
    ),
    part(Skin::Gear, [0.56, 0.24, 0.24], [0.06, 0.48, 0.0]),
    // The claw: the one line that stops a hammer reading as a lollipop.
    part(Skin::Gear, [0.16, 0.22, 0.22], [-0.30, 0.34, 0.0]),
];

const DRILL: &[Part] = &[
    part(
        Skin::Paint(Item::Drill),
        [0.44, 0.46, 0.34],
        [0.02, 0.12, 0.0],
    ),
    part(Skin::Dark, [0.19, 0.42, 0.21], [-0.08, -0.28, 0.0]),
    part(Skin::Gear, [0.52, 0.10, 0.10], [0.50, 0.18, 0.0]),
];

const HANDGUN: &[Part] = &[
    part(
        Skin::Paint(Item::Handgun),
        [0.72, 0.16, 0.14],
        [0.08, 0.14, 0.0],
    ),
    part(Skin::Gear, [0.56, 0.10, 0.15], [0.16, 0.26, 0.0]),
    part(
        Skin::Paint(Item::Handgun),
        [0.18, 0.40, 0.16],
        [-0.20, -0.14, 0.0],
    ),
];

const RIFLE: &[Part] = &[
    part(Skin::Gear, [1.22, 0.11, 0.11], [0.16, 0.08, 0.0]),
    part(
        Skin::Paint(Item::Rifle),
        [0.52, 0.19, 0.15],
        [-0.06, 0.00, 0.0],
    ),
    part(
        Skin::Paint(Item::Rifle),
        [0.36, 0.28, 0.14],
        [-0.44, -0.08, 0.0],
    ),
    // The scope, which is what tells it from the handgun at a glance.
    part(Skin::Dark, [0.32, 0.12, 0.12], [0.02, 0.26, 0.0]),
];

const PARACHUTE: &[Part] = &[
    part(
        Skin::Paint(Item::Parachute),
        [0.34, 0.24, 0.30],
        [0.0, 0.40, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.28, 0.19, 0.26],
        [-0.28, 0.31, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.28, 0.19, 0.26],
        [0.28, 0.31, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.24, 0.15, 0.22],
        [0.50, 0.16, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.24, 0.15, 0.22],
        [-0.50, 0.16, 0.0],
    ),
    part(Skin::Dark, [0.04, 0.46, 0.04], [-0.25, -0.06, 0.0]),
    part(Skin::Dark, [0.04, 0.46, 0.04], [0.25, -0.06, 0.0]),
    part(Skin::Gear, [0.32, 0.16, 0.20], [0.0, -0.34, 0.0]),
];

/// Spawns `item`'s silhouette, scaled and centred to fill a one-block cube whatever the
/// table's own scale was, and returns its root.
///
/// The fit is measured off the table rather than written next to it: a model is a shape,
/// and asking whoever adds one to also work out its bounding box is asking for a car that
/// is a speck or a nail that fills the screen.
pub fn spawn_icon(commands: &mut Commands, palette: &Palette, item: Item) -> Entity {
    let parts = icon(item);
    let (centre, size) = bounds(parts);
    let fit = 1.0 / size.max_element();
    commands
        .spawn((Transform::default(), Visibility::Visible))
        .with_children(|root| {
            root.spawn((
                Transform::from_translation(-centre * fit).with_scale(Vec3::splat(fit)),
                Visibility::Visible,
            ))
            .with_children(|fitted| build(fitted, palette, parts));
        })
        .id()
}

/// The centre and the extent of everything a table draws.
fn bounds(parts: &[Part]) -> (Vec3, Vec3) {
    let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in parts {
        let (at, half) = (Vec3::from(p.at), Vec3::from(p.size) / 2.0);
        low = low.min(at - half);
        high = high.max(at + half);
    }
    ((low + high) / 2.0, high - low)
}

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
    /// Built from the registry and indexed by [`Item::index`], so a new item is drawable
    /// the moment it exists and "no material for that item" is not a state to handle.
    items: [Handle<StandardMaterial>; Item::COUNT],
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
        // Linear, because that is the space the registry's colours are in — the same
        // numbers the mesher bakes into a block's faces.
        let items = std::array::from_fn(|i| {
            let [r, g, b] = Item::ALL[i].color();
            paint(Color::linear_rgb(r, g, b), 0.8)
        });
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
            Skin::Paint(item) => self.item(item),
        }
    }

    /// The colour the registry gives an item — what it is drawn in wherever it appears.
    pub fn item(&self, item: Item) -> Handle<StandardMaterial> {
        self.items[item.index()].clone()
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
        .with_children(|body| build(body, palette, SPACEMAN))
        .id();
    Body { root, hand }
}

/// Spawns one car at `transform` (its underside). The ONE site a car model is created —
/// the local player's own and every peer's alike, so what you drive and what everybody
/// else watches you drive is the same table of boxes.
pub fn spawn_car(commands: &mut Commands, palette: &Palette, transform: Transform) -> Entity {
    commands
        .spawn((transform, Visibility::Visible))
        .with_children(|car| build(car, palette, CAR))
        .id()
}

/// One table of boxes under a parent. The only place a [`Part`] becomes geometry.
fn build(model: &mut ChildSpawnerCommands, palette: &Palette, parts: &[Part]) {
    for p in parts {
        model.spawn((
            Mesh3d(palette.cube.clone()),
            MeshMaterial3d(palette.material(p.skin)),
            Transform {
                translation: Vec3::from(p.at),
                scale: Vec3::from(p.size),
                ..default()
            },
        ));
    }
}

/// Puts `held` in this body's hand, or empties it. Idempotent — the caller is a pose
/// stream, and re-stating what is already there costs a component write.
pub fn show_held(commands: &mut Commands, palette: &Palette, body: Body, held: Option<Item>) {
    match held {
        Some(item) => {
            commands.entity(body.hand).insert((
                Mesh3d(palette.cube.clone()),
                MeshMaterial3d(palette.item(item)),
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
    use crate::vehicle;

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
        for (name, parts) in [("spaceman", SPACEMAN), ("car", CAR)] {
            for (i, p) in parts.iter().enumerate() {
                assert!(
                    p.size.iter().all(|d| *d > 0.0),
                    "{name} part {i} has a zero dimension"
                );
            }
        }
    }

    /// The car stands on its wheels, and its wheels stand on the corners the physics
    /// samples the ground at. Drift between the two is a car visibly hovering over a step
    /// it has already climbed, or sunk into one it has not.
    #[test]
    fn the_car_sits_on_the_footprint_it_drives_on() {
        let lowest = CAR
            .iter()
            .map(|p| p.at[1] - p.size[1] / 2.0)
            .fold(f32::MAX, f32::min);
        assert!(
            lowest.abs() < 0.01,
            "the car's lowest part is at {lowest}, not on the ground"
        );

        // The wheels are the widest parts along both axes, and their centres are the
        // footprint.
        let corner = |x: f32, z: f32| {
            CAR.iter().any(|p| {
                (p.at[0] - x).abs() < 0.01 && (p.at[2] - z).abs() < 0.01 && p.size[1] > 0.3
            })
        };
        for (x, z) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let (x, z) = (x * vehicle::HALF_WIDTH, z * vehicle::HALF_LENGTH);
            assert!(corner(x, z), "no wheel at the footprint corner ({x}, {z})");
        }
    }

    /// The driver stands on the car's deck, not through it and not in the air above it.
    /// [`vehicle::SEAT`] is a number in another file; this is what keeps it honest.
    #[test]
    fn the_driver_stands_on_the_deck() {
        let seat = vehicle::SEAT;
        let deck = CAR
            .iter()
            .filter(|p| {
                let (half_x, half_z) = (p.size[0] / 2.0, p.size[2] / 2.0);
                (p.at[0] - seat.x).abs() <= half_x && (p.at[2] - seat.z).abs() <= half_z
            })
            .map(|p| p.at[1] + p.size[1] / 2.0)
            .fold(f32::MIN, f32::max);
        assert!(
            (seat.y - deck).abs() < 0.02,
            "the driver's feet are at {}, the deck under them at {deck}",
            seat.y
        );
        // ... and the whole standing figure fits inside the roll bar's footprint, so he
        // reads as being *in* the car rather than balanced on it.
        let widest = CAR
            .iter()
            .map(|p| p.at[0].abs() + p.size[0] / 2.0)
            .fold(0.0, f32::max);
        assert!(widest > REACH_OUT, "the car is narrower than its driver");
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

    /// The whole point of the icons: a child picks a thing out of the graph by its outline,
    /// so no two of them may *have* the same outline. Blocks are exempt from each other —
    /// they are all cubes in the world and telling them apart is the colour's job.
    #[test]
    fn every_made_thing_has_its_own_silhouette() {
        let shape = |item: Item| {
            let mut boxes: Vec<[i32; 6]> = icon(item)
                .iter()
                .map(|p| {
                    let (at, size) = (p.at, p.size);
                    // Rounded, because two tables that differ in the fourth decimal are the
                    // same picture to the eye this test is standing in for.
                    [
                        (at[0] * 50.0) as i32,
                        (at[1] * 50.0) as i32,
                        (at[2] * 50.0) as i32,
                        (size[0] * 50.0) as i32,
                        (size[1] * 50.0) as i32,
                        (size[2] * 50.0) as i32,
                    ]
                })
                .collect();
            boxes.sort();
            boxes
        };
        let made: Vec<Item> = Item::ALL
            .iter()
            .copied()
            .filter(|i| !matches!(i.class(), crate::registry::Class::Block(_)))
            .collect();
        for (a, b) in made
            .iter()
            .enumerate()
            .flat_map(|(i, a)| made[i + 1..].iter().map(move |b| (a, b)))
        {
            assert_ne!(shape(*a), shape(*b), "{a:?} and {b:?} are the same picture");
        }
    }

    /// Every icon fits the same cube, so a nail and a car are the same size on the row.
    #[test]
    fn icons_are_all_cut_to_one_size() {
        for item in Item::ALL {
            let (_, size) = bounds(icon(*item));
            assert!(
                size.min_element() > 0.0,
                "{item:?}'s icon is flat in some direction and vanishes edge-on"
            );
            let fit = 1.0 / size.max_element();
            assert!(
                ((size * fit).max_element() - 1.0).abs() < 1e-4,
                "{item:?} does not fill its cell"
            );
        }
    }
}
