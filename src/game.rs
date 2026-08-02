//! The Bevy app: one set of systems that runs identically whether you are hosting or
//! joining.
//!
//! [`Role`] is consulted only where authority differs: [`authorized`] (whose word counts),
//! [`submit_edit`] (who may change the world), [`net_receive`] (who relays), and the
//! status line. Everything else is role-blind, which is what "single player is
//! multiplayer with zero peers" has to mean if it is going to stay true.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use std::collections::{HashMap, HashSet};

use crate::avatar;
use crate::input::{Intent, PITCH_LIMIT, gather_intent};
use crate::mesh::build_chunk_mesh;
use crate::net::{Boot, Event, Msg, PlayerId, Pose, Role, Session, Target};
use crate::player::{self, Held, Player};
use crate::raycast;
use crate::registry::{Block, Item};
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
/// How far the player can reach to break or place, in blocks.
const REACH: f32 = 6.0;
/// Slack the host allows on [`REACH`] when checking a peer's edit. The host checks
/// against the pose that peer last sent, which is up to a round trip old; without the
/// slack a peer editing while sprinting would have legitimate edits refused.
const REACH_LAG: f32 = 1.5;

const SKY: Color = Color::srgb(0.52, 0.72, 0.95);

#[derive(Resource)]
struct Sim(World);

#[derive(Resource)]
struct Me(Player);

#[derive(Resource, Clone, Copy)]
struct NetRole(Role);

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
struct Peers(HashMap<PlayerId, PeerState>);

struct PeerState {
    /// Feet position from this player's most recent [`Msg::Pose`].
    pos: Vec3,
    /// The model drawing them.
    avatar: Entity,
    /// What this player is still allowed to change.
    budget: EditBudget,
}

/// Edits one peer may ask for per second, sustained.
const EDIT_RATE: f32 = 10.0;
/// Edits a peer may ask for at once. A person breaking blocks does one per click, so this
/// is slack for a laggy burst arriving together, not a play style.
const EDIT_BURST: f32 = 40.0;

/// A peer's allowance to change the world: a token bucket.
///
/// Every edit is permanent world state that the host stores and ships to every future
/// joiner, so an unlimited edit rate is an unlimited claim on the host's memory and on
/// everyone's join time. This is where that claim is bounded.
struct EditBudget {
    tokens: f32,
}

impl EditBudget {
    fn new() -> Self {
        EditBudget { tokens: EDIT_BURST }
    }

    fn refill(&mut self, dt: f32) {
        self.tokens = (self.tokens + EDIT_RATE * dt).min(EDIT_BURST);
    }

    /// Takes one edit's worth, or reports that this peer is asking too fast.
    fn spend(&mut self) -> bool {
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[derive(Component)]
struct ChunkTag;

#[derive(Component)]
struct Highlight;

#[derive(Component)]
struct StatusText;

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
    .init_resource::<Peers>()
    .insert_non_send_resource(session)
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            gather_intent,
            apply_intent,
            net_receive,
            target_and_edit,
            stream_chunks,
            remesh_dirty,
            net_send_pose,
            update_status,
            quit_on_request,
        )
            .chain(),
    );

    app.run();
    Ok(())
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
            fov: 75f32.to_radians(),
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

    // Crosshair. Sized for the Deck's 1280x800 panel, where a 1px reticle disappears.
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|ui| {
            ui.spawn((
                Text::new("+"),
                TextFont {
                    font_size: 30.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

    commands.spawn((
        StatusText,
        Text::new(""),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.0),
            bottom: Val::Px(16.0),
            ..default()
        },
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

    p.yaw += intent.look.x;
    p.pitch = (p.pitch + intent.look.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    if intent.toggle_fly {
        p.flying = !p.flying;
        p.velocity = Vec3::ZERO;
    }

    let (forward, right) = p.move_basis();
    let speed = if p.flying {
        player::FLY_SPEED
    } else if intent.sprint {
        player::SPRINT_SPEED
    } else {
        player::WALK_SPEED
    };
    let horizontal = (forward * intent.walk.y + right * intent.walk.x) * speed;

    let delta = if p.flying {
        p.velocity = Vec3::ZERO;
        (horizontal + Vec3::Y * intent.vertical * player::FLY_SPEED) * dt
    } else {
        if intent.jump && p.grounded {
            p.velocity.y = player::JUMP_SPEED;
        }
        p.velocity.y = (p.velocity.y - player::GRAVITY * dt).max(-player::MAX_FALL_SPEED);
        (horizontal + Vec3::Y * p.velocity.y) * dt
    };

    let expected_y = p.pos.y + delta.y;
    let (pos, grounded) = player::move_and_slide(&sim.0, p.pos, delta);
    // Hitting a floor or a ceiling kills the vertical velocity; without the ceiling half,
    // a jump under an overhang leaves the player pinned there until gravity wins.
    if (pos.y - expected_y).abs() > 1e-4 {
        p.velocity.y = 0.0;
    }
    p.pos = pos;
    p.grounded = grounded && !p.flying;

    if let Some(slot) = intent.item_pick
        && slot < Item::count()
    {
        held.0 = Item::from_slot(slot);
    }
    if intent.item_delta != 0 {
        let n = Item::count() as i32;
        let slot = (held.0.slot() as i32 + intent.item_delta).rem_euclid(n);
        held.0 = Item::from_slot(slot as usize);
    }

    if let Ok(mut t) = camera.single_mut() {
        t.translation = p.eye();
        t.rotation = Quat::from_euler(EulerRot::YXZ, p.yaw, p.pitch, 0.0);
    }
}

/// Does a block at `pos` overlap the box of a player standing at `actor`? Placing there
/// would trap them inside the world.
fn would_trap(actor: Vec3, pos: BlockPos) -> bool {
    let min = actor - Vec3::new(player::HALF_WIDTH, 0.0, player::HALF_WIDTH);
    let max = actor + Vec3::new(player::HALF_WIDTH, player::HEIGHT, player::HALF_WIDTH);
    let b = pos.corner();
    (min.x < b.x + 1.0 && max.x > b.x)
        && (min.y < b.y + 1.0 && max.y > b.y)
        && (min.z < b.z + 1.0 && max.z > b.z)
}

/// Is the block within arm's length of a player standing at `actor`? Measured eye to
/// nearest point of the block, which is the same thing the client's raycast bounds.
fn within_reach(actor: Vec3, pos: BlockPos) -> bool {
    let eye = actor + Vec3::Y * player::EYE_HEIGHT;
    let corner = pos.corner();
    eye.distance(eye.clamp(corner, corner + Vec3::ONE)) <= REACH + REACH_LAG
}

/// Every rule an edit must satisfy, in one place, applied by the host to its own player
/// and to every peer alike.
///
/// This is what makes host-authoritative *real*: none of it is taken on the asker's
/// word, so a modified client can ask for anything and still cannot break bedrock, reach
/// across the map, overwrite a block that is already there, or wall a player in.
fn edit_is_legal(world: &World, actor: Vec3, pos: BlockPos, block: Block) -> bool {
    if !within_reach(actor, pos) {
        return false;
    }
    match block {
        // Breaking. Bedrock is the floor of the world; breaking it drops you out of it.
        Block::Air => world.block(pos) != Block::Bedrock,
        // Placing: only a voxel some item actually places, only into empty space, and
        // never inside the player doing it.
        _ => block.placeable() && world.block(pos) == Block::Air && !would_trap(actor, pos),
    }
}

/// The host's half of an edit: apply a request if the rules allow it, and say whether
/// the world changed — which is exactly when the host announces it.
fn apply_if_legal(
    sim: &mut World,
    chunks: &mut Chunks,
    actor: Vec3,
    pos: BlockPos,
    block: Block,
) -> bool {
    if !edit_is_legal(sim, actor, pos, block) || !sim.set_block(pos, block) {
        return false;
    }
    mark_dirty(chunks, pos);
    true
}

/// A block change on its way into the world.
enum Edit {
    /// Somebody wants this change — the local player, or a peer over the wire. `actor` is
    /// that player's feet, and every rule is checked against it.
    Request {
        actor: Vec3,
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
/// it, and announces it. A peer only asks — it does not touch its own world, so what it
/// sees is always what the host said, never a local guess that has to be rolled back.
fn submit_edit(role: Role, session: &Session, sim: &mut World, chunks: &mut Chunks, edit: Edit) {
    match (role, edit) {
        (Role::Host, Edit::Request { actor, pos, block }) => {
            if apply_if_legal(sim, chunks, actor, pos, block) {
                session.send(Target::All, Msg::Edit { pos, block });
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

#[allow(clippy::too_many_arguments)]
fn target_and_edit(
    intent: Res<Intent>,
    me: Res<Me>,
    role: Res<NetRole>,
    held: Res<Held>,
    session: NonSend<Session>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut highlight: Query<(&mut Transform, &mut Visibility), With<Highlight>>,
) {
    let hit = raycast::cast(&sim.0, me.0.eye(), me.0.look_dir(), REACH);

    if let Ok((mut t, mut vis)) = highlight.single_mut() {
        match hit {
            Some(h) => {
                t.translation = h.block.center();
                *vis = Visibility::Visible;
            }
            None => *vis = Visibility::Hidden,
        }
    }

    let Some(hit) = hit else { return };

    // What the button asks for. Whether it is allowed is the host's call, not this
    // client's — see `edit_is_legal`.
    let edit = if intent.break_block {
        Some((hit.block, Block::Air))
    } else if intent.place_block {
        held.0.places().map(|block| (hit.adjacent(), block))
    } else {
        None
    };

    if let Some((pos, block)) = edit {
        let request = Edit::Request {
            actor: me.0.pos,
            pos,
            block,
        };
        submit_edit(role.0, &session, &mut sim.0, &mut chunks, request);
    }
}

/// Is `from` entitled to say this? The whole trust boundary, in one pure function.
///
/// A peer has exactly one link — to the host — so it believes that endpoint and nobody
/// else. The host talks to many peers, and a peer may only ever speak for itself: ask for
/// an edit, report its own pose, say hello. Everything else is the host's word, and a
/// peer sending one is claiming to be the host.
fn authorized(role: Role, from: PlayerId, msg: &Msg) -> bool {
    match role {
        Role::Peer { host } => from == host,
        Role::Host => match msg {
            Msg::Welcome { .. } | Msg::WorldPart { .. } | Msg::PeerLeft { .. } => false,
            Msg::Pose { id, .. } => *id == from,
            Msg::Hello | Msg::Edit { .. } => true,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn net_receive(
    role: Res<NetRole>,
    time: Res<Time>,
    mut session: NonSendMut<Session>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut peers: ResMut<Peers>,
    palette: Res<avatar::Palette>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let me = session.me();
    let dt = time.delta_secs();
    for peer in peers.0.values_mut() {
        peer.budget.refill(dt);
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
                        if role.0 == Role::Host {
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
                                // theirs to fill at machine speed.
                                if !peer.budget.spend() {
                                    continue;
                                }
                                Edit::Request {
                                    actor: peer.pos,
                                    pos,
                                    block,
                                }
                            }
                            // `authorized` proved this came from the host, and the host
                            // is the truth.
                            Role::Peer { .. } => Edit::Announcement { pos, block },
                        };
                        submit_edit(role.0, &session, &mut sim.0, &mut chunks, edit);
                    }
                    Msg::Pose { id, pose } => {
                        // The host is the only relay.
                        if role.0 == Role::Host {
                            session.send(Target::All, Msg::Pose { id, pose });
                        }
                        if id != me {
                            track_peer(&mut commands, &mut peers, &palette, id, pose);
                        }
                    }
                    Msg::PeerLeft { id } => forget_peer(&mut commands, &mut peers, id),
                    // The handshake already happened in `net::boot`, so a second Welcome
                    // has nothing left to do.
                    Msg::Welcome { .. } => {}
                }
            }
            Event::Left(id) => match role.0 {
                Role::Host => {
                    session.send(Target::All, Msg::PeerLeft { id });
                    forget_peer(&mut commands, &mut peers, id);
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

/// Records where a player is and keeps their model there, spawning it the first time.
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
            p.pos = pos;
            commands.entity(p.avatar).insert(transform);
        }
        None => {
            let avatar = avatar::spawn(commands, palette, transform);
            peers.0.insert(
                id,
                PeerState {
                    pos,
                    avatar,
                    budget: EditBudget::new(),
                },
            );
        }
    }
}

fn forget_peer(commands: &mut Commands, peers: &mut Peers, id: PlayerId) {
    if let Some(p) = peers.0.remove(&id) {
        commands.entity(p.avatar).despawn();
    }
}

fn net_send_pose(session: NonSend<Session>, me: Res<Me>) {
    session.send(
        Target::All,
        Msg::Pose {
            id: session.me(),
            pose: Pose {
                pos: me.0.pos.into(),
                yaw: me.0.yaw,
                pitch: me.0.pitch,
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
        if !neighbours_loaded(&sim.0, *cp) {
            continue;
        }
        refresh_chunk(
            &mut commands,
            &mut meshes,
            &material,
            &mut chunks,
            &sim.0,
            *cp,
        );
        built += 1;
    }
}

fn neighbours_loaded(world: &World, cp: ChunkPos) -> bool {
    [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .all(|(dx, dz)| world.is_loaded(ChunkPos::new(cp.x + dx, cp.z + dz)))
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
        if sim.0.is_loaded(cp) && chunks.entities.contains_key(&cp) {
            refresh_chunk(
                &mut commands,
                &mut meshes,
                &material,
                &mut chunks,
                &sim.0,
                cp,
            );
        }
    }
}

/// Rebuilds one chunk's mesh entity, creating it if it doesn't exist yet.
fn refresh_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &WorldMaterial,
    chunks: &mut Chunks,
    world: &World,
    cp: ChunkPos,
) {
    let origin = cp.origin();
    let translation = Vec3::new(origin.x as f32, 0.0, origin.z as f32);
    let entity = *chunks.entities.entry(cp).or_insert_with(|| {
        commands
            .spawn((ChunkTag, Transform::from_translation(translation)))
            .id()
    });

    match build_chunk_mesh(world, cp) {
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
}

fn update_status(
    held: Res<Held>,
    me: Res<Me>,
    role: Res<NetRole>,
    session: NonSend<Session>,
    peers: Res<Peers>,
    mut text: Query<&mut Text, With<StatusText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let mode = if me.0.flying { "flying" } else { "walking" };
    // The ticket gets its own line: 64 characters do not share a row with anything else
    // on the Deck's 1280px panel.
    let who = match role.0 {
        Role::Host => format!("join ticket:  {}", session.ticket()),
        Role::Peer { .. } => "in a friend's world".to_string(),
    };
    // ASCII only: bevy's built-in font has no glyph for a middle dot, and a missing glyph
    // draws as a tofu box.
    text.0 = format!(
        "{mode}  |  holding {}  |  {} player(s)\n{who}",
        held.0.name(),
        peers.0.len() + 1,
    );
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

    #[test]
    fn placing_a_block_inside_yourself_is_refused() {
        let feet = Vec3::new(8.5, 10.0, 8.5);
        assert!(would_trap(feet, BlockPos::new(8, 10, 8)), "feet");
        assert!(would_trap(feet, BlockPos::new(8, 11, 8)), "head");
        assert!(
            !would_trap(feet, BlockPos::new(8, 9, 8)),
            "the floor is fine"
        );
        assert!(
            !would_trap(feet, BlockPos::new(8, 12, 8)),
            "above the head is fine"
        );
        assert!(!would_trap(feet, BlockPos::new(7, 10, 8)), "beside is fine");
    }

    /// A deterministic stand-in player id. Ids are public keys, so distinct seed bytes
    /// give distinct ids.
    fn id(n: u8) -> PlayerId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    /// A world with one chunk loaded, plus a spot to stand and the block under it.
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
        assert!(!apply_if_legal(
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
        assert!(!apply_if_legal(
            &mut world,
            &mut chunks,
            surface_feet,
            air,
            Block::Bedrock
        ));
        assert_eq!(world.block(air), Block::Air);
    }

    #[test]
    fn the_host_applies_a_legal_edit() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        assert!(apply_if_legal(
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
        // The far corner of the same chunk: ~10 blocks away, well past REACH.
        let far = BlockPos::new(0, world.ground_height(0, 0), 0);
        assert!(world.solid(far), "the test needs a real block over there");
        assert!(!within_reach(feet, far));
        assert!(!apply_if_legal(
            &mut world,
            &mut chunks,
            feet,
            far,
            Block::Air
        ));
        assert!(world.solid(far), "an unreachable block is untouched");
    }

    #[test]
    fn a_block_may_only_be_placed_into_empty_space() {
        let (mut world, mut chunks, feet, surface) = standing_in_a_loaded_world();
        assert!(
            !apply_if_legal(&mut world, &mut chunks, feet, surface, Block::Stone),
            "the ground is already occupied"
        );
        assert_ne!(world.block(surface), Block::Stone);

        let inside_me = BlockPos::containing(feet);
        assert!(
            !apply_if_legal(&mut world, &mut chunks, feet, inside_me, Block::Stone),
            "nobody may be walled into the world"
        );

        let beside_me = BlockPos::new(7, surface.y + 1, 8);
        assert_eq!(world.block(beside_me), Block::Air);
        assert!(apply_if_legal(
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
        let mut budget = EditBudget::new();
        let burst = (0..1000).take_while(|_| budget.spend()).count();
        assert_eq!(burst as f32, EDIT_BURST, "the burst is the whole allowance");
        assert!(!budget.spend(), "spent out");

        budget.refill(1.0);
        let after_a_second = (0..1000).take_while(|_| budget.spend()).count();
        assert_eq!(after_a_second as f32, EDIT_RATE, "a second buys EDIT_RATE");

        budget.refill(3600.0);
        assert_eq!(
            (0..1000).take_while(|_| budget.spend()).count() as f32,
            EDIT_BURST,
            "idling does not bank an hour of edits"
        );
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
}
