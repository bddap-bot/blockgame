//! Everything drawn over the world in flat pixels: the crosshair, and the occasional
//! notice that fades.
//!
//! What you are carrying is not here. The hotbar used to be this file's — a row of cells
//! with the item's name in each and a line of text above saying what it cost — and the
//! players cannot read, so it is [`crate::chart`]'s now, in shapes: your hand, the stars
//! around it, and the d-pad code that reaches each one.
//!
//! What is left is the two things that are not about carrying: the reticle, and the one
//! line the game says to the player who just shared a join ticket. That line is words, and
//! it is meant for the grown-up at the keyboard reading a file path — nothing a child
//! needs to play is spelled anywhere.
//!
//! [`HudRoot`] is still how the crafting rig puts the overlay away while it is up.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// The screen the whole HUD is drawn for: the Deck's panel. Every size in this file is in
/// its pixels, and [`scale_to_screen`] scales them to the panel actually in front of the
/// player.
const DESIGN_ROWS: f32 = 800.0;

/// A top-level piece of the HUD. Marked so the crafting rig can put the whole overlay away
/// in one query: the rig says everything it has to say in shapes, and a crosshair floating
/// in the middle of a recipe graph is a reticle aimed at nothing.
#[derive(Component)]
pub struct HudRoot;

/// Shows or hides the whole overlay.
pub fn show(hud: &mut Query<&mut Visibility, With<HudRoot>>, visible: bool) {
    let want = if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in hud.iter_mut() {
        if *v != want {
            *v = want;
        }
    }
}

/// Something the game said and will stop saying.
#[derive(Component)]
pub struct NoticeText;

/// How long a notice stays up. Long enough to read a file path off a handheld held at
/// arm's length, twice.
const NOTICE_SECONDS: f32 = 9.0;

/// A line the game says to the player once — where the join ticket went, mostly — and
/// then stops saying.
///
/// Timed rather than dismissed: the player who pressed share is on their way back to the
/// world, and a message that needs acknowledging is one more thing between them and it.
#[derive(Resource, Default)]
pub struct Notice {
    text: String,
    left: f32,
}

impl Notice {
    pub fn say(&mut self, text: String) {
        self.text = text;
        self.left = NOTICE_SECONDS;
    }
}

pub fn setup(mut commands: Commands) {
    // Crosshair. Sized for the Deck's 1280x800 panel, where a 1px reticle disappears.
    commands
        .spawn((
            HudRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|ui| {
            ui.spawn((
                Text::new("+"),
                TextFont {
                    font_size: 30.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

    // Top centre, where the eye already is — and empty the rest of the time.
    commands.spawn((
        HudRoot,
        NoticeText,
        Text::new(""),
        TextFont {
            font_size: 26.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.9, 0.5)),
        TextLayout::new_with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            left: Val::Percent(15.0),
            width: Val::Percent(70.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        // Above the pause menu, which is what the player is looking at when this
        // appears.
        GlobalZIndex(20),
    ));
}

/// Keeps the HUD the size it looks on the Deck, whatever screen it is really on.
///
/// The window is borderless fullscreen, so on the TV it is 2160 rows rather than the Deck's
/// 800 and every size in this file would be a third of the height it was drawn for. Scaling
/// the UI rather than shrinking the window is what lets the world keep the pixels: the 3D
/// camera renders at the screen's real resolution either way.
pub fn scale_to_screen(window: Query<&Window, With<PrimaryWindow>>, mut ui: ResMut<UiScale>) {
    let Ok(window) = window.single() else {
        return;
    };
    let Some(want) = ui_scale(window.resolution.height()) else {
        return;
    };
    // Written only when it really changes: assigning through the `ResMut` marks `UiScale`
    // changed, and changed `UiScale` re-lays-out the whole HUD.
    if ui.0 != want {
        ui.0 = want;
    }
}

/// How much bigger this screen is than the one the HUD was drawn for. `None` for a window
/// with no height yet — a scale of zero collapses the HUD to nothing, and a window that
/// briefly reports no rows is ordinary on the way up.
fn ui_scale(rows: f32) -> Option<f32> {
    (rows > 0.0).then(|| rows / DESIGN_ROWS)
}

/// Shows the current [`Notice`] and counts it down to nothing.
///
/// The plate behind it goes away with the text: an empty black bar hanging at the top of
/// the screen for the rest of the session is worse than no notice at all.
pub fn fade_notice(
    time: Res<Time>,
    mut notice: ResMut<Notice>,
    mut shown: Query<(&mut Text, &mut BackgroundColor), With<NoticeText>>,
) {
    if notice.left > 0.0 {
        notice.left = (notice.left - time.delta_secs()).max(0.0);
    }
    let Ok((mut text, mut plate)) = shown.single_mut() else {
        return;
    };
    let (line, behind) = if notice.left > 0.0 {
        (notice.text.clone(), Color::srgba(0.0, 0.0, 0.0, 0.75))
    } else {
        (String::new(), Color::NONE)
    };
    // Written only when it really reads differently: assigning through the `Mut` marks the
    // text changed whatever it says, and changed text is re-shaped and re-laid-out.
    if text.0 != line {
        text.0 = line;
    }
    if plate.0 != behind {
        plate.0 = behind;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The HUD keeps its apparent size on every screen: unchanged on the Deck's panel it
    /// was drawn for, and 2.7x on the TV, where the same pixels would otherwise be a third
    /// of the height. A window with no rows yet gets no scale at all — zero would collapse
    /// the HUD to nothing.
    #[test]
    fn the_hud_is_scaled_to_the_screen_it_is_on() {
        assert_eq!(ui_scale(800.0), Some(1.0), "the Deck's own panel");
        assert_eq!(ui_scale(2160.0), Some(2.7), "the TV");
        assert_eq!(ui_scale(0.0), None);
    }
}
