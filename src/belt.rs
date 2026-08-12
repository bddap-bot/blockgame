//! The belt — what you are carrying, hanging off your own body, and the two-press d-pad
//! code that reaches it.
//!
//! The hotbar used to be a strip of named boxes with a number in each. This is the same
//! fact told as a *place*: your spaceman stands in the bottom of the screen holding what
//! is in your hand, and everything else you own hangs on him. Press one direction and he
//! blooms — four clusters, one per direction, each holding four things arranged those same
//! four ways. Press a second direction and that thing is in your hand.
//!
//! **The code is a route, and the route is drawn.** A string runs from his chest out to
//! each cluster with an arrowhead on it, and on from each cluster to each thing with
//! another. Nobody has to be told that a nail is "right, then left" — a nail is the thing
//! on the left of the cluster off his right shoulder, and the two arrows you would follow
//! to get there are lying on the rig pointing the way. You read it the first few times and
//! then your thumb knows it. Nothing on this surface is written and nothing on it is a
//! number: how many you own is the notch bar beside the thing, the same bar the forge
//! draws, lit by the same system.
//!
//! **Which code an item has comes from the registry** — see [`crate::rig::code`] — so a new
//! row in the item table arrives already reachable and already drawn.
//!
//! **The same two presses in both rooms.** The recipe rig in [`crate::forge`] is walked by
//! these codes too; there, a code puts the thing in the middle of the graph instead of in
//! your hand, and the arrows under every node are the same two arrows this rig hangs on its
//! strings. Choosing a thing and looking at what it is made of are one gesture aimed at
//! different rooms.

use bevy::prelude::*;

use crate::avatar::{self, Body, Palette};
use crate::inventory::{Held, Inventory};
use crate::registry::Item;
use crate::rig::{self, Dir, Kit, Stock, code, coded};

/// Where the belt is built, far above any world and far from [`crate::forge`]'s own
/// origin: two scenes, one set of coordinates each, and no render layer needed to keep
/// them off each other's cameras.
pub const ORIGIN: Vec3 = Vec3::new(0.0, 30_000.0, 0.0);

/// The hub every code leaves from: the middle of the spaceman's chest, in his own
/// coordinates. Both legs of every route start here, so the picture says "this hangs off
/// *you*".
const CHEST: Vec3 = Vec3::new(0.0, 1.06, 0.0);

/// How far a cluster hangs from the chest, and how far a thing hangs from its cluster.
/// The second is half the first, which is the whole of the layout: near enough that a
/// cluster reads as one place from across a room, far enough that the four things in it —
/// and the notch bar beside each — never touch when you are looking straight at them.
const HOOK: f32 = 2.75;
const FAN: f32 = 1.05;

/// How big a thing hangs: the one in your hand while the rig is shut, one in the cluster
/// you have opened, and one in a cluster you have not. The last is small on purpose — you
/// can still see what you own over there, and it is plainly not what the next press is
/// about.
const HELD_SCALE: f32 = 0.55;
const NEAR_SCALE: f32 = 0.78;
const FAR_SCALE: f32 = 0.38;
/// What is left of a thing's size when you own none of it: there, reachable, and visibly
/// not yours. Picking it is how you get to its recipe, so it is never hidden.
const GHOST: f32 = 0.52;

/// How far back the camera stands, shut and open. Shut is a figure with one thing held out
/// in front of him, small and low in the screen; open is the whole constellation.
const SHUT_REACH: f32 = 4.6;
const OPEN_REACH: f32 = 9.2;
/// How far above the chest the shut camera aims, which is what puts the figure into the
/// bottom of the frame rather than the middle of it.
const SHUT_LIFT: f32 = 0.55;

/// Where everything hangs while the rig is shut: out at the end of his right arm, which is
/// where the one thing still drawn ends up and where the other thirteen fold back into.
const SHUT_AT: Vec3 = Vec3::new(0.86, 1.02, 0.45);

/// Seconds the rig takes to bloom, and to fold back up.
const BLOOM: f32 = 0.18;

/// How big the arrowhead on a leg is drawn.
const ARROW: f32 = 0.42;

/// Where a thing hangs, in the spaceman's own coordinates: out to its cluster, then out
/// again to its place in it.
pub fn station(item: Item) -> Vec3 {
    let [hook, slot] = code(item);
    hub(hook) + slot.towards() * FAN
}

/// The middle of a cluster — where a code's first leg lands and its second leg leaves.
fn hub(hook: Dir) -> Vec3 {
    CHEST + hook.towards() * HOOK
}

/// Half-pressed, or not pressed: the whole of the belt's state between frames.
///
/// There is no timer on it. A child who has pressed one direction and is looking at four
/// things is *choosing*, and a window that expired under them would take the choice away
/// mid-thought. The bloom is the size of the screen, so being half-way through a code is
/// not hidden state anybody can be caught by.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct Belt {
    pub armed: Option<Dir>,
    /// How far open the rig is drawn, `0.0` shut and `1.0` bloomed. Eased rather than
    /// switched, so the thing grows out of the figure instead of replacing him.
    open: f32,
}

impl Belt {
    /// One direction, pressed. The first arms a cluster; the second names a thing, if the
    /// table has one there.
    pub fn press(&mut self, dir: Dir) -> Option<Item> {
        match self.armed {
            None => {
                self.armed = Some(dir);
                None
            }
            // An empty station keeps you where you were: the cluster stays open and the
            // press cost nothing.
            Some(hook) => match coded([hook, dir]) {
                None => None,
                found => {
                    self.armed = None;
                    found
                }
            },
        }
    }

    fn ease(&mut self, dt: f32) {
        let want = self.armed.is_some() as i32 as f32;
        self.open = (self.open + (want - self.open).clamp(-dt / BLOOM, dt / BLOOM)).clamp(0.0, 1.0);
    }
}

// ---------------------------------------------------------------------------
// The rig.
// ---------------------------------------------------------------------------

/// The camera the belt is seen through: over the world, under the recipe rig.
#[derive(Component)]
pub struct Eye;

/// The dark pane the bloomed rig is read against.
#[derive(Component)]
pub struct Pane;

/// One thing, hanging at its station. The parent that moves and hides; the silhouette and
/// the notch bar hang under it, so a cluster that folds away takes its counts with it.
#[derive(Component)]
pub struct Station {
    item: Item,
    at: Vec3,
}

/// The silhouette itself, which turns. Separate from [`Station`] because the notch bar
/// beside it must not: a bar of notches seen edge-on is a bar of nothing.
#[derive(Component)]
pub struct Spin(f32);

/// One leg of one route — carried by the string and by the two barbs of its arrowhead
/// alike, so lighting a leg is one query.
///
/// Which of the two presses a leg belongs to is the variant, and the direction it runs in
/// is the payload of that variant, so a leg cannot be built pointing somewhere its code
/// does not go. A slot leg carries the *first* press as well, because that is what decides
/// whether it is drawn at all — a leg that did not know its own cluster would have to look
/// the answer up.
#[derive(Component, Clone, Copy)]
pub enum Leg {
    /// Chest to cluster, running the way the first press points.
    Hook(Dir),
    /// Cluster to thing, running the way the second press points.
    Slot { hook: Dir, slot: Dir },
}

impl Leg {
    /// Which way the leg runs, and how far along it the arrowhead sits.
    ///
    /// A first leg wears its arrowhead near the *body* and a second leg near its *thing*,
    /// which is not a nicety: the left-hand station of a cluster lies on the line the first
    /// leg came in along, and an arrowhead at that end would be drawn on top of it.
    fn points(self) -> (Dir, f32) {
        match self {
            Leg::Hook(hook) => (hook, 0.4),
            Leg::Slot { slot, .. } => (slot, 0.68),
        }
    }
}

/// The spaceman himself, and the hand whose contents answer "what am I holding".
#[derive(Resource, Clone, Copy)]
pub struct Wearer(Body);

/// Builds the belt: a camera, a lamp, the figure, and every station and route around him.
///
/// Built once and shown or hidden after, rather than respawned as the state changes: it is
/// the same fourteen things all game, and geometry that comes and goes is geometry that
/// pops.
pub fn enter(mut commands: Commands, kit: Res<Kit>, palette: Res<Palette>) {
    commands.spawn((
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
            brightness: 850.0,
            ..default()
        },
        Transform::from_translation(ORIGIN + CHEST + Vec3::new(0.0, 0.0, SHUT_REACH)),
    ));
    commands.spawn((
        PointLight {
            intensity: 3_000_000.0,
            range: 90.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(ORIGIN + Vec3::new(-4.0, 5.0, 9.0)),
    ));

    // The dark pane the constellation is read against, and the reason the rig can be read
    // at all: fourteen coloured things hung over a sunlit hillside are fourteen things the
    // same colour as the hillside. Scaled from nothing rather than faded, so it irises open
    // with the bloom and takes no second material to animate an alpha through.
    commands.spawn((
        Pane,
        Mesh3d(kit.cube()),
        MeshMaterial3d(kit.backdrop()),
        Transform::from_translation(ORIGIN + CHEST + Vec3::new(0.0, 0.0, -3.0)),
    ));

    // Turned to face the camera: the model faces -Z and this rig is looked at from +Z, so
    // an unturned figure is a picture of a backpack.
    let body = avatar::spawn(
        &mut commands,
        &palette,
        Transform::from_translation(ORIGIN)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
    );
    commands.insert_resource(Wearer(body));

    for hook in Dir::ALL {
        route(&mut commands, &kit, CHEST, Leg::Hook(hook), HOOK);
    }
    for item in Item::ALL {
        let [hook, slot] = code(*item);
        let at = station(*item);
        route(
            &mut commands,
            &kit,
            hub(hook),
            Leg::Slot { hook, slot },
            FAN,
        );

        let holder = commands
            .spawn((
                Station { item: *item, at },
                Transform::from_translation(ORIGIN + at),
                Visibility::Hidden,
            ))
            .id();
        let shape = rig::silhouette(&mut commands, &palette, *item, Vec3::ZERO, Spin(at.x));
        commands.entity(holder).add_child(shape);
        // The same notch bar the recipe rig draws, run by the same system: how many you
        // own, as a height. It is what the number in the old hotbar cell was, and the one
        // place either surface states a count.
        for notch in rig::notch_bar(&mut commands, &kit, *item, Vec3::ZERO, ()) {
            commands.entity(holder).add_child(notch);
        }
    }
}

/// One leg: the string, and the arrowhead on it.
///
/// Built out of the same glowing string the recipe graph is wired with, because it is the
/// same claim — this leads to that — and a player who has been in the forge has already
/// been taught to follow one.
///
/// The leg says which way it runs and the caller says how far, so the string and the arrow
/// on it cannot disagree with the code that put them there — and there is no direction to
/// recover from the geometry afterwards.
fn route(commands: &mut Commands, kit: &Kit, from: Vec3, leg: Leg, reach: f32) {
    let (points, head) = leg.points();
    let along = points.towards() * reach;
    commands.spawn((
        leg,
        Mesh3d(kit.cube()),
        MeshMaterial3d(kit.string()),
        Transform::from_translation(ORIGIN + from + along / 2.0)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, points.towards()))
            .with_scale(Vec3::new(0.05, reach, 0.05)),
        Visibility::Hidden,
    ));
    rig::arrowhead(
        commands,
        kit,
        points,
        ORIGIN + from + along * head,
        ARROW,
        kit.string(),
        leg,
    );
}

/// The pad's two presses, and what they do to what is in your hand.
///
/// One function rather than a system, because the world and the recipe rig hand it the
/// same press from two different reads of the pad, and there is one answer to what a code
/// means.
pub fn press(dir: Option<Dir>, belt: &mut Belt, held: &mut Held) -> Option<Item> {
    let picked = dir.and_then(|d| belt.press(d))?;
    held.0 = picked;
    Some(picked)
}

/// Opens and shuts the rig, and puts whatever is in hand into the figure's hand.
#[allow(clippy::too_many_arguments)]
pub fn dress(
    time: Res<Time>,
    stock: Res<Stock>,
    held: Res<Held>,
    wearer: Option<Res<Wearer>>,
    palette: Res<Palette>,
    mut commands: Commands,
    mut belt: ResMut<Belt>,
    mut shown: Local<Option<Option<Item>>>,
) {
    belt.ease(time.delta_secs());
    let Some(wearer) = wearer else {
        return;
    };
    // Empty-handed until you own one: the hand is what you are *carrying*, and a station
    // you have merely pointed at is not in it.
    let holding = stock.0.in_hand(held.0);
    if *shown != Some(holding) {
        avatar::show_held(&mut commands, &palette, wearer.0, holding);
        *shown = Some(holding);
    }
}

/// Every station: shown or hidden, near or far, folded into the chest or out at its place.
pub fn stations(
    belt: Res<Belt>,
    held: Res<Held>,
    stock: Res<Stock>,
    mut stations: Query<(&Station, &mut Transform, &mut Visibility)>,
) {
    for (station, mut at, mut seen) in &mut stations {
        let mine = station.item == held.0;
        // Shut, the only thing hanging on him is the thing he is holding — so the picture
        // at rest is a man with his kit, and the route to it is still drawn beside him.
        let vis = if belt.armed.is_some() || mine {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *seen != vis {
            *seen = vis;
        }
        let near = belt.armed == Some(code(station.item)[0]);
        let size = match (belt.armed.is_some(), near) {
            (false, _) => HELD_SCALE,
            (true, true) => NEAR_SCALE,
            (true, false) => FAR_SCALE,
        };
        let owned = if stock.0.count(station.item) > 0 {
            1.0
        } else {
            GHOST
        };
        at.scale = Vec3::splat(size * owned);
        // Shut, everything is folded into the one place a thing can be while you are
        // carrying it: out at the end of the arm holding it. Opening the belt is that fold
        // coming apart — the kit coming *off him* — and shutting it is the same in reverse.
        at.translation = ORIGIN + SHUT_AT.lerp(station.at, belt.open);
    }
}

/// The silhouettes, turning. Not decoration: every model in this game is a table of boxes,
/// and a table of boxes seen dead-on is a flat coloured patch. The rig is looked at
/// dead-on so its four directions stay the pad's four directions, which leaves turning as
/// the only thing that makes a rifle read as a rifle from a sofa.
pub fn spin(time: Res<Time>, mut shapes: Query<(&Spin, &mut Transform)>) {
    let clock = time.elapsed_secs();
    for (spin, mut at) in &mut shapes {
        at.rotation = Quat::from_rotation_y(clock * 0.5 + spin.0);
    }
}

/// Every leg of every route: which are drawn, and which one is lit.
pub fn legs(
    belt: Res<Belt>,
    kit: Res<Kit>,
    mut legs: Query<(&Leg, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (leg, mut seen, mut paint) in &mut legs {
        // A route is a picture of where a thing hangs, so it is drawn only while things are
        // hanging: shut, they are all folded into his chest and there is nowhere for a
        // string to run. Every selection blooms the whole map again, which is what makes the
        // codes learnable without anybody having to remember one.
        let show = match (belt.armed, *leg) {
            (None, _) => false,
            // Open: all four first legs, and the second legs of the cluster you are in.
            (Some(_), Leg::Hook(_)) => true,
            (Some(armed), Leg::Slot { hook, .. }) => hook == armed,
        };
        let vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *seen != vis {
            *seen = vis;
        }
        // The leg you have already pressed burns amber — one press banked, drawn as the
        // thing you are standing on.
        let lit = matches!(*leg, Leg::Hook(hook) if belt.armed == Some(hook));
        let want = if lit { kit.waiting() } else { kit.string() };
        if paint.0 != want {
            paint.0 = want;
        }
    }
}

/// The pane, irising open with the rig and gone entirely while it is shut — a dark
/// rectangle hanging over the world with one spaceman on it is a thing between the player
/// and the game.
pub fn pane(belt: Res<Belt>, mut pane: Query<&mut Transform, With<Pane>>) {
    for mut at in &mut pane {
        let wide = belt.open * OPEN_REACH * 3.4;
        at.scale = Vec3::new(wide, wide, 0.1);
    }
}

/// The camera: back far enough to hold whatever is currently unfolded, and aimed above the
/// figure while the rig is shut so that he stands in the bottom of the screen, where a
/// hotbar goes.
pub fn eye(belt: Res<Belt>, mut eye: Query<&mut Transform, With<Eye>>) {
    let Ok(mut at) = eye.single_mut() else {
        return;
    };
    let open = belt.open;
    let reach = SHUT_REACH + (OPEN_REACH - SHUT_REACH) * open;
    let aim = ORIGIN + CHEST + Vec3::Y * SHUT_LIFT * (1.0 - open);
    at.translation = ORIGIN + CHEST + Vec3::new(0.0, 0.0, reach);
    at.rotation = Transform::from_translation(at.translation)
        .looking_at(aim, Vec3::Y)
        .rotation;
}

/// The two doors of the recipe rig, from this side.
///
/// The belt stops being drawn while the other room is up and starts again when it folds
/// away — switched rather than despawned, because it is a fixed set of geometry that lives
/// as long as the world does and rebuilding it on every visit would be a rig that pops back
/// into existence a frame late.
///
/// Both drop a half-spelled code. A code is two presses and both of them belong to the
/// surface that was on screen when they were made; carrying half of one through a door
/// means the first press *inside* the new room finishes something nobody aimed at — arm a
/// cluster in the world, press craft, and the graph re-centres on whatever that stale press
/// and your next one happen to spell between them.
pub fn shut(belt: ResMut<Belt>, cameras: Query<&mut Camera, With<Eye>>) {
    at_the_door(belt, cameras, false);
}

pub fn open(belt: ResMut<Belt>, cameras: Query<&mut Camera, With<Eye>>) {
    at_the_door(belt, cameras, true);
}

fn at_the_door(mut belt: ResMut<Belt>, mut cameras: Query<&mut Camera, With<Eye>>, drawn: bool) {
    belt.armed = None;
    for mut camera in &mut cameras {
        if camera.is_active != drawn {
            camera.is_active = drawn;
        }
    }
}

/// Hands both rigs the pile they draw. One [`Stock`] between them, because "what I have"
/// is one fact and two copies of it would eventually be two answers.
pub fn wear_the_stock(mine: &Inventory, stock: &mut Stock) {
    if stock.0 != *mine {
        stock.0 = mine.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pressing the code is what selects the item — the whole contract, checked against the
    /// same [`code`] the picture is drawn from.
    #[test]
    fn pressing_a_code_picks_the_thing_it_names() {
        for item in Item::ALL {
            let [first, second] = code(*item);
            let mut belt = Belt::default();
            let mut held = Held(Item::Grass);
            assert_eq!(press(Some(first), &mut belt, &mut held), None);
            assert_eq!(belt.armed, Some(first), "one press arms the cluster");
            assert_eq!(press(Some(second), &mut belt, &mut held), Some(*item));
            assert_eq!(held.0, *item);
            assert_eq!(belt.armed, None, "and the rig folds back up");
        }
    }

    /// A press into one of the two stations the item table has not reached yet leaves the
    /// cluster open, so a child who guessed is still where they were and can press again —
    /// not dropped back to the start, and never handed the nearest thing instead.
    #[test]
    fn an_empty_station_costs_nothing() {
        let mut belt = Belt::default();
        let mut held = Held(Item::Grass);
        assert_eq!(press(Some(Dir::Left), &mut belt, &mut held), None);
        assert_eq!(press(Some(Dir::Down), &mut belt, &mut held), None);
        assert_eq!(belt.armed, Some(Dir::Left), "the cluster stays open");
        assert_eq!(held.0, Item::Grass, "and nothing changed hands");
        assert_eq!(
            press(Some(Dir::Right), &mut belt, &mut held),
            Some(Item::Car)
        );
    }

    /// Nothing hangs on top of anything else, and nothing hangs inside the man. The rig is
    /// read at a glance from a sofa, so two things in one place is two things nobody can
    /// tell apart.
    #[test]
    fn no_two_things_hang_in_the_same_place() {
        for a in Item::ALL {
            assert!(
                station(*a).distance(CHEST) > 1.4,
                "{a:?} hangs inside the body"
            );
            for b in Item::ALL {
                if a != b {
                    assert!(
                        station(*a).distance(station(*b)) > 0.9,
                        "{a:?} and {b:?} overlap"
                    );
                }
            }
        }
    }

    /// A cluster is one direction from the chest and its things are one direction from it,
    /// which is the claim the arrows lying on the rig make. If the geometry stopped
    /// agreeing with [`code`], the picture would be teaching a code that does not work.
    #[test]
    fn the_picture_is_the_code() {
        for item in Item::ALL {
            let [hook, slot] = code(*item);
            let out = hub(hook) - CHEST;
            assert!(out.normalize().dot(hook.towards()) > 0.99, "{item:?} hook");
            let fan = station(*item) - hub(hook);
            assert!(fan.normalize().dot(slot.towards()) > 0.99, "{item:?} slot");
        }
    }

    /// The bloom is a fold, not a cut: it takes about [`BLOOM`] to open and the same to
    /// shut, and it never leaves the range the station positions are lerped through.
    #[test]
    fn the_rig_opens_and_shuts_within_its_range() {
        let mut belt = Belt {
            armed: Some(Dir::Up),
            ..Belt::default()
        };
        for _ in 0..60 {
            belt.ease(1.0 / 60.0);
            assert!((0.0..=1.0).contains(&belt.open));
        }
        assert_eq!(belt.open, 1.0, "a held cluster is fully open in a second");
        belt.armed = None;
        for _ in 0..60 {
            belt.ease(1.0 / 60.0);
            assert!((0.0..=1.0).contains(&belt.open));
        }
        assert_eq!(belt.open, 0.0);
    }
}
