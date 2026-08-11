//! The screen the game opens on: host a world, or join one somebody on this network is
//! already hosting.
//!
//! It exists because the players are children on Steam Decks. A join ticket is
//! sixty-four characters of hex, and a Deck in gaming mode has no keyboard to type one
//! with and nowhere to paste one from. Everything here is reachable with a d-pad and A.
//!
//! `blockgame join <TICKET>` still works and lands in the same place: a ticket on the
//! command line is a choice already made, dialed through this screen without stopping at
//! it. There is one way into a world, and this is it.

use bevy::prelude::*;
use iroh::EndpointAddr;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use crate::input::MenuIntent;
use crate::menu::{self, Panel, View};
use crate::net::{Boot, lan};

/// How often the join menu asks who is hosting. Often enough that a world started while
/// somebody is looking at this screen appears before they wander off; cheap enough that
/// it is one datagram on an idle LAN.
const ASK_EVERY: f32 = 0.7;

/// Which phase of its life the app is in.
///
/// The world exists only in [`Phase::World`], so every system that touches it is gated
/// on that — and none of them has a "there is no world yet" case to handle.
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    #[default]
    Title,
    World,
}

/// Whether the player is playing the world or reading a menu over the top of it.
///
/// A sub-state of [`Phase::World`] rather than a third [`Phase`], because entering the
/// world builds it: closing the pause menu must not be a second entry, and a state you
/// can only be in while the world exists cannot express that mistake.
#[derive(SubStates, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[source(Phase = Phase::World)]
pub enum Playing {
    #[default]
    Live,
    Paused,
    /// The crafting rig is up: the world is still there behind it, and the player is
    /// looking at what things are made of instead of at what they are standing on.
    Forge,
}

/// What the player asked for before there was a world.
#[derive(Resource)]
pub struct Start {
    pub seed: u64,
    /// What this machine's game is called on the LAN.
    pub name: String,
    /// A ticket from the command line: a choice made before the menu was drawn.
    pub join: Option<EndpointAddr>,
}

/// The booted network, on its way from this screen into the world.
///
/// Non-send, exactly as the [`crate::net::Session`] it carries is: the game holds the
/// network on the main thread and nowhere else.
#[derive(Default)]
pub struct Booted(pub Option<Boot>);

/// What the title screen is doing.
enum Doing {
    /// Reading the list, waiting to be told which world.
    Choosing,
    /// Dialing. The network boots on a thread of its own because joining waits on a host
    /// that may be slow or gone, and a window frozen for twenty seconds is
    /// indistinguishable from a crash.
    ///
    /// The lock is what lets the receiver — which is `Send` but not `Sync` — live in a
    /// resource. It is never contended: whoever polls it holds `&mut Title`.
    Dialing {
        what: String,
        outcome: Mutex<Receiver<anyhow::Result<Boot>>>,
    },
    /// It did not work, and why. Any button goes back to the list.
    Failed(String),
}

#[derive(Resource)]
pub struct Title {
    panel: Panel,
    /// `None` on a machine that could not open a socket to ask with, which can still
    /// host and can still be handed a ticket.
    search: Option<lan::Search>,
    games: Vec<lan::Game>,
    cursor: usize,
    doing: Doing,
    ask: Timer,
    /// What is currently on the screen, so it is redrawn when it changes rather than
    /// sixty times a second.
    drawn: View,
}

#[derive(Component)]
pub struct TitleCamera;

/// One row of the menu. An enum rather than a bare index, so "which row hosts" cannot
/// drift away from "which row was drawn first".
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    Host,
    Join(lan::Game),
}

/// The rows, in the order they are drawn. Hosting is first: it is the one that always
/// works, and it is what the first child to pick up a Deck wants.
fn picks(games: &[lan::Game]) -> Vec<Pick> {
    std::iter::once(Pick::Host)
        .chain(games.iter().cloned().map(Pick::Join))
        .collect()
}

fn label(pick: &Pick) -> String {
    match pick {
        Pick::Host => "Start a new world".to_string(),
        Pick::Join(game) => format!("Join {}", game.name),
    }
}

pub fn enter(mut commands: Commands, start: Res<Start>) {
    // UI needs a camera, and the world's does not exist yet.
    commands.spawn((Camera2d, TitleCamera));

    let search = lan::Search::open(lan::PORT)
        .map_err(|e| eprintln!("cannot look for games on this network: {e:#}"))
        .ok();
    let mut title = Title {
        panel: Panel::open(&mut commands),
        search,
        games: Vec::new(),
        cursor: 0,
        doing: Doing::Choosing,
        ask: Timer::from_seconds(ASK_EVERY, TimerMode::Repeating),
        drawn: View::default(),
    };
    if let Some(addr) = start.join.clone() {
        let named = format!("{}", addr.id.fmt_short());
        title.doing = dial(Some((named, addr)), start.seed, &start.name);
    }
    commands.insert_resource(title);
}

pub fn leave(mut commands: Commands, cameras: Query<Entity, With<TitleCamera>>, title: Res<Title>) {
    title.panel.close(&mut commands);
    for camera in &cameras {
        commands.entity(camera).despawn();
    }
    commands.remove_resource::<Title>();
}

/// Asks the network who is hosting, and takes in the answers.
pub fn look_for_games(mut title: ResMut<Title>, time: Res<Time>) {
    if !matches!(title.doing, Doing::Choosing) {
        return;
    }
    let due = title.ask.tick(time.delta()).just_finished();
    let Some(search) = title.search.as_mut() else {
        return;
    };
    if due {
        search.ask();
    }
    search.collect();
    let games = search.games();
    title.games = games;
}

/// Moves the cursor and acts on A.
pub fn choose(
    mut title: ResMut<Title>,
    menu_intent: Res<MenuIntent>,
    start: Res<Start>,
    mut exit: MessageWriter<AppExit>,
) {
    match title.doing {
        Doing::Failed(_) => {
            if menu_intent.confirm || menu_intent.back {
                title.doing = Doing::Choosing;
            }
            return;
        }
        // A dial in flight is not interruptible: there is a handshake in the air, and
        // the honest thing is to let it land or time out.
        Doing::Dialing { .. } => return,
        Doing::Choosing => {}
    }

    let rows = picks(&title.games);
    title.cursor = menu::step(title.cursor, rows.len(), menu_intent.step);

    if menu_intent.back {
        exit.write(AppExit::Success);
        return;
    }
    if !menu_intent.confirm {
        return;
    }
    let join = match rows.get(title.cursor) {
        Some(Pick::Join(game)) => Some((game.name.clone(), game.addr.clone())),
        // An empty list cannot happen — hosting is always a row — and if it somehow did,
        // hosting is the right thing to do with an A press on a join menu.
        Some(Pick::Host) | None => None,
    };
    title.doing = dial(join, start.seed, &start.name);
}

/// Boots the network off the main thread and hands back something to wait on.
fn dial(join: Option<(String, EndpointAddr)>, seed: u64, machine: &str) -> Doing {
    let what = match &join {
        Some((name, _)) => format!("Joining {name}..."),
        None => "Starting a world...".to_string(),
    };
    let addr = join.map(|(_, addr)| addr);
    let machine = machine.to_string();
    let (tx, outcome) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::net::boot(addr, seed, &machine, lan::PORT));
    });
    Doing::Dialing {
        what,
        outcome: Mutex::new(outcome),
    }
}

/// Hands the booted network to the world, or says why there isn't one.
pub fn wait_for_the_world(
    mut title: ResMut<Title>,
    mut booted: NonSendMut<Booted>,
    mut phase: ResMut<NextState<Phase>>,
) {
    let Doing::Dialing { outcome, .. } = &mut title.doing else {
        return;
    };
    let landed = match outcome.get_mut().expect("the dial lock").try_recv() {
        Ok(landed) => landed,
        Err(TryRecvError::Empty) => return,
        // The thread has no path to end without sending, so this is a bug rather than a
        // network problem — but saying so beats waiting for a world that is not coming.
        Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("the connection attempt vanished")),
    };
    title.doing = match landed {
        Ok(boot) => {
            booted.0 = Some(boot);
            phase.set(Phase::World);
            Doing::Choosing
        }
        Err(e) => Doing::Failed(format!("{e:#}")),
    };
}

pub fn redraw(mut title: ResMut<Title>, mut commands: Commands) {
    let view = match &title.doing {
        Doing::Choosing => View {
            heading: "blockgame".into(),
            rows: picks(&title.games).iter().map(label).collect(),
            cursor: title.cursor,
            hint: if title.games.is_empty() {
                "d-pad to choose, A to go   -   nobody else here is hosting yet".into()
            } else {
                "d-pad to choose, A to go".into()
            },
        },
        // No cursor while there is nothing to choose: `usize::MAX` is a row that does not
        // exist, which is exactly what a list of one un-pickable line wants.
        Doing::Dialing { what, .. } => View {
            heading: "blockgame".into(),
            rows: vec![what.clone()],
            cursor: usize::MAX,
            hint: String::new(),
        },
        Doing::Failed(why) => View {
            heading: "that did not work".into(),
            rows: vec![why.clone()],
            cursor: usize::MAX,
            hint: "A to go back".into(),
        },
    };
    if view == title.drawn {
        return;
    }
    title.panel.draw(&mut commands, &view);
    title.drawn = view;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str, n: u8) -> lan::Game {
        lan::Game {
            name: name.into(),
            addr: EndpointAddr::new(iroh::SecretKey::from_bytes(&[n; 32]).public()),
        }
    }

    /// Hosting is always offered and always first, so the row a child lands on when the
    /// menu opens is the one that cannot fail.
    #[test]
    fn hosting_is_the_first_row_with_or_without_company() {
        assert_eq!(picks(&[]), vec![Pick::Host]);
        let found = picks(&[game("attic", 1), game("sofa", 2)]);
        assert_eq!(found[0], Pick::Host);
        assert_eq!(
            found.iter().map(label).collect::<Vec<_>>(),
            ["Start a new world", "Join attic", "Join sofa"]
        );
    }
}
