//! The Bevy app: one set of systems that runs identically whether you are hosting or
//! joining.
//!
//! [`Role`] is consulted only where authority differs: [`authorized`] (whose word counts),
//! [`submit_edit`] and [`open_forge`] (who may change the world and who may spend a
//! pile), [`net_receive`] (who relays), and [`run`] (which of the two the player is told
//! they are). Everything else is role-blind, which is what "single player is multiplayer
//! with zero peers" has to mean if it is going to stay true.

use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, MonitorSelection, PrimaryWindow, WindowMode};
use std::collections::{HashMap, HashSet};

use crate::avatar;
use crate::belt::{self, Belt};
use crate::forge;
use crate::hud;
use crate::input::{
    Intent, MenuIntent, PITCH_LIMIT, gather_forge_nav, gather_intent, gather_menu_intent,
};
use crate::inventory::{Held, Inventories, Inventory};
use crate::mesh::{ChunkMesh, build_chunk_mesh};
use crate::net::wire::CarPose;
use crate::net::{Boot, Event, Msg, PlayerId, Pose, Role, Session, Target, lan};
use crate::pause;
use crate::player::{self, Player};
use crate::raycast;
use crate::registry::{Block, Class, Item, Use};
use crate::rig;
use crate::ticket::Share;
use crate::title::{self, Booted, Phase, Playing, Start};
use crate::vehicle::{self, Ride};
use crate::world::{BlockPos, CHUNK_SIZE, ChunkPos, World};

/// Chunks of terrain visible in every direction.
const VIEW_RADIUS: i32 = 8;
/// Chunks whose voxels are generated per frame. Spreading the work keeps the first
/// seconds interactive instead of freezing on a wall of worldgen.
const LOAD_BUDGET: usize = 6;
/// Chunk meshes built per frame — the expensive half.
const MESH_BUDGET: usize = 3;
/// Chunks are dropped further out than they are meshed, so a meshed chunk always has its
/// neighbours loaded to cull its seams against.
const UNLOAD_RADIUS: i32 = VIEW_RADIUS + 2;
const _: () = assert!(UNLOAD_RADIUS > VIEW_RADIUS);
/// Slack the host allows on a player's reach when checking their edit. The host checks
/// against the pose that peer last sent, which is up to a round trip old; without the
/// slack a peer editing while sprinting would have legitimate edits refused.
const REACH_LAG: f32 = 1.5;

/// The camera's field of view, unscoped. A [`Use::zoom`] divides it.
const FOV: f32 = 75.0 * std::f32::consts::PI / 180.0;

const SKY: Color = Color::srgb(0.52, 0.72, 0.95);

/// The player's own camera: their head, and the only camera that is a *view of the world*.
///
/// A marker rather than "the one `Camera3d`", because it stopped being the one: the belt
/// keeps a camera over the world for as long as the world lasts, and the recipe rig adds a
/// third while it is up. A `single_mut` over `With<Camera3d>` quietly returns an error the
/// moment there are two — a first-person camera that never moves off the origin, every
/// frame, silently.
#[derive(Component)]
struct Eye;

#[derive(Resource)]
struct Sim(World);

#[derive(Resource)]
pub struct Me(pub Player);

#[derive(Resource, Clone, Copy)]
pub struct NetRole(pub Role);

/// Chunk entities by position. An entry with no `Mesh3d` is a chunk that meshed to
/// nothing (all air) — still tracked, so it isn't re-meshed every frame.
#[derive(Resource, Default)]
struct Chunks {
    entities: HashMap<ChunkPos, Entity>,
    dirty: HashSet<ChunkPos>,
}

#[derive(Resource)]
struct WorldMaterial(Handle<StandardMaterial>);

/// Everyone else in this world, exactly as the host last declared them — see
/// [`Msg::Present`]. Their position is game state, not just something to draw: the host
/// checks each peer's edits against where that peer last said it was.
#[derive(Resource, Default)]
pub struct Peers(pub HashMap<PlayerId, PeerState>);

pub struct PeerState {
    /// Where they are and what they look like, or `None` for somebody who has not posed
    /// yet. Presence and position are separate facts because they arrive separately and
    /// only one of them is ordered: a player exists from the moment the host says so, and
    /// there is nothing to draw or to check an edit against until they say where they are.
    seen: Option<Seen>,
    /// What this player is still allowed to change.
    budget: Budget,
    /// How far this player may still claim to have moved. A pose is a claim, and every
    /// rule about *where* a peer may edit is checked against it, so a client that simply
    /// says it is standing across the map would otherwise reach across the map.
    travel: Budget,
}

/// Everything a player's poses say about them, which exists once they have sent one.
struct Seen {
    /// Feet position from this player's most recent believed [`Msg::Pose`].
    pos: Vec3,
    /// Their car, if they have one out — where it is and which way it points. Just the
    /// drawing: nothing about this player's own rules is checked against it, because a car
    /// only ever moves the driver's feet, and *those* are budgeted.
    car: Option<CarPose>,
    /// The model drawing them.
    body: avatar::Body,
    /// What their model is currently shown holding. Kept so a pose that says the same
    /// thing again — sixty a second of them — costs nothing.
    held: Option<Item>,
}

/// Edits one peer may ask for per second, sustained.
const EDIT_RATE: f32 = 10.0;
/// Edits a peer may ask for at once. A person breaking blocks does one per click, so this
/// is slack for a laggy burst arriving together, not a play style.
const EDIT_BURST: f32 = 40.0;
/// Edits per second across *everyone*. Identities are free — a peer can reconnect, or
/// arrive under a new key, and be handed a fresh per-peer bucket — so the world's growth
/// has to be bounded by something no identity resets.
const WORLD_EDIT_RATE: f32 = 40.0;
const WORLD_EDIT_BURST: f32 = 200.0;
// Four busy players' worth: past that the bound has to come from somewhere no identity
// resets, and it does.
const _: () = assert!(WORLD_EDIT_RATE < EDIT_RATE * 8.0);
/// Blocks per second a peer may claim to have travelled, and the jump one pose may make.
/// Well over [`player::FLY_SPEED`], because a pose lost on the wire is not resent: the
/// next one has to cover the gap.
const TRAVEL_RATE: f32 = player::FLY_SPEED * 3.0;
const TRAVEL_BURST: f32 = 64.0;
// A driver's feet are in the car, so a car quicker than this allowance would have its own
// driver refused as a teleporter: their pose would be dropped and nobody would see them
// move at all. `driving_stays_inside_the_travel_budget` walks the arithmetic.
const _: () = assert!(vehicle::TOP_SPEED < TRAVEL_RATE);

/// An allowance that refills with time: a token bucket.
///
/// Two things a peer must not have for free: edits, which are permanent world state the
/// host stores and ships to every future joiner, and movement, which is what every
/// proximity rule is checked against.
struct Budget {
    tokens: f32,
    rate: f32,
    max: f32,
}

impl Budget {
    fn new(rate: f32, max: f32) -> Self {
        Budget {
            tokens: max,
            rate,
            max,
        }
    }

    fn refill(&mut self, dt: f32) {
        self.tokens = (self.tokens + self.rate * dt).min(self.max);
    }

    /// Takes `cost`, or reports that this peer is asking for too much too fast.
    fn spend(&mut self, cost: f32) -> bool {
        if self.tokens < cost {
            return false;
        }
        self.tokens -= cost;
        true
    }
}

/// The world's own edit allowance, which no reconnect and no new identity resets.
#[derive(Resource)]
struct WorldBudget(Budget);

impl Default for WorldBudget {
    fn default() -> Self {
        WorldBudget(Budget::new(WORLD_EDIT_RATE, WORLD_EDIT_BURST))
    }
}

/// Peers already handed the world. `Msg::Hello` costs the host a copy of every edit and a
/// frame per edited chunk, so it is answered once per visit, not once per ask.
#[derive(Resource, Default)]
struct Welcomed(HashSet<PlayerId>);

/// The block the local player is part-way through breaking.
///
/// Local, per-frame, and never sent: this is an input accumulator, like a held movement
/// key, not world state. What it eventually *produces* — one edit — goes through the same
/// host check as every other edit, and the host's [`EDIT_RATE`] is what bounds a client
/// that skips the waiting. So a faster tool is a nicer game, never a bigger permission.
#[derive(Resource, Default)]
struct Digging(Option<Dig>);

#[derive(Clone, Copy, PartialEq, Debug)]
struct Dig {
    block: BlockPos,
    /// Zero to one. At one the block goes.
    done: f32,
}

/// A moment more of chewing at `block`.
///
/// Progress belongs to *one* block: aiming somewhere else, or letting go, starts the next
/// one from nothing. Without that a player chips away at soft ground and finishes the wall
/// behind it in a frame.
fn chew(previous: Option<Dig>, block: BlockPos, using: Use, dt: f32) -> Dig {
    let done = match previous {
        Some(d) if d.block == block => d.done,
        _ => 0.0,
    };
    Dig {
        block,
        done: done + using.speed * dt,
    }
}

/// Every car currently drawn, by whose it is — the local player's under their own id, so
/// there is one path from "somebody has a car out" to "a car is on the screen".
#[derive(Resource, Default)]
struct Cars(HashMap<PlayerId, Entity>);

#[derive(Component)]
struct ChunkTag;

#[derive(Component)]
struct Highlight;

/// Builds and runs the game. Returns when the window closes.
///
/// The app starts on the title screen with no world and no network: which world this is
/// is a choice the player has not made yet, and a ticket on the command line is that
/// same choice made early. [`Phase`] is what keeps the world's systems away from the
/// frames where there is no world.
pub fn run(start: Start) -> anyhow::Result<()> {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "blockgame".into(),
            // Borderless fullscreen at whatever the screen really is, never a fixed size:
            // a 1280x800 window is native on the Deck's panel but a quarter-resolution
            // image on the TV, which gamescope then stretches to 3840x2160. The HUD is
            // laid out for 800 rows and scaled back up by [`hud::scale_to_screen`], so
            // the world gets the pixels and the hotbar keeps its size.
            mode: WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(SKY))
    .insert_resource(start)
    .init_state::<Phase>()
    .add_sub_state::<Playing>()
    .init_resource::<Intent>()
    .init_resource::<MenuIntent>()
    .init_resource::<forge::Nav>()
    .init_resource::<rig::Stock>()
    .init_resource::<forge::CraftRequests>()
    .init_resource::<Belt>()
    .init_resource::<Held>()
    .init_resource::<Chunks>()
    .init_resource::<Digging>()
    .init_resource::<Peers>()
    .init_resource::<Cars>()
    .init_resource::<Inventories>()
    .init_resource::<Welcomed>()
    .init_resource::<WorldBudget>()
    .init_resource::<hud::Notice>()
    .insert_non_send_resource(Booted::default())
    .insert_non_send_resource(Share::default())
    // Menus read the pad every frame, whichever one is up and whether one is up at all:
    // a menu that only reads input while it is open cannot see the press that opened it.
    //
    // `after(InputSystems)` is load-bearing. `PreUpdate` is also where bevy refreshes
    // `ButtonInput`, and unordered this ran *first* — reading each frame's presses one
    // frame late, so the Escape that opened the pause menu was still "just pressed" on
    // the frame the menu came up and closed it again within it. The menu was
    // unreachable and nothing was on fire.
    .add_systems(
        PreUpdate,
        (gather_menu_intent, gather_forge_nav).after(InputSystems),
    )
    // Every screen the game draws is sized for the Deck's 800 rows, the title included, so
    // this runs whatever phase we are in and not just once: a TV that changes mode under
    // us is a resize like any other.
    .add_systems(Update, hud::scale_to_screen)
    .add_systems(OnEnter(Phase::Title), title::enter)
    .add_systems(OnExit(Phase::Title), title::leave)
    .add_systems(
        Update,
        (
            title::look_for_games,
            title::choose,
            title::wait_for_the_world,
            title::redraw,
        )
            .chain()
            .run_if(in_state(Phase::Title)),
    )
    .add_systems(
        OnEnter(Phase::World),
        (enter_world, rig::setup, setup, hud::setup, belt::enter).chain(),
    )
    .add_systems(OnEnter(Playing::Paused), pause::open)
    .add_systems(OnExit(Playing::Paused), pause::close)
    .add_systems(
        Update,
        (pause::drive, pause::redraw)
            .chain()
            .run_if(in_state(Playing::Paused)),
    )
    .add_systems(
        OnEnter(Playing::Forge),
        (forge::enter, belt::shut, let_go_of_the_mouse),
    )
    .add_systems(OnExit(Playing::Forge), (undress_the_forge, belt::open))
    .add_systems(
        // The same order every frame the rig is up: read the pad, take the pile as it
        // stands, act on it, build what the graph now needs, then move what moves.
        Update,
        (
            dress_the_forge,
            forge::drive,
            submit_forge_crafts,
            forge::rebuild,
            forge::react,
            forge::beads,
            rig::notches,
            forge::nodes,
            forge::cursor,
            forge::flight,
            forge::eye,
            close_forge,
        )
            .chain()
            .run_if(in_state(Playing::Forge)),
    )
    // The belt is drawn whatever the player is doing in the world — walking, flying,
    // driving — because what is in your hand is a thing you keep looking at. It stops at
    // the door of the recipe rig, which is a room of its own.
    .add_systems(
        Update,
        (
            wear_the_belt,
            belt::dress,
            belt::stations,
            belt::spin,
            belt::legs,
            belt::pane,
            rig::notches,
            belt::eye,
        )
            .chain()
            .run_if(in_state(Playing::Live).or(in_state(Playing::Paused))),
    )
    .add_systems(
        Update,
        (
            // Gameplay input alone stops while another room is up — the pause menu, the
            // recipe rig. The world keeps running behind them: a host that froze everybody
            // else's game by reading a menu would be a worse thing than an unpaused world.
            //
            // The *stale* read has to stop with it, which is what the second half is for.
            // `Intent` is a frame's worth of edges, and a resource nobody rewrote is last
            // frame's edges asserted again: a walk that carries on through the pause menu,
            // and — since the d-pad became a code — a direction re-pressed sixty times a
            // second, spelling things nobody aimed at.
            gather_intent.run_if(in_state(Playing::Live)),
            hush_intent.run_if(not(in_state(Playing::Live))),
            mind_the_body,
            park_or_ride,
            apply_intent,
            net_receive,
            answer_join_questions,
            draw_cars,
            target_and_edit,
            aim_zoom,
            open_forge.run_if(in_state(Playing::Live)),
            stream_chunks,
            remesh_dirty,
            net_send_pose,
            hud::fade_notice,
            open_pause_menu.run_if(in_state(Playing::Live)),
        )
            .chain()
            .run_if(in_state(Phase::World)),
    );

    // A bevy error exit is the process's error exit: swallowing it would report success to
    // whatever launched the game while the window died of something.
    match app.run() {
        AppExit::Success => Ok(()),
        AppExit::Error(code) => Err(anyhow::anyhow!("the game exited with code {code}")),
    }
}

/// Turns the network the title screen booted into a world. Runs once, on the way in.
fn enter_world(mut commands: Commands, mut booted: NonSendMut<Booted>) {
    let Boot {
        session,
        role,
        seed,
        edits,
        beacon,
    } = booted
        .0
        .take()
        .expect("the world was entered without a network to run it on");

    let world = World::new(seed, edits);
    let spawn = player::spawn_point(&world, session.me());

    if role == Role::Host {
        println!(
            "hosting — a friend joins with:\n\n    blockgame join {}\n",
            session.ticket()
        );
    }

    commands.insert_resource(Sim(world));
    commands.insert_resource(Me(Player::spawn_at(spawn)));
    commands.insert_resource(NetRole(role));
    if let Some(beacon) = beacon {
        commands.insert_resource(Beacon(beacon));
    }
    commands.queue(move |world: &mut bevy::ecs::world::World| {
        world.insert_non_send_resource(session);
    });
}

/// This host's answer to "who is hosting?" on the LAN. Absent on a peer, and on a host
/// whose discovery port was taken.
#[derive(Resource)]
struct Beacon(lan::Beacon);

/// Tells anybody at a join menu on this network where to find this world. A frame's
/// worth of non-blocking socket reads, and nothing at all when nobody is asking.
fn answer_join_questions(beacon: Option<Res<Beacon>>, session: NonSend<Session>) {
    if let Some(beacon) = beacon {
        beacon.0.answer(&session.addr());
    }
}

fn open_pause_menu(intent: Res<Intent>, mut playing: ResMut<NextState<Playing>>) {
    if intent.pause {
        playing.set(Playing::Paused);
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // One material for the whole world: block colour lives in the mesh's vertex colours,
    // so a new block type needs no asset work at all.
    let world_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.insert_resource(WorldMaterial(world_material));

    commands.spawn((
        // The eye, and the one camera in the game that is *the player looking at the world*.
        // Marked, because it is no longer the only `Camera3d` alive: the belt hangs one over
        // the world for as long as the world lasts, and a bare `With<Camera3d>` query for
        // "the camera" silently matches nothing once there are two of them.
        Eye,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV,
            far: 1200.0,
            ..default()
        }),
        // Ambient light is per-camera in bevy 0.18; without it every face the sun doesn't
        // reach goes pitch black.
        AmbientLight {
            color: Color::WHITE,
            brightness: 420.0,
            ..default()
        },
        Transform::default(),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            // Shadows over a voxel world cost more than they add at this scale, and the
            // mesher already shades faces by orientation.
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(50.0, 100.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Highlight,
        Mesh3d(meshes.add(Cuboid::new(1.02, 1.02, 1.02))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.30),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));

    grab_mouse(&mut cursor, true);
}

#[allow(clippy::too_many_arguments)]
fn apply_intent(
    intent: Res<Intent>,
    time: Res<Time>,
    sim: Res<Sim>,
    inventories: Res<Inventories>,
    session: NonSend<Session>,
    mut me: ResMut<Me>,
    mut belt: ResMut<Belt>,
    mut held: ResMut<Held>,
    mut camera: Query<&mut Transform, With<Eye>>,
) {
    // A long frame (window drag, a chunk-mesh hitch) must not turn into a teleport
    // through the floor.
    let dt = time.delta_secs().min(0.05);
    let p = &mut me.0;

    p.pitch = (p.pitch + intent.look.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    // Getting better happens whatever they are doing, and above the driving branch on
    // purpose: a winded child sitting in their car would otherwise never get up.
    p.condition.recover(dt);

    // Driving is the whole of movement while you are in a car: the car is what moves, the
    // seat is where you are, and steering is what turns you — so the camera follows the car
    // without a second camera to follow it with. Look is still free in pitch, which is what
    // lets a passing hill be shot at from the driver's seat with no extra rule.
    if let Ride::Driving(car) = p.ride {
        let driven = vehicle::drive(&sim.0, car, intent.walk, dt);
        p.yaw = driven.yaw;
        p.pos = driven.seat();
        p.ride = Ride::Driving(driven);
        belt::press(intent.dir, &mut belt, &mut held);
        aim_camera(p, &mut camera);
        return;
    }

    p.yaw += intent.look.x;

    // How the thing in hand falls: a parachute is whatever code you last pressed, exactly
    // as a drill is, so opening the canopy is the d-pad and needs no button of its own.
    let fall = inventories.of(session.me()).falling(held.0);
    player::advance(&sim.0, p, &intent, fall, dt);

    belt::press(intent.dir, &mut belt, &mut held);
    aim_camera(p, &mut camera);
}

/// Nothing at all, which is what a room other than the world is allowed to ask of the
/// player's body. One writer for [`Intent`] either way, so there is never a frame whose
/// intent is some older frame's.
fn hush_intent(mut intent: ResMut<Intent>) {
    *intent = Intent::default();
}

/// What a body flat on its back can still ask for.
///
/// One place, above everything that acts on an [`Intent`], so being winded gates driving,
/// digging, placing, crafting and climbing into a car for free — rather than each of those
/// systems having to remember a check, and one of them forgetting. Looking round and
/// pausing survive: a child who cannot see out, or cannot reach the menu that quits, is
/// stuck.
fn mind_the_body(me: Res<Me>, mut intent: ResMut<Intent>) {
    if !me.0.condition.on_their_feet() {
        *intent = Intent {
            look: intent.look,
            pause: intent.pause,
            ..Intent::default()
        };
    }
}

/// Puts the camera in the player's head. The one place it is moved, so a driver's view and
/// a walker's are the same view of the same eye — a car needs no camera of its own.
fn aim_camera(p: &Player, camera: &mut Query<&mut Transform, With<Eye>>) {
    if let Ok(mut t) = camera.single_mut() {
        t.translation = p.eye();
        t.rotation = Quat::from_euler(EulerRot::YXZ, p.yaw, p.pitch, 0.0);
    }
}

/// The car button, and the place button when a vehicle is in hand.
///
/// Putting a car down and picking it back up is the *place* button, which is what puts
/// whatever is in your hand into the world — which one it does comes off the class in the
/// registry, not a car-shaped branch. Getting in and out is its own button, because
/// standing next to your parked car and putting a *second* one down is not what anybody
/// means by pressing it.
///
/// A car is never spent. The [`Item::Car`] in your pocket is the title to it: you can have
/// one out because you built one, and pocketing it is how you get the field back.
fn park_or_ride(
    intent: Res<Intent>,
    held: Res<Held>,
    inventories: Res<Inventories>,
    session: NonSend<Session>,
    sim: Res<Sim>,
    mut me: ResMut<Me>,
) {
    let p = &mut me.0;
    let holding_a_vehicle = matches!(
        inventories
            .of(session.me())
            .in_hand(held.0)
            .map(Item::class),
        Some(Class::Vehicle { .. })
    );
    if intent.place_block && holding_a_vehicle {
        p.ride = match p.ride {
            Ride::Pocketed => match vehicle::park_in_front(&sim.0, p.pos, p.yaw) {
                Some(car) => Ride::Parked(car),
                // Nowhere generated to stand it on. Leave it in the pocket rather than drop
                // it into a chunk that has not arrived.
                None => Ride::Pocketed,
            },
            Ride::Parked(_) => Ride::Pocketed,
            // You cannot pocket the car you are sitting in.
            Ride::Driving(car) => Ride::Driving(car),
        };
    }

    if intent.ride {
        let (ride, feet) = vehicle::toggle_ride(p.ride, p.pos);
        if ride != p.ride {
            p.ride = ride;
            p.pos = feet;
            p.stand();
        }
    }
}

/// Draws every car anybody has out — the local player's and each peer's, from the one map,
/// so what you see of your own car and what you see of theirs is the same code.
fn draw_cars(
    me: Res<Me>,
    session: NonSend<Session>,
    peers: Res<Peers>,
    palette: Res<avatar::Palette>,
    mut cars: ResMut<Cars>,
    mut commands: Commands,
) {
    let mut out: HashMap<PlayerId, Transform> = peers
        .0
        .iter()
        .filter_map(|(id, p)| {
            let car = p.seen.as_ref()?.car?;
            Some((*id, car_transform(car.pos.into(), car.yaw)))
        })
        .collect();
    if let Some(car) = me.0.ride.car() {
        out.insert(session.me(), car_transform(car.pos, car.yaw));
    }

    // Pocketed, or its owner left. Either way there is no car there any more.
    cars.0.retain(|id, entity| {
        let gone = !out.contains_key(id);
        if gone {
            commands.entity(*entity).despawn();
        }
        !gone
    });
    for (id, transform) in out {
        match cars.0.get(&id) {
            Some(entity) => {
                commands.entity(*entity).insert(transform);
            }
            None => {
                cars.0
                    .insert(id, avatar::spawn_car(&mut commands, &palette, transform));
            }
        }
    }
}

/// A car's model transform. Its `pos` is the middle of its underside, which is the model's
/// origin — the same relationship a player's feet have to their body.
fn car_transform(pos: Vec3, yaw: f32) -> Transform {
    Transform {
        translation: pos,
        rotation: Quat::from_rotation_y(yaw),
        ..default()
    }
}

/// Is the block within `reach` of a player standing at `actor`? Measured eye to nearest
/// point of the block, which is the same thing the client's raycast bounds.
fn within_reach(actor: Vec3, pos: BlockPos, reach: f32) -> bool {
    let eye = actor + Vec3::Y * player::EYE_HEIGHT;
    let corner = pos.corner();
    eye.distance(eye.clamp(corner, corner + Vec3::ONE)) <= reach + REACH_LAG
}

/// Every rule an edit must satisfy, in one place, applied by the host to its own player
/// and to every peer alike.
///
/// This is what makes host-authoritative *real*: none of it is taken on the asker's
/// word, so a modified client can ask for anything and still cannot break bedrock, reach
/// across the map, overwrite a block that is already there, or wall anybody in.
///
/// `standing` is every player the host knows the position of, the actor included. A block
/// placed inside somebody wedges *them*, so whose box it is does not matter — checking only
/// the placer's own box left "build into the person next to you" wide open.
///
/// `reach` is how far this player's own things let them *break* — see [`Inventory::reach`].
/// Building is always arm's length, whatever they own: a gun knocks the top off the next
/// hill, and there is nothing it could mean to build one from here.
fn edit_is_legal(
    world: &World,
    actor: Vec3,
    standing: &[Vec3],
    pos: BlockPos,
    block: Block,
    reach: f32,
) -> bool {
    match block {
        // Breaking. Bedrock is the floor of the world; breaking it drops you out of it.
        Block::Air => within_reach(actor, pos, reach) && world.block(pos) != Block::Bedrock,
        // Placing: only a voxel some item actually places, only into empty space, and
        // never inside a player.
        _ => {
            within_reach(actor, pos, Use::BARE_HAND.reach)
                && block.placeable()
                && world.block(pos) == Block::Air
                && !standing.iter().any(|p| player::would_trap(*p, pos))
        }
    }
}

/// The host's half of an edit: apply a request if the rules allow it, and say whether
/// the world changed — which is exactly when the host announces it.
///
/// This is also where a block becomes a thing you own and back again. Placing spends the
/// item; breaking gathers whatever was standing there. One function, so the world and the
/// player's pile cannot end up disagreeing about an edit that half happened.
fn apply_if_legal(
    sim: &mut World,
    chunks: &mut Chunks,
    inventory: &mut Inventory,
    actor: Vec3,
    standing: &[Vec3],
    pos: BlockPos,
    block: Block,
) -> bool {
    // Every rule below asks the world what is already there, and an unloaded chunk answers
    // `Air`: bedrock unprotected, occupied space "empty". Peers edit far from wherever the
    // host's own player happens to be standing, so the host generates the chunk it is being
    // asked about rather than deciding blind. `stream_chunks` drops it again on its next
    // pass if nobody is near it.
    sim.load_chunk(pos.chunk());
    // How far this player may work is read off their own pile, here, rather than taken
    // from anything they said: a gun is a thing the host handed them, and it is the host
    // that decides what one is worth.
    if !edit_is_legal(sim, actor, standing, pos, block, inventory.reach()) {
        return false;
    }
    // What this costs and what it yields, both decided before the world moves: `set_block`
    // can still refuse an out-of-bounds write, and a pile paid out of for an edit that
    // never happened is exactly the sort of thing nobody notices until a chest is empty.
    let spend = Item::placing(block);
    if let Some(item) = spend
        && inventory.count(item) == 0
    {
        return false;
    }
    let gathered = Item::placing(sim.block(pos));
    if !sim.set_block(pos, block) {
        return false;
    }
    match spend {
        Some(item) => {
            let paid = inventory.take(item, 1);
            debug_assert!(paid, "the count was checked a moment ago");
        }
        // Breaking. Bedrock is unbreakable and air was never there, so an unowned block
        // simply yields nothing.
        None => {
            if let Some(item) = gathered {
                inventory.add(item, 1);
            }
        }
    }
    mark_dirty(chunks, pos);
    true
}

/// Whoever is asking for something: who they are, and where they say they are.
///
/// The two travel together because every rule reads both — reach is checked against the
/// position, and the cost is taken out of the pile belonging to the id. Splitting them
/// into two arguments is how an edit gets checked against one player and paid for by
/// another.
#[derive(Clone, Copy)]
struct Actor {
    id: PlayerId,
    pos: Vec3,
}

/// A block change on its way into the world.
enum Edit {
    /// Somebody wants this change — the local player, or a peer over the wire.
    Request {
        actor: Actor,
        pos: BlockPos,
        block: Block,
    },
    /// The host says this happened. Only a peer ever receives one, and it is not a
    /// request: the host has already checked it.
    Announcement { pos: BlockPos, block: Block },
}

/// The ONE place a block change enters the game, from local input or from the wire.
///
/// The host is authoritative: it checks every request against [`edit_is_legal`], applies
/// it, announces it, and tells the actor what they have left. A peer only asks — it does
/// not touch its own world or its own pile, so what it sees is always what the host said,
/// never a local guess that has to be rolled back.
#[allow(clippy::too_many_arguments)]
fn submit_edit(
    role: Role,
    session: &Session,
    sim: &mut World,
    chunks: &mut Chunks,
    inventories: &mut Inventories,
    standing: &[Vec3],
    edit: Edit,
) {
    match (role, edit) {
        (Role::Host, Edit::Request { actor, pos, block }) => {
            let inventory = inventories.0.entry(actor.id).or_default();
            if apply_if_legal(sim, chunks, inventory, actor.pos, standing, pos, block) {
                session.send(Target::All, Msg::Edit { pos, block });
                announce_inventory(session, actor.id, inventory);
            }
        }
        (Role::Peer { .. }, Edit::Request { pos, block, .. }) => {
            session.send(Target::All, Msg::Edit { pos, block })
        }
        (Role::Peer { .. }, Edit::Announcement { pos, block }) => {
            if sim.set_block(pos, block) {
                mark_dirty(chunks, pos);
            }
        }
        // `net_receive` builds an announcement only when this process is a peer, and a
        // host has no other host to hear one from.
        (Role::Host, Edit::Announcement { .. }) => {
            unreachable!("the host is the only announcer")
        }
    }
}

/// Tells one player what they now have. Host-side only, and only ever to the owner.
///
/// The host's own pile needs no message: it is already the map this reads from, which is
/// what keeps single player from being a second path through the inventory.
fn announce_inventory(session: &Session, who: PlayerId, inventory: &Inventory) {
    if who != session.me() {
        session.send(
            Target::One(who),
            Msg::Inventory {
                items: inventory.contents(),
            },
        );
    }
}

/// The host's half of a craft: pay the recipe out of that player's pile and hand them the
/// thing. Every rule lives in [`Inventory::craft`], so a modified client asking for a free
/// car gets what an honest one would — nothing, unless the nails are there.
fn submit_craft(session: &Session, inventories: &mut Inventories, who: PlayerId, item: Item) {
    let inventory = inventories.0.entry(who).or_default();
    if inventory.craft(item) {
        announce_inventory(session, who, inventory);
    }
}

/// Marks the chunk holding `pos` for a remesh, plus any neighbour whose seam it touches.
fn mark_dirty(chunks: &mut Chunks, pos: BlockPos) {
    chunks.dirty.insert(pos.chunk());
    let local_x = pos.x.rem_euclid(CHUNK_SIZE);
    let local_z = pos.z.rem_euclid(CHUNK_SIZE);
    for (edge, delta) in [
        (local_x == 0, ChunkPos::new(-1, 0)),
        (local_x == CHUNK_SIZE - 1, ChunkPos::new(1, 0)),
        (local_z == 0, ChunkPos::new(0, -1)),
        (local_z == CHUNK_SIZE - 1, ChunkPos::new(0, 1)),
    ] {
        if edge {
            let c = pos.chunk();
            chunks
                .dirty
                .insert(ChunkPos::new(c.x + delta.x, c.z + delta.z));
        }
    }
}

/// The block a use is chewed down to before it goes, drawn as the highlight shrinking. A
/// block that visibly disappears as you work at it is the only thing on screen that says a
/// drill is faster than a fist.
const DUG_SCALE: f32 = 0.45;

#[allow(clippy::too_many_arguments)]
fn target_and_edit(
    intent: Res<Intent>,
    me: Res<Me>,
    role: Res<NetRole>,
    held: Res<Held>,
    peers: Res<Peers>,
    time: Res<Time>,
    session: NonSend<Session>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut inventories: ResMut<Inventories>,
    mut digging: ResMut<Digging>,
    mut highlight: Query<(&mut Transform, &mut Visibility), With<Highlight>>,
) {
    // One table decides everything about the thing in hand: how far it works, how fast it
    // chews, and how far it zooms. There is no per-item branch below, and adding a tool
    // needs no change here at all.
    let using = inventories.of(session.me()).using(held.0);
    let hit = raycast::cast(&sim.0, me.0.eye(), me.0.look_dir(), using.reach);

    // A long frame must not be a free block: the same clamp `apply_intent` puts on motion.
    let dt = time.delta_secs().min(0.05);
    digging.0 = match (intent.use_item, hit) {
        (true, Some(h)) => Some(chew(digging.0, h.block, using, dt)),
        _ => None,
    };
    // Broken through. Forgetting the dig here is what stops a peer — whose block does not
    // vanish until the host says so — asking again every frame while the answer is in
    // flight; it starts the block over instead.
    let broke_through = digging.0.is_some_and(|d| d.done >= 1.0);
    if broke_through {
        digging.0 = None;
    }

    if let Ok((mut t, mut vis)) = highlight.single_mut() {
        match hit {
            Some(h) => {
                t.translation = h.block.center();
                t.scale = Vec3::splat(1.0 - DUG_SCALE * digging.0.map_or(0.0, |d| d.done));
                *vis = Visibility::Visible;
            }
            None => *vis = Visibility::Hidden,
        }
    }

    let Some(hit) = hit else { return };

    // What the button asks for. Whether it is allowed is the host's call, not this
    // client's — see `edit_is_legal`.
    let edit = if broke_through {
        Some((hit.block, Block::Air))
    } else if intent.place_block {
        // Holding something that is not a block places nothing: a rifle puts nothing in
        // the world, and pretending the button is broken beats pretending it is a block.
        held.0.places().map(|block| (hit.adjacent(), block))
    } else {
        None
    };

    if let Some((pos, block)) = edit {
        let request = Edit::Request {
            actor: Actor {
                id: session.me(),
                pos: me.0.pos,
            },
            pos,
            block,
        };
        let standing = standing(&me.0, &peers);
        submit_edit(
            role.0,
            &session,
            &mut sim.0,
            &mut chunks,
            &mut inventories,
            &standing,
            request,
        );
    }
}

/// The scope. The view narrows while a scoped tool is being used, and springs back the
/// moment the trigger is let go.
///
/// One button does both because the Deck's other trigger is the place button: holding R2
/// with the rifle is aiming *and* firing, which is also how a six-year-old expects a
/// trigger to work. Read from the same [`Use`] the reach and the speed come from, so a
/// scope is a number in the registry and not a rifle-shaped branch in the camera code.
fn aim_zoom(
    intent: Res<Intent>,
    held: Res<Held>,
    inventories: Res<Inventories>,
    session: NonSend<Session>,
    mut camera: Query<&mut Projection, With<Eye>>,
) {
    let zoom = if intent.use_item {
        inventories.of(session.me()).using(held.0).zoom
    } else {
        1.0
    };
    let Ok(mut projection) = camera.single_mut() else {
        return;
    };
    if let Projection::Perspective(p) = &mut *projection {
        let fov = FOV / zoom;
        // Only on a change: assigning through the `Mut` marks the projection dirty, and a
        // dirty projection is a matrix rebuilt for a camera that did not move.
        if p.fov != fov {
            p.fov = fov;
        }
    }
}

/// The local player's craft button. Like an edit, it is a *request*: the host owns every
/// pile, so a peer asks and waits to be told what it has.
/// The craft button in the world opens the rig, on whatever the hotbar cursor is on.
///
/// Making a thing is one press *inside* the rig now, not one press outside it: there is
/// one place a recipe is paid, and it is the place the recipe is drawn.
fn open_forge(
    intent: Res<Intent>,
    held: Res<Held>,
    session: NonSend<Session>,
    inventories: Res<Inventories>,
    mut commands: Commands,
    mut playing: ResMut<NextState<Playing>>,
) {
    if !intent.craft {
        return;
    }
    commands.insert_resource(forge::Forge::new(
        held.0,
        inventories.of(session.me()).clone(),
    ));
    playing.set(Playing::Forge);
}

/// Hands the recipe rig the pile it draws, and hides the crosshair while it is up.
fn dress_the_forge(
    session: NonSend<Session>,
    inventories: Res<Inventories>,
    mut stock: ResMut<rig::Stock>,
    mut hud: Query<&mut Visibility, With<hud::HudRoot>>,
) {
    belt::wear_the_stock(inventories.of(session.me()), &mut stock);
    hud::show(&mut hud, false);
}

/// The same pile, handed to the belt. One [`rig::Stock`] feeds both rigs, so what your
/// body is wearing and what the graph is counting cannot come apart.
fn wear_the_belt(
    session: NonSend<Session>,
    inventories: Res<Inventories>,
    mut stock: ResMut<rig::Stock>,
) {
    belt::wear_the_stock(inventories.of(session.me()), &mut stock);
}

/// Puts the rig's craft requests through the same door the hotbar's used: the host pays
/// them, a peer asks the host to.
fn submit_forge_crafts(
    role: Res<NetRole>,
    session: NonSend<Session>,
    mut requests: ResMut<forge::CraftRequests>,
    mut inventories: ResMut<Inventories>,
) {
    for item in requests.0.drain(..).collect::<Vec<_>>() {
        match role.0 {
            Role::Host => submit_craft(&session, &mut inventories, session.me(), item),
            Role::Peer { .. } => session.send(Target::All, Msg::Craft { item }),
        }
    }
}

fn close_forge(nav: Res<forge::Nav>, mut playing: ResMut<NextState<Playing>>) {
    if nav.leave {
        playing.set(Playing::Live);
    }
}

/// Back to the world: the recipe rig goes away, the crosshair and the belt come back.
fn undress_the_forge(
    mut commands: Commands,
    rig: Query<Entity, With<forge::Rig>>,
    mut hud: Query<&mut Visibility, With<hud::HudRoot>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    forge::leave(commands.reborrow(), rig);
    hud::show(&mut hud, true);
    grab_mouse(&mut cursor, true);
}

/// The rig is read, not aimed at: the mouse comes back so a player on a keyboard is not
/// spinning the world behind it while they read the graph.
///
/// A walk in progress is let go of by [`hush_intent`], which zeroes the intent on every
/// frame that is not the world's — the two rooms used to do it themselves on the way in,
/// once each, which left the frames *after* the first one asserting the old walk.
fn let_go_of_the_mouse(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    grab_mouse(&mut cursor, false);
}

/// Where everybody is: this player, plus the last pose each peer sent. What the placement
/// rules must not put a block inside of. Somebody who has not posed yet is nowhere in
/// particular, so there is nowhere to keep clear for them.
fn standing(me: &Player, peers: &Peers) -> Vec<Vec3> {
    std::iter::once(me.pos)
        .chain(
            peers
                .0
                .values()
                .filter_map(|p| p.seen.as_ref().map(|s| s.pos)),
        )
        .collect()
}

/// Is `from` entitled to say this? The whole trust boundary, in one pure function.
///
/// A peer has exactly one link — to the host — so it believes that endpoint and nobody
/// else. The host talks to many peers, and a peer may only ever speak for itself: ask for
/// an edit or a craft, report its own pose, say hello. Everything else is the host's word,
/// and a peer sending one is claiming to be the host.
///
/// Both halves are exhaustive on purpose. A request is not a thing a host says, any more
/// than an announcement is a thing a peer says, and a new message has to declare which it
/// is here before it can be acted on anywhere.
fn authorized(role: Role, from: PlayerId, msg: &Msg) -> bool {
    match role {
        Role::Peer { host } => {
            from == host
                && match msg {
                    Msg::Welcome { .. }
                    | Msg::WorldPart { .. }
                    | Msg::Edit { .. }
                    | Msg::Pose { .. }
                    | Msg::Present { .. }
                    | Msg::Inventory { .. } => true,
                    Msg::Hello | Msg::Craft { .. } => false,
                }
        }
        Role::Host => match msg {
            Msg::Welcome { .. }
            | Msg::WorldPart { .. }
            | Msg::Present { .. }
            | Msg::Inventory { .. } => false,
            Msg::Pose { id, .. } => *id == from,
            Msg::Hello | Msg::Edit { .. } | Msg::Craft { .. } => true,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn net_receive(
    role: Res<NetRole>,
    time: Res<Time>,
    my_player: Res<Me>,
    mut session: NonSendMut<Session>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut peers: ResMut<Peers>,
    mut inventories: ResMut<Inventories>,
    mut welcomed: ResMut<Welcomed>,
    mut world_budget: ResMut<WorldBudget>,
    palette: Res<avatar::Palette>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let me = session.me();
    let dt = time.delta_secs();
    world_budget.0.refill(dt);
    for peer in peers.0.values_mut() {
        peer.budget.refill(dt);
        peer.travel.refill(dt);
    }

    for event in session.drain() {
        match event {
            Event::Message(from, msg) => {
                if !authorized(role.0, from, &msg) {
                    continue;
                }
                match msg {
                    // Only a host is ever asked for a world; a peer has none to give.
                    Msg::Hello => {
                        // Answering costs a copy of every edit in the world and a frame
                        // per edited chunk, so a peer that keeps asking is answered once.
                        // Their link ending forgets them, so a real reconnect is served.
                        if role.0 == Role::Host && welcomed.0.insert(from) {
                            // One frame per edited chunk, announced by a count. A world
                            // sent as a single message outgrows what can be encoded and
                            // stops being joinable at all — see `Msg::WorldPart`.
                            let parts = sim.0.overlays();
                            session.send(
                                Target::One(from),
                                Msg::Welcome {
                                    seed: sim.0.seed(),
                                    parts: parts.len() as u32,
                                },
                            );
                            for edits in parts {
                                session.send(Target::One(from), Msg::WorldPart { edits });
                            }
                            // Who is here, after the world they are here in. The joiner
                            // is still collecting that world in `net::boot` when its
                            // arrival is announced to everybody else, so this is the copy
                            // it is actually in a position to read.
                            session.send(
                                Target::One(from),
                                Msg::Present {
                                    ids: live(&peers, me),
                                },
                            );
                        }
                    }
                    // The handshake already collected the world in `net::boot`, so a part
                    // arriving after it has nothing left to do.
                    Msg::WorldPart { .. } => {}
                    Msg::Edit { pos, block } => {
                        let edit = match role.0 {
                            // A peer's edit is a request, checked against where that peer
                            // last said it was standing, and against what it is still
                            // allowed to ask for.
                            Role::Host => {
                                // Somebody the host has never heard of, or who has not
                                // said where they are yet: nothing of theirs can be
                                // checked. Their next pose is a frame away.
                                let Some(peer) = peers.0.get_mut(&from) else {
                                    continue;
                                };
                                let Some(standing_at) = peer.seen.as_ref().map(|s| s.pos) else {
                                    continue;
                                };
                                // Asking faster than a person can play: the world is not
                                // theirs to fill at machine speed. Both buckets, because
                                // the per-peer one is only as strong as identities are
                                // scarce, and they are free.
                                if !peer.budget.spend(1.0) || !world_budget.0.spend(1.0) {
                                    continue;
                                }
                                Edit::Request {
                                    actor: Actor {
                                        id: from,
                                        pos: standing_at,
                                    },
                                    pos,
                                    block,
                                }
                            }
                            // `authorized` proved this came from the host, and the host
                            // is the truth.
                            Role::Peer { .. } => Edit::Announcement { pos, block },
                        };
                        let standing = standing(&my_player.0, &peers);
                        submit_edit(
                            role.0,
                            &session,
                            &mut sim.0,
                            &mut chunks,
                            &mut inventories,
                            &standing,
                            edit,
                        );
                    }
                    // Asking the host to make something is asking it to do work on your
                    // behalf, exactly as an edit is, so it comes out of the same
                    // allowance — and from somebody it has heard of.
                    Msg::Craft { item } => {
                        if role.0 == Role::Host
                            && let Some(peer) = peers.0.get_mut(&from)
                            && peer.budget.spend(1.0)
                        {
                            submit_craft(&session, &mut inventories, from, item);
                        }
                    }
                    // The host's word on what this player has. A peer keeps one pile —
                    // its own — under its own id, so every reader asks the same question
                    // whichever side it is on.
                    Msg::Inventory { items } => {
                        inventories.0.insert(me, Inventory::from_contents(items));
                    }
                    Msg::Pose { id, pose } => {
                        // The host is the only relay, and it vouches for what it relays.
                        let pose = match role.0 {
                            Role::Host => vouched(pose, inventories.of(id)),
                            Role::Peer { .. } => pose,
                        };
                        if role.0 == Role::Host {
                            session.send(Target::All, Msg::Pose { id, pose });
                        }
                        if id != me {
                            peers.posed(&mut commands, &palette, id, pose);
                        }
                    }
                    Msg::Present { ids } => {
                        peers.present(&mut commands, &mut inventories, me, &ids);
                    }
                    // The handshake already happened in `net::boot`, so a second Welcome
                    // has nothing left to do.
                    Msg::Welcome { .. } => {}
                }
            }
            // A link appearing or disappearing is the whole of who is in this world, and
            // only the host has more than one of them. A peer's single link is the host it
            // dialled, whose arrival it already knows about and whose departure ends the
            // game.
            Event::Joined(id) => {
                if role.0 == Role::Host {
                    present(
                        &session,
                        &mut commands,
                        &mut peers,
                        &mut inventories,
                        me,
                        Presence::Joined(id),
                    );
                }
            }
            Event::Left(id) => match role.0 {
                Role::Host => {
                    welcomed.0.remove(&id);
                    present(
                        &session,
                        &mut commands,
                        &mut peers,
                        &mut inventories,
                        me,
                        Presence::Left(id),
                    );
                }
                // A peer only ever links to the host, but a departure that is not the
                // host's must never end somebody's game.
                Role::Peer { host } => {
                    if id == host {
                        eprintln!("host disconnected");
                        exit.write(AppExit::Success);
                    }
                }
            },
        }
    }
}

/// The host's cut of a pose before it relays it: the parts it can actually vouch for.
///
/// A car is one of them. The host hands out every [`Item::Car`] there is, so a peer
/// claiming to have one out without owning one is claiming something the host knows is
/// false, and the car simply does not exist for anybody else.
///
/// *Where* the car is stays the driver's word — the same standing as where their own feet
/// are, and for the same reason: it is continuous motion nobody else simulates. What keeps
/// that honest is that a driver's feet are in the car, and feet are what
/// [`TRAVEL_RATE`] bites on.
fn vouched(pose: Pose, owner: &Inventory) -> Pose {
    Pose {
        car: pose.car.filter(|_| owner.count(Item::Car) > 0),
        ..pose
    }
}

/// What just happened to the host's set of links, which is the only thing that changes who
/// is in the world.
enum Presence {
    Joined(PlayerId),
    Left(PlayerId),
}

/// Everybody in this world as the host has it: the players it holds links to, and itself.
fn live(peers: &Peers, me: PlayerId) -> Vec<PlayerId> {
    peers.0.keys().copied().chain(std::iter::once(me)).collect()
}

/// A player arriving or leaving, told to everybody and applied here from the same list —
/// so the host's own world is built by believing exactly what it says, as a peer's is.
/// Host only: a peer has one link and nothing to declare about it.
fn present(
    session: &Session,
    commands: &mut Commands,
    peers: &mut Peers,
    inventories: &mut Inventories,
    me: PlayerId,
    change: Presence,
) {
    let mut ids = live(peers, me);
    match change {
        Presence::Joined(id) => ids.push(id),
        Presence::Left(id) => ids.retain(|here| *here != id),
    }
    session.send(Target::All, Msg::Present { ids: ids.clone() });
    peers.present(commands, inventories, me, &ids);
}

impl Peers {
    /// The host's word on who is here, applied whole — the only way a player comes into
    /// existence, and the only way one stops.
    ///
    /// Absolute, so it can be applied twice to the same answer and needs no order against
    /// itself. That is what closes the ghost: presence rides the stream and poses ride
    /// datagrams, with no order between the two, so a departure sent as a delta can be
    /// overtaken by a pose relayed an instant earlier — and the avatar that pose put back
    /// stands there for the rest of the session.
    fn present(
        &mut self,
        commands: &mut Commands,
        inventories: &mut Inventories,
        me: PlayerId,
        ids: &[PlayerId],
    ) {
        let here: HashSet<PlayerId> = ids.iter().copied().filter(|id| *id != me).collect();
        for id in &here {
            self.0.entry(*id).or_insert_with(PeerState::declared);
        }
        let gone: Vec<PlayerId> = self
            .0
            .keys()
            .copied()
            .filter(|id| !here.contains(id))
            .collect();
        for id in gone {
            forget_peer(commands, self, inventories, id);
        }
    }

    /// A pose *moves* a player the host has declared, and does nothing at all for anybody
    /// else. Creating one here is what #6 was: the host relays a pose and announces a
    /// departure on two transports with no order between them, so the loser of that race
    /// would resurrect somebody who has left.
    fn posed(
        &mut self,
        commands: &mut Commands,
        palette: &avatar::Palette,
        id: PlayerId,
        pose: Pose,
    ) {
        if let Some(peer) = self.0.get_mut(&id) {
            peer.moved(commands, palette, pose);
        }
    }
}

impl PeerState {
    /// A player the host has said is here and who has not spoken yet.
    fn declared() -> Self {
        PeerState {
            seen: None,
            budget: Budget::new(EDIT_RATE, EDIT_BURST),
            travel: Budget::new(TRAVEL_RATE, TRAVEL_BURST),
        }
    }

    /// Records where this player is and keeps their model there, spawning it on their
    /// first pose.
    ///
    /// A pose is a *claim*, and on the host it is what every proximity rule is checked
    /// against, so a claim that outruns [`TRAVEL_RATE`] is refused: without that, a
    /// modified client says it is standing anywhere and edits anything, and no reach check
    /// means a thing. The first pose is where they say they spawned, which nothing can
    /// contradict.
    fn moved(&mut self, commands: &mut Commands, palette: &avatar::Palette, pose: Pose) {
        // A pose is the player's feet, which is also the model's origin — see `avatar`.
        let pos = Vec3::from(pose.pos);
        let transform = Transform {
            translation: pos,
            rotation: Quat::from_rotation_y(pose.yaw),
            ..default()
        };
        match &mut self.seen {
            Some(seen) => {
                if !self.travel.spend(seen.pos.distance(pos)) {
                    return;
                }
                seen.pos = pos;
                seen.car = pose.car;
                commands.entity(seen.body.root).insert(transform);
                // Only when it changes: a pose arrives every frame, and re-stating the
                // same cube would be sixty component writes a second per player to no
                // effect.
                if seen.held != pose.held {
                    seen.held = pose.held;
                    avatar::show_held(commands, palette, seen.body, pose.held);
                }
            }
            None => {
                let body = avatar::spawn(commands, palette, transform);
                avatar::show_held(commands, palette, body, pose.held);
                self.seen = Some(Seen {
                    pos,
                    car: pose.car,
                    body,
                    held: pose.held,
                });
            }
        }
    }
}

/// Drops everything the game holds for a player who has gone: their model, and — on the
/// host — their things.
///
/// Their things go because identities are free, and a pile kept for somebody who left is
/// memory a stranger can grow by reconnecting under a new key. Nothing outlives the
/// session anyway: the world itself is not saved either.
fn forget_peer(
    commands: &mut Commands,
    peers: &mut Peers,
    inventories: &mut Inventories,
    id: PlayerId,
) {
    inventories.0.remove(&id);
    if let Some(seen) = peers.0.remove(&id).and_then(|p| p.seen) {
        commands.entity(seen.body.root).despawn();
    }
}

fn net_send_pose(
    session: NonSend<Session>,
    me: Res<Me>,
    held: Res<Held>,
    inventories: Res<Inventories>,
) {
    // What is in the hand, not what the cursor is on: the same question the use button
    // asks, so the cube everyone else sees is the thing that just broke their wall.
    let holding = inventories.of(session.me()).in_hand(held.0);
    session.send(
        Target::All,
        Msg::Pose {
            id: session.me(),
            pose: Pose {
                pos: me.0.pos.into(),
                yaw: me.0.yaw,
                pitch: me.0.pitch,
                held: holding,
                car: me.0.ride.car().map(|car| CarPose {
                    pos: car.pos.into(),
                    yaw: car.yaw,
                }),
            },
        },
    );
}

fn chebyshev(a: ChunkPos, b: ChunkPos) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

/// Chunks the player is close enough to want loaded, nearest first — so the world fills
/// in around them rather than in scan order.
fn wanted_chunks(center: ChunkPos) -> Vec<ChunkPos> {
    let mut wanted: Vec<ChunkPos> = Vec::new();
    for dz in -(VIEW_RADIUS + 1)..=(VIEW_RADIUS + 1) {
        for dx in -(VIEW_RADIUS + 1)..=(VIEW_RADIUS + 1) {
            wanted.push(ChunkPos::new(center.x + dx, center.z + dz));
        }
    }
    wanted.sort_by_key(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2));
    wanted
}

/// Loaded chunks the player has walked away from.
///
/// Read from the world's own loaded set, never from the chunk entities: the load ring
/// reaches a chunk further than the mesh ring, and a chunk can be loaded long before it
/// is meshed, so anything driven by entities leaks every chunk that never got one.
fn stale_chunks(world: &World, center: ChunkPos) -> Vec<ChunkPos> {
    world
        .loaded_chunks()
        .filter(|c| chebyshev(*c, center) > UNLOAD_RADIUS)
        .collect()
}

fn stream_chunks(
    me: Res<Me>,
    material: Res<WorldMaterial>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let center = BlockPos::containing(me.0.pos).chunk();

    for cp in stale_chunks(&sim.0, center) {
        if let Some(e) = chunks.entities.remove(&cp) {
            commands.entity(e).despawn();
        }
        sim.0.unload_chunk(cp);
    }

    let wanted = wanted_chunks(center);

    let mut generated = 0;
    for cp in &wanted {
        if generated >= LOAD_BUDGET {
            break;
        }
        if !sim.0.is_loaded(*cp) {
            sim.0.load_chunk(*cp);
            generated += 1;
        }
    }

    let mut built = 0;
    for cp in &wanted {
        if built >= MESH_BUDGET {
            break;
        }
        if chebyshev(*cp, center) > VIEW_RADIUS || chunks.entities.contains_key(cp) {
            continue;
        }
        if refresh_chunk(
            &mut commands,
            &mut meshes,
            &material,
            &mut chunks,
            &sim.0,
            *cp,
        ) {
            built += 1;
        }
    }
}

fn remesh_dirty(
    material: Res<WorldMaterial>,
    sim: Res<Sim>,
    mut chunks: ResMut<Chunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let dirty: Vec<ChunkPos> = chunks.dirty.drain().collect();
    for cp in dirty {
        // A chunk with no entity has never been drawn; `stream_chunks` builds it from
        // scratch, and from current voxels, when it comes into view.
        if !chunks.entities.contains_key(&cp) {
            continue;
        }
        let meshed = refresh_chunk(
            &mut commands,
            &mut meshes,
            &material,
            &mut chunks,
            &sim.0,
            cp,
        );
        // Not meshable yet — a neighbour is unloaded. Dropping it here would leave the
        // edit undrawn until something else happened to dirty the chunk, so it waits.
        if !meshed {
            chunks.dirty.insert(cp);
        }
    }
}

/// Rebuilds one chunk's mesh entity, creating it if it doesn't exist yet. False if the
/// chunk isn't meshable yet, in which case nothing is spawned or changed.
fn refresh_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &WorldMaterial,
    chunks: &mut Chunks,
    world: &World,
    cp: ChunkPos,
) -> bool {
    let mesh = match build_chunk_mesh(world, cp) {
        ChunkMesh::Ready(mesh) => Some(mesh),
        ChunkMesh::Empty => None,
        ChunkMesh::NotReady => return false,
    };

    let origin = cp.origin();
    let translation = Vec3::new(origin.x as f32, 0.0, origin.z as f32);
    let entity = *chunks.entities.entry(cp).or_insert_with(|| {
        commands
            .spawn((ChunkTag, Transform::from_translation(translation)))
            .id()
    });

    match mesh {
        Some(mesh) => {
            commands
                .entity(entity)
                .insert((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.0.clone())));
        }
        // An all-air chunk keeps its entity (so it isn't rebuilt every frame) but draws
        // nothing.
        None => {
            commands.entity(entity).remove::<Mesh3d>();
        }
    }
    true
}

/// Takes the mouse, or gives it back. The pause menu needs a pointer the player can see
/// and the world needs one it can keep, and this is the one place either happens.
pub fn grab_mouse(cursor: &mut Query<&mut CursorOptions, With<PrimaryWindow>>, grabbed: bool) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = if grabbed {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        c.visible = !grabbed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[test]
    fn an_edit_at_a_chunk_seam_dirties_the_neighbour() {
        let mut chunks = Chunks::default();
        mark_dirty(&mut chunks, BlockPos::new(0, 40, 5));
        assert!(chunks.dirty.contains(&ChunkPos::new(0, 0)));
        assert!(chunks.dirty.contains(&ChunkPos::new(-1, 0)), "west seam");

        let mut chunks = Chunks::default();
        mark_dirty(&mut chunks, BlockPos::new(8, 40, 8));
        assert_eq!(chunks.dirty.len(), 1, "an interior edit touches one chunk");
    }

    /// An edit next to an unloaded chunk cannot be meshed without drawing a wall along
    /// that seam, so the chunk stays dirty and is remeshed when the neighbour arrives.
    /// Dropping it instead loses the edit until something else dirties the chunk — and
    /// `stream_chunks` will not rebuild one that already has an entity.
    #[test]
    fn an_unmeshable_dirty_chunk_waits_for_its_neighbour() {
        let cp = ChunkPos::new(0, 0);
        let mut sim = World::new(5, []);
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            sim.load_chunk(ChunkPos::new(cp.x + dx, cp.z + dz));
        }

        let mut ecs = bevy::ecs::world::World::new();
        let entity = ecs.spawn_empty().id();
        ecs.insert_resource(Sim(sim));
        ecs.insert_resource(WorldMaterial(
            Assets::<StandardMaterial>::default().add(StandardMaterial::default()),
        ));
        ecs.insert_resource(Assets::<Mesh>::default());
        ecs.insert_resource(Chunks {
            entities: HashMap::from([(cp, entity)]),
            dirty: HashSet::from([cp]),
        });

        ecs.run_system_once(remesh_dirty).unwrap();
        assert!(
            ecs.resource::<Chunks>().dirty.is_empty(),
            "a meshable chunk is meshed and done"
        );

        ecs.resource_mut::<Sim>()
            .0
            .unload_chunk(ChunkPos::new(1, 0));
        ecs.resource_mut::<Chunks>().dirty.insert(cp);
        ecs.run_system_once(remesh_dirty).unwrap();
        assert!(
            ecs.resource::<Chunks>().dirty.contains(&cp),
            "the edit must not be dropped on the floor"
        );

        ecs.resource_mut::<Sim>().0.load_chunk(ChunkPos::new(1, 0));
        ecs.run_system_once(remesh_dirty).unwrap();
        assert!(
            ecs.resource::<Chunks>().dirty.is_empty(),
            "and it is drawn once the neighbour is there"
        );
    }

    /// A deterministic stand-in player id. Ids are public keys, so distinct seed bytes
    /// give distinct ids.
    fn id(n: u8) -> PlayerId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    /// A player with plenty of every *block*, so a test about the world's rules is not
    /// quietly answered by an empty pocket — and with no gun, because reach is the rule
    /// most of these tests are about and a rifle in the pocket answers half of them.
    fn stocked() -> Inventory {
        let mut inv = Inventory::default();
        for item in Item::ALL.iter().filter(|i| i.places().is_some()) {
            inv.add(*item, 64);
        }
        inv
    }

    /// The host's rule check with nobody else in the world — the single-player case, and
    /// the one most of these tests are about.
    fn apply_alone(
        world: &mut World,
        chunks: &mut Chunks,
        actor: Vec3,
        pos: BlockPos,
        block: Block,
    ) -> bool {
        apply_if_legal(world, chunks, &mut stocked(), actor, &[actor], pos, block)
    }

    fn standing_in_a_loaded_world() -> (World, Chunks, Vec3, BlockPos) {
        let mut world = World::new(4, []);
        world.load_chunk(ChunkPos::new(0, 0));
        let ground = world.ground_height(8, 8);
        let feet = Vec3::new(8.5, ground as f32 + 1.0, 8.5);
        (world, Chunks::default(), feet, BlockPos::new(8, ground, 8))
    }

    /// The point of host authority: the host runs the rules itself, so a client that
    /// simply asks for a forbidden edit gets nothing. Bedrock is the sharpest case —
    /// breaking it drops everybody out of the world.
    #[test]
    fn a_modified_client_cannot_delete_bedrock() {
        let (mut world, mut chunks, surface_feet, surface) = standing_in_a_loaded_world();
        let floor = BlockPos::new(8, 0, 8);
        assert_eq!(world.block(floor), Block::Bedrock);

        // Standing on the bedrock, so reach cannot be what refuses this.
        let feet = Vec3::new(8.5, 1.0, 8.5);
        assert!(!apply_alone(
            &mut world,
            &mut chunks,
            feet,
            floor,
            Block::Air
        ));
        assert_eq!(world.block(floor), Block::Bedrock, "bedrock survived");
        assert!(chunks.dirty.is_empty(), "a refused edit dirties nothing");

        // ... and placing bedrock is refused too: no item places it.
        let air = BlockPos::new(7, surface.y + 1, 8);
        assert_eq!(world.block(air), Block::Air);
        assert!(!apply_alone(
            &mut world,
            &mut chunks,
            surface_feet,
            air,
            Block::Bedrock
        ));
        assert_eq!(world.block(air), Block::Air);
    }

    /// Nobody may be walled in — not just the person placing the block. Checking only the
    /// placer's own box left "build into the player standing next to you" open, and a
    /// player inside a block cannot walk out of it or break their way out.
    #[test]
    fn a_block_may_not_be_placed_inside_another_player() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        // Somebody else standing one block over, within our reach.
        let them = Vec3::new(7.5, surface.y as f32 + 1.0, 8.5);
        let their_head = BlockPos::new(7, surface.y + 2, 8);
        assert!(player::would_trap(them, their_head));

        assert!(
            !apply_if_legal(
                &mut world,
                &mut chunks,
                &mut stocked(),
                feet,
                &[feet, them],
                their_head,
                Block::Stone
            ),
            "placed a block inside somebody"
        );
        assert_eq!(world.block(their_head), Block::Air);
        assert!(
            apply_alone(&mut world, &mut chunks, feet, their_head, Block::Stone),
            "with nobody there it is an ordinary place"
        );
    }

    /// The rules ask the world what is already there, and an unloaded chunk answers
    /// `Air` — so a peer editing anywhere the host's own player is not standing would get
    /// bedrock unprotected and every occupied block "empty". Peers are mostly somewhere
    /// else, so this is the ordinary case, not the corner.
    #[test]
    fn an_edit_in_an_unloaded_chunk_is_still_checked() {
        let far = ChunkPos::new(40, -17);
        let mut world = World::new(4, []);
        let mut chunks = Chunks::default();
        assert!(!world.is_loaded(far), "the host is nowhere near this");

        // Standing on the bedrock over there, so reach cannot be what refuses it.
        let floor = BlockPos::new(far.origin().x + 8, 0, far.origin().z + 8);
        let feet = floor.center() + Vec3::Y * 0.5;
        assert!(
            !apply_alone(&mut world, &mut chunks, feet, floor, Block::Air),
            "broke the floor of a world the host had not looked at"
        );

        // ... and the ground next to it is not empty space to build into, either.
        let ground = BlockPos::new(floor.x, world.ground_height(floor.x, floor.z), floor.z);
        let feet = ground.center() + Vec3::Y * 1.0;
        assert!(!apply_alone(
            &mut world,
            &mut chunks,
            feet,
            ground,
            Block::Stone
        ));
    }

    #[test]
    fn the_host_applies_a_legal_edit() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        assert!(apply_alone(
            &mut world,
            &mut chunks,
            feet,
            surface,
            Block::Air
        ));
        assert_eq!(world.block(surface), Block::Air);
        assert!(chunks.dirty.contains(&surface.chunk()));
    }

    #[test]
    fn an_edit_out_of_reach_is_refused() {
        let (mut world, mut chunks, feet, _) = standing_in_a_loaded_world();
        // The far corner of the same chunk: ~10 blocks away, well past an arm.
        let far = BlockPos::new(0, world.ground_height(0, 0), 0);
        assert!(world.solid(far), "the test needs a real block over there");
        assert!(!within_reach(feet, far, Use::BARE_HAND.reach));
        assert!(!apply_alone(&mut world, &mut chunks, feet, far, Block::Air));
        assert!(world.solid(far), "an unreachable block is untouched");
    }

    /// A gun's whole point is the block you cannot walk to, so the host has to allow the
    /// range — and allow it from the pile it keeps itself, not from a claim. An empty
    /// pocket still reaches an arm's length and no further.
    #[test]
    fn a_rifle_reaches_across_the_valley_and_a_fist_does_not() {
        let (mut world, mut chunks, feet, _) = standing_in_a_loaded_world();
        let far = BlockPos::new(0, world.ground_height(0, 0), 0);
        let break_it = |world: &mut World, chunks: &mut Chunks, inv: &mut Inventory| {
            apply_if_legal(world, chunks, inv, feet, &[feet], far, Block::Air)
        };

        assert!(
            !break_it(&mut world, &mut chunks, &mut stocked()),
            "an arm does not reach the next hill"
        );
        let mut armed = stocked();
        armed.add(Item::Rifle, 1);
        assert!(
            break_it(&mut world, &mut chunks, &mut armed),
            "a rifle does"
        );
        assert_eq!(world.block(far), Block::Air);

        // ... and the range it buys is for knocking blocks down, not putting them up.
        assert!(
            !apply_if_legal(
                &mut world,
                &mut chunks,
                &mut armed,
                feet,
                &[feet],
                far,
                Block::Stone
            ),
            "built a wall on the next hill with a rifle"
        );
        assert_eq!(world.block(far), Block::Air);
    }

    /// Nothing may dig faster than the host will accept edits, or the best tool in the
    /// game is the one whose blocks silently stop breaking after the first burst.
    #[test]
    fn no_tool_outruns_the_edit_budget() {
        for item in Item::ALL {
            assert!(
                item.using().speed <= EDIT_RATE,
                "{item:?} digs {} blocks a second, past the {EDIT_RATE} allowed",
                item.using().speed
            );
        }
    }

    /// Breaking takes time, and how much is the whole difference between the tools. The
    /// numbers a player feels: a fist is about half a second, a drill about an eighth.
    #[test]
    fn a_drill_breaks_a_block_sooner_than_a_fist() {
        let block = BlockPos::new(3, 40, 3);
        let seconds_to_break = |using: Use| {
            let mut dig = None;
            let mut ticks = 0;
            while !dig.is_some_and(|d: Dig| d.done >= 1.0) {
                dig = Some(chew(dig, block, using, 1.0 / 60.0));
                ticks += 1;
                assert!(ticks < 6000, "a use that never breaks anything");
            }
            ticks as f32 / 60.0
        };
        let hand = seconds_to_break(Use::BARE_HAND);
        let drill = seconds_to_break(Item::Drill.using());
        assert!((0.4..0.6).contains(&hand), "a fist took {hand}s");
        assert!(drill < hand / 3.0, "a drill took {drill}s against {hand}s");
        assert!(seconds_to_break(Item::Hammer.using()) < hand);
    }

    /// Progress is one block's, not the player's. Without that, chipping at soft ground
    /// and then aiming at a wall takes the wall with it.
    #[test]
    fn looking_away_starts_the_next_block_from_nothing() {
        let (soft, wall) = (BlockPos::new(0, 40, 0), BlockPos::new(1, 40, 0));
        let using = Item::Hammer.using();

        let mut dig = chew(None, soft, using, 0.08);
        dig = chew(Some(dig), soft, using, 0.08);
        assert!(dig.done > 0.5 && dig.done < 1.0, "part way into the ground");

        let moved = chew(Some(dig), wall, using, 0.01);
        assert_eq!(moved.block, wall);
        assert!(moved.done < 0.1, "the wall inherited the hole's progress");
    }

    #[test]
    fn a_block_may_only_be_placed_into_empty_space() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        assert!(
            !apply_alone(&mut world, &mut chunks, feet, surface, Block::Stone),
            "the ground is already occupied"
        );
        assert_ne!(world.block(surface), Block::Stone);

        let inside_me = BlockPos::containing(feet);
        assert!(
            !apply_alone(&mut world, &mut chunks, feet, inside_me, Block::Stone),
            "nobody may be walled into the world"
        );

        let beside_me = BlockPos::new(7, surface.y + 1, 8);
        assert_eq!(world.block(beside_me), Block::Air);
        assert!(apply_alone(
            &mut world,
            &mut chunks,
            feet,
            beside_me,
            Block::Stone
        ));
        assert_eq!(world.block(beside_me), Block::Stone);
    }

    /// A peer's world is built entirely from what the host says, so it must believe the
    /// host and nobody else — including for edits, which is what a stranger would use to
    /// rewrite somebody's blocks.
    #[test]
    fn a_peer_believes_only_the_host() {
        let (host, stranger) = (id(1), id(2));
        let role = Role::Peer { host };
        let edit = Msg::Edit {
            pos: BlockPos::new(0, 40, 0),
            block: Block::Stone,
        };
        assert!(authorized(role, host, &edit));
        assert!(!authorized(role, stranger, &edit));

        let pose = Msg::Pose {
            id: stranger,
            pose: Pose {
                pos: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                held: None,
                car: None,
            },
        };
        assert!(!authorized(role, stranger, &pose), "not the host");
        assert!(
            authorized(role, host, &pose),
            "the host relays everyone's pose"
        );
    }

    /// On the host, a peer speaks for itself and nothing more: it may not pose as another
    /// player, nor issue the host's own announcements.
    #[test]
    fn a_host_lets_a_peer_speak_only_for_itself() {
        let (peer, other) = (id(3), id(4));
        let pose = |id| Msg::Pose {
            id,
            pose: Pose {
                pos: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                held: None,
                car: None,
            },
        };
        assert!(authorized(Role::Host, peer, &pose(peer)));
        assert!(!authorized(Role::Host, peer, &pose(other)), "spoofed pose");
        assert!(!authorized(
            Role::Host,
            peer,
            &Msg::Present {
                ids: vec![peer, other]
            }
        ));
        assert!(!authorized(
            Role::Host,
            peer,
            &Msg::Welcome { seed: 0, parts: 0 }
        ));
        assert!(!authorized(
            Role::Host,
            peer,
            &Msg::WorldPart { edits: Vec::new() }
        ));
    }

    /// A peer may edit at a person's pace, not a machine's: every edit is world state the
    /// host keeps forever and ships to every future joiner.
    #[test]
    fn a_peer_cannot_edit_faster_than_its_budget() {
        let mut budget = Budget::new(EDIT_RATE, EDIT_BURST);
        let burst = (0..1000).take_while(|_| budget.spend(1.0)).count();
        assert_eq!(burst as f32, EDIT_BURST, "the burst is the whole allowance");
        assert!(!budget.spend(1.0), "spent out");

        budget.refill(1.0);
        let after_a_second = (0..1000).take_while(|_| budget.spend(1.0)).count();
        assert_eq!(after_a_second as f32, EDIT_RATE, "a second buys EDIT_RATE");

        budget.refill(3600.0);
        assert_eq!(
            (0..1000).take_while(|_| budget.spend(1.0)).count() as f32,
            EDIT_BURST,
            "idling does not bank an hour of edits"
        );
    }

    /// A pose is a claim, and the host checks every proximity rule against it. Believing
    /// one that teleports across the map is the same as having no reach rule at all.
    #[test]
    fn a_peer_cannot_claim_to_have_teleported() {
        let mut travel = Budget::new(TRAVEL_RATE, TRAVEL_BURST);
        assert!(
            !travel.spend(10_000.0),
            "a jump across the map is not a step"
        );
        assert!(travel.spend(TRAVEL_BURST), "an honest sprint is fine");
        assert!(!travel.spend(1.0), "and it is spent");
        travel.refill(1.0);
        assert!(
            travel.spend(TRAVEL_RATE) && !travel.spend(1.0),
            "a second of standing still buys a second of flying"
        );
    }

    /// Identities are free, so a bucket keyed by one is only ever half the bound: N fresh
    /// keys buy N times the edit rate unless something counts the world's own.
    #[test]
    fn the_world_has_an_edit_budget_of_its_own() {
        let mut world = WorldBudget::default().0;
        let burst = (0..10_000).take_while(|_| world.spend(1.0)).count();
        assert_eq!(burst as f32, WORLD_EDIT_BURST);
    }

    /// Walking in a straight line must not accumulate chunks. Unloading is driven from
    /// the world's loaded set for exactly this reason: chunks in the outer load ring
    /// never get an entity, so the render side cannot see — or free — them.
    #[test]
    fn walking_a_line_does_not_leak_chunks() {
        let mut world = World::new(11, []);
        let resident = ((2 * UNLOAD_RADIUS + 1) * (2 * UNLOAD_RADIUS + 1)) as usize;

        for step in 0..25 {
            let center = ChunkPos::new(step, 0);
            for cp in stale_chunks(&world, center) {
                world.unload_chunk(cp);
            }
            for cp in wanted_chunks(center) {
                world.load_chunk(cp);
            }
            let loaded = world.loaded_chunks().count();
            assert!(
                loaded <= resident,
                "step {step}: {loaded} chunks loaded, more than the {resident} in range"
            );
        }
        assert!(
            !world.is_loaded(ChunkPos::new(0, 0)),
            "the chunk walked away from is gone"
        );
    }

    /// The whole gathering loop, which is where every recipe's ingredients come from:
    /// break a block, and the block is yours.
    #[test]
    fn breaking_a_block_puts_it_in_your_pocket() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        let broken = world.block(surface);
        let item = Item::placing(broken).expect("the surface is something you can hold");
        let mut inventory = Inventory::default();

        assert!(apply_if_legal(
            &mut world,
            &mut chunks,
            &mut inventory,
            feet,
            &[feet],
            surface,
            Block::Air
        ));
        assert_eq!(
            inventory.count(item),
            1,
            "broke a {broken:?} and got nothing"
        );
    }

    /// Placing spends what you are holding, and an empty pocket is the end of it. Without
    /// this the counts are decoration and crafting is pointless: why make a cushion when
    /// you can place an infinite number of them?
    #[test]
    fn placing_spends_the_block_and_stops_when_it_runs_out() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        let mut inventory = Inventory::default();
        let beside_me = BlockPos::new(7, surface.y + 1, 8);
        let above_me = BlockPos::new(7, surface.y + 2, 8);

        assert!(
            !apply_if_legal(
                &mut world,
                &mut chunks,
                &mut inventory,
                feet,
                &[feet],
                beside_me,
                Block::Stone
            ),
            "placed a block out of an empty pocket"
        );
        assert_eq!(world.block(beside_me), Block::Air);

        inventory.add(Item::Stone, 1);
        assert!(apply_if_legal(
            &mut world,
            &mut chunks,
            &mut inventory,
            feet,
            &[feet],
            beside_me,
            Block::Stone
        ));
        assert_eq!(
            inventory.count(Item::Stone),
            0,
            "the block was not paid for"
        );
        assert!(!apply_if_legal(
            &mut world,
            &mut chunks,
            &mut inventory,
            feet,
            &[feet],
            above_me,
            Block::Stone
        ));
        assert_eq!(world.block(above_me), Block::Air);
    }

    /// An edit the world refuses must cost nothing. The reach check is the sharpest case:
    /// a player spraying blocks at a wall they cannot reach would otherwise empty their
    /// pockets into thin air.
    #[test]
    fn a_refused_edit_is_free() {
        let (mut world, mut chunks, feet, _) = standing_in_a_loaded_world();
        let mut inventory = stocked();
        let before = inventory.count(Item::Stone);
        let far = BlockPos::new(0, world.ground_height(0, 0) + 4, 0);

        assert!(!apply_if_legal(
            &mut world,
            &mut chunks,
            &mut inventory,
            feet,
            &[feet],
            far,
            Block::Stone
        ));
        assert_eq!(inventory.count(Item::Stone), before, "charged for nothing");
    }

    /// A car is a thing the host handed you, so a peer that never built one cannot make
    /// one appear in anybody's world by saying it has one. The rest of the pose is
    /// untouched: this strips a claim, it does not rewrite a player.
    #[test]
    fn a_car_nobody_built_is_not_relayed() {
        let claimed = Pose {
            pos: [1.0, 2.0, 3.0],
            yaw: 0.5,
            pitch: -0.25,
            held: Some(Item::Car),
            car: Some(CarPose {
                pos: [10.0, 40.0, 10.0],
                yaw: 1.0,
            }),
        };

        let empty = Inventory::default();
        let cut = vouched(claimed, &empty);
        assert_eq!(cut.car, None, "a car out of thin air");
        assert_eq!(
            Pose {
                car: claimed.car,
                ..cut
            },
            claimed,
            "the rest of the pose was rewritten too"
        );

        let mut owner = Inventory::default();
        owner.add(Item::Car, 1);
        assert_eq!(
            vouched(claimed, &owner),
            claimed,
            "he built it, he drives it"
        );
    }

    /// A driver's feet are in the car, so driving spends the same travel allowance walking
    /// does. A car quicker than the budget would have its own driver refused as a
    /// teleporter — the pose stream would drop them, and nobody would see them move.
    #[test]
    fn driving_stays_inside_the_travel_budget() {
        let mut travel = Budget::new(TRAVEL_RATE, TRAVEL_BURST);
        // A minute of flat out, a frame at a time, and never spent.
        for _ in 0..60 * 60 {
            travel.refill(1.0 / 60.0);
            assert!(
                travel.spend(vehicle::TOP_SPEED / 60.0),
                "a flat-out car outran its own driver's pose budget"
            );
        }
    }

    /// A peer's pile is the host's to state, and a peer's craft is the host's to grant.
    /// Either one going the wrong way is a client writing its own inventory.
    #[test]
    fn inventory_only_travels_from_the_host_and_crafts_only_towards_it() {
        let (host, peer) = (id(5), id(6));
        let full = Msg::Inventory {
            items: vec![(Item::Car, 99)],
        };
        let craft = Msg::Craft { item: Item::Car };

        assert!(!authorized(Role::Host, peer, &full), "peer stated a pile");
        assert!(authorized(Role::Host, peer, &craft), "peers may ask");
        assert!(authorized(Role::Peer { host }, host, &full));
        assert!(
            !authorized(Role::Peer { host }, host, &craft),
            "a host does not ask a peer to make things"
        );
    }

    /// Everything [`net_receive`] needs, around a real session. The shipped system runs
    /// against it, so what the tests below exercise is the path the game takes rather
    /// than a re-statement of it.
    fn playing(boot: Boot) -> bevy::ecs::world::World {
        let mut ecs = bevy::ecs::world::World::new();
        ecs.insert_resource(avatar::Palette::new(
            &mut Assets::<Mesh>::default(),
            &mut Assets::<StandardMaterial>::default(),
        ));
        ecs.insert_resource(NetRole(boot.role));
        ecs.insert_resource(Sim(World::new(boot.seed, boot.edits)));
        ecs.insert_resource(Me(Player::spawn_at(Vec3::new(0.0, 80.0, 0.0))));
        ecs.init_resource::<Chunks>();
        ecs.init_resource::<Peers>();
        ecs.init_resource::<Inventories>();
        ecs.init_resource::<Welcomed>();
        ecs.init_resource::<WorldBudget>();
        ecs.init_resource::<Time>();
        ecs.init_resource::<Messages<AppExit>>();
        ecs.insert_non_send_resource(boot.session);
        ecs
    }

    fn handle_the_network(ecs: &mut bevy::ecs::world::World) {
        ecs.run_system_once(net_receive)
            .expect("the network system");
    }

    fn who_is_here(ecs: &bevy::ecs::world::World) -> HashSet<PlayerId> {
        ecs.resource::<Peers>().0.keys().copied().collect()
    }

    /// Runs `f` until it says yes, or gives up after `patience` with `whining`.
    fn until(patience: Duration, whining: &str, mut f: impl FnMut() -> bool) -> Duration {
        let started = std::time::Instant::now();
        while !f() {
            assert!(started.elapsed() < patience, "{whining}");
            std::thread::sleep(Duration::from_millis(5));
        }
        started.elapsed()
    }

    /// The ghost one hop out, which is what #6 was: on the host a message can no longer
    /// follow its own departure, but the host announces on the stream and relays poses on
    /// datagrams, and there is no order between those two. The peer that loses that race
    /// used to re-spawn the avatar it had just despawned, and nothing ever removed it
    /// again.
    ///
    /// Three real endpoints, because the player under test is the one with *no link* to
    /// the player who left: A and B never meet, so nothing B can check tells it A is gone —
    /// only the host's word does. Both peers run the shipped [`net_receive`], and the last
    /// act is the negative control: the very pose that used to resurrect A, delivered
    /// after the departure.
    #[test]
    fn a_departure_reaches_the_peer_that_never_had_a_link() {
        const SEED: u64 = 909;
        // Discovery on a kernel-picked port: nothing here looks for a game on the LAN,
        // and taking the well-known one would fight a game somebody is playing.
        let host = crate::net::boot(None, SEED, "test-host", 0).expect("hosting");
        let host_id = host.session.me();
        let addr = host.session.addr();

        let serving = Arc::new(AtomicBool::new(true));
        let stop = serving.clone();
        let host_side = std::thread::spawn(move || {
            // The world is built on this thread because the session is a non-send
            // resource: bevy hands it out only to the thread that inserted it.
            let mut ecs = playing(host);
            while stop.load(Ordering::Relaxed) {
                handle_the_network(&mut ecs);
                std::thread::sleep(Duration::from_millis(2));
            }
            who_is_here(&ecs)
        });

        let mut a = playing(crate::net::boot(Some(addr.clone()), 0, "a", 0).expect("a joins"));
        let mut b = playing(crate::net::boot(Some(addr), 0, "b", 0).expect("b joins"));
        let a_id = a.non_send_resource::<Session>().me();
        let b_id = b.non_send_resource::<Session>().me();
        let standing_at = Pose {
            pos: [4.0, 80.0, 4.0],
            yaw: 0.0,
            pitch: 0.0,
            held: None,
            car: None,
        };

        until(Duration::from_secs(20), "b never saw a arrive", || {
            a.non_send_resource::<Session>().send(
                Target::All,
                Msg::Pose {
                    id: a_id,
                    pose: standing_at,
                },
            );
            handle_the_network(&mut a);
            handle_the_network(&mut b);
            b.resource::<Peers>().0.get(&a_id).is_some_and(|p| {
                // Not merely declared: a drawn avatar is what the ghost leaves behind.
                p.seen.is_some()
            })
        });
        assert_eq!(
            who_is_here(&b),
            HashSet::from([host_id, a_id]),
            "b's world is the host and a"
        );
        let ghost = b.resource::<Peers>().0[&a_id]
            .seen
            .as_ref()
            .expect("a is drawn")
            .body
            .root;

        drop(a);
        let took = until(Duration::from_secs(1), "a is still in b's world", || {
            handle_the_network(&mut b);
            !who_is_here(&b).contains(&a_id)
        });
        assert_eq!(
            who_is_here(&b),
            HashSet::from([host_id]),
            "nobody but the host is left, {took:?} after a went"
        );
        assert!(b.get_entity(ghost).is_err(), "a's model is still standing");

        // The negative control: the pose the host relayed a moment before the departure,
        // arriving after it. This is the message that used to bring a back for good.
        b.run_system_once(
            move |mut peers: ResMut<Peers>,
                  palette: Res<avatar::Palette>,
                  mut commands: Commands| {
                peers.posed(&mut commands, &palette, a_id, standing_at);
            },
        )
        .expect("the late pose");
        assert_eq!(
            who_is_here(&b),
            HashSet::from([host_id]),
            "a pose brought a departed player back"
        );

        serving.store(false, Ordering::Relaxed);
        let host_saw = host_side.join().expect("the host side");
        assert_eq!(host_saw, HashSet::from([b_id]), "the host's own set");
    }
}
