//! The one list a thumb can drive: a heading, some rows, a cursor, and a hint line.
//!
//! The join menu and the pause menu are both this. Two screens that each grew their own
//! list would drift — different fonts, different button to confirm, a cursor that wraps
//! in one and stops in the other — and the players here are children who learn one
//! screen and expect the next to behave.
//!
//! Sized for the Deck's 1280x800 panel held at arm's length: [`ROW_FONT`] is the
//! smallest text on either menu, and the menus are the only text in the game.

use bevy::prelude::*;

/// Heading text. Big enough to read across a room.
const HEADING_FONT: f32 = 46.0;
/// One row of the list.
const ROW_FONT: f32 = 30.0;
/// The line under the list that says which button does what.
const HINT_FONT: f32 = 22.0;
/// Padding around a row, which is also the target a cursor has to land on.
const ROW_PAD: f32 = 10.0;

const SELECTED: Color = Color::srgb(1.0, 0.85, 0.35);
const UNSELECTED: Color = Color::srgb(0.86, 0.88, 0.92);
const HINT: Color = Color::srgb(0.62, 0.66, 0.72);
/// The panel behind the list. Nearly opaque: a paused world is a sunlit voxel forest,
/// and at anything less the trees read straight through a row of text. What is left of
/// the world through it is enough to see the game is still there.
const PANEL: Color = Color::srgba(0.05, 0.06, 0.09, 0.96);
const SELECTED_ROW: Color = Color::srgba(1.0, 0.85, 0.35, 0.16);

/// What a panel shows. Compared against what is already drawn, so the UI is rebuilt when
/// it changes and never merely because a frame went by.
#[derive(Default, PartialEq, Clone, Debug)]
pub struct View {
    pub heading: String,
    pub rows: Vec<String>,
    /// Which row the cursor is on. Out of range draws no cursor, which is what an empty
    /// list wants.
    pub cursor: usize,
    pub hint: String,
}

/// A panel on screen. Held by whoever opened it, and closed by dropping the entity.
#[derive(Debug, Clone, Copy)]
pub struct Panel(Entity);

impl Panel {
    /// Spawns an empty panel filling the screen. Nothing is drawn until [`Panel::draw`].
    pub fn open(commands: &mut Commands) -> Self {
        let root = commands
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(12.0),
                    ..default()
                },
                // Above the pad and the crosshair, which the pause menu covers.
                GlobalZIndex(10),
            ))
            .id();
        Panel(root)
    }

    /// Replaces everything in the panel. Rebuilding beats patching at this size: a list
    /// of at most a handful of rows costs nothing to respawn, and there is then no way
    /// for what is on the screen to disagree with the [`View`] it came from.
    pub fn draw(&self, commands: &mut Commands, view: &View) {
        commands.entity(self.0).despawn_children();
        commands.entity(self.0).with_children(|ui| {
            ui.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(48.0), Val::Px(28.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(14.0),
                    min_width: Val::Px(520.0),
                    ..default()
                },
                BackgroundColor(PANEL),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(view.heading.clone()),
                    TextFont {
                        font_size: HEADING_FONT,
                        ..default()
                    },
                    TextColor(UNSELECTED),
                ));
                for (i, row) in view.rows.iter().enumerate() {
                    let selected = i == view.cursor;
                    panel
                        .spawn((
                            Node {
                                padding: UiRect::axes(Val::Px(20.0), Val::Px(ROW_PAD)),
                                width: Val::Percent(100.0),
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            BackgroundColor(if selected { SELECTED_ROW } else { Color::NONE }),
                        ))
                        .with_children(|cell| {
                            cell.spawn((
                                // The cursor is a mark in the text as well as a
                                // highlight: a washed-out panel in sunlight loses the
                                // colour long before it loses the arrow.
                                Text::new(if selected {
                                    format!("> {row}")
                                } else {
                                    format!("  {row}")
                                }),
                                TextFont {
                                    font_size: ROW_FONT,
                                    ..default()
                                },
                                TextColor(if selected { SELECTED } else { UNSELECTED }),
                            ));
                        });
                }
                panel.spawn((
                    Text::new(view.hint.clone()),
                    TextFont {
                        font_size: HINT_FONT,
                        ..default()
                    },
                    TextColor(HINT),
                ));
            });
        });
    }

    /// Takes the panel off the screen.
    pub fn close(self, commands: &mut Commands) {
        commands.entity(self.0).despawn();
    }
}

/// Moves a cursor by `delta` rows, wrapping at both ends.
///
/// Wrapping rather than stopping because the list is short and the alternative is a
/// child pressing down at the bottom and concluding the menu is broken. An empty list
/// has no row to be on and stays at zero, which draws no cursor.
pub fn step(cursor: usize, rows: usize, delta: i32) -> usize {
    if rows == 0 {
        return 0;
    }
    let rows = rows as i64;
    let moved = (cursor as i64 + delta as i64).rem_euclid(rows);
    moved as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_wraps_at_both_ends() {
        assert_eq!(step(0, 3, 1), 1);
        assert_eq!(step(2, 3, 1), 0, "past the bottom is the top");
        assert_eq!(step(0, 3, -1), 2, "above the top is the bottom");
        assert_eq!(step(1, 3, 0), 1);
    }

    /// The list shrinks under the cursor whenever a host quits, so a cursor left past
    /// the end must not be a panic or a join into nothing.
    #[test]
    fn a_cursor_past_the_end_comes_back() {
        assert_eq!(step(9, 2, 0), 1);
        assert_eq!(step(9, 0, 1), 0, "an empty list has no row to be on");
    }
}
