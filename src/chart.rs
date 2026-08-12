//! The constellation — the hotbar as a star chart, and a d-pad code for every item.
//!
//! Your empty hand is the middle of it. Every item in the game hangs off it as a star, one
//! press away or two, and the presses that reach a star **are** its address: stone is down,
//! a nail is right-up, the car is left-up. The keys never move, so a child learns
//! "right, up" the way they learn a jump, and punches it in without looking.
//!
//! **The code is a path, and the path is drawn.** Press a direction and the chart blooms
//! out of your hand: the stars, the threads between them, and the run of threads from the
//! hand to what you are holding lit up brighter than the rest. Stop pressing and it folds
//! back into the one thing in your hand. So the same fact is available two ways — as a
//! place to walk while you are learning it, and as a code to punch once you know it.
//!
//! **Nothing on it is written.** A star is the item's own silhouette from [`crate::icons`],
//! turning, drawn small if you have none and full size if you have some, with
//! [`crate::forge`]'s notch bar beside it for how many. A star you could make *right now*
//! wears the rig's green ring, so "what can I build?" is a colour and not a menu. Under
//! your hand sits the code itself, as one little d-pad per press with the arm you push lit
//! — and each press drawn at its own height, so the code reads as a two-note tune as much
//! as a shape.
//!
//! **It is the rig, zoomed out.** Craft opens [`crate::forge`] on whatever star you are
//! standing on; the rig is the same items in the same colours with the same rings, one
//! neighbourhood at a time, and it keeps what it always kept — the recipe and the paying
//! for it. One map, two zooms, one d-pad through both.
//!
//! The layout is read off [`Item::ALL`] and nothing else: the table filled onto a four-way
//! tree, breadth first. A new item takes the next free exit, so it arrives with a code of
//! its own and no existing code moves.

use bevy::prelude::*;
use std::collections::VecDeque;

use crate::avatar::Palette;
use crate::icons;
use crate::inventory::{Held, Inventory, Stock};
use crate::registry::Item;

/// Where the chart is built, far above the world and far above the rig: the game's camera
/// stops at 1200 blocks, so the three scenes cannot see each other and none of them needs
/// a render layer to say so.
const ORIGIN: Vec3 = Vec3::new(0.0, 80_000.0, 0.0);

/// How far the first ring of stars sits from the hand, and how far each press after that
/// carries. The first step is longer so the four things one press away read as the trunks
/// they are, with room for their own blossoms.
const HUB: f32 = 3.0;
const STEP: f32 = 1.9;

/// How many you own before the bar beside a star stops growing — the rig's number, because
/// it is the rig's bar.
const BAR_CAP: u32 = 10;
const BAR_NOTCH: f32 = 0.11;

/// Seconds the chart stays open after the last press, and how fast it blooms and folds.
const LINGER: f32 = 1.6;
const BLOOM: f32 = 5.0;

/// Seconds a star stays swollen after one of it appears.
const POP: f32 = 0.35;

/// A star you have none of, as a fraction of one you have: the size of a thing is whether
/// you own it, and the bar beside it is how many.
const EMPTY: f32 = 0.62;

/// Where the eye stands at rest and when the chart is open, and how far above the held star
/// it looks — which is what hangs that star low on the screen, where a hotbar goes.
const REST_BACK: f32 = 7.5;
const REST_LIFT: f32 = 1.05;
const OPEN_BACK: f32 = 13.0;

/// One press of the d-pad.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Right,
    Down,
    Left,
}

impl Dir {
    /// The order the exits off a star are handed out in, and so the order the item table
    /// is laid onto the chart.
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Right, Dir::Down, Dir::Left];

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Right => Dir::Left,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
        }
    }

    fn unit(self) -> Vec2 {
        match self {
            Dir::Up => Vec2::Y,
            Dir::Right => Vec2::X,
            Dir::Down => -Vec2::Y,
            Dir::Left => -Vec2::X,
        }
    }

    /// How high this press is drawn on the code under your hand: up is a high note, down is
    /// a low one, and the two sideways presses sit between them.
    ///
    /// It is the pitch the press would sound if the chart had a voice — an item's code is a
    /// little tune, and this is that tune written down where it can be seen instead.
    fn note(self) -> f32 {
        match self {
            Dir::Up => 1.0,
            Dir::Right => 0.34,
            Dir::Left => -0.34,
            Dir::Down => -1.0,
        }
    }
}

/// Where the cursor is: on a star, or back in your own empty hand.
///
/// The hand is a place on the chart and not a fourteenth item, which is what makes "let go
/// of this" a direction rather than a button — you walk back the way you came.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum At {
    /// You start with nothing in your hand, because you start with nothing.
    #[default]
    Hand,
    On(Item),
}

impl At {
    /// The item selected, or nothing at all — an empty hand is empty, not "grass you have
    /// none of".
    pub fn item(self) -> Option<Item> {
        match self {
            At::Hand => None,
            At::On(item) => Some(item),
        }
    }
}

/// One item, placed, with the presses that reach it.
#[derive(Debug, Clone, PartialEq)]
pub struct Star {
    pub item: Item,
    /// The code: the presses from the hand, in order.
    pub code: Vec<Dir>,
    pub at: Vec2,
    /// One press back down the code.
    pub back: At,
}

/// What the player is asking of the chart this frame. Filled from the pad in the game and
/// from a script under `craft-film`, so the chart itself never asks which — the same shape
/// [`crate::forge::Nav`] has, for the same reason.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Reach {
    pub press: Option<Dir>,
}

/// The chart: every star's place and code, how far open it is, and the pile as of last
/// frame so a count that went up can be cheered.
#[derive(Resource)]
pub struct Chart {
    stars: Vec<Star>,
    open: f32,
    linger: f32,
    seen: Inventory,
}

impl Default for Chart {
    fn default() -> Self {
        Chart::new()
    }
}

impl Chart {
    pub fn new() -> Chart {
        Chart {
            stars: layout(Item::ALL),
            open: 0.0,
            linger: 0.0,
            seen: Inventory::EMPTY,
        }
    }

    pub fn star(&self, item: Item) -> &Star {
        &self.stars[item.index()]
    }

    pub fn code(&self, item: Item) -> &[Dir] {
        &self.star(item).code
    }

    /// Where a place on the chart is drawn. The hand is the middle, which is the one fact
    /// the whole layout is written around.
    fn place(&self, at: At) -> Vec2 {
        match at {
            At::Hand => Vec2::ZERO,
            At::On(item) => self.star(item).at,
        }
    }

    /// One press. `None` when there is no star that way — a dead end you can see before you
    /// press it, because the chart draws the threads it does have.
    pub fn step(&self, at: At, dir: Dir) -> Option<At> {
        let At::On(item) = at else {
            return self.with_code(&[dir]).map(At::On);
        };
        let star = self.star(item);
        let last = *star.code.last().expect("every code is at least one press");
        if last == dir.opposite() {
            return Some(star.back);
        }
        let mut code = star.code.clone();
        code.push(dir);
        self.with_code(&code).map(At::On)
    }

    fn with_code(&self, code: &[Dir]) -> Option<Item> {
        self.stars
            .iter()
            .find(|s| s.code == code)
            .map(|star| star.item)
    }

    /// Is this star on the way from the hand to `held`? That run of threads is the one
    /// drawn lit — the code, as a path.
    fn on_the_way(&self, star: Item, held: At) -> bool {
        match held.item() {
            None => false,
            Some(item) => self.code(item).starts_with(self.code(star)),
        }
    }
}

/// The item table, laid onto a four-way tree from the hand, breadth first.
///
/// Breadth first is what keeps the codes short — everything in today's table is one press
/// away or two — and taking the exits in a fixed order is what keeps them *still*: a new
/// item takes the next free exit at the outside of the chart, and every code already
/// learned still reaches what it always reached.
///
/// A star's exits are every direction but the one it was arrived by, because that one is
/// the way back to the hand. So four stars hang off the hand, three off each of those, and
/// the chart cannot fold back over itself.
fn layout(items: &[Item]) -> Vec<Star> {
    let mut stars: Vec<Star> = Vec::new();
    let mut items = items.iter().copied();
    // Where the next exits leave from: a place, its position, the direction it was reached
    // by, and its code.
    let mut queue: VecDeque<(At, Vec2, Option<Dir>, Vec<Dir>)> =
        VecDeque::from([(At::Hand, Vec2::ZERO, None, Vec::new())]);
    while let Some((from, at, arrived, code)) = queue.pop_front() {
        let reach = if arrived.is_none() { HUB } else { STEP };
        for dir in Dir::ALL {
            if arrived == Some(dir.opposite()) {
                continue;
            }
            let Some(item) = items.next() else {
                return stars;
            };
            let mut code = code.clone();
            code.push(dir);
            let star = Star {
                item,
                at: at + dir.unit() * reach,
                back: from,
                code,
            };
            queue.push_back((At::On(item), star.at, Some(dir), star.code.clone()));
            stars.push(star);
        }
    }
    stars
}

// ---------------------------------------------------------------------------
// The scene. Everything below draws what the layout above decided.
// ---------------------------------------------------------------------------

/// The whole chart hangs off one entity, so putting it away while the rig is up is one
/// write and the local coordinates above are what the layout produced.
#[derive(Component)]
pub struct ChartRoot;

#[derive(Component)]
pub struct StarView {
    item: Item,
    /// Seconds left of the swell one of these appearing gives it.
    pop: f32,
}

/// Your own hand, at the middle.
#[derive(Component)]
pub struct HandView;

/// One notch of the bar beside a star: lit if you own at least this many.
#[derive(Component)]
pub struct Notch {
    item: Item,
    nth: u32,
    lit: Handle<StandardMaterial>,
}

/// The ring a star wears when the pile would pay for one right now.
#[derive(Component)]
pub struct Ready(Item);

/// The ring the cursor wears — what is in your hand.
#[derive(Component)]
pub struct Cursor;

/// One press of a code, drawn as a thread between two stars.
#[derive(Component)]
pub struct Thread {
    to: Item,
    length: f32,
}

/// A piece of the code drawn under your hand. Torn down and rebuilt when the cursor moves,
/// which is at most once a press.
#[derive(Component)]
pub struct Chip(Vec3);

/// The dark plate behind one star.
///
/// The rig can put a pane behind the whole screen because it is a mode you are standing in.
/// The chart is not — it hangs over a world being played, and dimming that world to read a
/// hotbar is the wrong trade. So the dark is exactly as big as the thing that needs it: a
/// white nail against a sunlit stone cliff is still a nail.
#[derive(Component)]
pub struct Plate(At);

#[derive(Component)]
pub struct Eye;

/// Meshes and paints the chart reuses. Built once, when the world is entered.
#[derive(Resource)]
pub struct Kit {
    palette: Palette,
    cube: Handle<Mesh>,
    ring: Handle<Mesh>,
    lit: [Handle<StandardMaterial>; Item::COUNT],
    dark: Handle<StandardMaterial>,
    thread: Handle<StandardMaterial>,
    trail: Handle<StandardMaterial>,
    pad: Handle<StandardMaterial>,
    ready: Handle<StandardMaterial>,
    cursor: Handle<StandardMaterial>,
    plate: Handle<StandardMaterial>,
}

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

pub fn enter(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    chart: Res<Chart>,
) {
    let palette = Palette::new(&mut meshes, &mut materials);
    let lit = std::array::from_fn(|i| {
        let [r, g, b] = Item::ALL[i].color();
        glow(&mut materials, Color::linear_rgb(r, g, b), 3.0)
    });
    let kit = Kit {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Torus::new(0.80, 0.90)),
        lit,
        dark: materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.11, 0.14),
            perceptual_roughness: 0.9,
            ..default()
        }),
        thread: glow(&mut materials, Color::srgb(0.16, 0.20, 0.29), 0.35),
        trail: glow(&mut materials, Color::srgb(0.70, 0.86, 1.0), 3.2),
        // The unpressed arms of the little d-pads under your hand. Pale rather than dark,
        // because the code has to read against a sunlit cliff as well as against sky, and
        // it is the only thing on the chart with no colour of its own to be seen by.
        pad: glow(&mut materials, Color::srgb(0.42, 0.46, 0.56), 0.5),
        ready: glow(&mut materials, Color::srgb(0.45, 1.0, 0.5), 4.0),
        cursor: glow(&mut materials, Color::srgb(1.0, 0.95, 0.75), 3.0),
        plate: materials.add(StandardMaterial {
            base_color: Color::srgba(0.03, 0.04, 0.08, 0.66),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
        palette,
    };

    // Over the world and over the rig: the chart is the one thing that is never behind
    // anything.
    commands.spawn((
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
        Transform::from_translation(ORIGIN + Vec3::new(0.0, 0.0, REST_BACK))
            .looking_at(ORIGIN, Vec3::Y),
    ));

    let root = commands
        .spawn((
            ChartRoot,
            Transform::from_translation(ORIGIN),
            Visibility::Visible,
        ))
        .id();

    commands.entity(root).with_children(|chart_root| {
        chart_root.spawn((
            PointLight {
                intensity: 3_000_000.0,
                range: 90.0,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_xyz(-4.0, 5.0, 10.0),
        ));
        // Your hand, at the middle. Drawn out of the spaceman's own mitten, so the thing
        // every code starts from is visibly you.
        chart_root.spawn((
            Plate(At::Hand),
            Mesh3d(kit.cube.clone()),
            MeshMaterial3d(kit.plate.clone()),
            Transform::from_xyz(0.0, 0.0, -0.6),
        ));
        chart_root
            .spawn((
                HandView,
                Transform::from_xyz(0.0, 0.0, 0.0),
                Visibility::Visible,
            ))
            .with_children(|hand| {
                for p in icons::MITTEN {
                    hand.spawn((
                        Mesh3d(kit.palette.cube()),
                        MeshMaterial3d(kit.palette.paint(p.skin)),
                        Transform {
                            translation: Vec3::from(p.at),
                            scale: Vec3::from(p.size),
                            ..default()
                        },
                    ));
                }
            });

        for item in Item::ALL {
            let star = chart.star(*item);
            let from = chart.place(star.back);
            let along = star.at - from;
            chart_root.spawn((
                Thread {
                    to: *item,
                    length: along.length(),
                },
                Mesh3d(kit.cube.clone()),
                MeshMaterial3d(kit.thread.clone()),
                Transform::from_translation(((from + star.at) / 2.0).extend(-0.3))
                    .with_rotation(Quat::from_rotation_z(along.to_angle()))
                    .with_scale(Vec3::new(along.length(), 0.05, 0.05)),
            ));

            chart_root.spawn((
                Plate(At::On(*item)),
                Mesh3d(kit.cube.clone()),
                MeshMaterial3d(kit.plate.clone()),
                // Off-centre by enough to take the notch bar in with it: the count is as
                // hard to read against a cliff as the thing it counts.
                Transform::from_translation((star.at + Vec2::new(0.18, 0.0)).extend(-0.6)),
            ));

            chart_root
                .spawn((
                    StarView {
                        item: *item,
                        pop: 0.0,
                    },
                    Transform::from_translation(star.at.extend(0.0)),
                    Visibility::Visible,
                ))
                .with_children(|body| {
                    let (parts, model_scale, lift) = icons::icon(*item);
                    for p in parts {
                        let p = icons::repaint(p, *item);
                        body.spawn((
                            Mesh3d(kit.palette.cube()),
                            MeshMaterial3d(kit.palette.paint(p.skin)),
                            Transform {
                                translation: Vec3::from(p.at) * model_scale
                                    + Vec3::new(0.0, lift, 0.0),
                                scale: Vec3::from(p.size) * model_scale,
                                ..default()
                            },
                        ));
                    }
                });

            chart_root.spawn((
                Ready(*item),
                Mesh3d(kit.ring.clone()),
                MeshMaterial3d(kit.ready.clone()),
                Transform::from_translation(star.at.extend(-0.1))
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Visibility::Hidden,
            ));

            // The bar: how many you own, as a height rather than a number. The rig's bar,
            // in the rig's place beside the thing it counts.
            for nth in 0..BAR_CAP {
                chart_root.spawn((
                    Notch {
                        item: *item,
                        nth,
                        lit: kit.lit[item.index()].clone(),
                    },
                    Mesh3d(kit.cube.clone()),
                    MeshMaterial3d(kit.dark.clone()),
                    Transform::from_translation(
                        (star.at + Vec2::new(0.80, -0.55 + nth as f32 * BAR_NOTCH)).extend(0.0),
                    )
                    .with_scale(Vec3::new(0.18, BAR_NOTCH * 0.72, 0.18)),
                ));
            }
        }

        chart_root.spawn((
            Cursor,
            Mesh3d(kit.ring.clone()),
            MeshMaterial3d(kit.cursor.clone()),
            Transform::from_xyz(0.0, 0.0, -0.05)
                .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        ));
    });

    commands.insert_resource(kit);
}

/// Shows or hides the whole chart — the rig says everything it has to say for itself, and
/// a star chart hanging in front of it would be two cursors on one screen.
pub fn show<F: bevy::ecs::query::QueryFilter>(root: &mut Query<&mut Visibility, F>, visible: bool) {
    let want = if visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut v in root.iter_mut() {
        if *v != want {
            *v = want;
        }
    }
}

/// Reads the pad: one press, one step along the chart.
///
/// A press with no star that way moves nothing and still opens the chart, because the
/// answer to "is there anything up here?" is the picture.
pub fn drive(reach: Res<Reach>, mut chart: ResMut<Chart>, mut held: ResMut<Held>) {
    let Some(dir) = reach.press else {
        return;
    };
    chart.linger = LINGER;
    if let Some(at) = chart.step(held.0, dir) {
        held.0 = at;
    }
}

/// Blooms the chart while it is being used and folds it away when it is not.
pub fn linger(time: Res<Time>, mut chart: ResMut<Chart>) {
    let dt = time.delta_secs();
    chart.linger = (chart.linger - dt).max(0.0);
    let want = if chart.linger > 0.0 { 1.0 } else { 0.0 };
    chart.open += (want - chart.open).clamp(-BLOOM * dt, BLOOM * dt);
}

/// Cheers whatever the pile says really appeared since last frame.
///
/// Off the pile rather than off the button, for [`crate::forge::react`]'s reason: on a peer
/// the host answers a craft a round trip later, and a chart that popped on the press would
/// cheer for things that never arrived.
pub fn react(stock: Res<Stock>, mut chart: ResMut<Chart>, mut stars: Query<&mut StarView>) {
    let grew: Vec<Item> = Item::ALL
        .iter()
        .copied()
        .filter(|i| stock.0.count(*i) > chart.seen.count(*i))
        .collect();
    if grew.is_empty() {
        return;
    }
    chart.seen = stock.0.clone();
    for mut star in stars.iter_mut() {
        if grew.contains(&star.item) {
            star.pop = POP;
        }
    }
}

/// The stars themselves: turning, sized by whether you own any, swelling when one arrives,
/// and folded away into the one in your hand when the chart is closed.
pub fn stars(
    time: Res<Time>,
    chart: Res<Chart>,
    stock: Res<Stock>,
    held: Res<Held>,
    mut stars: Query<(&mut StarView, &mut Transform)>,
    mut hand: Query<&mut Transform, (With<HandView>, Without<StarView>)>,
) {
    let (dt, clock) = (time.delta_secs(), time.elapsed_secs());
    for (mut star, mut at) in &mut stars {
        star.pop = (star.pop - dt).max(0.0);
        let owned = if stock.0.count(star.item) > 0 {
            1.0
        } else {
            EMPTY
        };
        // The held star is the one thing that is always there; the rest arrive with the
        // bloom, which is what makes this fold back into a hotbar of one cell.
        let shown = if held.0 == At::On(star.item) {
            1.15
        } else {
            chart.open
        };
        at.scale = Vec3::splat(owned * shown * (1.0 + 0.45 * (star.pop / POP)));
        // A table of boxes seen dead-on is a flat coloured patch; turning is what makes a
        // rifle read as a rifle from across a living room.
        at.rotation = Quat::from_rotation_y(clock * 0.55 + at.translation.x);
    }
    if let Ok(mut at) = hand.single_mut() {
        let shown = if held.0 == At::Hand { 1.15 } else { chart.open };
        at.scale = Vec3::splat(shown);
        at.rotation = Quat::from_rotation_y((clock * 0.4).sin() * 0.35);
    }
}

/// The dark plates, which come and go with the things they are behind.
///
/// Its own system, like everything else that moves here: two systems asking for
/// `&mut Transform` over overlapping sets is a query conflict bevy refuses at startup, and
/// a plate is not a star.
///
/// Unlike a star a plate does not shrink for a thing you have none of — an empty star is
/// exactly the one that needs the contrast most.
pub fn plates(chart: Res<Chart>, held: Res<Held>, mut plates: Query<(&Plate, &mut Transform)>) {
    for (plate, mut at) in &mut plates {
        let shown = if held.0 == plate.0 { 1.15 } else { chart.open };
        at.scale = Vec3::new(1.78 * shown, 1.55 * shown, 0.02);
    }
}

/// The bar beside each star: one notch lit per one owned.
pub fn notches(
    stock: Res<Stock>,
    chart: Res<Chart>,
    held: Res<Held>,
    kit: Res<Kit>,
    mut notches: Query<(
        &Notch,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    for (notch, mut paint, mut vis) in &mut notches {
        let want = if stock.0.count(notch.item).min(BAR_CAP) > notch.nth {
            &notch.lit
        } else {
            &kit.dark
        };
        if paint.0 != *want {
            paint.0 = want.clone();
        }
        // A bar beside a star nobody can see is a row of floating specks.
        let shown = held.0 == At::On(notch.item) || chart.open > 0.5;
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

/// The green ring: this one, you could make right now. The rig's ring, in the rig's
/// colour, so "ready" is one thing to learn and not two.
pub fn rings(
    time: Res<Time>,
    chart: Res<Chart>,
    stock: Res<Stock>,
    mut rings: Query<(&Ready, &mut Transform, &mut Visibility)>,
) {
    let clock = time.elapsed_secs();
    for (ready, mut at, mut vis) in &mut rings {
        let show = stock.0.can_craft(ready.0) && chart.open > 0.05;
        let want = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        at.scale = Vec3::splat(chart.open * (1.0 + 0.05 * (clock * 5.0).sin()));
    }
}

/// The threads between the stars, and the run of them from your hand to what you are
/// holding — the code, drawn as the path it is.
pub fn threads(
    chart: Res<Chart>,
    held: Res<Held>,
    kit: Res<Kit>,
    mut threads: Query<(
        &Thread,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    for (thread, mut at, mut paint) in &mut threads {
        let on_the_way = chart.on_the_way(thread.to, held.0);
        let want = if on_the_way { &kit.trail } else { &kit.thread };
        if paint.0 != *want {
            paint.0 = want.clone();
        }
        let thickness = if on_the_way { 0.10 } else { 0.05 };
        at.scale = Vec3::new(
            thread.length * chart.open,
            thickness * chart.open,
            thickness * chart.open,
        );
    }
}

/// The ring around whatever is in your hand, eased rather than cut so that a press reads as
/// the cursor *going* somewhere.
pub fn cursor(
    time: Res<Time>,
    chart: Res<Chart>,
    held: Res<Held>,
    mut ring: Query<&mut Transform, With<Cursor>>,
) {
    let (dt, clock) = (time.delta_secs(), time.elapsed_secs());
    let Ok(mut at) = ring.single_mut() else {
        return;
    };
    let want = chart.place(held.0).extend(-0.05);
    at.translation = at.translation.lerp(want, (dt * 14.0).min(1.0));
    at.scale = Vec3::splat(1.28 * (1.0 + 0.05 * (clock * 5.0).sin()));
}

/// The code under your hand: one little d-pad per press, with the arm you push lit, each
/// drawn at the height of the note it would sound.
///
/// Rebuilt when the cursor moves and at no other time — and folded away as the chart opens,
/// because an open chart is already showing the same code as a lit path.
#[allow(clippy::too_many_arguments)]
pub fn chip(
    mut commands: Commands,
    chart: Res<Chart>,
    held: Res<Held>,
    kit: Res<Kit>,
    root: Query<Entity, With<ChartRoot>>,
    chips: Query<Entity, With<Chip>>,
    mut drawn: Local<Option<At>>,
    mut showing: Query<(&Chip, &mut Transform)>,
) {
    if *drawn != Some(held.0) {
        *drawn = Some(held.0);
        for old in chips.iter() {
            commands.entity(old).despawn();
        }
        let Ok(root) = root.single() else {
            return;
        };
        let Some(item) = held.0.item() else {
            return;
        };
        let code = chart.code(item);
        let middle = (code.len() as f32 - 1.0) / 2.0;
        let under = chart.star(item).at + Vec2::new(0.0, -1.45);
        commands.entity(root).with_children(|chip| {
            // Its own dark plate, for the stars' reason: a pale grey d-pad against a
            // sunlit stone cliff is a code nobody can read back.
            let plate = Vec3::new(code.len() as f32 * 1.05 + 0.45, 1.0, 0.02);
            chip.spawn((
                Chip(plate),
                Mesh3d(kit.cube.clone()),
                MeshMaterial3d(kit.plate.clone()),
                Transform::from_translation(under.extend(-0.5)).with_scale(plate),
            ));
            for (nth, dir) in code.iter().enumerate() {
                let at = under + Vec2::new((nth as f32 - middle) * 1.05, dir.note() * 0.20);
                // The pad itself: a hub and four arms, one of which is the press. The
                // pressed arm wears the thing's own colour, so the code and the star it
                // reaches are the same colour as well as the same shape.
                let hub = Vec3::splat(0.20);
                chip.spawn((
                    Chip(hub),
                    Mesh3d(kit.cube.clone()),
                    MeshMaterial3d(kit.pad.clone()),
                    Transform::from_translation(at.extend(0.0)).with_scale(hub),
                ));
                for arm in Dir::ALL {
                    let (paint, size) = if arm == *dir {
                        (kit.lit[item.index()].clone(), Vec3::splat(0.28))
                    } else {
                        (kit.pad.clone(), Vec3::splat(0.19))
                    };
                    chip.spawn((
                        Chip(size),
                        Mesh3d(kit.cube.clone()),
                        MeshMaterial3d(paint),
                        Transform::from_translation((at + arm.unit() * 0.27).extend(0.0))
                            .with_scale(size),
                    ));
                }
            }
        });
        return;
    }
    // Open, the lit path says it better than the chip does, so the chip gets out of the way.
    let left = 1.0 - chart.open;
    for (chip, mut at) in showing.iter_mut() {
        let want = chip.0 * left;
        if at.scale != want {
            at.scale = want;
        }
    }
}

/// The eye: close on what is in your hand at rest, and back far enough to hold the whole
/// chart while it is open. One camera easing between two framings, so the chart is seen to
/// unfold out of the thing you are carrying rather than to replace it.
pub fn eye(
    time: Res<Time>,
    chart: Res<Chart>,
    held: Res<Held>,
    mut eye: Query<&mut Transform, With<Eye>>,
) {
    let Ok(mut at) = eye.single_mut() else {
        return;
    };
    let rest = chart.place(held.0) + Vec2::new(0.0, REST_LIFT);
    let focus = ORIGIN + rest.lerp(Vec2::ZERO, chart.open).extend(0.0);
    let back = REST_BACK + (OPEN_BACK - REST_BACK) * chart.open;
    let want = focus + Vec3::new(0.0, 0.0, back);
    at.translation = at
        .translation
        .lerp(want, (time.delta_secs() * 9.0).min(1.0));
    at.rotation = Transform::from_translation(at.translation)
        .looking_at(focus, Vec3::Y)
        .rotation;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every item is somewhere, exactly once, with a code of its own — the property the
    /// whole idea rests on, since the code is how a child asks for the thing.
    #[test]
    fn every_item_has_one_code_and_no_two_share_one() {
        let chart = Chart::new();
        let mut codes: Vec<&[Dir]> = Item::ALL.iter().map(|i| chart.code(*i)).collect();
        assert_eq!(codes.len(), Item::COUNT);
        assert!(codes.iter().all(|c| !c.is_empty()));
        codes.sort_by_key(|c| c.iter().map(|d| format!("{d:?}")).collect::<String>());
        codes.dedup();
        assert_eq!(codes.len(), Item::COUNT, "two items answer to one code");
    }

    /// What makes it a code and not a menu: today's whole table is one press away or two.
    #[test]
    fn nothing_is_more_than_two_presses_away() {
        let chart = Chart::new();
        for item in Item::ALL {
            assert!(
                chart.code(*item).len() <= 2,
                "{item:?} is {} presses in",
                chart.code(*item).len()
            );
        }
    }

    /// Walking the code from an empty hand arrives at the thing it names. This is the test
    /// that would catch a layout whose drawing and whose navigation disagreed — the child
    /// punching in a code they read off the chart is doing exactly this.
    #[test]
    fn a_code_walks_to_the_thing_it_names() {
        let chart = Chart::new();
        for item in Item::ALL {
            let mut at = At::Hand;
            for dir in chart.code(*item) {
                at = chart
                    .step(at, *dir)
                    .expect("a code with no star at the end");
            }
            assert_eq!(at, At::On(*item));
        }
    }

    /// Pressing back the way you came puts you back — the one rule that makes a wrong turn
    /// cost nothing, and the reason an empty hand needs no button of its own.
    #[test]
    fn every_press_can_be_taken_back() {
        let chart = Chart::new();
        let places = std::iter::once(At::Hand).chain(Item::ALL.iter().copied().map(At::On));
        for from in places {
            for dir in Dir::ALL {
                let Some(to) = chart.step(from, dir) else {
                    continue;
                };
                assert_eq!(
                    chart.step(to, dir.opposite()),
                    Some(from),
                    "{from:?} pressed {dir:?} does not come back"
                );
            }
        }
    }

    /// Exits are axis-true: pressing up goes up the screen. A chart whose threads and
    /// whose d-pad disagreed would be a code nobody could read off the picture.
    #[test]
    fn a_press_moves_the_way_it_points() {
        let chart = Chart::new();
        let places = std::iter::once(At::Hand).chain(Item::ALL.iter().copied().map(At::On));
        for from in places {
            for dir in Dir::ALL {
                let Some(to) = chart.step(from, dir) else {
                    continue;
                };
                let step = chart.place(to) - chart.place(from);
                assert!(
                    step.dot(dir.unit()) > 0.9 * step.length(),
                    "{from:?} pressed {dir:?} moves {step:?}"
                );
            }
        }
    }

    /// A constellation you can read: no two stars sit on top of each other, and none of
    /// them sits on the hand.
    #[test]
    fn no_two_stars_land_in_the_same_place() {
        let chart = Chart::new();
        for a in Item::ALL {
            assert!(chart.star(*a).at.length() > 1.4, "{a:?} is on the hand");
            for b in Item::ALL {
                if a == b {
                    continue;
                }
                let gap = chart.star(*a).at.distance(chart.star(*b).at);
                assert!(gap > 1.4, "{a:?} and {b:?} are {gap} apart");
            }
        }
    }

    /// The codes a player learns are the codes they keep: laying out a shorter table gives
    /// every item in it the same code it has in the full one, so an item appended to the
    /// registry lands at the outside and moves nothing.
    #[test]
    fn adding_an_item_moves_nobody() {
        let whole = Chart::new();
        for n in 1..Item::COUNT {
            for star in layout(&Item::ALL[..n]) {
                assert_eq!(
                    star.code,
                    whole.code(star.item),
                    "{:?} moved when the table was {n} long",
                    star.item
                );
            }
        }
    }

    /// The four things one press from the hand, and two codes spelled out — a registry
    /// reorder is a wire-format break already, and this is the other half of what it would
    /// break: the motor patterns in the players' thumbs.
    #[test]
    fn the_codes_are_the_ones_that_have_been_learned() {
        let chart = Chart::new();
        assert_eq!(chart.code(Item::Grass), [Dir::Up]);
        assert_eq!(chart.code(Item::Dirt), [Dir::Right]);
        assert_eq!(chart.code(Item::Stone), [Dir::Down]);
        assert_eq!(chart.code(Item::Sand), [Dir::Left]);
        assert_eq!(chart.code(Item::Nail), [Dir::Right, Dir::Up]);
        assert_eq!(chart.code(Item::Car), [Dir::Left, Dir::Up]);
    }

    /// The lit path is the code: the stars you pressed through to get here, the thing
    /// itself, and nothing else. An empty hand has pressed nothing, so nothing is lit.
    #[test]
    fn the_trail_is_the_presses_that_got_you_there() {
        let chart = Chart::new();
        assert!(
            chart.on_the_way(Item::Sand, At::On(Item::Car)),
            "left, then"
        );
        assert!(chart.on_the_way(Item::Car, At::On(Item::Car)));
        assert!(!chart.on_the_way(Item::Grass, At::On(Item::Car)));
        for item in Item::ALL {
            assert!(!chart.on_the_way(*item, At::Hand));
        }
    }
}
