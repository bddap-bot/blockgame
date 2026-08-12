//! `blockgame craft-film` — drives the belt and the recipe rig through a scripted session
//! and saves every frame.
//!
//! Both rigs are things that *move*: a body blooms into a constellation of everything you
//! own, beads light one at a time, parts fly up their strings, a graph re-centres. A still
//! picture of that is a picture of none of it, and a paragraph about it is exactly the text
//! these modes exist to avoid. So the way a change to [`crate::belt`] or [`crate::forge`]
//! is reviewed is by watching it, and this is what makes the film.
//!
//! It presses the same d-pad the player does — one [`Dir`] a frame, through the same
//! [`belt::press`] and the same [`forge::Nav`] — and runs the same systems the game runs,
//! so what comes out is the prototype and not a mock-up of it. What it stands in for is the
//! host: crafts are paid straight out of the film's own pile.
//!
//! On a box with no display: `xvfb-run -s '-screen 0 1024x640x24' blockgame craft-film`.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;

use crate::belt::{self, Belt};
use crate::forge;
use crate::inventory::{Held, Inventory};
use crate::registry::{Block, Item};
use crate::rig::{self, Dir};

/// Frames a second the film is shot and played at. Fixed rather than measured, so a frame
/// that took a software rasteriser half a second still advances the animation by one
/// frame's worth and the film runs at the same speed on every machine.
const FPS: u32 = 24;

/// Frames of nothing at the end: a screenshot is written asynchronously, and quitting on
/// the frame the last one was asked for loses it.
const TAIL: u32 = 12;

/// Frames run before the first one is kept. A bevy app's first frames have a half-built
/// render graph, and the rigs' cameras are the *second* and third in this app — early on
/// they draw nothing at all, which reads as a bug in the rig rather than as the warm-up it
/// is.
const SETTLE: u32 = 10;

/// Which room the film is in. The same two the game has, entered the same way — the belt
/// is where you choose a thing, and pressing craft on it takes you to what that thing is
/// made of.
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Stage {
    #[default]
    Belt,
    Forge,
}

#[derive(Resource)]
struct Film {
    out: PathBuf,
    frame: u32,
    last: u32,
}

/// One scripted press. There is no third kind: everything either spells a code, makes a
/// thing, or leaves the room.
#[derive(Clone, Copy)]
enum Press {
    Pad(Dir),
    Craft,
    Leave,
}

/// One press, and the frame it happens on.
struct Beat(u32, Press);

/// The session the film shows, in the order a child would do it.
///
/// It is written as presses rather than as outcomes on purpose: if a change to either rig
/// breaks the navigation, this script walks into a wall and the film shows it, where a
/// script that set the held item directly would keep looking correct.
fn script() -> Vec<Beat> {
    let mut beats = vec![
        // Open the belt and take a nail off it — right shoulder, then left. The cluster
        // blooms on the first press and the second one hands it over.
        Beat(22, Press::Pad(Dir::Right)),
        Beat(48, Press::Pad(Dir::Left)),
        // And now the car, which is two presses from anywhere: left shoulder, then right.
        Beat(80, Press::Pad(Dir::Left)),
        Beat(104, Press::Pad(Dir::Right)),
        // Craft on the car is the way *into* the recipe rig, on the thing in your hand.
        Beat(132, Press::Craft),
    ];
    // Eight presses of the craft button, one nail each: the bead row on the string up to
    // the car lights one bead at a time, which is the whole idea in eight seconds.
    for i in 0..8 {
        beats.push(Beat(176 + i * 11, Press::Craft));
    }
    beats.extend([
        // The car's own ring has just gone green. Build it.
        Beat(286, Press::Craft),
        // The same two presses, in here: up then down is stone, and the graph re-centres
        // on it — everything stone goes into, on crossing strings, which is what a graph
        // looks like when you stand at the bottom of one.
        Beat(330, Press::Pad(Dir::Up)),
        Beat(352, Press::Pad(Dir::Down)),
        // Back out to the world, still holding the stone the code named.
        Beat(408, Press::Leave),
        // One last bloom: the whole kit, hanging on a man who now owns a car.
        Beat(444, Press::Pad(Dir::Down)),
    ]);
    beats
}

/// What the player has when the film opens: a morning's digging, and nothing made yet.
fn starting_stock() -> Inventory {
    let mut inv = Inventory::default();
    inv.add(Item::Wood, 6);
    inv.add(Item::Stone, 10);
    inv.add(Item::Leaves, 6);
    inv
}

pub fn run(out: PathBuf, frames: u32) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "blockgame craft-film".into(),
                resolution: (1024u32, 640u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.52, 0.72, 0.95)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / FPS as f64,
        )))
        .insert_resource(forge::Stock(starting_stock()))
        .insert_resource(Held(Item::Grass))
        .init_state::<Stage>()
        .init_resource::<Belt>()
        .init_resource::<forge::Nav>()
        .init_resource::<forge::CraftRequests>()
        .insert_resource(Film {
            out,
            frame: 0,
            last: frames,
        })
        .add_systems(Startup, (scenery, rig::setup, belt::enter).chain())
        .add_systems(OnEnter(Stage::Forge), (forge::enter, shut_the_belt_camera))
        .add_systems(OnExit(Stage::Forge), fold_the_forge_away)
        .add_systems(
            Update,
            (
                press,
                belt::dress,
                belt::stations,
                belt::spin,
                belt::legs,
                belt::eye,
            )
                .chain()
                .run_if(in_state(Stage::Belt)),
        )
        .add_systems(
            Update,
            (
                forge::drive,
                pay_for_it,
                forge::rebuild,
                forge::react,
                forge::beads,
                forge::nodes,
                forge::cursor,
                forge::flight,
                forge::eye,
            )
                .chain()
                .run_if(in_state(Stage::Forge)),
        )
        // Notches are lit the same way in both rooms, and the shutter runs in both.
        .add_systems(Update, (rig::notches, shoot).chain())
        .run();
    Ok(())
}

/// A patch of world for the rigs to hang in front of. They draw over whatever is behind
/// them without wiping it, and that is the thing worth showing: these are surfaces you see
/// standing where you were standing.
fn scenery(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let paint = |materials: &mut Assets<StandardMaterial>, b: Block| {
        let [r, g, bl] = b.color();
        materials.add(StandardMaterial {
            base_color: Color::linear_rgb(r, g, bl),
            perceptual_roughness: 0.95,
            ..default()
        })
    };
    let ground = [
        paint(&mut materials, Block::Grass),
        paint(&mut materials, Block::Stone),
        paint(&mut materials, Block::Sand),
        paint(&mut materials, Block::Wood),
        paint(&mut materials, Block::Leaves),
    ];
    // A hill, built out of the same voxels the world is: enough to read as terrain from
    // behind a dark pane, and no worldgen dragged in to get it.
    for x in -18..18i32 {
        for z in -14..6i32 {
            let h = (2.0 + 3.0 * ((x as f32 * 0.31).sin() + (z as f32 * 0.24).cos())) as i32;
            for y in (h - 2).max(0)..=h {
                let skin = if y == h && h > 3 {
                    &ground[1]
                } else if y == h {
                    &ground[0]
                } else {
                    &ground[2]
                };
                commands.spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(skin.clone()),
                    Transform::from_xyz(x as f32, y as f32 - 8.0, z as f32),
                ));
            }
        }
    }
    // A couple of trees, so the horizon is not a lawn.
    for (x, z) in [(-9, -6), (7, -9), (12, -2)] {
        for y in 0..4 {
            commands.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(ground[3].clone()),
                Transform::from_xyz(x as f32, y as f32 - 3.0, z as f32),
            ));
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                commands.spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(ground[4].clone()),
                    Transform::from_xyz((x + dx) as f32, 1.0, (z + dz) as f32),
                ));
            }
        }
    }

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 55f32.to_radians(),
            far: 400.0,
            ..default()
        }),
        AmbientLight {
            color: Color::WHITE,
            brightness: 380.0,
            ..default()
        },
        Transform::from_xyz(0.0, 3.0, 16.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 11_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(-6.0, 12.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// The script's thumb on the pad. Counted in *kept* frames, so the beats land where the
/// film shows them landing however long the warm-up takes.
///
/// The presses go through the same door the player's do: a direction is a direction, and
/// which room is listening is what decides whether the thing it names lands in a hand or in
/// the middle of a graph.
#[allow(clippy::too_many_arguments)]
fn press(
    film: Res<Film>,
    stock: Res<forge::Stock>,
    stage: Res<State<Stage>>,
    mut nav: ResMut<forge::Nav>,
    mut belt: ResMut<Belt>,
    mut held: ResMut<Held>,
    mut next: ResMut<NextState<Stage>>,
    mut commands: Commands,
) {
    *nav = forge::Nav::default();
    let Some(kept) = film.frame.checked_sub(SETTLE) else {
        return;
    };
    for beat in script() {
        if beat.0 != kept {
            continue;
        }
        match (*stage.get(), beat.1) {
            (Stage::Belt, Press::Pad(dir)) => {
                belt::press(Some(dir), &mut belt, &mut held);
            }
            // Craft in the world is the door into the recipe rig, on whatever is in hand.
            (Stage::Belt, Press::Craft) => {
                commands.insert_resource(forge::Forge::new(held.0, stock.0.clone()));
                next.set(Stage::Forge);
            }
            (Stage::Belt, Press::Leave) => {}
            (Stage::Forge, Press::Pad(dir)) => nav.dir = Some(dir),
            (Stage::Forge, Press::Craft) => nav.craft = true,
            (Stage::Forge, Press::Leave) => next.set(Stage::Belt),
        }
    }
}

/// One room at a time, exactly as the game does it.
fn shut_the_belt_camera(mut cameras: Query<&mut Camera, With<belt::Rig>>) {
    for mut camera in &mut cameras {
        camera.is_active = false;
    }
}

fn fold_the_forge_away(
    mut commands: Commands,
    rig: Query<Entity, With<forge::Rig>>,
    mut cameras: Query<&mut Camera, With<belt::Rig>>,
) {
    forge::leave(commands.reborrow(), rig);
    for mut camera in &mut cameras {
        camera.is_active = true;
    }
}

/// The film stands in for the host: a request it can afford is paid on the spot.
fn pay_for_it(mut requests: ResMut<forge::CraftRequests>, mut stock: ResMut<forge::Stock>) {
    for item in requests.0.drain(..).collect::<Vec<_>>() {
        stock.0.craft(item);
    }
}

fn shoot(mut film: ResMut<Film>, mut commands: Commands, mut exit: MessageWriter<AppExit>) {
    let frame = film.frame;
    film.frame += 1;
    if frame < SETTLE {
        return;
    }
    let kept = frame - SETTLE;
    if kept < film.last {
        let path = film.out.join(format!("frame_{kept:05}.png"));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    } else if kept >= film.last + TAIL {
        exit.write(AppExit::Success);
    }
}
