//! `blockgame portrait` — renders the player model alone and saves it as a PNG.
//!
//! The model is a table of numbers ([`avatar::SPACEMAN`]), and the only honest way to
//! review a change to a table of numbers is to look at the thing it builds. This spawns
//! that model through the same [`avatar::spawn`] the game uses — no second body — points
//! a camera at it, and screenshots one frame. `docs/spaceman.png` is its output.
//!
//! It needs a display. On a headless box: `xvfb-run -s '-screen 0 720x960x24'`.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::avatar;

/// Portrait-shaped, because the subject is a standing figure.
const SIZE: (u32, u32) = (720, 960);

/// The sky the game clears to, so the model is lit and backed the way it is in play.
const BACKDROP: Color = Color::srgb(0.52, 0.72, 0.95);

/// Frames rendered before the shot is taken. The first frame of a Bevy app has no
/// shadows, no prepared meshes and a half-built render graph; a few frames of settling
/// costs milliseconds and is the difference between a portrait and a grey rectangle.
const SETTLE_FRAMES: u32 = 8;

#[derive(Resource)]
struct Out(PathBuf);

#[derive(Resource, Default)]
struct Frame(u32);

pub fn run(out: PathBuf) -> anyhow::Result<()> {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "blockgame portrait".into(),
                resolution: SIZE.into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BACKDROP))
        .insert_resource(Out(out))
        .init_resource::<Frame>()
        .add_systems(Startup, setup)
        .add_systems(Update, shoot_once_settled)
        .run();
    Ok(())
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let palette = avatar::Palette::new(&mut meshes, &mut materials);
    avatar::spawn(&mut commands, &palette, Transform::default());

    // A patch of ground to stand on, so the boots read as boots and not as a float.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(6.0, 0.2, 6.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.36, 0.60, 0.26),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 40f32.to_radians(),
            ..default()
        }),
        AmbientLight {
            color: Color::WHITE,
            brightness: 420.0,
            ..default()
        },
        // Three-quarter front. The model faces -Z, and square-on would flatten it and
        // hide the backpack.
        Transform::from_xyz(-1.15, 1.45, -2.7).looking_at(Vec3::new(0.0, 0.92, 0.0), Vec3::Y),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(-4.0, 8.0, -6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Takes the shot on frame [`SETTLE_FRAMES`] and quits once it is on disk.
fn shoot_once_settled(mut frame: ResMut<Frame>, out: Res<Out>, mut commands: Commands) {
    frame.0 += 1;
    if frame.0 != SETTLE_FRAMES {
        return;
    }
    let path = out.0.clone();
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(
            |_: On<bevy::render::view::screenshot::ScreenshotCaptured>,
             mut exit: MessageWriter<AppExit>| {
                exit.write(AppExit::Success);
            },
        );
}
