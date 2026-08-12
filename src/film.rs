//! `blockgame craft-film` — drives a surface through a scripted session and saves every
//! frame.
//!
//! Both of the game's wordless surfaces are things that *move*: beads light one at a time,
//! a chart blooms out of a hand and folds back into it, parts fly up their strings. A still
//! picture of either is a picture of none of that, and a paragraph about it is exactly the
//! text they exist to avoid. So the way a change to [`crate::chart`] or [`crate::forge`] is
//! reviewed is by watching it, and this is what makes the film.
//!
//! It presses the same [`chart::Reach`] and [`forge::Nav`] the pad fills and runs the same
//! systems the game runs, so what comes out is the prototype and not a mock-up of it. What
//! it stands in for is the host: crafts are paid straight out of the film's own pile.
//!
//! `--scene chart` is the whole loop in one take — punch a code, lean into the rig on that
//! star, build, come back out and watch the pile show up on the chart — because "one map,
//! two zooms" is a claim about a *transition*, and a transition can only be shown.
//!
//! On a box with no display: `xvfb-run -s '-screen 0 1024x640x24' blockgame craft-film`.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;
use clap::ValueEnum;

use crate::chart::{self, Dir};
use crate::forge;
use crate::inventory::{Held, Inventory, Stock};
use crate::registry::{Block, Item};

/// Frames a second the film is shot and played at. Fixed rather than measured, so a frame
/// that took a software rasteriser half a second still advances the animation by one
/// frame's worth and the film runs at the same speed on every machine.
const FPS: u32 = 24;

/// Frames of nothing at the end: a screenshot is written asynchronously, and quitting on
/// the frame the last one was asked for loses it.
const TAIL: u32 = 12;

/// Frames run before the first one is kept. A bevy app's first frames have a half-built
/// render graph, and these scenes hang off a *second* camera — early on it draws nothing at
/// all, which reads as a bug rather than as the warm-up it is.
const SETTLE: u32 = 10;

/// Which surface the film is of.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Scene {
    /// The crafting rig on its own, opened on a car.
    Rig,
    /// The constellation, and the rig as the place a code leads.
    Chart,
}

impl Scene {
    /// How long its script needs, in frames. Written here rather than as a flag default,
    /// because a script that is cut off halfway is a film of an unfinished thought.
    fn length(self) -> u32 {
        match self {
            Scene::Rig => 268,
            Scene::Chart => 470,
        }
    }
}

/// Which frame the film is on, and where the PNGs go.
#[derive(Resource)]
struct Film {
    out: PathBuf,
    frame: u32,
    last: u32,
}

impl Film {
    /// The frame number the script is written in: frames actually kept, so a beat lands
    /// where the film shows it landing however long the warm-up takes.
    fn kept(&self) -> Option<u32> {
        self.frame.checked_sub(SETTLE)
    }
}

/// Whether the chart film is standing on the chart or leaning into the rig — the state the
/// game calls [`crate::title::Playing`], with the same two systems sets hanging off it.
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
enum Reel {
    #[default]
    Chart,
    Rig,
}

/// One scripted press, and the frame it happens on.
struct Beat(u32, Act);

/// What a beat does — the four things a thumb can do to these two surfaces.
enum Act {
    /// One press of the d-pad: a step along the chart, and a symbol of a code.
    Reach(Dir),
    /// Craft, from the chart: lean into the rig on this star.
    Open,
    /// Craft, in the rig: make the next thing.
    Craft,
    /// B: back out to the chart.
    Leave,
}

/// What the player has when a film opens: a morning's digging, and nothing made yet.
///
/// Six wood, ten stone and six leaves is exactly a car — eight nails out of eight of the
/// stone, and the two that are left over are the two a car asks for on its own. The chart
/// film spends it to the last block.
fn starting_stock() -> Inventory {
    let mut inv = Inventory::default();
    inv.add(Item::Wood, 6);
    inv.add(Item::Stone, 10);
    inv.add(Item::Leaves, 6);
    inv
}

pub fn run(out: PathBuf, scene: Scene, frames: Option<u32>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    let frames = frames.unwrap_or_else(|| scene.length());
    let stock = starting_stock();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
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
    .insert_resource(Stock(stock.clone()))
    .init_resource::<forge::Nav>()
    .init_resource::<forge::CraftRequests>()
    .insert_resource(Film {
        out,
        frame: 0,
        last: frames,
    })
    .add_systems(Startup, scenery);

    match scene {
        Scene::Rig => rig_film(&mut app, stock),
        Scene::Chart => chart_film(&mut app),
    }
    // Registered once for both scenes and gated on the surface being there at all, so the
    // rig animates in exactly the frames a rig exists and the chart in the frames a chart
    // does — the film's version of the states the game runs these under.
    app.add_systems(
        Update,
        (
            (
                forge::drive,
                forge::rebuild,
                forge::react,
                forge::beads,
                forge::notches,
                forge::nodes,
                forge::cursor,
                forge::flight,
                forge::eye,
            )
                .chain()
                .run_if(rig_is_up),
            pay_for_it,
            (
                chart::drive.run_if(chart_has_the_pad),
                chart::linger,
                chart::react,
                chart::stars,
                chart::notches,
                chart::rings,
                chart::threads,
                chart::cursor,
                chart::chip,
                chart::eye,
            )
                .chain()
                .run_if(chart_is_up),
        )
            .chain(),
    )
    .add_systems(Last, shoot);
    app.run();
    Ok(())
}

fn rig_is_up(rig: Option<Res<forge::Forge>>) -> bool {
    rig.is_some()
}

fn chart_is_up(chart: Option<Res<chart::Chart>>) -> bool {
    chart.is_some()
}

/// The chart takes presses only when the rig is not up: one d-pad, and whatever is in
/// front of the player has it.
fn chart_has_the_pad(chart: Option<Res<chart::Chart>>, rig: Option<Res<forge::Forge>>) -> bool {
    chart.is_some() && rig.is_none()
}

/// The rig, opened on a car and walked with the pad — the film `docs/design/` has always
/// carried.
fn rig_film(app: &mut App, stock: Inventory) {
    app.insert_resource(forge::Forge::new(Item::Car, stock))
        .add_systems(Startup, forge::enter)
        .add_systems(PreUpdate, press_the_rig);
}

/// The constellation, and the rig as the place a code leads: two presses to a nail, in for
/// eight of them, out, four presses to the car, in, build it, out.
///
/// The presses land in `PreUpdate`, which is where a state set on a beat is applied before
/// the frame it was meant for — the same reason the game reads its pad there.
fn chart_film(app: &mut App) {
    app.init_state::<Reel>()
        .init_resource::<chart::Chart>()
        .init_resource::<chart::Reach>()
        .init_resource::<Held>()
        .add_systems(Startup, chart::enter)
        .add_systems(OnEnter(Reel::Rig), (open_the_rig, forge::enter).chain())
        .add_systems(OnExit(Reel::Rig), forge::leave)
        .add_systems(PreUpdate, press_the_chart)
        .add_systems(Update, put_the_chart_away);
}

/// The rig's session, in the order a child would do it.
///
/// Written as presses rather than as outcomes on purpose: if a change breaks the
/// navigation, this script walks into a wall and the film shows it, where a script that set
/// the cursor directly would keep looking correct.
type RigBeat = (u32, fn(&mut forge::Nav));

fn rig_script() -> Vec<RigBeat> {
    let mut beats: Vec<RigBeat> = vec![
        // Down onto the row of parts, then across to the nails.
        (34, |n| n.down = 1),
        (52, |n| n.across = 1),
    ];
    // Eight presses of the craft button, one nail each: the bead row on the string up to
    // the car lights one bead at a time, which is the whole idea in eight seconds.
    for i in 0..8 {
        beats.push((70 + i * 11, |n| n.craft = true));
    }
    let rest: [RigBeat; 4] = [
        // Back up to the car, whose ring has just gone green, and build it.
        (172, |n| n.down = -1),
        (190, |n| n.craft = true),
        // Then down into the parts and re-centre there: the whole tree the wood is in
        // sprouts at once, six products across the top on crossing strings, which is what
        // a graph looks like when you stand at the bottom of one.
        (232, |n| n.down = 1),
        (246, |n| n.focus = true),
    ];
    beats.extend(rest);
    beats
}

/// The script's finger on the pad, for the rig on its own.
fn press_the_rig(film: Res<Film>, mut nav: ResMut<forge::Nav>) {
    *nav = forge::Nav::default();
    let Some(kept) = film.kept() else {
        return;
    };
    for (at, press) in rig_script() {
        if at == kept {
            press(&mut nav);
        }
    }
}

/// The chart's script, in the order a child does it: punch a code, lean in, build, come
/// back out, and do it again for the thing the first one was for.
fn chart_script() -> Vec<Beat> {
    let mut beats = vec![
        // Right, up: the nail. Two presses, and the chart blooms on the first.
        Beat(24, Act::Reach(Dir::Right)),
        Beat(44, Act::Reach(Dir::Up)),
        // Its ring is green, so craft leans in.
        Beat(76, Act::Open),
    ];
    // Eight nails, one press each, watched happening on the rig.
    for i in 0..8 {
        beats.push(Beat(104 + i * 12, Act::Craft));
    }
    beats.extend([
        Beat(210, Act::Leave),
        // Back out of the nail the way we came in, through the empty hand, and out the
        // other side to the car: down, left, left, up. The lit run of threads behind the
        // cursor is that walk, and the code under the hand is where it ends up.
        Beat(248, Act::Reach(Dir::Down)),
        Beat(268, Act::Reach(Dir::Left)),
        Beat(288, Act::Reach(Dir::Left)),
        Beat(308, Act::Reach(Dir::Up)),
        // Eight nails and two stone later, the car's ring is green too.
        Beat(346, Act::Open),
        Beat(372, Act::Craft),
        Beat(410, Act::Leave),
    ]);
    beats
}

/// The script's finger on the pad.
fn press_the_chart(
    film: Res<Film>,
    mut reach: ResMut<chart::Reach>,
    mut nav: ResMut<forge::Nav>,
    mut reel: ResMut<NextState<Reel>>,
) {
    reach.press = None;
    *nav = forge::Nav::default();
    let Some(kept) = film.kept() else {
        return;
    };
    for Beat(at, act) in chart_script() {
        if at != kept {
            continue;
        }
        match act {
            Act::Reach(dir) => reach.press = Some(dir),
            Act::Open => reel.set(Reel::Rig),
            Act::Craft => nav.craft = true,
            Act::Leave => reel.set(Reel::Chart),
        }
    }
}

/// The rig opens on the star the cursor is standing on — the game's own rule, run here so
/// the film cannot show a transition the game does not have.
fn open_the_rig(world: &mut World) {
    let stock = world.resource::<Stock>().0.clone();
    let focus = world
        .resource::<Held>()
        .0
        .item()
        .unwrap_or_else(|| forge::something_to_make(&stock));
    world.insert_resource(forge::Forge::new(focus, stock));
}

/// The chart is put away while the rig is up, exactly as the game puts it away.
fn put_the_chart_away(
    reel: Res<State<Reel>>,
    mut sky: Query<&mut Visibility, With<chart::ChartRoot>>,
) {
    chart::show(&mut sky, *reel.get() == Reel::Chart);
}

/// A patch of world for these surfaces to hang in front of. They draw over whatever is
/// behind them without wiping it, and that is the thing worth showing: this is a hotbar and
/// a mode you use standing where you were standing.
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

/// The film stands in for the host: a request it can afford is paid on the spot.
fn pay_for_it(mut requests: ResMut<forge::CraftRequests>, mut stock: ResMut<Stock>) {
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
