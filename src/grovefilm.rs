//! `blockgame grove` — plays the grove to itself and saves every frame.
//!
//! The screen in [`crate::grove`] reads a pile and a set of presses and nothing else, so
//! the only thing this file supplies is a pile and a set of presses. It is the *host* half
//! of the demo, not a second copy of the screen: it answers the craft the grove asks for by
//! spending the recipe out of the pile it owns, which is what the real host does.
//!
//! The clock is stepped by hand ([`FPS`]), so how long lavapipe takes over a frame changes
//! nothing about what the film shows — the film is the same however slow the box is.
//!
//! Headless on a box with no display: `Xvfb :99 -screen 0 900x560x24` and
//! `DISPLAY=:99 VK_ICD_FILENAMES=…/lvp_icd.x86_64.json blockgame grove --out <dir>`.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;

use crate::avatar::Palette;
use crate::grove::{self, Nav, OnHand};
use crate::input::Intent;
use crate::inventory::{Held, Inventory};
use crate::registry::Item;

pub const WIDTH: u32 = 900;
pub const HEIGHT: u32 = 560;
/// Frames per second of the finished film, and the exact amount the clock is advanced by
/// each update.
pub const FPS: u32 = 16;

/// Frames rendered before the first is kept. A Bevy app's first frames have no prepared
/// meshes and a half-built render graph.
const SETTLE: u32 = 10;
/// Frames after the last one is asked for, so the screenshots in flight land on disk.
const DRAIN: u32 = 90;

/// One moment of the film: what was pressed, and what the pile gained.
struct Beat {
    /// Frames after the previous beat.
    wait: u32,
    step: IVec2,
    craft: bool,
    /// What turned up in the pocket — a block broken somewhere off screen.
    dug: &'static [(Item, u32)],
}

const fn beat(wait: u32, step: IVec2, craft: bool, dug: &'static [(Item, u32)]) -> Beat {
    Beat {
        wait,
        step,
        craft,
        dug,
    }
}

const HOLD: IVec2 = IVec2::ZERO;
const RIGHT: IVec2 = IVec2::new(1, 0);
const LEFT: IVec2 = IVec2::new(-1, 0);
const UP: IVec2 = IVec2::new(0, 1);
const DOWN: IVec2 = IVec2::new(0, -1);

/// What the pile starts as: an afternoon of digging, and not one nail.
const POCKET: &[(Item, u32)] = &[(Item::Wood, 3), (Item::Stone, 4), (Item::Leaves, 5)];

/// The film. A child walks the bottom row, climbs to the nail, makes three, goes on up to
/// the car, is told no, and then watches the car's lines fill in as the missing pieces
/// arrive — which is the one thing the whole screen exists to show.
const FILM: &[Beat] = &[
    beat(14, HOLD, false, &[]),
    // Along the ground: grass, dirt, stone. Stone is the only one of the three that makes
    // anything, and its one line up is the only line out of that stretch of the row.
    beat(8, RIGHT, false, &[]),
    beat(8, RIGHT, false, &[]),
    beat(12, UP, false, &[]),
    beat(11, HOLD, true, &[]),
    beat(8, HOLD, true, &[]),
    beat(12, HOLD, true, &[]),
    // Up out of the nail into the things nails are for: the hammer, two wood and a nail,
    // and then one step sideways to the biggest thing on the row.
    beat(16, UP, false, &[]),
    beat(16, LEFT, false, &[]),
    beat(10, HOLD, false, &[]),
    // Six wood, two stone and eight nails, and not one of the three is there. The refusal
    // is the shake, and then the car's lines fill in a bead at a time.
    beat(10, HOLD, true, &[]),
    beat(11, HOLD, false, &[(Item::Nail, 1)]),
    beat(5, HOLD, false, &[(Item::Nail, 1)]),
    beat(5, HOLD, false, &[(Item::Wood, 1)]),
    beat(5, HOLD, false, &[(Item::Nail, 1)]),
    beat(5, HOLD, false, &[(Item::Wood, 1)]),
    beat(5, HOLD, false, &[(Item::Nail, 1)]),
    beat(5, HOLD, false, &[(Item::Wood, 1)]),
    beat(5, HOLD, false, &[(Item::Nail, 1)]),
    beat(6, HOLD, false, &[(Item::Nail, 1)]),
    // The last piece. Every line into the car starts marching, and the car starts bobbing.
    beat(14, HOLD, false, &[(Item::Stone, 1)]),
    beat(11, HOLD, true, &[]),
    beat(16, HOLD, false, &[]),
    // Back down into what it was made of, which is where a child goes next.
    beat(14, DOWN, false, &[]),
    beat(14, HOLD, false, &[]),
];

/// Where the film is up to.
#[derive(Resource)]
struct Reel {
    out: PathBuf,
    frame: u32,
    /// The next beat, and the frame it fires on.
    beat: usize,
    due: u32,
    last: u32,
}

pub fn run(out: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    let last = SETTLE + FILM.iter().map(|b| b.wait).sum::<u32>();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "blockgame grove".into(),
                resolution: (WIDTH, HEIGHT).into(),
                ..default()
            }),
            ..default()
        }))
        // Every update is exactly one frame of the film, however long it really took.
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / FPS as f64,
        )))
        .insert_resource(OnHand(Inventory::from_contents(POCKET.iter().copied())))
        .insert_resource(Reel {
            out,
            frame: 0,
            beat: 0,
            due: SETTLE,
            last,
        })
        .init_resource::<Intent>()
        .init_resource::<Held>()
        .add_systems(Startup, (stage, grove::open).chain())
        .add_systems(
            Update,
            (
                cue,
                grove::navigate,
                answer,
                grove::nodes,
                grove::beads,
                grove::lines,
                grove::tallies,
                grove::marks,
                grove::fly,
                shoot,
            )
                .chain(),
        )
        .run();
    Ok(())
}

/// The one thing [`grove::open`] needs that only the game usually has.
fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(Palette::new(&mut meshes, &mut materials));
}

/// Presses the buttons the script says to press, and drops what it says was dug up.
fn cue(mut reel: ResMut<Reel>, mut nav: ResMut<Nav>, mut on_hand: ResMut<OnHand>) {
    *nav = Nav::default();
    if reel.frame != reel.due || reel.beat >= FILM.len() {
        return;
    }
    let beat = &FILM[reel.beat];
    nav.step = beat.step;
    nav.craft = beat.craft;
    for (item, n) in beat.dug {
        on_hand.0.add(*item, *n);
    }
    reel.beat += 1;
    reel.due += FILM.get(reel.beat).map_or(u32::MAX, |b| b.wait);
}

/// The host's half: the grove asked for a craft, and this is the pile answering. The same
/// [`Inventory::craft`] the real host runs, on the pile this film owns.
fn answer(intent: Res<Intent>, held: Res<Held>, mut on_hand: ResMut<OnHand>) {
    if intent.craft {
        on_hand.0.craft(held.0);
    }
}

/// Keeps one frame, and stops once the reel has run out and the writes have caught up.
fn shoot(mut reel: ResMut<Reel>, mut commands: Commands, mut exit: MessageWriter<AppExit>) {
    reel.frame += 1;
    if reel.frame > SETTLE && reel.frame <= reel.last {
        let path = reel.out.join(format!("{:05}.png", reel.frame - SETTLE));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
    if reel.frame > reel.last + DRAIN {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craftgraph::Dir;

    /// Replays the script through the real cursor and the real pile, and insists it tells
    /// the story it was written to tell.
    ///
    /// This is the film's whole correctness: the directions are written blind against a
    /// layout the graph works out for itself, so a recipe added tomorrow can move the car
    /// out from under `RIGHT, RIGHT, RIGHT` — and it says so here rather than in a GIF
    /// nobody watched to the end.
    #[test]
    fn the_script_walks_to_the_car_is_refused_and_then_builds_it() {
        let graph = crate::craftgraph::Graph::of_registry();
        let mut pile = Inventory::from_contents(POCKET.iter().copied());
        let mut at = 0;
        let mut made: Vec<Item> = Vec::new();
        let mut refused: Vec<Item> = Vec::new();

        for beat in FILM {
            for (item, n) in beat.dug {
                pile.add(*item, *n);
            }
            for dir in [
                (beat.step.x > 0).then_some(Dir::Right),
                (beat.step.x < 0).then_some(Dir::Left),
                (beat.step.y > 0).then_some(Dir::Up),
                (beat.step.y < 0).then_some(Dir::Down),
            ]
            .into_iter()
            .flatten()
            {
                at = graph.step(at, dir).unwrap_or_else(|| {
                    panic!(
                        "the script presses {dir:?} into a wall at {:?}",
                        graph.nodes()[at].item
                    )
                });
            }
            if beat.craft {
                let item = graph.nodes()[at].item;
                if pile.craft(item) {
                    made.push(item);
                } else {
                    refused.push(item);
                }
            }
        }

        assert_eq!(
            made,
            vec![Item::Nail, Item::Nail, Item::Nail, Item::Car],
            "three nails and then the car"
        );
        assert_eq!(
            refused,
            vec![Item::Car],
            "the car is refused exactly once, first"
        );
        assert_eq!(
            graph.nodes()[at].item,
            Item::Nail,
            "the film ends looking down at what most of the car was"
        );
        assert!(
            pile.count(Item::Nail) > 0,
            "the last nail was spent — a tally that empties reads as a bug, not a purchase"
        );
    }

    /// A film long enough to read and short enough to loop.
    #[test]
    fn the_film_is_a_sane_length() {
        let frames = FILM.iter().map(|b| b.wait).sum::<u32>();
        let seconds = frames as f32 / FPS as f32;
        assert!(
            (8.0..24.0).contains(&seconds),
            "{seconds}s is not a short loop"
        );
    }
}
