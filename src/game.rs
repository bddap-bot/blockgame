//! The Bevy app: one set of systems that runs identically whether you are hosting or
//! joining.
//!
//! [`Role`] is consulted in exactly two places — [`submit_edit`] (who may change the
//! world) and [`net_receive`] (who relays). Everything else is role-blind, which is what
//! "single player is multiplayer with zero peers" has to mean if it is going to stay
//! true.

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

#[derive(Resource, Default)]
struct Avatars(HashMap<PlayerId, Entity>);

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
    .init_resource::<Avatars>()
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

/// Does a block at `pos` overlap the player's box? Placing there would trap them inside
/// the world.
fn would_trap(p: &Player, pos: BlockPos) -> bool {
    let min = p.pos - Vec3::new(player::HALF_WIDTH, 0.0, player::HALF_WIDTH);
    let max = p.pos + Vec3::new(player::HALF_WIDTH, player::HEIGHT, player::HALF_WIDTH);
    let b = pos.corner();
    (min.x < b.x + 1.0 && max.x > b.x)
        && (min.y < b.y + 1.0 && max.y > b.y)
        && (min.z < b.z + 1.0 && max.z > b.z)
}

/// The ONE place a block change enters the game, from local input or from the wire.
///
/// The host is authoritative: it applies and announces. A peer only asks — it does not
/// touch its own world, so what it sees is always what the host said, never a local
/// guess that has to be rolled back.
fn submit_edit(
    role: Role,
    session: &Session,
    sim: &mut World,
    chunks: &mut Chunks,
    pos: BlockPos,
    block: Block,
) {
    match role {
        Role::Host => {
            if sim.set_block(pos, block) {
                mark_dirty(chunks, pos);
                session.send(Target::All, Msg::Edit { pos, block });
            }
        }
        Role::Peer => session.send(Target::All, Msg::Edit { pos, block }),
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

    let edit = if intent.break_block {
        // Bedrock is the floor of the world; breaking it drops you out of it.
        (sim.0.block(hit.block) != Block::Bedrock).then_some((hit.block, Block::Air))
    } else if intent.place_block {
        held.0.places().and_then(|block| {
            let pos = hit.adjacent();
            (!would_trap(&me.0, pos)).then_some((pos, block))
        })
    } else {
        None
    };

    if let Some((pos, block)) = edit {
        submit_edit(role.0, &session, &mut sim.0, &mut chunks, pos, block);
    }
}

#[allow(clippy::too_many_arguments)]
fn net_receive(
    role: Res<NetRole>,
    mut session: NonSendMut<Session>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut avatars: ResMut<Avatars>,
    palette: Res<avatar::Palette>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let me = session.me();
    let host = role.0 == Role::Host;

    for event in session.drain() {
        match event {
            Event::Message(from, Msg::Hello) if host => {
                session.send(
                    Target::One(from),
                    Msg::Welcome {
                        seed: sim.0.seed(),
                        edits: sim.0.edit_log(),
                    },
                );
            }
            Event::Message(_, Msg::Edit { pos, block }) => {
                if host {
                    // A peer's edit is an intent; applying it here is what makes it real,
                    // and the same call broadcasts it to everybody including the asker.
                    submit_edit(Role::Host, &session, &mut sim.0, &mut chunks, pos, block);
                } else if sim.0.set_block(pos, block) {
                    mark_dirty(&mut chunks, pos);
                }
            }
            Event::Message(from, Msg::Pose { id, pose }) => {
                // A peer may only speak for itself; the host is the only relay.
                if host && id != from {
                    continue;
                }
                if host {
                    session.send(Target::All, Msg::Pose { id, pose });
                }
                if id != me {
                    place_avatar(&mut commands, &mut avatars, &palette, id, pose);
                }
            }
            Event::Message(_, Msg::PeerLeft { id }) => {
                despawn_avatar(&mut commands, &mut avatars, id);
            }
            Event::Left(id) => {
                if host {
                    session.send(Target::All, Msg::PeerLeft { id });
                    despawn_avatar(&mut commands, &mut avatars, id);
                } else {
                    eprintln!("host disconnected");
                    exit.write(AppExit::Success);
                }
            }
            // A second Welcome, or Hello as a peer: the handshake already happened in
            // `net::boot`, so there is nothing left to do with it.
            Event::Message(_, Msg::Welcome { .. } | Msg::Hello) => {}
        }
    }
}

fn place_avatar(
    commands: &mut Commands,
    avatars: &mut Avatars,
    palette: &avatar::Palette,
    id: PlayerId,
    pose: Pose,
) {
    // A pose is the player's feet, which is also the model's origin — see `avatar`.
    let transform = Transform {
        translation: Vec3::from(pose.pos),
        rotation: Quat::from_rotation_y(pose.yaw),
        ..default()
    };
    match avatars.0.get(&id) {
        Some(e) => {
            commands.entity(*e).insert(transform);
        }
        None => {
            let e = avatar::spawn(commands, palette, transform);
            avatars.0.insert(id, e);
        }
    }
}

fn despawn_avatar(commands: &mut Commands, avatars: &mut Avatars, id: PlayerId) {
    if let Some(e) = avatars.0.remove(&id) {
        commands.entity(e).despawn();
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

fn stream_chunks(
    me: Res<Me>,
    material: Res<WorldMaterial>,
    mut sim: ResMut<Sim>,
    mut chunks: ResMut<Chunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let center = BlockPos::containing(me.0.pos).chunk();

    let gone: Vec<ChunkPos> = chunks
        .entities
        .keys()
        .copied()
        .filter(|c| chebyshev(*c, center) > UNLOAD_RADIUS)
        .collect();
    for cp in gone {
        if let Some(e) = chunks.entities.remove(&cp) {
            commands.entity(e).despawn();
        }
        sim.0.unload_chunk(cp);
    }

    // Nearest first, so the world fills in around the player rather than in scan order.
    let mut wanted: Vec<ChunkPos> = Vec::new();
    for dz in -(VIEW_RADIUS + 1)..=(VIEW_RADIUS + 1) {
        for dx in -(VIEW_RADIUS + 1)..=(VIEW_RADIUS + 1) {
            wanted.push(ChunkPos::new(center.x + dx, center.z + dz));
        }
    }
    wanted.sort_by_key(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2));

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
    avatars: Res<Avatars>,
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
        Role::Peer => "in a friend's world".to_string(),
    };
    // ASCII only: bevy's built-in font has no glyph for a middle dot, and a missing glyph
    // draws as a tofu box.
    text.0 = format!(
        "{mode}  |  holding {}  |  {} player(s)\n{who}",
        held.0.name(),
        avatars.0.len() + 1,
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
        let p = Player::spawn_at(Vec3::new(8.5, 10.0, 8.5));
        assert!(would_trap(&p, BlockPos::new(8, 10, 8)), "feet");
        assert!(would_trap(&p, BlockPos::new(8, 11, 8)), "head");
        assert!(!would_trap(&p, BlockPos::new(8, 9, 8)), "the floor is fine");
        assert!(
            !would_trap(&p, BlockPos::new(8, 12, 8)),
            "above the head is fine"
        );
        assert!(!would_trap(&p, BlockPos::new(7, 10, 8)), "beside is fine");
    }
}
