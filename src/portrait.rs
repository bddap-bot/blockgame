//! `blockgame portrait` — renders a model alone and saves it as a PNG.
//!
//! A model is a table of numbers ([`avatar::SPACEMAN`], [`avatar::CAR`]), and the only
//! honest way to review a change to a table of numbers is to look at the thing it builds.
//! This spawns them through the same [`avatar::spawn`] and [`avatar::spawn_car`] the game
//! uses — no second body, and no second seat — points a camera at them, and screenshots one
//! frame. `docs/spaceman.png` and `docs/car.png` are its output.
//!
//! It needs a display. On a headless box: `xvfb-run -s '-screen 0 960x960x24'`.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::avatar;
use crate::registry::Item;
use crate::vehicle;

/// What is being looked at, and how to frame it.
#[derive(Clone, Copy, PartialEq, Debug, Resource)]
pub enum Subject {
    /// The spaceman on his own, holding whatever is in [`Holding`].
    Spaceman,
    /// The car with him standing at the wheel — the seat offset is half the model, so a
    /// picture of the car without a driver in it proves the wrong half.
    Car,
}

impl Subject {
    /// Portrait-shaped for a standing figure, landscape for a car: framing something twice
    /// as wide as it is tall in a portrait window is a picture of two skies.
    fn size(self) -> (u32, u32) {
        match self {
            Subject::Spaceman => (720, 960),
            Subject::Car => (1120, 800),
        }
    }

    /// Where the camera stands, and what it looks at. Three-quarter front for both: the
    /// model faces -Z, and square-on flattens it.
    fn view(self) -> (Vec3, Vec3) {
        match self {
            Subject::Spaceman => (Vec3::new(-1.15, 1.45, -2.7), Vec3::new(0.0, 0.92, 0.0)),
            Subject::Car => (Vec3::new(-3.9, 2.7, -5.4), Vec3::new(0.0, 0.80, 0.0)),
        }
    }
}

/// The sky the game clears to, so the model is lit and backed the way it is in play.
const BACKDROP: Color = Color::srgb(0.52, 0.72, 0.95);

/// Frames rendered before the shot is taken. The first frame of a Bevy app has no
/// shadows, no prepared meshes and a half-built render graph; a few frames of settling
/// costs milliseconds and is the difference between a portrait and a grey rectangle.
const SETTLE_FRAMES: u32 = 8;

#[derive(Resource)]
struct Out(PathBuf);

/// What to put in the model's hand. A held item is part of the body now, and a table of
/// numbers you cannot look at is a table of numbers nobody can review.
#[derive(Resource)]
struct Holding(Option<Item>);

#[derive(Resource, Default)]
struct Frame(u32);

pub fn run(out: PathBuf, holding: Option<Item>, subject: Subject) -> anyhow::Result<()> {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "blockgame portrait".into(),
                resolution: subject.size().into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BACKDROP))
        .insert_resource(Out(out))
        .insert_resource(Holding(holding))
        .insert_resource(subject)
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
    holding: Res<Holding>,
    subject: Res<Subject>,
) {
    let palette = avatar::Palette::new(&mut meshes, &mut materials);
    // Where the figure stands: on the ground on his own, in the driver's seat with a car
    // under him. The offset is `vehicle::SEAT` and nothing else, so this picture is the
    // proof that the seat really is on the deck.
    let stands_at = match *subject {
        Subject::Spaceman => Vec3::ZERO,
        Subject::Car => {
            avatar::spawn_car(&mut commands, &palette, Transform::default());
            vehicle::SEAT
        }
    };
    let body = avatar::spawn(
        &mut commands,
        &palette,
        Transform::from_translation(stands_at),
    );
    avatar::show_held(&mut commands, &palette, body, holding.0);

    // A patch of ground to stand on, so the boots read as boots and not as a float.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(12.0, 0.2, 12.0))),
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
        Transform::from_translation(subject.view().0).looking_at(subject.view().1, Vec3::Y),
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
