//! `blockgame reel` — plays the crafting screen to itself and saves every frame.
//!
//! [`crate::recipes`] is a screen about *movement*: dots running up a belt, a thing
//! swelling as it is made, what is missing being shaken at you. None of that survives a
//! screenshot, and none of it can be reviewed by reading the source. This drives the real
//! screen with a scripted thumb and writes out the frames, which is the only honest way to
//! look at it — the same argument [`crate::portrait`] makes about a table of numbers.
//!
//! It is the game's own systems throughout. There is no second copy of the screen here:
//! what this app leaves out is the world, the network and the hands, and what it puts in
//! is a script and a pile of stuff to spend.
//!
//! Time runs on [`TimeUpdateStrategy::ManualDuration`], so a frame is a frame however long
//! the software renderer took over it and the reel comes out the same on any machine.
//! Headless: `xvfb-run -s '-screen 0 1280x800x24' blockgame reel --out frames/`.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;

use crate::input::{Intent, MenuIntent};
use crate::inventory::{Craft, Inventories, Inventory, Whoami};
use crate::recipes;
use crate::registry::Item;
use crate::title::{Phase, Playing};

/// Frames a second the reel is played and written at.
const FPS: u32 = 30;

/// One press of the scripted thumb. Only the four this reel actually uses — a press
/// nothing presses is a press nobody has watched.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Press {
    Right,
    Down,
    /// The green button: press the key under the cursor, or build what is on the screen.
    Go,
    /// The red one: take back the last symbol.
    Back,
}

/// The reel, as a list of `(frame, press)`. Everything between two presses is the screen
/// being watched, which is most of it.
///
/// It is one story in two halves, and the halves are the two answers the screen can give.
/// First a cushion, which this player is one leaf short of: the missing leaves are shaken
/// at them and nothing is spent. Then a car, which they can afford without knowing it —
/// eight nails they never asked for are made in front of them, and then the car.
const REEL: &[(u32, Press)] = &[
    (20, Press::Go),    // the cube key: the first symbol of the code
    (42, Press::Down),  // down a row, into the things you make
    (56, Press::Right), // leaves
    (68, Press::Right), // the cushion — and the red bar under it
    (84, Press::Go),    // its graph: four leaves and a wood
    (120, Press::Go),   // refused, and told which leaf is missing
    (168, Press::Back),
    (180, Press::Back),
    (194, Press::Right), // walking the first symbol along to the car
    (206, Press::Right),
    (218, Press::Right),
    (230, Press::Right),
    (244, Press::Go),
    (260, Press::Go), // the car's graph, all the way down to the ground
    (296, Press::Go), // eight nails and a car, in one press
];

/// How long the reel runs. Long enough for the last of the nine steps to finish and be
/// looked at.
const FRAMES: u32 = 430;

/// What the player in the reel has been digging up. Enough stone and wood for a car
/// without knowing it, and one leaf short of a cushion.
fn pockets() -> Inventory {
    let mut inv = Inventory::default();
    inv.add(Item::Wood, 6);
    inv.add(Item::Stone, 10);
    inv.add(Item::Leaves, 3);
    inv
}

/// The player the reel is about. Any id will do — there is nobody else in it.
fn nobody() -> crate::net::PlayerId {
    iroh::SecretKey::from_bytes(&[7; 32]).public()
}

#[derive(Resource)]
struct Reel {
    out: PathBuf,
    frame: u32,
}

pub fn run(out: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    let me = nobody();
    let mut inventories = Inventories::default();
    inventories.0.insert(me, pockets());

    match App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "blockgame reel".into(),
                resolution: (1280u32, 800u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_nanos(
            1_000_000_000 / FPS as u64,
        )))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Reel { out, frame: 0 })
        .insert_resource(inventories)
        .insert_resource(Whoami(me))
        .init_resource::<Intent>()
        .init_resource::<MenuIntent>()
        .init_state::<Phase>()
        .add_sub_state::<Playing>()
        .add_message::<Craft>()
        .add_systems(Startup, open_on_the_crafting_screen)
        .add_systems(
            OnEnter(Phase::World),
            |mut playing: ResMut<NextState<Playing>>| playing.set(Playing::Crafting),
        )
        .add_systems(OnEnter(Playing::Crafting), recipes::open)
        .add_systems(OnExit(Playing::Crafting), recipes::close)
        .add_systems(
            Update,
            (
                // The scripted thumb stands in for `input::gather_menu_intent`, and is the
                // only thing in this app the game does not also run.
                thumb,
                recipes::drive,
                // A reel is a host with nobody in it, so it answers its own request out of
                // the one pile there is — through the same [`Inventory::build`] a real host
                // runs, not a copy of it.
                spend,
                recipes::redraw,
                shoot,
            )
                .chain()
                .run_if(in_state(Playing::Crafting)),
        )
        .run()
    {
        // Swallowing this would report a reel to whatever asked for one and leave a
        // half-written directory behind, exactly as `game::run` refuses to.
        AppExit::Success => Ok(()),
        AppExit::Error(code) => Err(anyhow::anyhow!("the reel exited with code {code}")),
    }
}

fn open_on_the_crafting_screen(mut commands: Commands, mut phase: ResMut<NextState<Phase>>) {
    commands.spawn(Camera2d);
    phase.set(Phase::World);
}

fn thumb(reel: Res<Reel>, mut menu: ResMut<MenuIntent>) {
    let pressed = REEL
        .iter()
        .find(|(frame, _)| *frame == reel.frame)
        .map(|(_, press)| *press);
    let mut out = MenuIntent::default();
    out.across = (pressed == Some(Press::Right)) as i32;
    out.step = (pressed == Some(Press::Down)) as i32;
    out.confirm = pressed == Some(Press::Go);
    out.back = pressed == Some(Press::Back);
    *menu = out;
}

fn spend(mut asked: MessageReader<Craft>, mut inventories: ResMut<Inventories>, me: Res<Whoami>) {
    for Craft(item) in asked.read() {
        inventories.0.entry(me.0).or_default().build(*item);
    }
}

/// Frames the app keeps running after the last one is asked for.
///
/// A screenshot is a GPU readback that lands a frame or two later, and writing `AppExit`
/// in the same breath as the request threw away the tail of the reel — 427 files for 430
/// frames, and nobody noticed until the last three were counted.
const DRAIN: u32 = 8;

/// Writes this frame out and stops the app once the reel has played and landed.
fn shoot(mut reel: ResMut<Reel>, mut commands: Commands, mut exit: MessageWriter<AppExit>) {
    if reel.frame < FRAMES {
        let path = reel.out.join(format!("{:05}.png", reel.frame));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
    reel.frame += 1;
    if reel.frame >= FRAMES + DRAIN {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reel has to still be playing when the last press lands, or the frames stop
    /// before the thing everybody is here to see.
    #[test]
    fn the_reel_outlives_its_script() {
        let last = REEL.last().expect("a reel presses something").0;
        assert!(FRAMES > last + FPS, "{FRAMES} frames, last press at {last}");
        let mut frames: Vec<u32> = REEL.iter().map(|(f, _)| *f).collect();
        let pressed = frames.len();
        frames.dedup();
        assert_eq!(frames.len(), pressed, "two presses on one frame");
        assert!(frames.windows(2).all(|w| w[0] < w[1]), "out of order");
    }

    /// The pockets the reel starts with are what make its two halves say different things:
    /// a car it can afford, and a cushion it cannot.
    #[test]
    fn the_reel_can_build_a_car_but_not_a_cushion() {
        let inv = pockets();
        assert!(
            inv.plan(Item::Car).short.is_empty(),
            "the car is affordable"
        );
        assert_eq!(
            inv.plan(Item::Cushion).short,
            vec![(Item::Leaves, 1)],
            "one leaf short, and the screen has to say which"
        );
    }
}
