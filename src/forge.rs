//! The Forge — the recipe graph as a thing you look at, with no words on it.
//!
//! Press craft and a rig unfolds in the air in front of you: the thing you are making
//! hangs in the middle, what it is made of hangs below it, and what *it* goes into hangs
//! above. Parts are wired to their products by glowing strings, and threaded on each
//! string is one bead per unit that recipe asks for — lit if you have that one, dark if
//! you still owe it. A four-year-old reads "three lit, five dark" without being able to
//! read anything.
//!
//! **Why a neighbourhood and not the whole table.** Every node is placed by its *longest*
//! path down from the focus, so every string runs strictly upward and a part used by two
//! different products is one node with two strings leaving it — which is already true in
//! the registry today, where stone feeds both the car and the nail the car needs. That is
//! the whole reason this is a graph layout and not a tree walk: a second parent adds a
//! string, never a copy.
//!
//! **No words, and no second inventory.** What the player is told is carried by shape
//! (each craftable has its own silhouette), by colour (the registry's own), by the beads,
//! and by the bar beside each node — one notch per one you own, so crafting makes a
//! visible thing grow. The only text in the mode is none.
//!
//! **What this module does not know.** It draws [`crate::rig::Stock`] and it asks for crafts by
//! pushing to [`CraftRequests`]. Whose pile that is, and who is allowed to say a craft
//! happened, stays with the host in [`crate::game`] — so the same rig runs in the game,
//! where the host answers, and under `blockgame craft-film`, where a test harness does.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::avatar::Palette;
use crate::belt::{self, Belt};
use crate::inventory::{Held, Inventory};
use crate::registry::Item;
use crate::rig::{self, Dir, Kit, Stock};

/// Where the rig is built, far above any world: the game's own camera stops at 1200
/// blocks, so the two scenes cannot see each other and neither needs a render layer to
/// say so.
const ORIGIN: Vec3 = Vec3::new(0.0, 40_000.0, 0.0);

/// Vertical distance between one tier of the graph and the next.
const TIER_GAP: f32 = 3.0;
/// Horizontal distance between neighbours on a tier, and the widest a tier may get before
/// it is squeezed to fit.
const COL_GAP: f32 = 2.6;
const ROW_WIDTH: f32 = 11.5;

/// How big the two arrows under a node are drawn — the code that puts that node in the
/// middle. Small enough to read as a caption on the thing, big enough to press from.
const GLYPH: f32 = 0.46;

/// Seconds a craft's flying parts take to reach the thing they were spent on, and how
/// long the product stays swollen afterwards.
const FLIGHT: f32 = 0.42;
const POP: f32 = 0.30;

/// What the player is asking of the rig this frame. Filled from the pad in the game and
/// from a script under `craft-film`, so the rig itself never asks which.
///
/// A direction, not a cursor step: this rig is walked by the same two-press code that
/// takes a thing off the belt, and a d-pad that meant "one cell right" in here and "half a
/// code" out there would be two alphabets for one thumb.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Nav {
    /// A d-pad direction, pressed this frame.
    pub dir: Option<Dir>,
    /// Make one of whatever is in the middle.
    pub craft: bool,
    /// Back to the world.
    pub leave: bool,
}

/// Crafts the player has asked for and nobody has answered yet.
///
/// Pushed here rather than performed, because who may say a craft happened is the host's
/// question and this module is a drawing. The game drains it into the same request the
/// hotbar used to make.
#[derive(Resource, Default)]
pub struct CraftRequests(pub Vec<Item>);

// ---------------------------------------------------------------------------
// The layout — pure, and the part worth testing.
// ---------------------------------------------------------------------------

/// One item, placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub item: Item,
    /// Which row it landed on. Lower numbers are higher up the screen and are the things
    /// made *from* what is below them; the focus is normally `0`.
    pub tier: i32,
    pub at: Vec3,
}

/// One recipe line, drawn as a string from the part up to the product.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wire {
    /// Index of the ingredient node — always on a *lower* tier than `to`.
    pub from: usize,
    pub to: usize,
    /// How many of `from` one `to` costs.
    pub need: u32,
}

/// The focus, everything it is transitively made of, and everything it goes into.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    /// What is in the middle. There is no second cursor beside it: a code names a thing,
    /// and the thing a code names is what the rig is built around.
    pub focus: Item,
}

impl Graph {
    /// Lays the neighbourhood of `focus` out.
    ///
    /// Depth is the *longest* path from the focus rather than the shortest, which is
    /// the one decision that makes this a graph layout: stone is reachable from the car
    /// in one hop and in two, and taking the longer one puts it below the nail that also
    /// needs it, so both strings run upward and neither item is drawn twice.
    pub fn around(focus: Item) -> Graph {
        let mut tier: HashMap<Item, i32> = HashMap::from([(focus, 0)]);
        // Upwards first: everything that needs the focus, however far around, each as far
        // above as its *longest* chain puts it. A drill needs a nail and a nail needs
        // stone, so standing on stone the drill has to sit a tier above the nail — laying
        // both consumers of stone on one row would draw the nail-into-drill recipe
        // sideways, which is a line with no direction in it.
        loop {
            let mut changed = false;
            for product in Item::ALL {
                for (part, _) in product.recipe() {
                    let Some(above) = tier.get(part).map(|t| t - 1) else {
                        continue;
                    };
                    if tier.get(product).is_none_or(|t| *t > above) {
                        tier.insert(*product, above);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // Then downwards from everything now on the board, by longest path again. This is
        // what makes the picture honest about *cost*: a drill drawn above a nail is drawn
        // with the wood and the stone it also takes, not with the one ingredient that
        // happens to lead back to what the player is standing on.
        loop {
            let mut changed = false;
            for (item, at) in tier.clone() {
                for (part, _) in item.recipe() {
                    let below = at + 1;
                    if tier.get(part).is_none_or(|d| *d < below) {
                        tier.insert(*part, below);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let highest = *tier.values().min().expect("the focus is in the table");
        let lowest = *tier.values().max().expect("the focus is in the table");
        let mut nodes: Vec<Node> = Vec::new();
        for t in highest..=lowest {
            // Registry order, so the same graph lays out the same way every time it is
            // built — a cursor that jumped when the rig was rebuilt would be unusable.
            let mut here: Vec<Item> = Item::ALL
                .iter()
                .copied()
                .filter(|i| tier.get(i) == Some(&t))
                .collect();
            here.sort_by_key(|i| i.index());
            if here.is_empty() {
                continue;
            }
            let gap = COL_GAP.min(ROW_WIDTH / here.len() as f32);
            for (i, item) in here.iter().enumerate() {
                let x = (i as f32 - (here.len() as f32 - 1.0) / 2.0) * gap;
                nodes.push(Node {
                    item: *item,
                    tier: t,
                    at: Vec3::new(x, -(t as f32) * TIER_GAP, 0.0),
                });
            }
        }

        let index: HashMap<Item, usize> =
            nodes.iter().enumerate().map(|(i, n)| (n.item, i)).collect();
        let mut wires = Vec::new();
        for (to, node) in nodes.iter().enumerate() {
            for (ingredient, need) in node.item.recipe() {
                if let Some(from) = index.get(ingredient) {
                    wires.push(Wire {
                        from: *from,
                        to,
                        need: *need,
                    });
                }
            }
        }

        assert!(
            nodes.iter().any(|n| n.item == focus),
            "the focus is laid out"
        );
        Graph {
            nodes,
            wires,
            focus,
        }
    }

    /// Where a node hangs, for anything that has to point at one.
    fn pos(&self, item: Item) -> Vec3 {
        self.nodes
            .iter()
            .find(|n| n.item == item)
            .map(|n| n.at)
            .expect("a node is only ever asked about while it is laid out")
    }

    /// The middle of everything laid out — where the camera points.
    fn centre(&self) -> Vec3 {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for n in &self.nodes {
            lo = lo.min(n.at);
            hi = hi.max(n.at);
        }
        (lo + hi) / 2.0
    }

    /// How far back the camera has to stand to hold the whole rig in frame.
    fn framing(&self) -> f32 {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for n in &self.nodes {
            lo = lo.min(n.at);
            hi = hi.max(n.at);
        }
        // Padding for what a node is drawn with but is not placed at: the notch bar out to
        // its right, and the swell a fresh craft gives it.
        let span = (hi - lo) + Vec3::splat(4.0);
        // Trigonometry rather than a number that looked right once: at a 50-degree vertical
        // field the frame is `2 * d * tan(25°)` tall, so holding a span of `h` needs
        // `d >= h * 1.08`, and the sixteen-by-ten panels this ships on are 1.6 times as
        // wide as they are tall. A tenth over each is the margin — the alternative is a
        // car with its roof cut off, which is what the constants below replaced.
        (span.x * 0.74).max(span.y * 1.18).max(6.0)
    }
}

/// The next single craft that gets the player closer to `want`, or nothing if the chain
/// is blocked on something that has to be dug up.
///
/// One press makes one thing, deepest first, so holding the button builds a car from the
/// bottom up and the player watches each step happen. That is the whole of "multi-step
/// crafting with no wait": the wait is replaced by the animation of the step.
pub fn next_step(stock: &Inventory, want: Item) -> Option<Item> {
    if stock.can_craft(want) {
        return Some(want);
    }
    want.recipe()
        .iter()
        .filter(|(part, need)| stock.count(*part) < *need)
        .find_map(|(part, _)| next_step(stock, *part))
}

// ---------------------------------------------------------------------------
// The rig — components, and the systems that build and animate it.
// ---------------------------------------------------------------------------

/// Everything the rig owns, so leaving is one despawn.
#[derive(Component, Clone, Copy)]
pub struct Rig;

/// The part of the rig that is a picture of *this* graph — everything but the camera, the
/// lamp, and the parts currently in flight. Re-centring throws exactly these away and
/// builds them again, which is why it is a marker and not a list of `Without`s that has to
/// be extended every time the rig grows a new kind of thing.
#[derive(Component, Clone, Copy)]
pub struct Built;

/// The rig's state between frames: what is in the middle, and nothing else. There is no
/// cursor to keep — the code names a thing and the thing it names is the middle, so
/// "where am I" and "what am I looking at" cannot disagree.
#[derive(Resource)]
pub struct Forge {
    pub graph: Graph,
    /// The pile as of the last frame, so a count that went up can be spotted and cheered.
    seen: Inventory,
    /// Rebuilt geometry is wanted next frame — set when the focus moves.
    dirty: bool,
}

impl Forge {
    pub fn new(focus: Item, seen: Inventory) -> Forge {
        Forge {
            graph: Graph::around(focus),
            seen,
            dirty: true,
        }
    }

    pub fn middle(&self) -> Item {
        self.graph.focus
    }

    fn refocus(&mut self, item: Item) {
        self.graph = Graph::around(item);
        self.dirty = true;
    }
}

#[derive(Component)]
pub struct NodeView {
    item: Item,
    at: Vec3,
    /// Seconds left of the swell a fresh craft gives this node.
    pop: f32,
}

/// One bead on a string: the `n`th of the `need` this recipe asks for.
#[derive(Component)]
pub struct Bead {
    part: Item,
    nth: u32,
    /// Distance along the string, so the wave that runs up a satisfied string is a
    /// function of this and the clock and needs no per-bead state.
    along: f32,
    lit: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

/// The ring around whatever is in the middle.
#[derive(Component)]
pub struct Cursor;

/// A part flying up a string into the thing it was just spent on.
#[derive(Component)]
pub struct InFlight {
    from: Vec3,
    to: Vec3,
    left: f32,
}

/// The rig's camera, eased toward whatever the graph currently needs framed.
#[derive(Component)]
pub struct Eye;

pub fn enter(mut commands: Commands, forge: Res<Forge>) {
    // The rig's own camera, drawn over the world without wiping it: the graph hangs in
    // the air in front of wherever the player was standing, which is the point of
    // building it out of blocks rather than out of panels. Above the belt's camera, which
    // is switched off while this one is up — one room at a time.
    commands.spawn((
        Rig,
        Eye,
        Camera3d::default(),
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 50f32.to_radians(),
            far: 400.0,
            ..default()
        }),
        AmbientLight {
            color: Color::WHITE,
            brightness: 900.0,
            ..default()
        },
        // Framed on the graph from the first frame: the rig is a thing that was already
        // there when the player pressed the button, not a camera move they have to wait out.
        Transform::from_translation(
            ORIGIN + forge.graph.centre() + Vec3::new(0.0, 0.0, forge.graph.framing()),
        )
        .looking_at(ORIGIN + forge.graph.centre(), Vec3::Y),
    ));
    commands.spawn((
        Rig,
        PointLight {
            intensity: 4_000_000.0,
            range: 90.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(ORIGIN + Vec3::new(-6.0, 7.0, 12.0)),
    ));
}

pub fn leave(mut commands: Commands, rig: Query<Entity, With<Rig>>) {
    for e in rig {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<Forge>();
}

/// Reads the pad: re-centres the rig on whatever the code named, and asks for crafts.
///
/// A code pressed in here changes what is in your hand as well as what is in the middle,
/// because it is the same press meaning the same thing — you walk out of the rig holding
/// the last thing you looked at, which is nearly always the thing you came in to make.
pub fn drive(
    nav: Res<Nav>,
    stock: Res<Stock>,
    mut belt: ResMut<Belt>,
    mut held: ResMut<Held>,
    mut forge: ResMut<Forge>,
    mut requests: ResMut<CraftRequests>,
) {
    // The same two presses that take a thing off the belt, aimed at this room instead: a
    // completed code is put in the middle, and its whole neighbourhood unfolds around it.
    // Anything in the game is two presses away from here — including the things this graph
    // is not currently drawing.
    if let Some(item) = belt::press(nav.dir, &mut belt, &mut held)
        && item != forge.middle()
    {
        forge.refocus(item);
    }
    if nav.craft {
        // The deepest thing that is actually makeable, not the thing in the middle: a
        // child holding the button on a car makes the nails first and sees each appear.
        if let Some(step) = next_step(&stock.0, forge.middle()) {
            requests.0.push(step);
        }
    }
}

/// Builds the geometry for the current graph, when the graph has changed.
pub fn rebuild(
    mut commands: Commands,
    mut forge: ResMut<Forge>,
    kit: Res<Kit>,
    palette: Res<Palette>,
    built: Query<Entity, With<Built>>,
) {
    if !forge.dirty {
        return;
    }
    forge.dirty = false;
    for e in built {
        commands.entity(e).despawn();
    }

    let graph = &forge.graph;
    let middle = graph.focus;
    let centre = graph.centre();
    let reach = graph.framing();

    // A dark pane behind the rig, so a nail against a sunlit sand cliff is still a nail.
    commands.spawn((
        Rig,
        Built,
        Mesh3d(kit.cube()),
        MeshMaterial3d(kit.backdrop()),
        Transform::from_translation(ORIGIN + centre + Vec3::new(0.0, 0.0, -3.0))
            .with_scale(Vec3::new(reach * 4.0, reach * 4.0, 0.1)),
    ));

    for wire in &graph.wires {
        let (from, to) = (graph.nodes[wire.from].at, graph.nodes[wire.to].at);
        let along = to - from;
        commands.spawn((
            Rig,
            Built,
            Mesh3d(kit.cube()),
            MeshMaterial3d(kit.string()),
            Transform::from_translation(ORIGIN + (from + to) / 2.0)
                .looking_at(ORIGIN + to, Vec3::Y)
                .with_scale(Vec3::new(0.045, 0.045, along.length())),
        ));
        // Beads sit on the middle two thirds of the string, so they never disappear
        // behind the things at either end of it.
        for nth in 0..wire.need {
            let along_frac = if wire.need == 1 {
                0.5
            } else {
                0.22 + 0.56 * nth as f32 / (wire.need - 1) as f32
            };
            let part = graph.nodes[wire.from].item;
            commands.spawn((
                Rig,
                Built,
                Bead {
                    part,
                    nth,
                    along: along_frac,
                    lit: kit.lit(part),
                    dark: kit.dark(),
                },
                Mesh3d(kit.bead()),
                MeshMaterial3d(kit.dark()),
                Transform::from_translation(ORIGIN + from.lerp(to, along_frac))
                    .with_scale(Vec3::splat(0.26)),
            ));
        }
    }

    for node in &graph.nodes {
        rig::silhouette(
            &mut commands,
            &palette,
            node.item,
            ORIGIN + node.at,
            (
                Rig,
                Built,
                NodeView {
                    item: node.item,
                    at: node.at,
                    pop: 0.0,
                },
            ),
        );

        // The notch bar: how many of this you own, as a height rather than a number.
        rig::notch_bar(
            &mut commands,
            &kit,
            node.item,
            ORIGIN + node.at,
            (Rig, Built),
        );

        // And under it, the two presses that put it in the middle. The same arrowheads the
        // belt hangs on its strings, so a child learns the whole alphabet in whichever room
        // they happen to be standing in.
        //
        // Stood in front of the graph rather than in it: directly under a node is where
        // every string it owes leaves from, and a code drawn among the beads is a code read
        // through them.
        rig::code_glyph(
            &mut commands,
            &kit,
            node.item,
            ORIGIN + node.at + Vec3::new(0.0, -0.92, 0.7),
            GLYPH,
            (Rig, Built),
        );
    }

    commands.spawn((
        Rig,
        Built,
        Cursor,
        Mesh3d(kit.ring()),
        MeshMaterial3d(kit.waiting()),
        Transform::from_translation(ORIGIN + graph.pos(middle))
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
    ));
}

// ---------------------------------------------------------------------------
// Everything that moves. One system per moving thing, because two of them asking for
// `&mut Transform` in the same system is a query conflict bevy refuses at startup — and
// six small systems each say what they animate in their own name.
// ---------------------------------------------------------------------------

/// Beads light up as the pile fills, and a string whose beads are all lit runs a slow
/// wave up itself — the difference between "ready" and "still owed", with no tick, no
/// number and no colour anybody has to be taught.
pub fn beads(
    time: Res<Time>,
    stock: Res<Stock>,
    mut beads: Query<(&Bead, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let clock = time.elapsed_secs();
    for (bead, mut at, mut paint) in &mut beads {
        let lit = stock.0.count(bead.part) > bead.nth;
        let want = if lit { &bead.lit } else { &bead.dark };
        if paint.0 != *want {
            paint.0 = want.clone();
        }
        let wave = if lit {
            (clock * 2.4 - bead.along * 6.0).sin() * 0.5 + 0.5
        } else {
            0.0
        };
        at.scale = Vec3::splat(0.24 + 0.10 * wave);
    }
}

/// The things themselves: turning slowly, and swelling for a moment when one is made.
///
/// The turn is not decoration. Every model in this game is a table of boxes, and a table
/// of boxes seen dead-on is a flat coloured patch; turning is what makes a rifle read as
/// a rifle from the other side of a living room.
pub fn nodes(
    time: Res<Time>,
    forge: Res<Forge>,
    mut nodes: Query<(&mut NodeView, &mut Transform)>,
) {
    let (dt, clock) = (time.delta_secs(), time.elapsed_secs());
    let middle = forge.graph.focus;
    for (mut view, mut at) in &mut nodes {
        view.pop = (view.pop - dt).max(0.0);
        let base = if view.item == middle { 1.25 } else { 0.92 };
        at.scale = Vec3::splat(base * (1.0 + 0.45 * (view.pop / POP)));
        at.rotation = Quat::from_rotation_y(clock * 0.55 + view.at.x);
    }
}

/// The ring around the middle: green when the button in their hand would make one right
/// now, amber when something under it still has to be dug up or built.
pub fn cursor(
    time: Res<Time>,
    stock: Res<Stock>,
    forge: Res<Forge>,
    kit: Res<Kit>,
    mut ring: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>), With<Cursor>>,
) {
    let (dt, clock) = (time.delta_secs(), time.elapsed_secs());
    let Ok((mut at, mut paint)) = ring.single_mut() else {
        return;
    };
    let middle = forge.graph.focus;
    at.translation = at
        .translation
        .lerp(ORIGIN + forge.graph.pos(middle), (dt * 14.0).min(1.0));
    at.scale = Vec3::splat(1.42 * (1.0 + 0.06 * (clock * 5.0).sin()));
    let want = if stock.0.can_craft(middle) {
        kit.ready()
    } else {
        kit.waiting()
    };
    if paint.0 != want {
        paint.0 = want;
    }
}

/// The parts a craft just spent, on their way up into the thing they became.
pub fn flight(
    time: Res<Time>,
    mut commands: Commands,
    mut flying: Query<(Entity, &mut InFlight, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut flight, mut at) in &mut flying {
        flight.left -= dt;
        if flight.left <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let done = 1.0 - flight.left / FLIGHT;
        at.translation = flight.from.lerp(flight.to, done * done);
        at.scale = Vec3::splat(0.34 * (1.0 - done * 0.7));
    }
}

/// The camera, easing to hold whatever the graph currently is. Eased rather than cut, so
/// that re-centring on a nail reads as the rig *moving* and not as a new screen.
pub fn eye(time: Res<Time>, forge: Res<Forge>, mut eye: Query<&mut Transform, With<Eye>>) {
    let Ok(mut at) = eye.single_mut() else {
        return;
    };
    let centre = ORIGIN + forge.graph.centre();
    let want = centre + Vec3::new(0.0, 0.0, forge.graph.framing());
    at.translation = at
        .translation
        .lerp(want, (time.delta_secs() * 4.0).min(1.0));
    at.rotation = Transform::from_translation(at.translation)
        .looking_at(centre, Vec3::Y)
        .rotation;
}

/// Celebrates whatever the pile says really appeared since last frame.
///
/// Driven off the [`Stock`] rather than off the button, because the button is a *request*
/// and the host is what answers it: on a peer the answer arrives a round trip later, and
/// a rig that popped on the press would cheer for crafts the host refused. One rule
/// covers both — the thing that grew is the thing that was made.
pub fn react(
    mut commands: Commands,
    kit: Res<Kit>,
    stock: Res<Stock>,
    forge: ResMut<Forge>,
    mut nodes: Query<&mut NodeView>,
) {
    let grew: Vec<Item> = Item::ALL
        .iter()
        .copied()
        .filter(|i| stock.0.count(*i) > forge.seen.count(*i))
        .collect();
    if grew.is_empty() {
        return;
    }
    let forge = forge.into_inner();
    forge.seen = stock.0.clone();
    for made in grew {
        for mut view in nodes.iter_mut() {
            if view.item == made {
                view.pop = POP;
            }
        }
        // The parts it cost, thrown up their strings into the thing they became.
        let Some(product) = forge.graph.nodes.iter().find(|n| n.item == made) else {
            continue;
        };
        for (ingredient, n) in made.recipe() {
            let Some(from) = forge.graph.nodes.iter().find(|n| n.item == *ingredient) else {
                continue;
            };
            for i in 0..*n {
                commands.spawn((
                    Rig,
                    InFlight {
                        from: ORIGIN + from.at,
                        to: ORIGIN + product.at,
                        // Staggered, so eight nails arrive as eight nails and not as one
                        // blob: the count is still visible while it is being spent.
                        left: FLIGHT + i as f32 * 0.05,
                    },
                    Mesh3d(kit.bead()),
                    MeshMaterial3d(kit.lit(*ingredient)),
                    Transform::from_translation(ORIGIN + from.at).with_scale(Vec3::splat(0.34)),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole layout rests on: every string runs from a lower tier to a
    /// higher one. Break it and a recipe draws a line going down, which reads as the
    /// product making the part.
    #[test]
    fn every_string_runs_upward() {
        for focus in Item::ALL {
            let g = Graph::around(*focus);
            for wire in &g.wires {
                assert!(
                    g.nodes[wire.from].tier > g.nodes[wire.to].tier,
                    "{:?} -> {:?} does not go up, with {focus:?} in the middle",
                    g.nodes[wire.from].item,
                    g.nodes[wire.to].item
                );
            }
        }
    }

    /// The graph case, live in today's registry: a car needs stone directly *and* through
    /// the nails it needs. One stone node, two strings — the thing that must not turn
    /// into two stones the day a recipe gains a second parent.
    #[test]
    fn a_shared_part_is_one_node_with_two_strings() {
        let g = Graph::around(Item::Car);
        let stones: Vec<&Node> = g.nodes.iter().filter(|n| n.item == Item::Stone).collect();
        assert_eq!(stones.len(), 1, "stone was drawn twice");
        let out = g
            .wires
            .iter()
            .filter(|w| g.nodes[w.from].item == Item::Stone)
            .count();
        assert_eq!(out, 2, "stone feeds the car and the nail");
        assert!(
            stones[0].tier > g.nodes.iter().find(|n| n.item == Item::Nail).unwrap().tier,
            "the longer path is what puts stone below the nail"
        );
    }

    /// No item is laid out twice, whatever is in the middle — a cursor cannot be in two
    /// places, and a bead row that counted one of two copies would be a lie.
    #[test]
    fn every_item_appears_at_most_once() {
        for focus in Item::ALL {
            let g = Graph::around(*focus);
            let mut seen: Vec<Item> = g.nodes.iter().map(|n| n.item).collect();
            let before = seen.len();
            seen.sort_by_key(|i| i.index());
            seen.dedup();
            assert_eq!(
                seen.len(),
                before,
                "something is drawn twice around {focus:?}"
            );
        }
    }

    /// What a player goes to the rig for: standing on a car, they can see what a car is
    /// made of and what those are made of, without moving the cursor.
    #[test]
    fn the_focus_shows_its_whole_ancestry() {
        let g = Graph::around(Item::Car);
        let items: Vec<Item> = g.nodes.iter().map(|n| n.item).collect();
        for wanted in [Item::Car, Item::Wood, Item::Stone, Item::Nail] {
            assert!(items.contains(&wanted), "{wanted:?} is missing");
        }
        assert_eq!(g.focus, Item::Car);
    }

    /// The way up is drawn too, which is the half a tree view does not have: a nail is
    /// worth making because of the six things above it, and that is the picture.
    #[test]
    fn a_part_shows_what_it_goes_into() {
        let g = Graph::around(Item::Nail);
        let above: Vec<Item> = g
            .nodes
            .iter()
            .filter(|n| n.tier == -1)
            .map(|n| n.item)
            .collect();
        for wanted in [
            Item::Hammer,
            Item::Drill,
            Item::Handgun,
            Item::Rifle,
            Item::Car,
        ] {
            assert!(
                above.contains(&wanted),
                "{wanted:?} is not shown above a nail"
            );
        }
    }

    /// The d-pad's rule in here is the belt's rule: two presses name a thing, and the thing
    /// named goes in the middle — whether or not the graph on screen was drawing it. There
    /// is no grid to fall off, which is the point of having deleted the cursor.
    #[test]
    fn a_code_puts_anything_in_the_middle() {
        let mut forge = Forge::new(Item::Car, Inventory::default());
        let mut belt = Belt::default();
        let mut held = Held(Item::Car);
        for want in [Item::Parachute, Item::Sand, Item::Nail, Item::Car] {
            let [first, second] = crate::rig::code(want);
            assert_eq!(belt::press(Some(first), &mut belt, &mut held), None);
            assert_eq!(belt::press(Some(second), &mut belt, &mut held), Some(want));
            forge.refocus(want);
            assert_eq!(forge.middle(), want);
            assert!(forge.graph.nodes.iter().any(|n| n.item == want));
        }
    }

    /// One press, one thing made, deepest first. The whole of "multi-step crafting with
    /// no wait": from six wood and ten stone, a car is a run of presses and nothing else.
    #[test]
    fn holding_the_button_builds_the_car_from_the_bottom_up() {
        let mut stock = Inventory::default();
        stock.add(Item::Wood, 6);
        stock.add(Item::Stone, 10);

        let mut made = Vec::new();
        while let Some(step) = next_step(&stock, Item::Car) {
            assert!(stock.craft(step));
            made.push(step);
            assert!(made.len() < 40, "the chain did not close");
        }
        assert_eq!(stock.count(Item::Car), 1);
        assert_eq!(
            made.iter().filter(|i| **i == Item::Nail).count(),
            8,
            "the nails get made first, one press each"
        );
        assert_eq!(made.last(), Some(&Item::Car), "and the car last");
    }

    /// A chain that bottoms out on something you have to dig up asks for nothing, rather
    /// than asking for the thing at the bottom for ever.
    #[test]
    fn a_chain_that_needs_digging_asks_for_nothing() {
        let stock = Inventory::default();
        assert_eq!(next_step(&stock, Item::Car), None);
        assert_eq!(next_step(&stock, Item::Wood), None, "wood is dug, not made");
    }
}
