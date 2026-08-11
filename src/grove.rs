//! **The grove** — the recipe graph as a place you fly around, with no words in it.
//!
//! Everything a child needs to know is a shape, a colour, a size or a movement:
//!
//! * **Height is distance from the ground.** The bottom row is the six things you dig up,
//!   sitting on a strip of earth. Every row above is one more craft away from dirt, so the
//!   car standing near the top *is* the statement that a car is a long way from a shovel.
//! * **A line is an ingredient**, painted the colour of what travels along it, so a car is
//!   visibly fed by brown, by grey, and by steel.
//! * **The beads on a line are the count.** Eight nails is eight beads. They light up from
//!   the bottom as you gather, so three lit and five dark is "three of the eight".
//! * **Movement means ready.** A thing you can make right now bobs, and every line into it
//!   marches. Nothing else in the picture moves at all, so the eye goes straight to what is
//!   possible — which for a child with an empty pocket is the six blocks on the ground.
//! * **The cursor lights its family.** What the selected thing is made of, all the way down
//!   to the ground, and everything it goes into, stay full size; the rest of the graph
//!   shrinks back. That is the answer to "what do I have to go and get".
//! * **The arrows round the cursor are the d-pad.** One appears for each direction that
//!   leads somewhere, so which way to press is never a guess.
//! * **The row of pips under a thing is how many you have**, up to the largest number any
//!   recipe asks for. Counting your six wood against the car's six beads is the whole game.
//!
//! The graph itself is [`crate::craftgraph`], which knows nothing about any of this.
//!
//! **What drives it is somebody else's business.** This screen reads a pile ([`OnHand`])
//! and a set of presses ([`Nav`]), and the only thing it does about crafting is *ask*: it
//! moves the hotbar selection and raises [`Intent::craft`], exactly as pointing the hotbar
//! at a thing and pressing craft used to, and the host answers. In the game the pile is the
//! host's word and the presses come off the pad; under `blockgame grove` both come off a
//! script. One screen, two drivers, and no second copy of the picture.

use bevy::camera::ClearColorConfig;
use bevy::prelude::*;

use crate::avatar::{self, Palette};
use crate::craftgraph::{Dir, Edge, Graph, Node};
use crate::input::{KEYS, PAD};
use crate::inventory::{Held, Inventory};
use crate::registry::Item;

/// Sideways room per layout unit, in blocks.
const COLUMN: f32 = 3.4;
/// How much higher each row stands than the one it is made from.
const RISE: f32 = 5.0;
/// How much further away, too. The graph leans back as it climbs, so travelling up it is
/// travelling *into* the picture rather than sliding along one flat wall.
const RECEDE: f32 = 1.6;
/// How big an icon is drawn. Icons are cut to a one-block cube, so this is that cube.
const NODE: f32 = 1.7;
/// How far short of a thing the lines into and out of it stop.
const HUB: f32 = 1.3;
/// How thick a line is.
const LINE: f32 = 0.10;

/// A bead you have.
const BEAD: f32 = 0.36;
/// A bead you have not. Smaller as well as darker: on a handheld in sunlight, colour on its
/// own is not a difference.
const BEAD_UNMET: f32 = 0.20;
/// Laps a bead makes along its line each second, once the recipe is affordable.
const FLOW: f32 = 0.30;

/// The most of one thing the tally under a node counts out. The question it exists to
/// answer is "have I got enough for the recipe", and no recipe asks for more than this.
const TALLY_MAX: u32 = 8;
const TALLY_PITCH: f32 = 0.36;

/// How much is left of the part of the graph the cursor has nothing to do with: small
/// enough to read as background, big enough that the shape of the whole is still there.
const FADED: f32 = 0.42;

/// Roughly how long the camera and the node sizes take to catch up, in seconds. Everything
/// eases rather than cuts — a cut costs a child the thread of where they were.
const EASE: f32 = 0.16;

/// Behind it all. Dark, so the lit beads and the ring are the brightest things on screen.
const BACKDROP: Color = Color::srgb(0.05, 0.07, 0.11);

/// The pile the grove is drawing — the local player's, as whoever is driving states it.
///
/// A copy rather than a borrow of [`crate::inventory::Inventories`] on purpose: the game
/// mirrors the host's word into it each frame and the film writes it directly, and the
/// screen never asks which of the two it is looking at.
#[derive(Resource, Default, Clone)]
pub struct OnHand(pub Inventory);

/// What was pressed this frame.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Nav {
    /// Which way the d-pad went, one step per press: `x` sideways, `y` up the graph.
    pub step: IVec2,
    /// Make the thing under the cursor.
    pub craft: bool,
    /// Put the grove away.
    pub close: bool,
}

/// Where the cursor is, and how the last press went.
#[derive(Resource)]
pub struct Cursor {
    pub at: usize,
    /// Above zero the node is celebrating a craft, below zero it is refusing one, and it
    /// decays to nothing either way. One number, because a node is never doing both.
    answer: f32,
}

/// Everything the grove drew, so putting it away is one despawn.
#[derive(Resource)]
pub struct Grove {
    graph: Graph,
    root: Entity,
    /// Which nodes are lit. Recomputed only when the cursor moves.
    family: Vec<bool>,
}

#[derive(Component)]
pub struct GroveNode(usize);

/// The model that turns, inside the node that moves.
#[derive(Component)]
pub struct GroveIcon(usize);

/// The `ordinal`th bead along a line.
#[derive(Component)]
pub struct Bead {
    edge: usize,
    ordinal: u32,
}

#[derive(Component)]
pub struct Line(usize);

/// One pip of the count under a node: "you have at least this many".
#[derive(Component)]
pub struct Tally {
    node: usize,
    ordinal: u32,
}

/// The ring round the cursor, and the four arrows outside it.
#[derive(Component)]
pub struct Ring;

#[derive(Component)]
pub struct Arrow(Dir);

/// The camera that flies the grove — tagged, because the world's camera is still pointed
/// at a hillside behind all of this and must not be moved by it.
#[derive(Component)]
pub struct GroveCam;

/// Where a node stands.
fn stand(node: &Node) -> Vec3 {
    let row = node.depth as f32;
    Vec3::new(node.x * COLUMN, row * RISE, -row * RECEDE)
}

/// The two ends of a line, pulled back clear of the things it joins.
fn span(graph: &Graph, edge: &Edge) -> (Vec3, Vec3) {
    let (a, b) = (
        stand(&graph.nodes()[edge.from]),
        stand(&graph.nodes()[edge.to]),
    );
    let along = (b - a).normalize_or_zero();
    (a + along * HUB, b - along * HUB)
}

/// Where along its line a bead sits, `0` at the ingredient and `1` at the thing it makes.
fn bead_at(ordinal: u32, count: u32, flowing: f32) -> f32 {
    ((ordinal as f32 + 0.5) / count as f32 + flowing).fract()
}

/// The shapes and the two colours the grove paints with that are not an item's own.
#[derive(Resource)]
pub struct Ink {
    /// A bead you have not got. Dark whatever colour it would otherwise be.
    unmet: Handle<StandardMaterial>,
    /// The ring and the arrows: the only thing on screen that is its own light source.
    lit: Handle<StandardMaterial>,
    bead: Handle<Mesh>,
    line: Handle<Mesh>,
    plate: Handle<Mesh>,
    ring: Handle<Mesh>,
    arrow: Handle<Mesh>,
}

/// Builds the grove and the camera that flies it. [`close`] takes both down again.
pub fn open(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    palette: Res<Palette>,
) {
    let ink = Ink {
        unmet: materials.add(StandardMaterial {
            base_color: Color::srgb(0.13, 0.14, 0.17),
            perceptual_roughness: 0.95,
            ..default()
        }),
        lit: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.96, 0.76),
            emissive: LinearRgba::rgb(3.0, 2.6, 1.2),
            ..default()
        }),
        bead: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        // A unit cylinder up `+Y`, stretched and turned into whichever line it is.
        line: meshes.add(Cylinder::new(1.0, 1.0)),
        plate: meshes.add(Cylinder::new(1.15, 0.16)),
        ring: meshes.add(Torus::new(1.34, 1.54)),
        arrow: meshes.add(Cone {
            radius: 0.34,
            height: 0.66,
        }),
    };

    let graph = Graph::of_registry();
    let root = commands
        .spawn((Transform::default(), Visibility::Visible))
        .id();

    // The strip of earth the bottom row is dug out of — what makes "these six came from
    // down here" a picture rather than a caption.
    let ground = graph.rows()[0]
        .iter()
        .map(|i| graph.nodes()[*i].x.abs())
        .fold(0.0, f32::max)
        * 2.0
        * COLUMN
        + 7.0;
    commands.entity(root).with_children(|grove| {
        grove.spawn((
            Mesh3d(meshes.add(Cuboid::new(ground, 2.6, 4.5))),
            MeshMaterial3d(palette.item(Item::Dirt)),
            Transform::from_xyz(0.0, -2.6, 0.0),
        ));
    });

    for (i, node) in graph.nodes().iter().enumerate() {
        let icon = avatar::spawn_icon(&mut commands, &palette, node.item);
        commands
            .entity(icon)
            .insert((GroveIcon(i), Transform::from_scale(Vec3::splat(NODE))));
        let at = commands
            .spawn((
                GroveNode(i),
                Transform::from_translation(stand(node)),
                Visibility::Visible,
            ))
            .add_child(icon)
            .with_children(|node_root| {
                node_root.spawn((
                    Mesh3d(ink.plate.clone()),
                    MeshMaterial3d(palette.item(node.item)),
                    Transform::from_xyz(0.0, -1.08, 0.0),
                ));
                for ordinal in 0..TALLY_MAX {
                    node_root.spawn((
                        Tally { node: i, ordinal },
                        Mesh3d(ink.bead.clone()),
                        MeshMaterial3d(palette.item(node.item)),
                        Transform::from_xyz(
                            (ordinal as f32 - (TALLY_MAX - 1) as f32 / 2.0) * TALLY_PITCH,
                            -1.46,
                            0.7,
                        )
                        .with_scale(Vec3::splat(0.26)),
                        Visibility::Hidden,
                    ));
                }
            })
            .id();
        commands.entity(root).add_child(at);
    }

    for (e, edge) in graph.edges().iter().enumerate() {
        let (a, b) = span(&graph, edge);
        let paint = palette.item(graph.nodes()[edge.from].item);
        commands.entity(root).with_children(|grove| {
            grove.spawn((
                Line(e),
                Mesh3d(ink.line.clone()),
                MeshMaterial3d(paint.clone()),
                Transform::from_translation((a + b) / 2.0)
                    .with_rotation(Quat::from_rotation_arc(
                        Vec3::Y,
                        (b - a).normalize_or_zero(),
                    ))
                    .with_scale(Vec3::new(LINE, (b - a).length(), LINE)),
            ));
            for ordinal in 0..edge.count {
                grove.spawn((
                    Bead { edge: e, ordinal },
                    Mesh3d(ink.bead.clone()),
                    MeshMaterial3d(paint.clone()),
                    Transform::from_translation(a.lerp(b, bead_at(ordinal, edge.count, 0.0)))
                        .with_scale(Vec3::splat(BEAD)),
                ));
            }
        });
    }

    commands.entity(root).with_children(|grove| {
        grove.spawn((
            Ring,
            Mesh3d(ink.ring.clone()),
            MeshMaterial3d(ink.lit.clone()),
            Transform::default(),
        ));
        for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
            grove.spawn((
                Arrow(dir),
                Mesh3d(ink.arrow.clone()),
                MeshMaterial3d(ink.lit.clone()),
                Transform::default(),
                Visibility::Hidden,
            ));
        }
        grove.spawn((
            DirectionalLight {
                illuminance: 6_500.0,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_xyz(-6.0, 14.0, 18.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    });

    commands.spawn((
        GroveCam,
        Camera3d::default(),
        Camera {
            // Its own backdrop rather than the world's sky, so the grove is dark and the
            // ring is the brightest thing in it — and so closing it restores nothing.
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 45f32.to_radians(),
            far: 400.0,
            ..default()
        }),
        AmbientLight {
            color: Color::srgb(0.8, 0.86, 1.0),
            brightness: 340.0,
            ..default()
        },
        Transform::from_xyz(0.0, 6.0, 22.0).looking_at(Vec3::new(0.0, 4.0, 0.0), Vec3::Y),
    ));

    commands.insert_resource(Grove {
        family: graph.family(0),
        graph,
        root,
    });
    commands.insert_resource(Cursor { at: 0, answer: 0.0 });
    commands.insert_resource(ink);
    commands.init_resource::<Nav>();
}

pub fn close(mut commands: Commands, grove: Res<Grove>, cam: Query<Entity, With<GroveCam>>) {
    commands.entity(grove.root).despawn();
    for cam in cam.iter() {
        commands.entity(cam).despawn();
    }
    commands.remove_resource::<Grove>();
    commands.remove_resource::<Cursor>();
    commands.remove_resource::<Ink>();
}

/// The d-pad, off the keyboard and off the pad, into [`Nav`]. The game's driver; the film
/// writes [`Nav`] itself and never runs this.
pub fn read_pad(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut pushed: Local<IVec2>,
    mut nav: ResMut<Nav>,
) {
    let key = |a: KeyCode, b: KeyCode| keys.just_pressed(b) as i32 - keys.just_pressed(a) as i32;
    let mut out = Nav {
        step: IVec2::new(
            key(KeyCode::ArrowLeft, KeyCode::ArrowRight) + key(KEYS.left, KEYS.right),
            key(KeyCode::ArrowDown, KeyCode::ArrowUp) + key(KEYS.back, KEYS.forward),
        ),
        // The same key that opened it makes the thing under the cursor, which is what it
        // did when the hotbar was the crafting menu.
        craft: keys.just_pressed(KEYS.craft) || keys.just_pressed(KeyCode::Enter),
        close: keys.just_pressed(KEYS.pause),
    };

    let mut stick = IVec2::ZERO;
    for pad in &pads {
        out.step.x +=
            pad.just_pressed(PAD.next_item) as i32 - pad.just_pressed(PAD.prev_item) as i32;
        out.step.y += pad.just_pressed(PAD.prev_row) as i32 - pad.just_pressed(PAD.next_row) as i32;
        out.craft |= pad.just_pressed(PAD.craft) || pad.just_pressed(PAD.jump);
        out.close |= pad.just_pressed(PAD.ride) || pad.just_pressed(PAD.pause);
        // The stick reads as a second d-pad, and has to come back to centre before it
        // gives another step — the same rule the menus use.
        let raw = pad.left_stick();
        let axis = |v: f32| {
            if v > 0.5 {
                1
            } else if v < -0.5 {
                -1
            } else {
                0
            }
        };
        stick += IVec2::new(axis(raw.x), axis(raw.y));
    }
    let stick = stick.clamp(IVec2::splat(-1), IVec2::splat(1));
    out.step += (stick - *pushed).clamp(IVec2::splat(-1), IVec2::splat(1)) * stick.abs();
    *pushed = stick;

    *nav = out;
}

/// Moves the cursor, and asks for a craft. **Asks** — the pile is not touched here; this
/// points the hotbar at the thing and raises the same flag the hotbar raised, and whoever
/// owns the pile answers.
pub fn navigate(
    nav: Res<Nav>,
    time: Res<Time>,
    on_hand: Res<OnHand>,
    mut cursor: ResMut<Cursor>,
    mut grove: ResMut<Grove>,
    mut held: ResMut<Held>,
    mut intent: ResMut<crate::input::Intent>,
) {
    intent.craft = false;
    cursor.answer -= cursor.answer.signum() * (time.delta_secs() / 0.35);
    if cursor.answer.abs() < 0.02 {
        cursor.answer = 0.0;
    }

    let asked = [
        (nav.step.x > 0).then_some(Dir::Right),
        (nav.step.x < 0).then_some(Dir::Left),
        (nav.step.y > 0).then_some(Dir::Up),
        (nav.step.y < 0).then_some(Dir::Down),
    ];
    for dir in asked.into_iter().flatten() {
        match grove.graph.step(cursor.at, dir) {
            Some(next) => {
                cursor.at = next;
                grove.family = grove.graph.family(next);
            }
            // A wall gets the same refusal a recipe you cannot afford gets, so "that did
            // not work" is one thing to learn rather than two.
            None => cursor.answer = -1.0,
        }
    }

    let item = grove.graph.nodes()[cursor.at].item;
    held.0 = item;
    if nav.craft {
        if on_hand.0.can_craft(item) {
            intent.craft = true;
            cursor.answer = 1.0;
        } else {
            cursor.answer = -1.0;
        }
    }
}

/// Sizes and moves every node: lit or faded, bobbing if it can be made now, and shoved
/// about if it was just asked for.
pub fn nodes(
    time: Res<Time>,
    grove: Res<Grove>,
    cursor: Res<Cursor>,
    on_hand: Res<OnHand>,
    mut nodes: Query<(&GroveNode, &mut Transform)>,
    mut icons: Query<(&GroveIcon, &mut Transform), Without<GroveNode>>,
) {
    let t = time.elapsed_secs();
    let ease = 1.0 - (-time.delta_secs() / EASE).exp();
    for (node, mut at) in &mut nodes {
        let n = &grove.graph.nodes()[node.0];
        let ready = on_hand.0.can_craft(n.item);
        let mut want = if grove.family[node.0] { 1.0 } else { FADED };
        let mut home = stand(n);
        if ready {
            home.y += (t * 3.1 + node.0 as f32).sin() * 0.14;
        }
        if node.0 == cursor.at {
            want *= 1.0 + cursor.answer.max(0.0) * 0.4;
            home.x += (t * 34.0).sin() * cursor.answer.min(0.0).abs() * 0.22;
        }
        at.translation = at.translation.lerp(home, ease);
        at.scale = at.scale.lerp(Vec3::splat(want), ease);
    }
    for (icon, mut at) in &mut icons {
        // Only a thing you can make right now turns. That is the whole reason a still
        // picture of this screen still says which things are possible.
        let turning = on_hand.0.can_craft(grove.graph.nodes()[icon.0].item);
        let want = if turning {
            Quat::from_rotation_y((t * 1.1 + icon.0 as f32).sin() * 0.45)
        } else {
            Quat::IDENTITY
        };
        at.rotation = at.rotation.slerp(want, ease);
    }
}

/// Lights, dims, spaces and marches the beads. The count, the progress and the "ready" are
/// all this one system, because they are all the same three numbers.
pub fn beads(
    time: Res<Time>,
    grove: Res<Grove>,
    on_hand: Res<OnHand>,
    ink: Res<Ink>,
    palette: Res<Palette>,
    mut beads: Query<(&Bead, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (bead, mut at, mut paint) in &mut beads {
        let edge = grove.graph.edges()[bead.edge];
        let (from, to) = (
            grove.graph.nodes()[edge.from].item,
            grove.graph.nodes()[edge.to].item,
        );
        let have = on_hand.0.count(from) > bead.ordinal;
        let flowing = on_hand.0.can_craft(to);
        let (a, b) = span(&grove.graph, &edge);
        let along = bead_at(
            bead.ordinal,
            edge.count,
            if flowing {
                time.elapsed_secs() * FLOW
            } else {
                0.0
            },
        );
        at.translation = a.lerp(b, along);
        let size = if have { BEAD } else { BEAD_UNMET };
        at.scale = Vec3::splat(if grove.family[edge.to] {
            size
        } else {
            size * FADED
        });
        let want = if have {
            palette.item(from)
        } else {
            ink.unmet.clone()
        };
        if paint.0 != want {
            paint.0 = want;
        }
    }
}

/// Thins the lines that have nothing to do with the cursor.
pub fn lines(time: Res<Time>, grove: Res<Grove>, mut lines: Query<(&Line, &mut Transform)>) {
    let ease = 1.0 - (-time.delta_secs() / EASE).exp();
    for (line, mut at) in &mut lines {
        let edge = grove.graph.edges()[line.0];
        let want = if grove.family[edge.to] && grove.family[edge.from] {
            1.0
        } else {
            0.35
        };
        at.scale.x += (LINE * want - at.scale.x) * ease;
        at.scale.z = at.scale.x;
    }
}

/// Counts out what you have under each thing.
pub fn tallies(
    on_hand: Res<OnHand>,
    grove: Res<Grove>,
    mut pips: Query<(&Tally, &mut Visibility)>,
) {
    for (pip, mut shown) in &mut pips {
        let item = grove.graph.nodes()[pip.node].item;
        *shown = if on_hand.0.count(item) > pip.ordinal {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Puts the ring and its arrows on whatever the cursor is on, and shows an arrow only for a
/// direction that goes somewhere.
pub fn marks(
    time: Res<Time>,
    grove: Res<Grove>,
    cursor: Res<Cursor>,
    mut ring: Query<&mut Transform, With<Ring>>,
    mut arrows: Query<(&Arrow, &mut Transform, &mut Visibility), Without<Ring>>,
) {
    let t = time.elapsed_secs();
    let ease = 1.0 - (-time.delta_secs() / EASE).exp();
    let here = stand(&grove.graph.nodes()[cursor.at]);
    for mut at in &mut ring {
        at.translation = at.translation.lerp(here, ease);
        at.rotation =
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_y(t * 1.3);
        at.scale = Vec3::splat(1.0 + (t * 4.0).sin() * 0.04);
    }
    for (arrow, mut at, mut shown) in &mut arrows {
        let (offset, turn) = match arrow.0 {
            Dir::Up => (Vec3::Y, 0.0),
            Dir::Down => (Vec3::NEG_Y, std::f32::consts::PI),
            Dir::Right => (Vec3::X, -std::f32::consts::FRAC_PI_2),
            Dir::Left => (Vec3::NEG_X, std::f32::consts::FRAC_PI_2),
        };
        let out = 2.15 + (t * 4.0 + offset.x + offset.y).sin() * 0.12;
        at.translation = at.translation.lerp(here + offset * out, ease);
        at.rotation = Quat::from_rotation_z(turn);
        *shown = if grove.graph.step(cursor.at, arrow.0).is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Flies the camera. It frames the cursor together with the part of the graph the cursor
/// lit, so what a thing is made of is on screen at the same time as the thing.
pub fn fly(
    time: Res<Time>,
    grove: Res<Grove>,
    cursor: Res<Cursor>,
    mut cam: Query<&mut Transform, With<GroveCam>>,
) {
    let here = stand(&grove.graph.nodes()[cursor.at]);
    let lit: Vec<Vec3> = grove
        .graph
        .nodes()
        .iter()
        .enumerate()
        .filter(|(i, _)| grove.family[*i])
        .map(|(_, n)| stand(n))
        .collect();
    let middle = lit.iter().copied().sum::<Vec3>() / lit.len().max(1) as f32;
    let spread = lit
        .iter()
        .map(|p| p.distance(middle))
        .fold(0.0, f32::max)
        .min(24.0);

    let look = here.lerp(middle, 0.34);
    let sway = Quat::from_rotation_y((time.elapsed_secs() * 0.21).sin() * 0.15);
    let want = Transform::from_translation(look + sway * Vec3::new(0.0, 2.6, 12.0 + spread * 0.55))
        .looking_at(look, Vec3::Y);

    let ease = 1.0 - (-time.delta_secs() / (EASE * 3.0)).exp();
    for mut at in &mut cam {
        at.translation = at.translation.lerp(want.translation, ease);
        at.rotation = at.rotation.slerp(want.rotation, ease);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows never overlap in space, whatever the layout does sideways: a thing always
    /// stands clear of what it is made of, which is the one thing height is saying.
    #[test]
    fn a_thing_stands_clear_of_what_it_is_made_of() {
        let graph = Graph::of_registry();
        for edge in graph.edges() {
            let (a, b) = (
                stand(&graph.nodes()[edge.from]),
                stand(&graph.nodes()[edge.to]),
            );
            assert!(
                b.y - a.y >= RISE - 1e-3,
                "{edge:?} does not climb a whole row"
            );
            assert!(
                (b - a).length() > 2.0 * HUB,
                "a line between them has no length"
            );
        }
    }

    #[test]
    fn beads_are_spread_along_the_whole_line_and_come_back_round() {
        for count in 1..=TALLY_MAX {
            let mut seats: Vec<f32> = (0..count).map(|k| bead_at(k, count, 0.0)).collect();
            seats.sort_by(f32::total_cmp);
            assert!(seats.iter().all(|t| (0.0..1.0).contains(t)));
            for pair in seats.windows(2) {
                assert!(
                    (pair[1] - pair[0] - 1.0 / count as f32).abs() < 1e-5,
                    "{count} beads are not evenly spaced"
                );
            }
            // A whole lap puts every bead back where it started, so the march has no seam.
            for k in 0..count {
                assert!((bead_at(k, count, 1.0) - bead_at(k, count, 0.0)).abs() < 1e-5);
            }
        }
    }
}
