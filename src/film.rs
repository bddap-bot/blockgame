//! `blockgame craft-film` — drives the crafting rig through a scripted session and saves
//! every frame.
//!
//! The rig is a thing that *moves*: beads light one at a time, parts fly up their strings,
//! a graph re-centres. A still picture of it is a picture of none of that, and a paragraph
//! about it is exactly the text the mode exists to avoid. So the way a change to
//! [`crate::forge`] is reviewed is by watching it, and this is what makes the film.
//!
//! It presses the same [`forge::Nav`] the pad fills and runs the same systems the game
//! runs, so what comes out is the prototype and not a mock-up of it. What it stands in for
//! is the host: crafts are paid straight out of the film's own pile.
//!
//! On a box with no display: `xvfb-run -s '-screen 0 1024x640x24' blockgame craft-film`.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;

use crate::forge;
use crate::inventory::Inventory;
use crate::registry::{Block, Item};

/// Frames a second the film is shot and played at. Fixed rather than measured, so a frame
/// that took a software rasteriser half a second still advances the animation by one
/// frame's worth and the film runs at the same speed on every machine.
const FPS: u32 = 24;

/// Frames of nothing at the end: a screenshot is written asynchronously, and quitting on
/// the frame the last one was asked for loses it.
const TAIL: u32 = 12;

/// Frames run before the first one is kept. A bevy app's first frames have a half-built
/// render graph, and the rig's camera is the *second* one in this app — early on it draws
/// nothing at all, which reads as a bug in the rig rather than as the warm-up it is.
const SETTLE: u32 = 10;

#[derive(Resource)]
struct Film {
    out: PathBuf,
    frame: u32,
    last: u32,
}

/// One scripted press, and the frame it happens on.
struct Beat(u32, fn(&mut forge::Nav));

/// The session the film shows, in the order a child would do it.
///
/// It is written as presses rather than as outcomes on purpose: if a change to the rig
/// breaks the navigation, this script walks into a wall and the film shows it, where a
/// script that set the cursor directly would keep looking correct.
fn script() -> Vec<Beat> {
    let mut beats = vec![
        // Down onto the row of parts, then across to the nails.
        Beat(34, |n| n.down = 1),
        Beat(52, |n| n.across = 1),
    ];
    // Eight presses of the craft button, one nail each: the bead row on the string up to
    // the car lights one bead at a time, which is the whole idea in eight seconds.
    for i in 0..8 {
        beats.push(Beat(70 + i * 11, |n| n.craft = true));
    }
    beats.extend([
        // Back up to the car, whose ring has just gone green, and build it.
        Beat(172, |n| n.down = -1),
        Beat(190, |n| n.craft = true),
        // Then down into the parts and re-centre there: the whole tree the wood is in
        // sprouts at once, six products across the top on crossing strings, which is what
        // a graph looks like when you stand at the bottom of one.
        Beat(232, |n| n.down = 1),
        Beat(246, |n| n.focus = true),
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
    let stock = starting_stock();
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
        .insert_resource(forge::Stock(stock.clone()))
        .insert_resource(forge::Forge::new(Item::Car, stock))
        .init_resource::<forge::Nav>()
        .init_resource::<forge::CraftRequests>()
        .insert_resource(Film {
            out,
            frame: 0,
            last: frames,
        })
        .add_systems(Startup, (scenery, forge::enter))
        .add_systems(
            Update,
            (
                press,
                forge::drive,
                pay_for_it,
                forge::rebuild,
                forge::react,
                forge::beads,
                forge::notches,
                forge::nodes,
                forge::cursor,
                forge::flight,
                forge::eye,
                shoot,
            )
                .chain(),
        )
        .run();
    Ok(())
}

/// A patch of world for the rig to hang in front of. The rig draws over whatever is
/// behind it without wiping it, and that is the thing worth showing: this is a mode you
/// enter standing where you were standing.
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

/// The script's finger on the pad. Counted in *kept* frames, so the beats land where the
/// film shows them landing however long the warm-up takes.
fn press(film: Res<Film>, mut nav: ResMut<forge::Nav>) {
    *nav = forge::Nav::default();
    let Some(kept) = film.frame.checked_sub(SETTLE) else {
        return;
    };
    for beat in script() {
        if beat.0 == kept {
            beat.1(&mut nav);
        }
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
