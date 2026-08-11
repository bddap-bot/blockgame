//! Everything drawn over the world: the crosshair, the hotbar, and the two lines the game
//! says — what the cursor is on, and the occasional notice that fades.
//!
//! Nothing sits permanently at the top of the screen. A status plate lived up there once,
//! carrying the host's 64-character join ticket; it is the first thing you see across a
//! living room and the last thing anybody reads, and the pause menu shares the ticket
//! on request anyway.
//!
//! The hotbar is also the crafting menu. There is no second screen to open, no grid to
//! arrange and no order to get right — you walk the cursor onto a thing and press craft,
//! and the line above the hotbar tells you what it costs and whether you can afford it.
//! That is the whole mechanism, and it is the one a six-year-old can be shown once.
//!
//! Every cell comes from [`Item::ALL`], so an item added to the registry appears here with
//! no change to this file.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::inventory::{Held, Inventories};
use crate::net::Session;
use crate::registry::{Class, HOTBAR_COLUMNS, Item};

/// Cell size, in the Deck's 1280x800 pixels. Seven of these across is 766px — wide enough
/// for the longest item name at [`CELL_FONT`], narrow enough that two rows leave the world
/// visible.
const CELL: (f32, f32) = (100.0, 50.0);
const CELL_FONT: f32 = 16.0;
/// The white ring drawn around the selected cell: a border made of padding, so the whole
/// hotbar is two kinds of coloured box and nothing else.
const RING: f32 = 3.0;

/// The screen the whole HUD is drawn for: the Deck's panel. Every size in this file is in
/// its pixels, and [`scale_to_screen`] scales them to the panel actually in front of the
/// player.
const DESIGN_ROWS: f32 = 800.0;

/// What the cursor is on and what it costs, directly above the hotbar.
#[derive(Component)]
pub struct RecipeText;

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

/// The ring around one cell — coloured when that cell is selected.
#[derive(Component)]
pub struct HotbarCell(Item);

/// The cell itself: the item's colour, its name, and how many you have.
#[derive(Component)]
pub struct HotbarLabel(Item);

pub fn setup(mut commands: Commands) {
    // Crosshair. Sized for the Deck's 1280x800 panel, where a 1px reticle disappears.
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
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

    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(16.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|hud| {
            // On its own dark plate: this line is read against whatever the player
            // happens to be looking at, and a sunlit sand cliff swallows white text.
            hud.spawn((
                RecipeText,
                Text::new(""),
                TextFont {
                    font_size: 20.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            ));
            // Rows of exactly HOTBAR_COLUMNS, so the d-pad's up and down land where they
            // look like they will.
            for row in Item::ALL.chunks(HOTBAR_COLUMNS) {
                hud.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|rank| {
                    for item in row {
                        spawn_cell(rank, *item);
                    }
                });
            }
        });
}

fn spawn_cell(parent: &mut ChildSpawnerCommands, item: Item) {
    parent
        .spawn((
            HotbarCell(item),
            Node {
                padding: UiRect::all(Val::Px(RING)),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|cell| {
            cell.spawn((
                HotbarLabel(item),
                Node {
                    width: Val::Px(CELL.0),
                    height: Val::Px(CELL.1),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                Text::new(item.name()),
                TextFont {
                    font_size: CELL_FONT,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::new_with_justify(Justify::Center),
            ));
        });
}

/// Keeps the HUD the size it looks on the Deck, whatever screen it is really on.
///
/// The window is borderless fullscreen, so on the TV it is 2160 rows rather than the Deck's
/// 800 and every size in this file would be a third of the height it was drawn for —
/// a hotbar nobody can read from a sofa. Scaling the UI rather than shrinking the window is
/// what lets the world keep the pixels: the 3D camera renders at the screen's real
/// resolution either way.
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

/// Sets a label, and only when it actually reads differently.
///
/// Assigning through the `Mut` marks the text changed whatever it says, and changed text
/// is re-shaped and re-laid-out. None of these lines change on most frames, so writing
/// them unconditionally is a text layout of the whole HUD, sixty times a second, to
/// produce the same pixels.
fn write(text: &mut Mut<Text>, line: String) {
    if text.0 != line {
        text.0 = line;
    }
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
    write(&mut text, line);
    if plate.0 != behind {
        plate.0 = behind;
    }
}

#[allow(clippy::type_complexity)]
pub fn update_hotbar(
    held: Res<Held>,
    inventories: Res<Inventories>,
    session: NonSend<Session>,
    mut cells: Query<(
        &HotbarLabel,
        &mut Text,
        &mut TextColor,
        &mut BackgroundColor,
    )>,
    mut rings: Query<(&HotbarCell, &mut BackgroundColor), Without<HotbarLabel>>,
    mut recipe: Query<(&mut Text, &mut TextColor), (With<RecipeText>, Without<HotbarLabel>)>,
) {
    let inventory = inventories.of(session.me());

    for (cell, mut text, mut color, mut background) in &mut cells {
        let n = inventory.count(cell.0);
        let [r, g, b] = cell.0.color();
        // An item you have none of is still shown, and still selectable — that is how you
        // read its recipe and decide to make one. It is just dim.
        let dim = if n > 0 { 1.0 } else { 0.28 };
        *background = BackgroundColor(Color::linear_rgb(r * dim, g * dim, b * dim));
        *color = TextColor(readable_on(r * dim, g * dim, b * dim));
        write(&mut text, format!("{}\n{n}", cell.0.name()));
    }

    for (cell, mut background) in &mut rings {
        *background = BackgroundColor(if cell.0 == held.0 {
            Color::WHITE
        } else {
            Color::NONE
        });
    }

    if let Ok((mut text, mut color)) = recipe.single_mut() {
        let item = held.0;
        let (line, tint) = if let Some(how) = using_it(item, inventory.count(item)) {
            // Once you own one, the line stops costing it and starts telling you which
            // buttons work it. A car in the pocket is no use if nobody says how to get in.
            (format!("{} - {how}", title(item)), Color::WHITE)
        } else if item.recipe().is_empty() {
            (format!("{} - dig it up", title(item)), Color::WHITE)
        } else {
            let cost: Vec<String> = item
                .recipe()
                .iter()
                .map(|(g, n)| format!("{n} {}", g.name()))
                .collect();
            let cost = cost.join(" + ");
            if inventory.can_craft(item) {
                (
                    format!("{} = {cost}   press X to make one", title(item)),
                    Color::srgb(0.55, 1.0, 0.55),
                )
            } else {
                (
                    format!("{} = {cost}   you need more", title(item)),
                    Color::srgb(0.85, 0.85, 0.85),
                )
            }
        };
        write(&mut text, line);
        *color = TextColor(tint);
    }
}

/// What a thing is and what it does: "rifle (tool, shoots 64 blocks, scoped)".
///
/// Both halves come out of the registry, so a new tool describes itself here — and what a
/// player is told the trigger does is read from the same row the trigger obeys.
fn title(item: Item) -> String {
    let what = item.class().word();
    match item.summary() {
        Some(does) => format!("{} ({what}, {does})", item.name()),
        None => format!("{} ({what})", item.name()),
    }
}

/// Which buttons work the thing, for the classes whose button is not the trigger — and
/// only once the player has one, because until then what they need told is the price.
///
/// A tool needs no row here: [`crate::registry::Use::summary`] already says what holding
/// the trigger does, and holding the trigger is the whole of using one.
fn using_it(item: Item, owned: u32) -> Option<&'static str> {
    if owned == 0 {
        return None;
    }
    match item.class() {
        Class::Vehicle { .. } => Some("L2 puts it down and picks it up, B drives"),
        // Selecting it *is* opening it, which is the part nobody would guess.
        Class::Equippable { .. } => Some("pick it before you land, then steer"),
        Class::Block(_) | Class::Tool { .. } | Class::Component { .. } => None,
    }
}

/// Black or white, whichever can be read on a background of this linear colour. Sand and
/// stone are light enough that white text on them is a smear.
fn readable_on(r: f32, g: f32, b: f32) -> Color {
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.25 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every item gets exactly one cell, and no row is wider than the d-pad's row step —
    /// otherwise pressing down skips a cell nobody can then reach in a straight line.
    #[test]
    fn every_item_is_drawn_once() {
        let drawn: Vec<Item> = Item::ALL
            .chunks(HOTBAR_COLUMNS)
            .flatten()
            .copied()
            .collect();
        assert_eq!(drawn, Item::ALL, "an item lost or doubled in the layout");
        for row in Item::ALL.chunks(HOTBAR_COLUMNS) {
            assert!(!row.is_empty() && row.len() <= HOTBAR_COLUMNS);
        }
    }

    /// The whole hotbar fits the Deck's 1280px panel with room to spare. Wider than the
    /// screen and the last cells are simply not there.
    #[test]
    fn the_hotbar_fits_the_deck_panel() {
        let per_cell = CELL.0 + 2.0 * RING + 4.0;
        let width = per_cell * HOTBAR_COLUMNS as f32;
        assert!(width <= 1280.0, "the hotbar is {width}px wide");
    }

    /// The longest thing the recipe line can ever say still fits the Deck's panel. It is
    /// one line on a dark plate, and the character it loses off the right edge is the one
    /// telling a child what the trigger in their hand does.
    #[test]
    fn the_recipe_line_fits_the_deck_panel() {
        // 20px of bevy's default font advances well under 12px a character; 100 of them is
        // 1200px at that pessimistic width, inside the 1280 panel.
        const BUDGET: usize = 100;
        for item in Item::ALL {
            let cost: Vec<String> = item
                .recipe()
                .iter()
                .map(|(g, n)| format!("{n} {}", g.name()))
                .collect();
            let line = format!(
                "{} = {}   press X to make one",
                title(*item),
                cost.join(" + ")
            );
            assert!(line.len() <= BUDGET, "{} chars: {line}", line.len());
        }
    }

    /// A tool's row in the registry is what the player is told about it, so the words and
    /// the behaviour cannot drift. The two things that change the physics have to say so
    /// too: a cushion that reads like any other block is a recipe nobody has a reason for.
    #[test]
    fn the_line_says_what_the_trigger_does() {
        assert_eq!(title(Item::Rifle), "rifle (tool, shoots 64 blocks, scoped)");
        assert_eq!(title(Item::Drill), "drill (tool, digs 4x)");
        assert_eq!(title(Item::Nail), "nail (part)", "a nail does nothing");
        assert_eq!(title(Item::Cushion), "cushion (block, soft to land on)");
        assert_eq!(
            title(Item::Parachute),
            "parachute (wearable, floats down and steers)"
        );
        assert_eq!(title(Item::Dirt), "dirt (block)", "dirt is just dirt");
    }

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

    /// Light cells get dark text, dark cells get light text. Sand is the case that bites:
    /// it is the brightest thing in the registry, and white on it is a smear.
    #[test]
    fn text_contrasts_with_the_cell_under_it() {
        let text_on = |item: Item| {
            let [r, g, b] = item.color();
            readable_on(r, g, b)
        };
        assert_eq!(text_on(Item::Sand), Color::BLACK);
        assert_eq!(text_on(Item::Grass), Color::BLACK);
        assert_eq!(text_on(Item::Handgun), Color::WHITE);
        assert_eq!(text_on(Item::Rifle), Color::WHITE);
        // A dimmed empty cell is darker than the same cell full, and must not keep the
        // dark text that suited it bright.
        assert_eq!(
            readable_on(0.80 * 0.28, 0.74 * 0.28, 0.48 * 0.28),
            Color::WHITE
        );
    }
}
