//! The crafting screen: punch in a two-symbol code, watch the recipe graph appear, press
//! once and the whole chain is built.
//!
//! **There is not a single letter on it.** The players are children who cannot read, so
//! everything that means something here is a shape, a colour, a place on the screen or a
//! movement: the pictures come from [`crate::glyph`], the structure from
//! [`crate::registry`]'s recipes, and the arithmetic from [`Inventory::plan`].
//!
//! **The code.** Two presses pick any item in the game. The first is *what kind of thing*
//! — a cube, a nail, a hammer, a parachute, a car — and the second is *which one of those*,
//! shown as its own picture in its own colour. The keys never move, so a child who has
//! made a drill once can make the next one by the motor pattern alone: right, right, A,
//! right, A. Both symbols stay up in the slots along the top for as long as the graph is
//! open, which is what makes it a code rather than a menu they walked down.
//!
//! **The graph.** What you gather is on the bottom row, what you make out of it is above,
//! and belts run upward from ingredient to product with the count on them as dots. Every
//! item is drawn **once**, however many things it goes into: a node with two parents is a
//! node with two belts leaving it, not a second copy of itself. That is why this survives
//! the recipes stopping being trees — [`layout`] never assumed one parent, and
//! `a_node_feeding_two_things_is_drawn_once` is what holds it to that.
//!
//! **The build.** One press runs the whole plan — eight nails and then the car — and it
//! costs no time at all: the pile changes the instant the button goes down. What plays out
//! afterwards is a *picture* of what already happened, dots travelling up the belts and
//! each thing popping as it is made, and you may walk away from it at any point with the
//! car already in your pocket.

use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::game::grab_mouse;
use crate::glyph;
use crate::input::{Intent, MenuIntent};
use crate::inventory::{Craft, Inventories, Inventory, Whoami};
use crate::menu;
use crate::registry::{Item, Kind};
use crate::title::Playing;

/// The panel this is laid out for — the Deck's screen, and the window the game opens.
/// Every number below is in those pixels.
const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);

/// The strip along the top that holds the code, and the band under it the rest lives in.
const SLOT: f32 = 92.0;
const SLOT_TOP: f32 = 26.0;
const BODY_TOP: f32 = 152.0;
/// The graph stops here, and what a node says about itself hangs in the gap below —
/// which is why this is well clear of the button hints at the bottom.
const BODY_BOTTOM: f32 = 654.0;

/// A key on the code pad, and the space between two of them.
const KEY: f32 = 148.0;
const KEY_GAP: f32 = 26.0;
/// Keys per row on the second symbol. Seven blocks is the widest kind there is, and four
/// across leaves them big enough to tell apart across a living room.
const PICK_COLUMNS: usize = 4;

/// The most a node in the graph is drawn at. Nodes shrink to fit a deep graph; they never
/// grow past this, or a two-node recipe fills the screen.
const NODE_MAX: f32 = 150.0;
/// How wide a belt is, and the biggest a dot on or beside one gets — everything smaller
/// than a node is scaled off the node, so a deep graph stays in proportion.
const BELT: f32 = 12.0;
const PIP: f32 = 18.0;

/// How long one step of a build takes to draw. Nine steps of it is a car, which is about
/// three seconds of watching nails appear — long enough to see it happen and short enough
/// that nobody waits for it. The pile is already spent; this is only the picture.
const STEP_SECONDS: f32 = 0.34;
/// The part of a step spent moving ingredients up the belts, before the thing pops.
const FLOW: f32 = 0.7;

/// The panel this is drawn on, and the cursor that walks it, are [`menu`]'s — the same
/// dark and the same gold as the pause menu and the join menu, because a child walks
/// between the three of them and a second gold would be a second meaning.
const BACKDROP: Color = menu::PANEL;
const CHOSEN: Color = menu::SELECTED;
const PLATE: Color = Color::srgb(0.13, 0.15, 0.20);
const RIM: Color = Color::srgb(0.27, 0.30, 0.37);
const READY: Color = Color::srgb(0.33, 0.86, 0.44);
/// Red says one thing on this screen and it is **"you have not got this"**. The button
/// that backs out is deliberately not red: it is the quiet one, because backing out is
/// not a problem and a missing leaf is.
const MISSING: Color = Color::srgb(0.95, 0.33, 0.33);
const BACK: Color = Color::srgb(0.52, 0.58, 0.70);
/// The kind keys are drawn in no item's colour: the first symbol is about shape, and the
/// second one is where colour starts meaning something. Cooler and lighter than any block,
/// so the grey cube standing for *every* block is not mistaken for the stone one.
const GREY: [f32; 3] = [0.64, 0.70, 0.85];

/// One rectangle of the picture.
///
/// The whole screen is a flat list of these, produced by [`paint`] out of nothing but the
/// stage, the pile and the clock — so a test can assert about what a player sees rather
/// than about the entities that happen to be drawing it.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Quad {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// Corner radius in pixels. Half the shorter side, or more, is a circle.
    round: f32,
    color: Color,
}

/// Which of the two symbols is being pressed, or the graph the pair of them opened.
#[derive(Debug, Clone, PartialEq)]
enum Stage {
    /// The first symbol: what kind of thing.
    Kind { cursor: usize },
    /// The second: which one of that kind.
    Pick { kind: Kind, cursor: usize },
    /// Both are in. This is the thing, and this is what it is made of.
    ///
    /// The first symbol is not kept beside the second: an item already knows its own kind,
    /// and a pair of fields could be set to a tool and a car — a code the pad cannot spell.
    Graph { item: Item, built: Option<Built> },
}

/// A build playing out on the screen.
///
/// The pile changed the moment the button went down — this is only the picture catching
/// up, which is why nothing here can fail and why walking away mid-animation still leaves
/// the thing in your pocket.
#[derive(Debug, Clone, PartialEq)]
struct Built {
    /// What was made, in the order it was made.
    steps: Vec<Item>,
    /// Seconds since the button went down.
    t: f32,
}

impl Built {
    /// How many steps have finished being drawn.
    fn done(&self) -> usize {
        ((self.t / STEP_SECONDS) as usize).min(self.steps.len())
    }

    /// How far through the step being drawn now, or nothing once they have all played.
    fn doing(&self) -> Option<(Item, f32)> {
        let step = self.done();
        let item = *self.steps.get(step)?;
        Some((item, (self.t / STEP_SECONDS) - step as f32))
    }

    /// How many of `item` this build has finished making.
    fn made(&self, item: Item) -> usize {
        self.steps[..self.done()]
            .iter()
            .filter(|made| **made == item)
            .count()
    }
}

/// The crafting screen, while it is up.
#[derive(Resource)]
pub struct Screen {
    root: Entity,
    stage: Stage,
    /// Seconds this screen has been open. Every pulse and wobble is a function of it.
    t: f32,
    /// When the last refused build was, so the things that are missing can be shaken at
    /// the player rather than a message being written that nobody can read.
    nagged: Option<f32>,
}

impl Screen {
    /// What the code currently spells: the kind, and the item, as far as they are pressed.
    fn code(&self) -> (Option<Kind>, Option<Item>) {
        match self.stage {
            Stage::Kind { .. } => (None, None),
            Stage::Pick { kind, .. } => (Some(kind), None),
            Stage::Graph { item, .. } => (Some(item.class().kind()), Some(item)),
        }
    }
}

pub fn open(
    mut commands: Commands,
    mut intent: ResMut<Intent>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // The world keeps running underneath, so a walk in progress has to be let go of and
    // not merely stop being read — the same rule the pause menu follows, for the same
    // reason: nobody wants to come back to this screen having fallen off a cliff.
    *intent = Intent::default();
    grab_mouse(&mut cursor, false);
    let root = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            // Over the hotbar and the crosshair, both of which this replaces while it is
            // up.
            GlobalZIndex(15),
        ))
        .id();
    commands.insert_resource(Screen {
        root,
        stage: Stage::Kind { cursor: 0 },
        t: 0.0,
        nagged: None,
    });
}

pub fn close(
    mut commands: Commands,
    screen: Res<Screen>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    commands.entity(screen.root).despawn();
    commands.remove_resource::<Screen>();
    grab_mouse(&mut cursor, true);
}

/// The d-pad and the two buttons, and everything they do.
pub fn drive(
    time: Res<Time>,
    menu_intent: Res<MenuIntent>,
    mut screen: ResMut<Screen>,
    inventories: Res<Inventories>,
    me: Res<Whoami>,
    mut asked: MessageWriter<Craft>,
    mut playing: ResMut<NextState<Playing>>,
) {
    let dt = time.delta_secs();
    screen.t += dt;
    if let Stage::Graph {
        built: Some(built), ..
    } = &mut screen.stage
    {
        built.t += dt;
    }

    if menu_intent.back {
        // Back out one symbol at a time, so a mis-press is one press to undo rather than
        // a trip out to the world and back in.
        screen.stage = match screen.stage {
            Stage::Kind { .. } => {
                playing.set(Playing::Live);
                return;
            }
            Stage::Pick { kind, .. } => Stage::Kind {
                cursor: Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0),
            },
            Stage::Graph { item, .. } => {
                let kind = item.class().kind();
                Stage::Pick {
                    kind,
                    cursor: kind.items().iter().position(|i| *i == item).unwrap_or(0),
                }
            }
        };
        return;
    }

    let shelf = inventories.of(me.0).clone();
    match &mut screen.stage {
        Stage::Kind { cursor } => {
            *cursor = menu::step(*cursor, Kind::ALL.len(), menu_intent.across);
            if menu_intent.confirm {
                screen.stage = Stage::Pick {
                    kind: Kind::ALL[*cursor],
                    cursor: 0,
                };
            }
        }
        Stage::Pick { kind, cursor } => {
            let items = kind.items();
            *cursor = menu::step(*cursor, items.len(), menu_intent.across);
            *cursor = menu::step(
                *cursor,
                items.len(),
                menu_intent.step * columns(items.len()) as i32,
            );
            if menu_intent.confirm {
                screen.stage = Stage::Graph {
                    item: items[*cursor],
                    built: None,
                };
            }
        }
        Stage::Graph { item, built, .. } => {
            // A press is ignored while the last one is still playing out. The pile is
            // already spent by then, but on a peer it is the *host's* answer that has not
            // arrived yet — so a second press would be planned against a stale shelf and
            // draw a second car nobody was given. Waiting for the picture to finish is
            // waiting for the answer, and it costs a child nothing.
            let playing = built.as_ref().is_some_and(|b| b.doing().is_some());
            if menu_intent.confirm && !playing {
                let plan = shelf.plan(*item);
                if plan.short.is_empty() {
                    // The screen only asks. Who is allowed to spend a pile is the host's
                    // business, and it answers by sending back a new one.
                    asked.write(Craft(*item));
                    *built = Some(Built {
                        steps: plan.steps,
                        t: 0.0,
                    });
                } else {
                    // Back to showing what is missing, which is the only useful thing to
                    // say to somebody whose press did nothing — including when the press
                    // before it was the one that emptied their pockets.
                    *built = None;
                    let now = screen.t;
                    screen.nagged = Some(now);
                }
            }
        }
    }
}

/// How many keys a row of the second symbol holds — the width the grid is drawn in, and
/// the step one press of up or down takes through it. One number so that changing the
/// shape of the pad cannot leave the cursor jumping the wrong distance across it.
fn columns(keys: usize) -> usize {
    PICK_COLUMNS.min(keys).max(1)
}

pub fn redraw(
    mut commands: Commands,
    screen: Res<Screen>,
    inventories: Res<Inventories>,
    me: Res<Whoami>,
) {
    // Rebuilt from scratch every frame, exactly as the list menus are: a few hundred
    // coloured rectangles cost nothing to respawn, and there is then no way for what is on
    // the screen to disagree with the [`paint`] it came from.
    commands.entity(screen.root).despawn_children();
    let quads = paint(&screen, inventories.of(me.0));
    commands.entity(screen.root).with_children(|ui| {
        for q in quads {
            ui.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(q.x),
                    top: Val::Px(q.y),
                    width: Val::Px(q.w),
                    height: Val::Px(q.h),
                    border_radius: BorderRadius::all(Val::Px(q.round)),
                    ..default()
                },
                BackgroundColor(q.color),
            ));
        }
    });
}

fn quad(x: f32, y: f32, w: f32, h: f32, round: f32, color: Color) -> Quad {
    Quad {
        x,
        y,
        w,
        h,
        round,
        color,
    }
}

/// The picture of a thing and the colour to draw it in — an item, or the item a whole
/// kind is shown as.
///
/// The two always travel together and are never chosen apart: a glyph drawn in somebody
/// else's colour is a lie about which item it is, and this is what stops the pair being
/// passed as two arguments that could be crossed.
#[derive(Clone, Copy)]
struct Face {
    base: [f32; 3],
    parts: &'static [glyph::Part],
}

impl Face {
    fn of(item: Item) -> Face {
        Face {
            base: item.color(),
            parts: glyph::of(item),
        }
    }

    /// A kind, in no item's colour. The first symbol of the code is about shape; the
    /// second is where colour starts meaning something.
    fn kind(kind: Kind) -> Face {
        Face {
            base: GREY,
            parts: glyph::of(glyph::stands_for(kind)),
        }
    }
}

/// A square with a ring around it, drawn as two squares — the ring is the outer one
/// showing through. Every framed thing on this screen is one of these, so a cursor, a
/// verdict and a slot cannot end up wearing three different borders.
fn framed(out: &mut Vec<Quad>, at: Vec2, size: f32, round: f32, ring: Color, fill: Color) {
    const RING: f32 = 5.0;
    out.push(quad(at.x, at.y, size, size, round, ring));
    out.push(quad(
        at.x + RING,
        at.y + RING,
        size - 2.0 * RING,
        size - 2.0 * RING,
        (round - RING).max(0.0),
        fill,
    ));
}

/// Multiplies a colour toward black. What "not yet" looks like: the thing is still there,
/// still the right shape, just not lit.
fn dim(color: Color, k: f32) -> Color {
    let c = color.to_linear();
    Color::linear_rgb(c.red * k, c.green * k, c.blue * k)
}

/// Draws an item's picture in a square. `lit` fades it toward the dark of the panel — a
/// dim node is a thing this build has not made yet.
fn picture(out: &mut Vec<Quad>, at: Vec2, size: f32, face: Face, lit: f32) {
    for part in face.parts {
        let short = part.w.min(part.h) * size;
        out.push(quad(
            at.x + part.x * size,
            at.y + part.y * size,
            part.w * size,
            part.h * size,
            part.round * short * 0.5,
            dim(glyph::shaded(face.base, part.tone), lit),
        ));
    }
}

/// The most dots that are ever drawn of anything. Nothing in the registry asks for more,
/// which `every_count_fits_in_dots` is what keeps true.
const MOST: u32 = 12;
/// A recipe's count is dots in blocks of four, and a tally of what a build made is one
/// long row. Two shapes for two meanings, so neither can be read as the other.
const BLOCK: u32 = 4;
const ROW: u32 = MOST;

/// A count, drawn as dots, centred on a point and wrapped at `per_row`.
///
/// Nobody here can read a numeral, and eight nails is eight dots — countable by a child
/// who is still learning to count, and recognisable at a glance by one who is not.
fn dots(out: &mut Vec<Quad>, at: Vec2, size: f32, n: u32, per_row: u32, color: Color) -> Vec2 {
    let n = n.min(MOST);
    let rows = n.div_ceil(per_row).max(1);
    let gap = size * 0.33;
    let step = size + gap;
    for i in 0..n {
        let row = i / per_row;
        let in_row = (n - row * per_row).min(per_row);
        let col = i % per_row;
        let x = at.x - (in_row as f32 * step - gap) / 2.0 + col as f32 * step;
        let y = at.y - (rows as f32 * step - gap) / 2.0 + row as f32 * step;
        out.push(quad(x, y, size, size, size, color));
    }
    Vec2::new(n.min(per_row) as f32 * step - gap, rows as f32 * step - gap)
}

/// The same count, hollow: what is *not* there. Rings rather than dots, so a shortfall can
/// never be mistaken for a tally at a glance.
fn holes(out: &mut Vec<Quad>, at: Vec2, size: f32, n: u32) {
    let first = out.len();
    dots(out, at, size, n, BLOCK, MISSING);
    let bore = size * 0.5;
    for i in first..out.len() {
        let (x, y) = (out[i].x, out[i].y);
        out.push(quad(
            x + (size - bore) / 2.0,
            y + (size - bore) / 2.0,
            bore,
            bore,
            bore,
            PLATE,
        ));
    }
}

/// A count on a dark plate, sized to the dots on it.
///
/// The plate is what makes a pale count readable where it lands on a belt of nearly its
/// own colour — nails on a nail belt were the case that needed it.
fn badge(out: &mut Vec<Quad>, at: Vec2, n: u32, per_row: u32, pip: f32, color: Color) {
    let plate = out.len();
    out.push(quad(0.0, 0.0, 0.0, 0.0, 0.0, PLATE));
    let pad = pip * 0.62;
    let size = dots(out, at, pip, n, per_row, color) + Vec2::splat(2.0 * pad);
    out[plate] = quad(
        at.x - size.x / 2.0,
        at.y - size.y / 2.0,
        size.x,
        size.y,
        pad,
        PLATE,
    );
}

/// The whole screen, as a flat list of rectangles.
fn paint(screen: &Screen, shelf: &Inventory) -> Vec<Quad> {
    let mut out = vec![quad(0.0, 0.0, SCREEN.x, SCREEN.y, 0.0, BACKDROP)];
    code_strip(&mut out, screen);
    match &screen.stage {
        Stage::Kind { cursor } => kind_pad(&mut out, screen, *cursor),
        Stage::Pick { kind, cursor } => pick_pad(&mut out, screen, shelf, *kind, *cursor),
        Stage::Graph { item, built, .. } => graph(&mut out, screen, shelf, *item, built.as_ref()),
    }
    buttons(&mut out, screen);
    out
}

/// The two slots along the top: what has been pressed so far, and which one is being
/// pressed now. They stay up under the graph, which is what makes the pair of presses a
/// code you can learn rather than a menu you walked down and forgot.
fn code_strip(out: &mut Vec<Quad>, screen: &Screen) {
    let (kind, item) = screen.code();
    let pulse = 0.55 + 0.45 * (screen.t * 5.0).sin();
    let left = (SCREEN.x - (2.0 * SLOT + 26.0)) / 2.0;
    let waiting = if kind.is_none() { 0 } else { 1 };
    for slot in 0..2 {
        let x = left + slot as f32 * (SLOT + 26.0);
        let live = slot == waiting && item.is_none();
        let ring = if live {
            dim(CHOSEN, 0.35 + 0.65 * pulse)
        } else {
            RIM
        };
        framed(out, Vec2::new(x, SLOT_TOP), SLOT, 20.0, ring, PLATE);
        let pad = SLOT * 0.14;
        let inside = Vec2::new(x + pad, SLOT_TOP + pad);
        match (slot, kind, item) {
            (0, Some(kind), _) => picture(out, inside, SLOT - 2.0 * pad, Face::kind(kind), 1.0),
            (1, _, Some(item)) => picture(out, inside, SLOT - 2.0 * pad, Face::of(item), 1.0),
            // An empty slot is a hole, not a blank: a dark well with a lip, so the two
            // presses are visibly two presses before either has been made.
            _ => out.push(quad(
                x + SLOT * 0.3,
                SLOT_TOP + SLOT * 0.44,
                SLOT * 0.4,
                SLOT * 0.12,
                SLOT * 0.06,
                RIM,
            )),
        }
    }
}

/// A key on either pad: a plate, a picture on it, and a ring that says whether the cursor
/// is here.
fn key(out: &mut Vec<Quad>, at: Vec2, size: f32, face: Face, picked: bool, pulse: f32) {
    // The cursor lifts its key as well as ringing it: on a sunlit handheld the colour is
    // the first thing to go, and the one that has moved is still the one that has moved.
    let at = Vec2::new(at.x, at.y - if picked { LIFT } else { 0.0 });
    let ring = if picked {
        dim(CHOSEN, 0.6 + 0.4 * pulse)
    } else {
        RIM
    };
    framed(out, at, size, 22.0, ring, PLATE);
    let pad = size * 0.13;
    picture(out, at + Vec2::splat(pad), size - 2.0 * pad, face, 1.0);
}

/// How far the cursor lifts the key it is on. Named because the bar under a key has to
/// come up with it.
const LIFT: f32 = 8.0;

/// The first symbol: one key per kind of thing there is.
fn kind_pad(out: &mut Vec<Quad>, screen: &Screen, cursor: usize) {
    let pulse = 0.5 + 0.5 * (screen.t * 5.0).sin();
    let n = Kind::ALL.len();
    let width = n as f32 * KEY + (n - 1) as f32 * KEY_GAP;
    let x0 = (SCREEN.x - width) / 2.0;
    let y = BODY_TOP + (BODY_BOTTOM - BODY_TOP - KEY) / 2.0;
    for (i, kind) in Kind::ALL.iter().enumerate() {
        let at = Vec2::new(x0 + i as f32 * (KEY + KEY_GAP), y);
        key(out, at, KEY, Face::kind(*kind), i == cursor, pulse);
    }
}

/// The second symbol: every item of the kind that was pressed, in its own colour.
///
/// The bar under each one is the part a player acts on before choosing, and it says the
/// same thing twice — in colour *and* in shape, because a sunlit handheld loses the colour
/// first. Whole and green: one press away. Broken in two and red: something is still in
/// the ground. Dotted and grey: this is a thing you dig up, not one you make.
fn pick_pad(out: &mut Vec<Quad>, screen: &Screen, shelf: &Inventory, kind: Kind, cursor: usize) {
    let pulse = 0.5 + 0.5 * (screen.t * 5.0).sin();
    let items = kind.items();
    let cols = columns(items.len());
    let rows = items.len().div_ceil(cols);
    let height = rows as f32 * KEY + (rows - 1) as f32 * KEY_GAP;
    let y0 = BODY_TOP + (BODY_BOTTOM - BODY_TOP - height) / 2.0;
    for (i, item) in items.iter().enumerate() {
        let row = i / cols;
        let in_row = (items.len() - row * cols).min(cols);
        let width = in_row as f32 * KEY + (in_row - 1) as f32 * KEY_GAP;
        let x = (SCREEN.x - width) / 2.0 + (i % cols) as f32 * (KEY + KEY_GAP);
        let y = y0 + row as f32 * (KEY + KEY_GAP);
        key(
            out,
            Vec2::new(x, y),
            KEY,
            Face::of(*item),
            i == cursor,
            pulse,
        );
        let lift = if i == cursor { LIFT } else { 0.0 };
        let (colour, pieces) = if item.recipe().is_empty() {
            (RIM, 4)
        } else if shelf.plan(*item).short.is_empty() {
            (READY, 1)
        } else {
            (dim(MISSING, 0.7), 2)
        };
        let bar = KEY - 40.0;
        let gap = 8.0;
        let piece = (bar - (pieces - 1) as f32 * gap) / pieces as f32;
        for k in 0..pieces {
            out.push(quad(
                x + 20.0 + k as f32 * (piece + gap),
                y - lift + KEY + 10.0,
                piece,
                10.0,
                5.0,
                colour,
            ));
        }
    }
}

/// One item in a recipe graph, and where it sits.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Spot {
    item: Item,
    /// Rows above the ground. Everything with no recipe is row zero.
    row: usize,
    /// Position within the row, left to right.
    col: usize,
}

/// So many of `from` go into one `to`.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Belt {
    from: usize,
    to: usize,
    count: u32,
}

/// A recipe graph, laid out.
#[derive(Clone, PartialEq, Debug, Default)]
struct Web {
    spots: Vec<Spot>,
    belts: Vec<Belt>,
    rows: usize,
    /// The busiest row, which is what decides how big a node can be drawn.
    widest: usize,
}

/// How many rows above the ground an item sits: the *longest* way up from something you
/// dig, so a thing is always drawn above everything it is made of however many ways round
/// there are to it. That is the one property the drawing leans on, and it is the property
/// that keeps working when the recipes stop being trees.
/// Everything that goes into `want`, once each, placed on a grid.
fn layout(want: Item) -> Web {
    fn walk(item: Item, found: &mut Vec<Item>) {
        if found.contains(&item) {
            return;
        }
        found.push(item);
        for (ingredient, _) in item.recipe() {
            walk(*ingredient, found);
        }
    }
    let mut found = Vec::new();
    walk(want, &mut found);

    // Heights by repeated relaxation over the items actually on screen, rather than by
    // recursing down each recipe: a shared ingredient is then visited once instead of once
    // per path into it, and a recipe that reached itself would stop this loop rather than
    // the stack. It settles in as many passes as the graph is deep.
    let mut heights = vec![0usize; found.len()];
    let index = |item: Item| found.iter().position(|f| *f == item).expect("walked");
    for _ in 0..found.len() {
        let mut moved = false;
        for (i, item) in found.iter().enumerate() {
            let want = item
                .recipe()
                .iter()
                .map(|(g, _)| heights[index(*g)] + 1)
                .max()
                .unwrap_or(0);
            moved |= want > heights[i];
            heights[i] = heights[i].max(want);
        }
        if !moved {
            break;
        }
    }

    let rows = heights.iter().copied().max().unwrap_or(0) + 1;
    let mut filled = vec![0usize; rows];
    let mut spots = Vec::new();
    for (i, item) in found.iter().enumerate() {
        spots.push(Spot {
            item: *item,
            row: heights[i],
            col: filled[heights[i]],
        });
        filled[heights[i]] += 1;
    }
    let belts = found
        .iter()
        .flat_map(|item| {
            item.recipe().iter().map(move |(ingredient, count)| Belt {
                from: index(*ingredient),
                to: index(*item),
                count: *count,
            })
        })
        .collect();

    Web {
        rows,
        widest: filled.iter().copied().max().unwrap_or(1),
        spots,
        belts,
    }
}

/// A laid-out graph in screen pixels: where every node is, how big one is, and how much
/// room there is between two rows — which is also all the room the belts and their counts
/// have to work in.
struct Placed {
    at: Vec<Vec2>,
    size: f32,
    row_gap: f32,
}

/// Where every node in a laid-out graph lands on the screen, and how big they are.
///
/// The gaps are fractions of a node rather than pixel constants, so a deeper graph shrinks
/// as a whole instead of shrinking its nodes inside gaps that stay the size they were. At
/// five rows the fixed version drew thirty-pixel items eighty pixels apart.
fn placed(web: &Web) -> Placed {
    const ROW_GAP: f32 = 0.62;
    const COL_GAP: f32 = 0.42;
    /// Room left either side of the widest row, for what the belts write beside a node.
    const MARGIN: f32 = 200.0;

    let rows = web.rows as f32;
    let cols = web.widest as f32;
    let by_height = (BODY_BOTTOM - BODY_TOP) / (rows + (rows - 1.0) * ROW_GAP);
    let by_width = (SCREEN.x - MARGIN) / (cols + (cols - 1.0) * COL_GAP);
    let size = NODE_MAX.min(by_height).min(by_width);
    let (row_gap, col_gap) = (size * ROW_GAP, size * COL_GAP);

    let height = rows * size + (rows - 1.0) * row_gap;
    let top = BODY_TOP + (BODY_BOTTOM - BODY_TOP - height) / 2.0;
    let per_row: Vec<usize> = (0..web.rows)
        .map(|r| web.spots.iter().filter(|s| s.row == r).count())
        .collect();

    let at = |spot: &Spot| {
        let n = per_row[spot.row] as f32;
        let width = n * size + (n - 1.0) * col_gap;
        Vec2::new(
            (SCREEN.x - width) / 2.0 + spot.col as f32 * (size + col_gap),
            // Row zero is the ground, so it is drawn at the bottom and everything made out
            // of it stacks upward — which is the direction a child already reads a tower in.
            top + (rows - 1.0 - spot.row as f32) * (size + row_gap),
        )
    };
    Placed {
        at: web.spots.iter().map(at).collect(),
        size,
        row_gap,
    }
}

/// The corners one belt turns: up out of the ingredient, across its own lane, and down
/// into the product.
fn route(exit: Vec2, entry: Vec2, lane: f32) -> [Vec2; 4] {
    [
        exit,
        Vec2::new(exit.x, lane),
        Vec2::new(entry.x, lane),
        entry,
    ]
}

/// One belt, drawn: the corners it turns, and where its count sits.
struct Wire {
    path: [Vec2; 4],
    /// Where the count goes: on the belt's own riser, just above the ingredient it is
    /// counting. Beside the thing rather than beside the product, because "six of *these*"
    /// is what a child needs told — and because three counts converging on one car do not
    /// fit in the gap under it, however the lanes are spread.
    count: Vec2,
}

/// Every belt's route, with no two of them sharing a pixel.
///
/// Three things go into a car and they must be seen to: belts leave an ingredient through
/// their own slot along its top edge, arrive at the product through their own slot along
/// its bottom, and cross between the two along their own lane in the gap. Sharing any of
/// those made the wood belt run *underneath* the nail count and stop, which reads as a car
/// made out of nails.
///
/// The slots are ordered by where the other end is, so the belts fan out without crossing —
/// and none of it assumes a node has one parent or one child, which is what makes this
/// survive the recipes becoming graphs.
fn routes(web: &Web, p: &Placed) -> Vec<Wire> {
    let centre = |node: usize| p.at[node].x + p.size / 2.0;
    // Belts sharing an end, in the order their other ends stand along the screen.
    let fanned = |pick: &dyn Fn(&Belt) -> usize, end: usize, other: &dyn Fn(&Belt) -> usize| {
        let mut group: Vec<usize> = (0..web.belts.len())
            .filter(|b| pick(&web.belts[*b]) == end)
            .collect();
        group.sort_by(|a, b| {
            centre(other(&web.belts[*a])).total_cmp(&centre(other(&web.belts[*b])))
        });
        group
    };
    let slot = |x: f32, rank: usize, of: usize| x + p.size * (rank + 1) as f32 / (of + 1) as f32;

    web.belts
        .iter()
        .enumerate()
        .map(|(b, belt)| {
            let out = fanned(&|belt| belt.from, belt.from, &|belt| belt.to);
            let into = fanned(&|belt| belt.to, belt.to, &|belt| belt.from);
            let (k, j) = (
                out.iter().position(|i| *i == b).expect("its own belt"),
                into.iter().position(|i| *i == b).expect("its own belt"),
            );
            // Lanes fill the gap under the product from just below it downward, in the same
            // order as the slots, so a belt from the far left takes the lane furthest from
            // the product and nothing has to cross anything.
            let spread = j as f32 / (into.len().max(2) - 1) as f32;
            let exit = Vec2::new(slot(p.at[belt.from].x, k, out.len()), p.at[belt.from].y);
            let entry = Vec2::new(
                slot(p.at[belt.to].x, j, into.len()),
                p.at[belt.to].y + p.size,
            );
            let lane = p.at[belt.to].y + p.size + p.row_gap * (0.30 + 0.40 * spread);
            // Counts sit in the gap above the ingredient, fanned sideways off their own
            // risers — never stacked, because a gap between two rows holds one count and
            // not two, and a stack of them climbs into the row above.
            let fan = (k as f32 - (out.len() - 1) as f32 / 2.0) * p.size * 0.5;
            Wire {
                path: route(exit, entry, lane),
                count: Vec2::new(exit.x + fan, exit.y - p.size * 0.26),
            }
        })
        .collect()
}

/// A point some fraction of the way along a route, by distance.
fn along(route: [Vec2; 4], s: f32) -> Vec2 {
    let legs = [
        (route[0], route[1]),
        (route[1], route[2]),
        (route[2], route[3]),
    ];
    let total: f32 = legs.iter().map(|(a, b)| a.distance(*b)).sum();
    let mut want = s.clamp(0.0, 1.0) * total;
    for (a, b) in legs {
        let leg = a.distance(b);
        if want <= leg || leg == 0.0 {
            let k = if leg == 0.0 { 0.0 } else { want / leg };
            return a + (b - a) * k;
        }
        want -= leg;
    }
    route[3]
}

fn belt_quads(out: &mut Vec<Quad>, route: [Vec2; 4], color: Color) {
    for (a, b) in [
        (route[0], route[1]),
        (route[1], route[2]),
        (route[2], route[3]),
    ] {
        let min = a.min(b);
        let max = a.max(b);
        out.push(quad(
            min.x - BELT / 2.0,
            min.y - BELT / 2.0,
            (max.x - min.x) + BELT,
            (max.y - min.y) + BELT,
            BELT / 2.0,
            color,
        ));
    }
}

/// The graph itself.
fn graph(
    out: &mut Vec<Quad>,
    screen: &Screen,
    shelf: &Inventory,
    want: Item,
    built: Option<&Built>,
) {
    let web = layout(want);
    let p = placed(&web);
    let (at, size) = (&p.at, p.size);
    let paths = routes(&web, &p);
    // Everything drawn beside a node is scaled off the node, so a deep graph stays in
    // proportion instead of being buried under counts the size it used to be.
    let pip = (size * 0.16).clamp(9.0, PIP);
    let plan = shelf.plan(want);
    let doing = built.and_then(Built::doing);
    // A refusal is shown by shaking what is missing, because the only honest place to
    // point at is the thing that is not there. Big enough to be unmistakable across a
    // room, and over in three quarters of a second.
    let shake = screen
        .nagged
        .map(|when| screen.t - when)
        .filter(|since| *since < 0.75)
        .map_or(0.0, |since| (since * 30.0).sin() * (0.75 - since) * 62.0);

    // Belts first, so anything that passes near a node is covered by it rather than drawn
    // over it.
    for (belt, wire) in web.belts.iter().zip(&paths) {
        let path = &wire.path;
        let from = web.spots[belt.from].item;
        let flowing = doing.is_some_and(|(item, _)| item == web.spots[belt.to].item);
        belt_quads(out, *path, dim(glyph::shaded(from.color(), 1.0), 0.42));
        if flowing {
            // Dots running up the belt: the one thing on the screen that says *this* is
            // being made out of *that*, right now.
            let phase = doing.map(|(_, p)| p).unwrap_or(0.0) / FLOW;
            for k in 0..3 {
                let s = (phase - k as f32 * 0.16).clamp(0.0, 1.0);
                let at = along(*path, s);
                out.push(quad(
                    at.x - pip / 2.0,
                    at.y - pip / 2.0,
                    pip,
                    pip,
                    pip,
                    glyph::shaded(from.color(), 1.3),
                ));
            }
        }
        badge(
            out,
            wire.count,
            belt.count,
            BLOCK,
            pip * 0.78,
            glyph::shaded(from.color(), 1.25),
        );
    }

    for (i, spot) in web.spots.iter().enumerate() {
        // What is missing is a question about a build that has not happened. Once one has,
        // the picture is about what was made — asking again would paint the whole bottom
        // row red the moment a car empties the pockets that paid for it. A refusal clears
        // `built` (see [`drive`]), so a press that cannot be afforded still gets its red.
        let short = built
            .is_none()
            .then(|| plan.short.iter().find(|(item, _)| *item == spot.item))
            .flatten()
            .map(|(_, n)| *n);
        let made = built.map_or(0, |b| b.made(spot.item));
        let being_made = doing.is_some_and(|(item, _)| item == spot.item);
        // The thing you asked for keeps its ring throughout — it is why the screen is up.
        // Everything else is green once it is accounted for, red while it is still in the
        // ground, and white for the one step happening this instant.
        let gathered = spot.item.recipe().is_empty();
        let ring = match (spot.item == want, being_made, short) {
            (true, _, _) => CHOSEN,
            (_, true, _) => Color::WHITE,
            (_, _, Some(_)) => MISSING,
            // Something you dig up is never waiting to be made, so it stays accounted for
            // all the way through a build — it is what the build is being paid out of.
            _ if built.is_some() && made == 0 && !gathered => RIM,
            _ => READY,
        };
        let wobble = if short.is_some() { shake } else { 0.0 };
        // The thing being made right now swells, and settles back.
        let pop = doing
            .filter(|(item, p)| *item == spot.item && *p > FLOW)
            .map_or(0.0, |(_, p)| {
                let k = ((p - FLOW) / (1.0 - FLOW)).clamp(0.0, 1.0);
                (k * std::f32::consts::PI).sin() * size * 0.16
            });
        let lit = if spot.item.recipe().is_empty() || made > 0 || built.is_none() {
            1.0
        } else {
            0.35
        };
        let corner = Vec2::new(at[i].x + wobble - pop / 2.0, at[i].y - pop / 2.0);
        let (x, y) = (corner.x, corner.y);
        let s = size + pop;
        framed(out, corner, s, 24.0, ring, PLATE);
        let pad = s * 0.13;
        picture(
            out,
            corner + Vec2::splat(pad),
            s - 2.0 * pad,
            Face::of(spot.item),
            lit,
        );
        // Beside the node: hollow dots are how many more of this you have to go and dig up,
        // solid ones are how many this build has made. Never both — the first is a question
        // about a build that has not happened and the second is an answer.
        //
        // Belts leave a node's top and arrive at its bottom, so a tally goes on whichever
        // side is empty: over the thing you asked for, which nothing is made out of, and
        // under everything else. The tally is one long row where a belt count is a block of
        // four, so the two can never be read as each other.
        let feeds_something = web.belts.iter().any(|b| b.from == i);
        let clear = pip * 1.3;
        let beside = if feeds_something {
            Vec2::new(x + s / 2.0, y + s + clear)
        } else {
            Vec2::new(x + s / 2.0, y - clear)
        };
        match (short, made) {
            (Some(n), _) => holes(out, beside, pip, n),
            (None, n) if n > 0 => badge(
                out,
                beside,
                n as u32,
                ROW,
                pip,
                glyph::shaded(spot.item.color(), 1.35),
            ),
            _ => {}
        }
    }
}

/// The bottom of the screen: which way the d-pad goes, and what the two buttons do. Drawn,
/// not written — a green button is go and a red one is back, in every menu in the game.
fn buttons(out: &mut Vec<Quad>, screen: &Screen) {
    let y = SCREEN.y - 76.0;
    let pulse = 0.6 + 0.4 * (screen.t * 5.0).sin();
    // A d-pad, drawn as a d-pad.
    let (px, s) = (SCREEN.x / 2.0 - 130.0, 22.0);
    for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
        out.push(quad(
            px + dx * (s + 3.0),
            y + dy * (s + 3.0),
            s,
            s,
            6.0,
            dim(Color::WHITE, 0.62),
        ));
    }
    out.push(quad(px, y, s, s, 6.0, dim(Color::WHITE, 0.3)));

    for (i, (color, label)) in [(READY, true), (BACK, false)].into_iter().enumerate() {
        let cx = SCREEN.x / 2.0 + 40.0 + i as f32 * 92.0;
        let d = 54.0;
        let live = if label { pulse } else { 0.75 };
        out.push(quad(cx, y - d / 2.0 + s / 2.0, d, d, d, dim(color, live)));
        // A press goes in; a back-out comes out. One is a dot, the other a ring.
        if label {
            out.push(quad(cx + 16.0, y - 11.0 + s / 2.0, 22.0, 22.0, 22.0, PLATE));
        } else {
            out.push(quad(cx + 12.0, y - 15.0 + s / 2.0, 30.0, 30.0, 30.0, PLATE));
            out.push(quad(
                cx + 20.0,
                y - 7.0 + s / 2.0,
                14.0,
                14.0,
                14.0,
                dim(color, 0.75),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(stage: Stage) -> Screen {
        Screen {
            root: Entity::PLACEHOLDER,
            stage,
            t: 0.4,
            nagged: None,
        }
    }

    /// The property the whole drawing rests on, and the one the brief asks for: a thing
    /// that goes into two other things is **one** node with two belts leaving it, not two
    /// copies of itself. Nothing here would change if a recipe gained a second parent
    /// tomorrow.
    #[test]
    fn a_node_feeding_two_things_is_drawn_once() {
        let web = layout(Item::Car);
        let mut items: Vec<Item> = web.spots.iter().map(|s| s.item).collect();
        let before = items.len();
        items.sort_by_key(|i| i.index());
        items.dedup();
        assert_eq!(items.len(), before, "an item was drawn twice");

        let stone = web
            .spots
            .iter()
            .position(|s| s.item == Item::Stone)
            .unwrap();
        let out: Vec<&Belt> = web.belts.iter().filter(|b| b.from == stone).collect();
        assert_eq!(out.len(), 2, "stone feeds the nails and the car itself");
        assert_eq!(
            out.iter().map(|b| b.count).collect::<Vec<_>>(),
            vec![2, 1],
            "two stone into the car, one into a nail"
        );
    }

    /// Every belt runs strictly upward. It is what makes the picture readable — you make
    /// things out of the row below — and it holds for a graph as well as a tree because
    /// [`height`] is the *longest* way up rather than the first one found.
    #[test]
    fn nothing_is_made_out_of_something_beside_it() {
        for item in Item::ALL {
            let web = layout(*item);
            for belt in &web.belts {
                assert!(
                    web.spots[belt.from].row < web.spots[belt.to].row,
                    "{item:?}: {:?} feeds {:?} from the same row or above",
                    web.spots[belt.from].item,
                    web.spots[belt.to].item
                );
            }
        }
    }

    /// The bottom row is exactly the things you dig up, and the top is the one thing you
    /// asked for — the two ends a child reads the picture between.
    #[test]
    fn the_ground_is_at_the_bottom_and_your_thing_is_on_top() {
        let web = layout(Item::Car);
        assert_eq!(web.rows, 3, "stone and wood, nails, car");
        for spot in &web.spots {
            assert_eq!(
                spot.row == 0,
                spot.item.recipe().is_empty(),
                "{:?} is on the wrong row",
                spot.item
            );
        }
        let top: Vec<Item> = web
            .spots
            .iter()
            .filter(|s| s.row == web.rows - 1)
            .map(|s| s.item)
            .collect();
        assert_eq!(
            top,
            vec![Item::Car],
            "only the thing you asked for is up top"
        );
    }

    /// A thing you gather has nothing under it, and the screen still has to draw it rather
    /// than fall over on an empty graph.
    #[test]
    fn a_gathered_thing_is_one_lonely_node() {
        let web = layout(Item::Grass);
        assert_eq!(web.rows, 1);
        assert_eq!(web.spots.len(), 1);
        assert!(web.belts.is_empty());
    }

    /// Nothing may be drawn off the edge of the Deck's panel, in any stage, for any item —
    /// a node that spills is a node whose picture is half missing.
    #[test]
    fn nothing_is_drawn_off_the_screen() {
        let mut shelf = Inventory::default();
        shelf.add(Item::Wood, 6);
        shelf.add(Item::Stone, 10);
        let mut stages = vec![Stage::Kind { cursor: 0 }];
        for kind in Kind::ALL {
            stages.push(Stage::Pick {
                kind: *kind,
                cursor: kind.items().len() - 1,
            });
            for item in kind.items() {
                stages.push(Stage::Graph { item, built: None });
                // And every moment of a build of it, tallies and all — the counts a build
                // hangs off a node are the ones with room to fall off the panel.
                let steps = shelf.clone().build(item);
                if steps.is_empty() {
                    continue;
                }
                for step in 0..=steps.len() {
                    stages.push(Stage::Graph {
                        item,
                        built: Some(Built {
                            steps: steps.clone(),
                            t: step as f32 * STEP_SECONDS,
                        }),
                    });
                }
            }
        }
        for stage in stages {
            let label = format!("{stage:?}");
            for q in paint(&screen(stage), &shelf) {
                assert!(
                    q.x >= -2.0
                        && q.y >= -2.0
                        && q.x + q.w <= SCREEN.x + 2.0
                        && q.y + q.h <= SCREEN.y + 2.0,
                    "{label}: {q:?} is off the panel"
                );
            }
        }
    }

    /// The dots are how a count is said, so no recipe may ask for more than a child can
    /// take in at a glance — [`dots`] caps at twelve, and a thirteenth would silently not
    /// be drawn.
    #[test]
    fn every_count_fits_in_dots() {
        for item in Item::ALL {
            for (ingredient, n) in item.recipe() {
                assert!(*n <= 12, "{item:?} asks for {n} {ingredient:?}");
            }
        }
    }

    /// The build plays out step by step and then stops, with every step credited to the
    /// thing it made. Eight nails is eight pops, not one.
    #[test]
    fn a_build_is_drawn_one_step_at_a_time() {
        let mut shelf = Inventory::default();
        shelf.add(Item::Wood, 6);
        shelf.add(Item::Stone, 10);
        let steps = shelf.build(Item::Car);
        let built = |t: f32| Built {
            steps: steps.clone(),
            t,
        };
        assert_eq!(built(0.0).made(Item::Nail), 0);
        assert_eq!(built(STEP_SECONDS * 3.5).made(Item::Nail), 3);
        assert_eq!(built(STEP_SECONDS * 8.5).made(Item::Car), 0, "nails first");
        let end = built(STEP_SECONDS * 20.0);
        assert_eq!(end.doing(), None, "it finishes");
        assert_eq!((end.made(Item::Nail), end.made(Item::Car)), (8, 1));
    }

    /// A point on a belt is on the belt: the ends are the ends, and the middle is somewhere
    /// between them.
    #[test]
    fn dots_travel_from_the_ingredient_to_the_thing() {
        let path = route(Vec2::new(100.0, 500.0), Vec2::new(400.0, 200.0), 260.0);
        assert_eq!(along(path, 0.0), path[0]);
        assert_eq!(along(path, 1.0), path[3]);
        let mid = along(path, 0.5);
        assert!(mid.y < path[0].y && mid.y >= path[3].y, "{mid:?}");
        assert!(mid.x >= 100.0 && mid.x <= 400.0, "{mid:?}");
    }

    /// The one that keeps three ingredients visibly reaching one car: no belt may share a
    /// lane, a way out or a way in with another, and no count may land on a node.
    ///
    /// This is the rule the drawing rests on once recipes are graphs rather than trees, and
    /// it is checked over every item in the game rather than over the one that broke.
    #[test]
    fn no_two_belts_run_over_each_other_and_no_count_lands_on_a_node() {
        for item in Item::ALL {
            let web = layout(*item);
            let p = placed(&web);
            let paths = routes(&web, &p);
            for (a, one) in paths.iter().enumerate() {
                for (b, two) in paths.iter().enumerate().skip(a + 1) {
                    if web.belts[a].to == web.belts[b].to {
                        assert_ne!(
                            one.path[1].y, two.path[1].y,
                            "{item:?}: two belts on one lane"
                        );
                        assert_ne!(
                            one.path[3].x, two.path[3].x,
                            "{item:?}: two belts into one slot"
                        );
                    }
                    if web.belts[a].from == web.belts[b].from {
                        assert_ne!(
                            one.path[0].x, two.path[0].x,
                            "{item:?}: two belts out of one slot"
                        );
                    }
                    // Counts are the biggest thing the gaps have to hold, and two of them
                    // on top of each other is a number a child cannot read.
                    let apart = (one.count - two.count).abs();
                    assert!(
                        apart.x > p.size * 0.35 || apart.y > p.size * 0.35,
                        "{item:?}: two counts at {:?} and {:?}",
                        one.count,
                        two.count
                    );
                }
                // A count hangs above the ingredient it counts, which has to be clear of
                // everything drawn in either row.
                for at in &p.at {
                    let clear = one.count.y < at.y || one.count.y > at.y + p.size;
                    assert!(clear, "{item:?}: a count landed on a node at {at:?}");
                }
            }
        }
    }

    /// What the code spells, at each of the three stages.
    #[test]
    fn the_code_fills_up_one_symbol_at_a_time() {
        assert_eq!(screen(Stage::Kind { cursor: 0 }).code(), (None, None));
        assert_eq!(
            screen(Stage::Pick {
                kind: Kind::Tool,
                cursor: 0
            })
            .code(),
            (Some(Kind::Tool), None)
        );
        assert_eq!(
            screen(Stage::Graph {
                item: Item::Car,
                built: None
            })
            .code(),
            (Some(Kind::Vehicle), Some(Item::Car))
        );
    }
}
