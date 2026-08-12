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
//! **Two presses to any recipe.** The rig has no cursor of its own: it hangs whatever the
//! pad ([`crate::hotbar`]) is holding in the middle, and the pad stays on screen while the
//! rig is up. So a code typed in here re-centres the graph on that thing, and the same code
//! typed out in the world puts that thing in your hand — one selection, two views of it.
//!
//! **No words, and no second inventory.** What the player is told is carried by shape
//! (each craftable has its own silhouette), by colour (the registry's own), by the beads,
//! and by the bar beside each node — one notch per one you own, so crafting makes a
//! visible thing grow. The only text in the mode is none.
//!
//! **What this module does not know.** It draws [`Pocket`] and it asks for crafts by
//! pushing to [`CraftRequests`]. Whose pile that is, and who is allowed to say a craft
//! happened, stays with the host in [`crate::game`] — so the same rig runs in the game,
//! where the host answers, and under `blockgame film`, where a test harness does.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::avatar::{self, Palette, Part, Skin, part};
use crate::inventory::{Held, Inventory, Pocket};
use crate::registry::{Class, Item};

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

/// How many you own before the bar beside a node stops growing. Ten notches is already
/// taller than the node; past that the number stops being a shape and starts being a
/// number, which is the thing this mode does not do.
const BAR_CAP: u32 = 10;
const BAR_NOTCH: f32 = 0.11;

/// Seconds a craft's flying parts take to reach the thing they were spent on, and how
/// long the product stays swollen afterwards.
const FLIGHT: f32 = 0.42;
const POP: f32 = 0.30;

/// What the player is asking of the rig this frame. Filled from the pad in the game and
/// from a script under `film`, so the rig itself never asks which.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Nav {
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
    /// Node indices tier by tier, topmost tier first — the shelves the rig is built on.
    pub rows: Vec<Vec<usize>>,
    /// Where in `rows` the focus is.
    pub focus: (usize, usize),
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
        let mut rows: Vec<Vec<usize>> = Vec::new();
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
            let row = here
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let x = (i as f32 - (here.len() as f32 - 1.0) / 2.0) * gap;
                    nodes.push(Node {
                        item: *item,
                        tier: t,
                        at: Vec3::new(x, -(t as f32) * TIER_GAP, 0.0),
                    });
                    nodes.len() - 1
                })
                .collect();
            rows.push(row);
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

        let focus_at = rows
            .iter()
            .enumerate()
            .find_map(|(r, row)| {
                row.iter()
                    .position(|n| nodes[*n].item == focus)
                    .map(|c| (r, c))
            })
            .expect("the focus is laid out");

        Graph {
            nodes,
            wires,
            rows,
            focus: focus_at,
        }
    }

    pub fn item_at(&self, cursor: (usize, usize)) -> Item {
        self.nodes[self.rows[cursor.0][cursor.1]].item
    }

    fn pos(&self, cursor: (usize, usize)) -> Vec3 {
        self.nodes[self.rows[cursor.0][cursor.1]].at
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
// The silhouettes — what makes a nail read as a nail with no label under it.
// ---------------------------------------------------------------------------

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

/// What one item looks like on the rig, and how big to draw the table.
///
/// The car is the game's own car — the one the player drives — shrunk onto a shelf,
/// because the strongest possible label for "this makes a car" is a car.
fn icon(item: Item) -> (&'static [Part], f32, f32) {
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
fn repaint(p: &Part, as_item: Item) -> Part {
    Part {
        skin: match p.skin {
            Skin::Paint(_) => Skin::Paint(as_item),
            other => other,
        },
        size: p.size,
        at: p.at,
    }
}

// ---------------------------------------------------------------------------
// The rig — components, and the systems that build and animate it.
// ---------------------------------------------------------------------------

/// Everything the rig owns, so leaving is one despawn.
#[derive(Component)]
pub struct Rig;

/// The part of the rig that is a picture of *this* graph — everything but the camera, the
/// lamp, and the parts currently in flight. Re-centring throws exactly these away and
/// builds them again, which is why it is a marker and not a list of `Without`s that has to
/// be extended every time the rig grows a new kind of thing.
#[derive(Component)]
pub struct Built;

/// The rig's state between frames: what is in the middle, and what the pile looked like
/// when it was last drawn.
///
/// There is no cursor. The thing in the middle is the thing the pad is holding, and the pad
/// is the same two presses inside the rig as it is outside — so a rig cursor would be a
/// second way to point at an item, next to a way that already reaches every item in two
/// presses. The one that could be somewhere else was the one that went.
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

    /// What the rig is centred on, and so what the craft button pays for.
    pub fn middle(&self) -> Item {
        self.graph.item_at(self.graph.focus)
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

/// The notch bar beside a node: one notch lit per one owned.
#[derive(Component)]
pub struct Notch {
    item: Item,
    nth: u32,
    lit: Handle<StandardMaterial>,
}

/// The ring the cursor wears.
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

/// Meshes and paints the rig reuses. Built once, on the way in.
#[derive(Resource)]
pub struct Kit {
    palette: Palette,
    cube: Handle<Mesh>,
    ring: Handle<Mesh>,
    bead: Handle<Mesh>,
    /// Per item: the bright paint a lit bead or notch wears.
    lit: [Handle<StandardMaterial>; Item::COUNT],
    dark: Handle<StandardMaterial>,
    string: Handle<StandardMaterial>,
    ready: Handle<StandardMaterial>,
    waiting: Handle<StandardMaterial>,
    backdrop: Handle<StandardMaterial>,
}

pub fn enter(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    forge: Res<Forge>,
) {
    let palette = Palette::new(&mut meshes, &mut materials);
    fn glow(
        materials: &mut Assets<StandardMaterial>,
        c: Color,
        strength: f32,
    ) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: c,
            emissive: LinearRgba::from(c) * strength,
            perceptual_roughness: 0.4,
            ..default()
        })
    }
    let lit = std::array::from_fn(|i| {
        let [r, g, b] = Item::ALL[i].color();
        glow(&mut materials, Color::linear_rgb(r, g, b), 3.0)
    });
    let kit = Kit {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Torus::new(0.86, 0.95)),
        bead: meshes.add(Sphere::new(0.5)),
        lit,
        dark: materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.11, 0.14),
            perceptual_roughness: 0.9,
            ..default()
        }),
        string: glow(&mut materials, Color::srgb(0.26, 0.32, 0.42), 0.6),
        ready: glow(&mut materials, Color::srgb(0.45, 1.0, 0.5), 4.0),
        waiting: glow(&mut materials, Color::srgb(1.0, 0.72, 0.28), 2.0),
        backdrop: materials.add(StandardMaterial {
            base_color: Color::srgba(0.02, 0.03, 0.06, 0.86),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
        palette,
    };

    // The rig's own camera, drawn over the world without wiping it: the graph hangs in
    // the air in front of wherever the player was standing, which is the point of
    // building it out of blocks rather than out of panels.
    commands.spawn((
        Rig,
        Eye,
        Camera3d::default(),
        Camera {
            order: 1,
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

    commands.insert_resource(kit);
}

pub fn leave(mut commands: Commands, rig: Query<Entity, With<Rig>>) {
    for e in rig {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<Kit>();
    commands.remove_resource::<Forge>();
}

/// Follows the pad, and asks for crafts.
///
/// The rig has no navigation of its own: whatever the pad is holding is what hangs in the
/// middle, so typing a code inside the rig re-centres it on that thing — and typing the
/// same code outside puts the same thing in your hand. Two presses reach any recipe in the
/// game from any other, which is a thing walking a graph could never promise.
pub fn drive(
    nav: Res<Nav>,
    held: Res<Held>,
    pocket: Res<Pocket>,
    mut forge: ResMut<Forge>,
    mut requests: ResMut<CraftRequests>,
) {
    if held.0 != forge.middle() {
        forge.refocus(held.0);
    }
    if nav.craft {
        // The deepest thing that is actually makeable, not the thing pointed at: a child
        // holding the button on a car makes the nails first and sees each one appear.
        if let Some(step) = next_step(&pocket.0, forge.middle()) {
            requests.0.push(step);
        }
    }
}

/// Builds the geometry for the current graph, when the graph has changed.
pub fn rebuild(
    mut commands: Commands,
    mut forge: ResMut<Forge>,
    kit: Res<Kit>,
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
    let middle = graph.item_at(graph.focus);
    let centre = graph.centre();
    let reach = graph.framing();

    // A dark pane behind the rig, so a nail against a sunlit sand cliff is still a nail.
    commands.spawn((
        Rig,
        Built,
        Mesh3d(kit.cube.clone()),
        MeshMaterial3d(kit.backdrop.clone()),
        Transform::from_translation(ORIGIN + centre + Vec3::new(0.0, 0.0, -3.0))
            .with_scale(Vec3::new(reach * 4.0, reach * 4.0, 0.1)),
    ));

    for wire in &graph.wires {
        let (from, to) = (graph.nodes[wire.from].at, graph.nodes[wire.to].at);
        let along = to - from;
        commands.spawn((
            Rig,
            Built,
            Mesh3d(kit.cube.clone()),
            MeshMaterial3d(kit.string.clone()),
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
                    lit: kit.lit[part.index()].clone(),
                    dark: kit.dark.clone(),
                },
                Mesh3d(kit.bead.clone()),
                MeshMaterial3d(kit.dark.clone()),
                Transform::from_translation(ORIGIN + from.lerp(to, along_frac))
                    .with_scale(Vec3::splat(0.26)),
            ));
        }
    }

    for node in &graph.nodes {
        let scale = if node.item == middle { 1.25 } else { 0.92 };
        let (parts, model_scale, lift) = icon(node.item);
        let body = commands
            .spawn((
                Rig,
                Built,
                NodeView {
                    item: node.item,
                    at: node.at,
                    pop: 0.0,
                },
                Transform::from_translation(ORIGIN + node.at).with_scale(Vec3::splat(scale)),
                Visibility::Visible,
            ))
            .id();
        commands.entity(body).with_children(|model| {
            for p in parts {
                let p = repaint(p, node.item);
                model.spawn((
                    Mesh3d(kit.palette.cube()),
                    MeshMaterial3d(kit.palette.paint(p.skin)),
                    Transform {
                        translation: Vec3::from(p.at) * model_scale + Vec3::new(0.0, lift, 0.0),
                        scale: Vec3::from(p.size) * model_scale,
                        ..default()
                    },
                ));
            }
        });

        // The notch bar: how many of this you own, as a height rather than a number.
        for nth in 0..BAR_CAP {
            commands.spawn((
                Rig,
                Built,
                Notch {
                    item: node.item,
                    nth,
                    lit: kit.lit[node.item.index()].clone(),
                },
                Mesh3d(kit.cube.clone()),
                MeshMaterial3d(kit.dark.clone()),
                Transform::from_translation(
                    ORIGIN + node.at + Vec3::new(0.86, -0.62 + nth as f32 * BAR_NOTCH, 0.0),
                )
                .with_scale(Vec3::new(0.20, BAR_NOTCH * 0.72, 0.20)),
            ));
        }
    }

    commands.spawn((
        Rig,
        Built,
        Cursor,
        Mesh3d(kit.ring.clone()),
        MeshMaterial3d(kit.waiting.clone()),
        Transform::from_translation(ORIGIN + graph.pos(graph.focus))
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
    pocket: Res<Pocket>,
    mut beads: Query<(&Bead, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let clock = time.elapsed_secs();
    for (bead, mut at, mut paint) in &mut beads {
        let lit = pocket.0.count(bead.part) > bead.nth;
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

/// The bar beside each node: one notch lit per one owned.
pub fn notches(
    pocket: Res<Pocket>,
    kit: Res<Kit>,
    mut notches: Query<(&Notch, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (notch, mut paint) in &mut notches {
        let lit = pocket.0.count(notch.item).min(BAR_CAP) > notch.nth;
        let want = if lit { &notch.lit } else { &kit.dark };
        if paint.0 != *want {
            paint.0 = want.clone();
        }
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
    let middle = forge.graph.item_at(forge.graph.focus);
    for (mut view, mut at) in &mut nodes {
        view.pop = (view.pop - dt).max(0.0);
        let base = if view.item == middle { 1.25 } else { 0.92 };
        at.scale = Vec3::splat(base * (1.0 + 0.45 * (view.pop / POP)));
        at.rotation = Quat::from_rotation_y(clock * 0.55 + view.at.x);
    }
}

/// The ring around the thing in the middle: green when the button in their hand would make
/// one right now, amber when something under it still has to be dug up or built.
///
/// It slides rather than jumps, so re-centring the rig on a code reads as the ring
/// travelling to the new thing and the graph rearranging around it.
pub fn cursor(
    time: Res<Time>,
    pocket: Res<Pocket>,
    forge: Res<Forge>,
    kit: Res<Kit>,
    mut ring: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>), With<Cursor>>,
) {
    let (dt, clock) = (time.delta_secs(), time.elapsed_secs());
    let Ok((mut at, mut paint)) = ring.single_mut() else {
        return;
    };
    let graph = &forge.graph;
    at.translation = at
        .translation
        .lerp(ORIGIN + graph.pos(graph.focus), (dt * 14.0).min(1.0));
    at.scale = Vec3::splat(1.42 * (1.0 + 0.06 * (clock * 5.0).sin()));
    let want = if pocket.0.can_craft(forge.middle()) {
        &kit.ready
    } else {
        &kit.waiting
    };
    if paint.0 != *want {
        paint.0 = want.clone();
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
/// Driven off the [`Pocket`] rather than off the button, because the button is a *request*
/// and the host is what answers it: on a peer the answer arrives a round trip later, and
/// a rig that popped on the press would cheer for crafts the host refused. One rule
/// covers both — the thing that grew is the thing that was made.
pub fn react(
    mut commands: Commands,
    kit: Res<Kit>,
    pocket: Res<Pocket>,
    forge: ResMut<Forge>,
    mut nodes: Query<&mut NodeView>,
) {
    let grew: Vec<Item> = Item::ALL
        .iter()
        .copied()
        .filter(|i| pocket.0.count(*i) > forge.seen.count(*i))
        .collect();
    if grew.is_empty() {
        return;
    }
    let forge = forge.into_inner();
    forge.seen = pocket.0.clone();
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
                    Mesh3d(kit.bead.clone()),
                    MeshMaterial3d(kit.lit[ingredient.index()].clone()),
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
        assert_eq!(g.nodes[g.rows[g.focus.0][g.focus.1]].item, Item::Car);
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
