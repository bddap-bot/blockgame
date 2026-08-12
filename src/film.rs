//! `blockgame film` — drives the pad and the crafting rig through a scripted session and
//! saves every frame.
//!
//! Both surfaces are things that *move*: a pad blooms open and shuts on two presses, beads
//! light one at a time, parts fly up their strings, a graph re-centres. A still picture is
//! a picture of none of that, and a paragraph about it is exactly the text these surfaces
//! exist to avoid. So the way a change to [`crate::hotbar`] or [`crate::forge`] is reviewed
//! is by watching it, and this is what makes the film.
//!
//! It presses the same [`Drum`] the pad fills from a real thumb and runs the same systems
//! the game runs, so what comes out is the prototype and not a mock-up of it. What it
//! stands in for is the host: crafts are paid straight out of the film's own pile.
//!
//! On a box with no display: `xvfb-run -s '-screen 0 1280x800x24' blockgame film`.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;

use crate::code::{self, Dir, Pad};
use crate::forge;
use crate::hotbar;
use crate::input::Drum;
use crate::inventory::{Held, Inventory, Pocket};
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

/// What the script does on one frame. Everything the film can do is something a player
/// can do with one thumb, which is what keeps the film honest.
enum Press {
    /// One key of the pad. Two of these in a row is a code.
    Key(Dir),
    /// X: open the rig on what is held, or make one while it is open.
    Craft,
    /// B: back out of the rig.
    Leave,
}

/// One scripted press, and the frame it happens on.
struct Beat(u32, Press);

/// The session the film shows, in the order a child would do it.
///
/// Written as presses rather than as outcomes on purpose: if a change to the pad or the rig
/// breaks the navigation, this script walks into a wall and the film shows it, where a
/// script that set the held item directly would keep looking correct.
///
/// It is one story in three parts. Type a code out in the world and watch the pad bloom and
/// shut. Type another, faster, to show that the second one costs exactly what the first
/// did however far apart the two things are. Then press craft, and go on typing the *same*
/// codes at the rig — because that is the whole idea: the pad does not change meaning when
/// the crafting screen comes up, it steers it.
fn script() -> Vec<Beat> {
    let mut beats = Vec::new();
    let mut at = 18;
    let code = |beats: &mut Vec<Beat>, at: &mut u32, item: Item, gap: u32| {
        let c = code::of(item);
        beats.push(Beat(*at, Press::Key(c.arm)));
        beats.push(Beat(*at + 9, Press::Key(c.key)));
        *at += gap;
    };

    // Out in the world: wood, then the rifle — opposite corners of the pad, both two
    // presses away.
    code(&mut beats, &mut at, Item::Wood, 46);
    code(&mut beats, &mut at, Item::Rifle, 44);
    // And the car, which is what we are here to build.
    code(&mut beats, &mut at, Item::Car, 30);

    // Craft: the rig unfolds on the car, under the same pad.
    beats.push(Beat(at, Press::Craft));
    at += 46;
    // Type the nail's code at the rig and it re-centres on the nail — no walking.
    code(&mut beats, &mut at, Item::Nail, 34);
    // Eight presses of craft, one nail each: the bead row on the string up to the car
    // lights one bead at a time, which is the whole idea in eight seconds.
    for _ in 0..8 {
        beats.push(Beat(at, Press::Craft));
        at += 11;
    }
    // Back to the car by its code, and build it — its ring has just gone green.
    at += 8;
    code(&mut beats, &mut at, Item::Car, 40);
    beats.push(Beat(at, Press::Craft));
    at += 40;
    // Out of the rig, back to the world, with a car in the pocket and a notch under it.
    beats.push(Beat(at, Press::Leave));
    at += 20;
    code(&mut beats, &mut at, Item::Car, 40);
    beats
}

/// How long the script runs, plus a beat to land on. The film is exactly as long as what
/// it has to show, rather than as long as a number somebody typed on the command line.
pub fn length() -> u32 {
    script().iter().map(|b| b.0).max().unwrap_or(0) + 40
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
                title: "blockgame film".into(),
                // The Deck's own panel, which is the screen the pad is laid out in — so
                // the film is the pixels a player gets and not a scaled guess at them.
                resolution: (1280u32, 800u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.52, 0.72, 0.95)))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / FPS as f64,
        )))
        .insert_resource(Pocket(stock))
        .init_resource::<Pad>()
        .init_resource::<Drum>()
        .init_resource::<Held>()
        .init_resource::<forge::Nav>()
        .init_resource::<forge::CraftRequests>()
        .insert_resource(Film {
            out,
            frame: 0,
            last: frames,
        })
        .add_systems(Startup, (scenery, hotbar::setup))
        .add_systems(
            Update,
            (
                press,
                hotbar::drum,
                open_the_rig.run_if(not(up)),
                forge::enter.run_if(resource_added::<forge::Forge>),
                (
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
                    close_the_rig,
                )
                    .chain()
                    .run_if(up),
                hotbar::redraw,
                hush,
                shoot,
            )
                .chain(),
        )
        .run();
    Ok(())
}

/// The rig is up exactly while its state resource exists — one fact, not a bool beside it
/// that could say something else.
fn up(forge: Option<Res<forge::Forge>>) -> bool {
    forge.is_some()
}

/// A patch of world for the pad to sit over and the rig to hang in front of. The rig draws
/// over whatever is behind it without wiping it, and that is the thing worth showing: both
/// of these are surfaces you meet standing where you were standing.
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
fn press(film: Res<Film>, mut drum: ResMut<Drum>, mut nav: ResMut<forge::Nav>) {
    *drum = Drum::default();
    *nav = forge::Nav::default();
    let Some(kept) = film.frame.checked_sub(SETTLE) else {
        return;
    };
    for beat in script() {
        if beat.0 != kept {
            continue;
        }
        match beat.1 {
            Press::Key(dir) => drum.press = Some(dir),
            Press::Craft => nav.craft = true,
            Press::Leave => nav.leave = true,
        }
    }
}

/// The craft button opens the rig on what is held. It also *eats* that press, exactly as
/// the game does by leaving the state: the press that opens the rig is not the press that
/// pays for something in it.
fn open_the_rig(
    mut commands: Commands,
    mut nav: ResMut<forge::Nav>,
    held: Res<Held>,
    pocket: Res<Pocket>,
) {
    if nav.craft {
        nav.craft = false;
        commands.insert_resource(forge::Forge::new(held.0, pocket.0.clone()));
    }
}

/// B, and the rig goes away — resource and geometry together, which is what makes [`up`]
/// the only thing anybody has to ask.
fn close_the_rig(commands: Commands, nav: Res<forge::Nav>, rig: Query<Entity, With<forge::Rig>>) {
    if nav.leave {
        forge::leave(commands, rig);
    }
}

/// The film has no speaker, and a queue nobody empties is a queue that grows all session.
/// What the notes would have sounded like is on the screen anyway: a press lights its
/// cluster or flashes its landing key in that key's own colour — the same fact for an eye
/// instead of an ear.
fn hush(mut pad: ResMut<Pad>) {
    pad.sounded.clear();
}

/// The film stands in for the host: a request it can afford is paid on the spot.
fn pay_for_it(mut requests: ResMut<forge::CraftRequests>, mut pocket: ResMut<Pocket>) {
    for item in requests.0.drain(..).collect::<Vec<_>>() {
        pocket.0.craft(item);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The script is a thumb, not a stage direction: every beat is one of the three
    /// buttons a player has, and no two land on the same frame — a pad that took two keys
    /// in one frame would be a code typed by nobody.
    #[test]
    fn the_script_is_something_a_thumb_could_do() {
        let beats = script();
        let mut frames: Vec<u32> = beats.iter().map(|b| b.0).collect();
        frames.sort_unstable();
        frames.dedup();
        assert_eq!(frames.len(), beats.len(), "two presses on one frame");
        assert!(length() > frames.last().copied().unwrap_or(0));
    }

    /// Playing the script through the real pad ends with a car in hand — so the film shows
    /// the codes working rather than a sequence that only looks like it does.
    #[test]
    fn the_script_really_types_its_way_to_a_car() {
        let mut pad = Pad::default();
        let mut held = Item::Grass;
        for beat in script() {
            if let Press::Key(dir) = beat.1
                && let Some(item) = pad.press(dir)
            {
                held = item;
            }
        }
        assert_eq!(held, Item::Car, "the last code typed is the car's");
        assert!(pad.arm().is_none(), "the film ends mid-code");
    }
}
