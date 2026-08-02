//! The Bevy app: one set of systems that runs identically whether you are hosting or
//! joining.
//!
//! [`Role`] is consulted only where authority differs: [`authorized`] (whose word counts),
//! [`submit_edit`] and [`craft_on_request`] (who may change the world and who may spend a
//! pile), [`net_receive`] (who relays), and [`run`] (which of the two the player is told
//! they are). Everything else is role-blind, which is what "single player is multiplayer
//! with zero peers" has to mean if it is going to stay true.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use std::collections::{HashMap, HashSet};

use crate::avatar;
use crate::hud;
use crate::input::{Intent, PITCH_LIMIT, gather_intent};
use crate::inventory::{Held, Inventories, Inventory};
use crate::mesh::{ChunkMesh, build_chunk_mesh};
use crate::net::wire::CarPose;
use crate::net::{Boot, Event, Msg, PlayerId, Pose, Role, Session, Target};
use crate::player::{self, Motion, Player};
use crate::raycast;
use crate::registry::{Block, Class, Item, Use};
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

/// Everyone else in this world. Their position is game state, not just something to draw:
/// the host checks each peer's edits against where that peer last said it was.
#[derive(Resource, Default)]
pub struct Peers(pub HashMap<PlayerId, PeerState>);

pub struct PeerState {
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
    /// What this player is still allowed to change.
    budget: Budget,
    /// How far this player may still claim to have moved. A pose is a claim, and every
    /// rule about *where* a peer may edit is checked against it, so a client that simply
    /// says it is standing across the map would otherwise reach across the map.
    travel: Budget,
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
pub fn run(boot: Boot) -> anyhow::Result<()> {
    let Boot {
        session,
        role,
        seed,
        edits,
    } = boot;

    let world = World::new(seed, edits);
    let spawn = player::spawn_point(&world, 8, 8);

    if role == Role::Host {
        println!(
            "hosting — a friend joins with:\n\n    blockgame join {}\n",
            session.ticket()
        );
    }

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "blockgame".into(),
            resolution: (1280u32, 800u32).into(),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(SKY))
    .insert_resource(Sim(world))
    .insert_resource(Me(Player::spawn_at(spawn)))
    .insert_resource(NetRole(role))
    .init_resource::<Intent>()
    .init_resource::<Held>()
    .init_resource::<Chunks>()
    .init_resource::<Digging>()
    .init_resource::<Peers>()
    .init_resource::<Cars>()
    .init_resource::<Inventories>()
    .init_resource::<Welcomed>()
    .init_resource::<WorldBudget>()
    .insert_non_send_resource(session)
    .add_systems(Startup, (setup, hud::setup))
    .add_systems(
        Update,
        (
            gather_intent,
            park_or_ride,
            apply_intent,
            net_receive,
            draw_cars,
            target_and_edit,
            aim_zoom,
            craft_on_request,
            stream_chunks,
            remesh_dirty,
            net_send_pose,
            hud::update_status,
            hud::update_hotbar,
            quit_on_request,
        )
            .chain(),
    );

    // A bevy error exit is the process's error exit: swallowing it would report success to
    // whatever launched the game while the window died of something.
    match app.run() {
        AppExit::Success => Ok(()),
        AppExit::Error(code) => Err(anyhow::anyhow!("the game exited with code {code}")),
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

    commands.insert_resource(avatar::Palette::new(&mut meshes, &mut materials));

    commands.spawn((
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

    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = false;
    }
}

fn apply_intent(
    intent: Res<Intent>,
    time: Res<Time>,
    sim: Res<Sim>,
    mut me: ResMut<Me>,
    mut held: ResMut<Held>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    // A long frame (window drag, a chunk-mesh hitch) must not turn into a teleport
    // through the floor.
    let dt = time.delta_secs().min(0.05);
    let p = &mut me.0;

    p.pitch = (p.pitch + intent.look.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    // Driving is the whole of movement while you are in a car: the car is what moves, the
    // seat is where you are, and steering is what turns you — so the camera follows the car
    // without a second camera to follow it with. Look is still free in pitch, which is what
    // lets a passing hill be shot at from the driver's seat with no extra rule.
    if let Ride::Driving(car) = p.ride {
        let driven = vehicle::drive(&sim.0, car, intent.walk, dt);
        p.yaw = driven.yaw;
        p.pos = driven.seat();
        p.ride = Ride::Driving(driven);
        pick_item(&intent, &mut held);
        aim_camera(p, &mut camera);
        return;
    }

    p.yaw += intent.look.x;
    if intent.toggle_fly {
        p.toggle_fly();
    }

    let (forward, right) = p.move_basis();
    let speed = if p.is_flying() {
        player::FLY_SPEED
    } else if intent.sprint {
        player::SPRINT_SPEED
    } else {
        player::WALK_SPEED
    };
    let horizontal = (forward * intent.walk.y + right * intent.walk.x) * speed;

    let delta = match &mut p.motion {
        Motion::Flying => (horizontal + Vec3::Y * intent.vertical * player::FLY_SPEED) * dt,
        Motion::Walking { velocity, grounded } => {
            if intent.jump && *grounded {
                velocity.y = player::JUMP_SPEED;
            }
            velocity.y = (velocity.y - player::GRAVITY * dt).max(-player::MAX_FALL_SPEED);
            (horizontal + Vec3::Y * velocity.y) * dt
        }
    };

    let moved = player::move_and_slide(&sim.0, p.pos, delta);
    p.pos = moved.pos;
    if let Motion::Walking { velocity, grounded } = &mut p.motion {
        // Whatever stopped the move kills the speed along that axis. The ceiling half
        // matters as much as the floor: without it, a jump under an overhang leaves the
        // player pinned there until gravity wins.
        *velocity = Vec3::select(moved.blocked, Vec3::ZERO, *velocity);
        *grounded = moved.grounded;
    }

    pick_item(&intent, &mut held);
    aim_camera(p, &mut camera);
}

/// Moves the hotbar cursor. The number row reaches the first nine cells; stepping reaches
/// every cell. A key pointed past the end of the hotbar does nothing rather than wrapping
/// round to something the player was not aiming at.
fn pick_item(intent: &Intent, held: &mut Held) {
    if let Some(cell) = intent.item_pick.and_then(|slot| Item::ALL.get(slot)) {
        held.0 = *cell;
    }
    if intent.item_delta != 0 {
        held.0 = held.0.step(intent.item_delta);
    }
}

/// Puts the camera in the player's head. The one place it is moved, so a driver's view and
/// a walker's are the same view of the same eye — a car needs no camera of its own.
fn aim_camera(p: &Player, camera: &mut Query<&mut Transform, With<Camera3d>>) {
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
            p.car
                .map(|car| (*id, car_transform(car.pos.into(), car.yaw)))
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
    mut camera: Query<&mut Projection, With<Camera3d>>,
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
fn craft_on_request(
    intent: Res<Intent>,
    held: Res<Held>,
    role: Res<NetRole>,
    session: NonSend<Session>,
    mut inventories: ResMut<Inventories>,
) {
    if !intent.craft {
        return;
    }
    match role.0 {
        Role::Host => submit_craft(&session, &mut inventories, session.me(), held.0),
        Role::Peer { .. } => session.send(Target::All, Msg::Craft { item: held.0 }),
    }
}

/// Where everybody is: this player, plus the last pose each peer sent. What the placement
/// rules must not put a block inside of.
fn standing(me: &Player, peers: &Peers) -> Vec<Vec3> {
    std::iter::once(me.pos)
        .chain(peers.0.values().map(|p| p.pos))
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
                    | Msg::PeerLeft { .. }
                    | Msg::Inventory { .. } => true,
                    Msg::Hello | Msg::Craft { .. } => false,
                }
        }
        Role::Host => match msg {
            Msg::Welcome { .. }
            | Msg::WorldPart { .. }
            | Msg::PeerLeft { .. }
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
                                // Nobody has said where they are yet, so nothing of theirs
                                // can be checked. Their next pose is a frame away.
                                let Some(peer) = peers.0.get_mut(&from) else {
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
                                        pos: peer.pos,
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
                            track_peer(&mut commands, &mut peers, &palette, id, pose);
                        }
                    }
                    Msg::PeerLeft { id } => {
                        welcomed.0.remove(&id);
                        forget_peer(&mut commands, &mut peers, &mut inventories, id);
                    }
                    // The handshake already happened in `net::boot`, so a second Welcome
                    // has nothing left to do.
                    Msg::Welcome { .. } => {}
                }
            }
            Event::Left(id) => match role.0 {
                Role::Host => {
                    session.send(Target::All, Msg::PeerLeft { id });
                    welcomed.0.remove(&id);
                    forget_peer(&mut commands, &mut peers, &mut inventories, id);
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

/// Records where a player is and keeps their model there, spawning it the first time.
///
/// A pose is a *claim*, and on the host it is what every proximity rule is checked
/// against, so a claim that outruns [`TRAVEL_RATE`] is refused: without that, a modified
/// client says it is standing anywhere and edits anything, and no reach check means a
/// thing. The first pose is where they say they spawned, which nothing can contradict.
fn track_peer(
    commands: &mut Commands,
    peers: &mut Peers,
    palette: &avatar::Palette,
    id: PlayerId,
    pose: Pose,
) {
    // A pose is the player's feet, which is also the model's origin — see `avatar`.
    let pos = Vec3::from(pose.pos);
    let transform = Transform {
        translation: pos,
        rotation: Quat::from_rotation_y(pose.yaw),
        ..default()
    };
    match peers.0.get_mut(&id) {
        Some(p) => {
            if !p.travel.spend(p.pos.distance(pos)) {
                return;
            }
            p.pos = pos;
            p.car = pose.car;
            commands.entity(p.body.root).insert(transform);
            // Only when it changes: a pose arrives every frame, and re-stating the same
            // cube would be sixty component writes a second per player to no effect.
            if p.held != pose.held {
                p.held = pose.held;
                avatar::show_held(commands, palette, p.body, pose.held);
            }
        }
        None => {
            let body = avatar::spawn(commands, palette, transform);
            avatar::show_held(commands, palette, body, pose.held);
            peers.0.insert(
                id,
                PeerState {
                    pos,
                    car: pose.car,
                    body,
                    held: pose.held,
                    budget: Budget::new(EDIT_RATE, EDIT_BURST),
                    travel: Budget::new(TRAVEL_RATE, TRAVEL_BURST),
                },
            );
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
    if let Some(p) = peers.0.remove(&id) {
        commands.entity(p.body.root).despawn();
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

fn quit_on_request(
    intent: Res<Intent>,
    mut exit: MessageWriter<AppExit>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !intent.quit {
        return;
    }
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
    exit.write(AppExit::Success);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        use bevy::ecs::system::RunSystemOnce;

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
        assert!(!authorized(Role::Host, peer, &Msg::PeerLeft { id: other }));
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
}
