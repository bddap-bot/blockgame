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
use crate::forge::Stock;
use crate::inventory::{Held, Inventory};
use crate::registry::Item;
use crate::rig::{self, Dir, Kit, code, coded};

/// Where the belt is built, far above any world and far from [`crate::forge`]'s own
/// origin: two scenes, one set of coordinates each, and no render layer needed to keep
/// them off each other's cameras.
pub const ORIGIN: Vec3 = Vec3::new(0.0, 30_000.0, 0.0);

/// The hub every code leaves from: the middle of the spaceman's chest, in his own
/// coordinates. Both legs of every route start here, so the picture says "this hangs off
/// *you*".
const CHEST: Vec3 = Vec3::new(0.0, 1.06, 0.0);

/// How far a cluster hangs from the chest, and how far a thing hangs from its cluster.
/// The second is under a third of the first, so a cluster reads as one place at a glance
/// and falls into four when you are looking at it.
const HOOK: f32 = 2.55;
const FAN: f32 = 0.74;

/// How big a thing hangs: the one in your hand while the rig is shut, one in the cluster
/// you have opened, and one in a cluster you have not. The last is small on purpose — you
/// can still see what you own over there, and it is plainly not what the next press is
/// about.
const HELD_SCALE: f32 = 0.92;
const NEAR_SCALE: f32 = 0.82;
const FAR_SCALE: f32 = 0.38;
/// What is left of a thing's size when you own none of it: there, reachable, and visibly
/// not yours. Picking it is how you get to its recipe, so it is never hidden.
const GHOST: f32 = 0.52;

/// How far back the camera stands, shut and open. Shut is a figure standing low in the
/// screen with his kit in his hand; open is the whole constellation.
const SHUT_REACH: f32 = 5.2;
const OPEN_REACH: f32 = 8.6;
/// How far above the chest the shut camera aims, which is what puts the figure in the
/// bottom of the frame rather than the middle of it.
const SHUT_LIFT: f32 = 2.3;

/// Seconds the rig takes to bloom, and to fold back up.
const BLOOM: f32 = 0.18;

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

/// Everything the belt owns, so putting it away is one despawn.
#[derive(Component, Clone, Copy)]
pub struct Rig;

/// The camera the belt is seen through: over the world, under nothing.
#[derive(Component)]
pub struct Eye;

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

/// One leg of one route — the string and the two barbs of its arrowhead alike, so lighting
/// a leg is one query. `hook` is the direction of its first press; `second` is set on the
/// legs that run from a cluster out to a thing, which is exactly the difference between
/// "drawn while that cluster is open" and "drawn while that thing is in your hand".
#[derive(Component, Clone, Copy)]
pub struct Leg {
    hook: Dir,
    second: Option<Item>,
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
            brightness: 850.0,
            ..default()
        },
        Transform::from_translation(ORIGIN + CHEST + Vec3::new(0.0, 0.0, SHUT_REACH)),
    ));
    commands.spawn((
        Rig,
        PointLight {
            intensity: 3_000_000.0,
            range: 90.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(ORIGIN + Vec3::new(-4.0, 5.0, 9.0)),
    ));

    let body = avatar::spawn(&mut commands, &palette, Transform::from_translation(ORIGIN));
    commands.entity(body.root).insert(Rig);
    commands.insert_resource(Wearer(body));

    for hook in Dir::ALL {
        route(&mut commands, &kit, CHEST, hub(hook), hook, None);
    }
    for item in Item::ALL {
        let [hook, _] = code(*item);
        let at = station(*item);
        route(&mut commands, &kit, hub(hook), at, hook, Some(*item));

        let holder = commands
            .spawn((
                Rig,
                Station { item: *item, at },
                Transform::from_translation(ORIGIN + at),
                Visibility::Hidden,
            ))
            .id();
        let shape = rig::silhouette(
            &mut commands,
            &palette,
            *item,
            Vec3::ZERO,
            (Rig, Spin(at.x)),
        );
        commands.entity(holder).add_child(shape);
        // The same notch bar the recipe rig draws, run by the same system: how many you
        // own, as a height. It is what the number in the old hotbar cell was, and the one
        // place either surface states a count.
        for notch in rig::notch_bar(&mut commands, &kit, *item, Vec3::ZERO, Rig) {
            commands.entity(holder).add_child(notch);
        }
    }
}

/// One leg: the string, and the arrowhead sitting near the far end of it.
///
/// Built out of the same glowing string the recipe graph is wired with, because it is the
/// same claim — this leads to that — and a player who has been in the forge has already
/// been taught to follow one.
fn route(
    commands: &mut Commands,
    kit: &Kit,
    from: Vec3,
    to: Vec3,
    hook: Dir,
    second: Option<Item>,
) {
    let along = to - from;
    let leg = Leg { hook, second };
    let points = Dir::ALL
        .into_iter()
        .find(|d| d.towards().dot(along.normalize_or_zero()) > 0.9)
        .unwrap_or(hook);
    commands.spawn((
        Rig,
        leg,
        Mesh3d(kit.cube()),
        MeshMaterial3d(kit.string()),
        Transform::from_translation(ORIGIN + (from + to) / 2.0)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, along.normalize_or_zero()))
            .with_scale(Vec3::new(0.05, along.length(), 0.05)),
        Visibility::Hidden,
    ));
    rig::arrowhead(
        commands,
        kit,
        points,
        ORIGIN + to - along * 0.3,
        0.42,
        kit.string(),
        (Rig, leg),
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
        // The one thing on show while the rig is shut stays where it is; everything else
        // folds back into his chest, so opening the belt is the kit coming *off him*.
        let out = belt.open.max(mine as i32 as f32);
        at.translation = ORIGIN + CHEST + (station.at - CHEST) * out;
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
    held: Res<Held>,
    kit: Res<Kit>,
    mut legs: Query<(&Leg, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (leg, mut seen, mut paint) in &mut legs {
        let show = match (belt.armed, leg.second) {
            // Shut: only the two legs that reach what is in your hand, so the code for the
            // thing you are carrying is on the screen the whole time you carry it.
            (None, Some(item)) => item == held.0,
            (None, None) => code(held.0)[0] == leg.hook,
            // Open: all four first legs, and the second legs of the cluster you are in.
            (Some(_), None) => true,
            (Some(hook), Some(_)) => leg.hook == hook,
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
        let lit = belt.armed == Some(leg.hook) && leg.second.is_none();
        let want = if lit { kit.waiting() } else { kit.string() };
        if paint.0 != want {
            paint.0 = want;
        }
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
