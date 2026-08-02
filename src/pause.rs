//! The menu over the world: share this world's join ticket, or stop playing.
//!
//! Same list, same buttons and same look as the title screen ([`crate::menu`]), because
//! it is the same widget. The only thing here that is not a list is what "share" does,
//! and that lives in [`crate::ticket`].

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::game::{NetRole, grab_mouse};
use crate::hud::Notice;
use crate::input::{Intent, MenuIntent};
use crate::menu::{self, Panel, View};
use crate::net::{Role, Session};
use crate::ticket::Share;
use crate::title::Playing;

/// A row of the pause menu.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Choice {
    ShareTicket,
    Resume,
    Quit,
}

/// The rows, in the order they are drawn.
///
/// A peer is offered no ticket: the only ticket that leads into this world is the
/// host's, and offering a peer's would hand somebody a door into an empty room.
fn choices(role: Role) -> Vec<Choice> {
    match role {
        Role::Host => vec![Choice::ShareTicket, Choice::Resume, Choice::Quit],
        Role::Peer { .. } => vec![Choice::Resume, Choice::Quit],
    }
}

fn label(choice: Choice) -> &'static str {
    match choice {
        Choice::ShareTicket => "Share the join ticket",
        Choice::Resume => "Back to the world",
        Choice::Quit => "Quit",
    }
}

#[derive(Resource)]
pub struct Pause {
    panel: Panel,
    cursor: usize,
    drawn: View,
}

pub fn open(
    mut commands: Commands,
    mut intent: ResMut<Intent>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // The world keeps running while the menu is up, so a walk that was in progress has
    // to be *let go of* rather than merely stop being read — otherwise the player walks
    // off a cliff while choosing.
    *intent = Intent::default();
    grab_mouse(&mut cursor, false);
    let panel = Panel::open(&mut commands);
    commands.insert_resource(Pause {
        panel,
        cursor: 0,
        drawn: View::default(),
    });
}

pub fn close(
    mut commands: Commands,
    pause: Res<Pause>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    pause.panel.close(&mut commands);
    commands.remove_resource::<Pause>();
    grab_mouse(&mut cursor, true);
}

/// The three halves of sharing a ticket — what to share, where to put it, and where the
/// player is told about it — gathered into one parameter.
#[derive(SystemParam)]
pub struct Sharing<'w> {
    session: NonSend<'w, Session>,
    share: NonSendMut<'w, Share>,
    notice: ResMut<'w, Notice>,
}

impl Sharing<'_> {
    fn ticket(&mut self) {
        let told = self.share.put(&self.session.ticket());
        self.notice.say(told);
    }
}

pub fn drive(
    mut pause: ResMut<Pause>,
    menu_intent: Res<MenuIntent>,
    role: Res<NetRole>,
    mut sharing: Sharing,
    mut playing: ResMut<NextState<Playing>>,
    mut exit: MessageWriter<AppExit>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let rows = choices(role.0);
    pause.cursor = menu::step(pause.cursor, rows.len(), menu_intent.step);

    if menu_intent.back {
        playing.set(Playing::Live);
        return;
    }
    if !menu_intent.confirm {
        return;
    }
    match rows.get(pause.cursor) {
        Some(Choice::ShareTicket) => sharing.ticket(),
        Some(Choice::Resume) | None => playing.set(Playing::Live),
        Some(Choice::Quit) => {
            // Hand the mouse back before going. No [`close`] runs on the way out — there
            // is no state to leave — and a compositor that outlives the grab leaves the
            // pointer stuck in a window that is not there any more.
            grab_mouse(&mut cursor, false);
            exit.write(AppExit::Success);
        }
    }
}

pub fn redraw(mut pause: ResMut<Pause>, role: Res<NetRole>, mut commands: Commands) {
    let view = View {
        heading: "paused".into(),
        rows: choices(role.0)
            .into_iter()
            .map(|c| label(c).into())
            .collect(),
        cursor: pause.cursor,
        hint: "d-pad to choose, A to pick, B to go back".into(),
    };
    if view == pause.drawn {
        return;
    }
    pause.panel.draw(&mut commands, &view);
    pause.drawn = view;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> Role {
        Role::Peer {
            host: iroh::SecretKey::from_bytes(&[1; 32]).public(),
        }
    }

    /// Only a host has a ticket worth sharing, and quitting is on the menu for both —
    /// it is the only way out now that the chord is gone.
    #[test]
    fn a_peer_is_offered_no_ticket_and_both_can_leave() {
        assert_eq!(
            choices(Role::Host),
            vec![Choice::ShareTicket, Choice::Resume, Choice::Quit]
        );
        assert_eq!(choices(peer()), vec![Choice::Resume, Choice::Quit]);
        for role in [Role::Host, peer()] {
            assert!(choices(role).contains(&Choice::Quit));
        }
    }
}
