//! Everything drawn over the world: the crosshair, the status line, and the hotbar.
//!
//! The hotbar is also the crafting menu. There is no second screen to open, no grid to
//! arrange and no order to get right — you walk the cursor onto a thing and press craft,
//! and the line above the hotbar tells you what it costs and whether you can afford it.
//! That is the whole mechanism, and it is the one a six-year-old can be shown once.
//!
//! Every cell comes from [`Item::ALL`], so an item added to the registry appears here with
//! no change to this file.

use bevy::prelude::*;

use crate::game::{Me, NetRole, Peers};
use crate::inventory::{Held, Inventories};
use crate::net::{Role, Session};
use crate::player::{self, Condition, Player};
use crate::registry::{Class, HOTBAR_COLUMNS, Item};

/// Cell size, in the Deck's 1280x800 pixels. Seven of these across is 766px — wide enough
/// for the longest item name at [`CELL_FONT`], narrow enough that two rows leave the world
/// visible.
const CELL: (f32, f32) = (100.0, 50.0);
const CELL_FONT: f32 = 16.0;
/// The white ring drawn around the selected cell: a border made of padding, so the whole
/// hotbar is two kinds of coloured box and nothing else.
const RING: f32 = 3.0;

/// The status line, top left.
#[derive(Component)]
pub struct StatusText;

/// What the cursor is on and what it costs, directly above the hotbar.
#[derive(Component)]
pub struct RecipeText;

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

    // Top left, because the hotbar owns the bottom of the screen and the host's join
    // ticket is 64 characters wide — the two would overlap anywhere else.
    commands.spawn((
        StatusText,
        Text::new(""),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.0),
            top: Val::Px(16.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
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

pub fn update_status(
    me: Res<Me>,
    role: Res<NetRole>,
    session: NonSend<Session>,
    peers: Res<Peers>,
    mut text: Query<&mut Text, With<StatusText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    // The ticket gets its own line: 64 characters do not share a row with anything else
    // on the Deck's 1280px panel.
    let who = match role.0 {
        Role::Host => format!("join ticket:  {}", session.ticket()),
        Role::Peer { .. } => "in a friend's world".to_string(),
    };
    write(
        &mut text,
        format!("{}\n{who}", status(&me.0, peers.0.len() + 1)),
    );
}

/// Characters wide the health bar is: one per heart.
const HEARTS: usize = player::MAX_HEALTH as usize;
const _: () = assert!(HEARTS as f32 == player::MAX_HEALTH);

/// The first line of the status text: what they are doing, who else is here, and how they
/// are. Built here rather than inline so `the_status_line_fits_the_deck_panel` measures the
/// line the player really sees and not a copy of it.
///
/// ASCII only: bevy's built-in font has no glyph for a middle dot, and a missing glyph draws
/// as a tofu box.
fn status(me: &Player, players: usize) -> String {
    let mode = match me.condition {
        // What their body is doing beats where it is: "walking" beside "winded" is the HUD
        // contradicting itself in one line, and a child reads the first word.
        Condition::Winded { .. } => "lying down",
        Condition::Well { .. } if me.is_driving() => "driving",
        Condition::Well { .. } if me.is_flying() => "flying",
        Condition::Well { .. } => "walking",
    };
    format!("{mode}  |  {players} player(s)  |  {}", health_bar(me))
}

/// The health bar, or what is happening instead of one.
///
/// Drawn out of hashes because bevy's built-in font has no heart in it, and a bar of
/// characters is legible across the room on the Deck's panel where a number is not.
fn health_bar(me: &Player) -> String {
    match me.condition {
        Condition::Well { health } => {
            // Floor, so a fall that cost most of a heart takes one off the bar. Rounding up
            // instead hides every drop under seven blocks, which is the whole range a small
            // child lives in — and the `1` floor is what stops it reading empty while its
            // owner is up and walking about.
            let full = (health.floor() as usize).clamp(1, HEARTS);
            format!("health [{}{}]", "#".repeat(full), "-".repeat(HEARTS - full))
        }
        Condition::Winded { .. } => "winded - catching your breath".to_string(),
    }
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

    /// A player who has taken `hearts` of damage — built through [`Condition::hurt`] rather
    /// than by writing a `Condition` down, so the states the bar is shown are states the game
    /// can really reach.
    fn hurt_by(hearts: f32) -> Player {
        let mut me = Player::spawn_at(Vec3::ZERO);
        me.condition.hurt(hearts);
        me
    }

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

    /// The one place a child is told they are hurt. Full reads full, a sliver still reads
    /// as a heart — a bar that empties before its owner is down is a bar that lies — and
    /// being down says what is happening instead of showing an empty bar.
    #[test]
    fn the_health_bar_says_how_hurt_you_are() {
        assert_eq!(health_bar(&hurt_by(0.0)), "health [##########]");
        assert_eq!(health_bar(&hurt_by(6.0)), "health [####------]");
        assert_eq!(
            health_bar(&hurt_by(0.3)),
            "health [#########-]",
            "a fall you felt has to show on the bar"
        );
        assert_eq!(
            health_bar(&hurt_by(player::MAX_HEALTH - 0.1)),
            "health [#---------]",
            "still standing, so still a heart"
        );
        assert_eq!(
            health_bar(&hurt_by(player::MAX_HEALTH)),
            "winded - catching your breath"
        );
    }

    /// The status line shares the Deck's 1280px panel with nothing, but it is one line and
    /// the health bar is new on it. Measured over every state it can be in, because the
    /// longest is not the one anybody thinks of.
    #[test]
    fn the_status_line_fits_the_deck_panel() {
        for hearts in [0.0, 6.0, player::MAX_HEALTH] {
            // 22px of bevy's default font advances well under 13px a character.
            let line = status(&hurt_by(hearts), 4);
            assert!(line.len() * 13 <= 1280, "{} chars: {line}", line.len());
        }
    }

    /// A player flat on their back is not "walking", whatever their feet are doing: the two
    /// halves of one line are read together and must not contradict each other.
    #[test]
    fn the_line_does_not_say_walking_while_you_are_down() {
        let down = status(&hurt_by(player::MAX_HEALTH), 1);
        assert!(down.starts_with("lying down"), "{down}");
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
