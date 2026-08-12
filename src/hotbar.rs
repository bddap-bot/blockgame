//! The hotbar, drawn as the thing you press it with.
//!
//! There is no bar. What sits over the world is one picture of what you are holding, and
//! under it two little d-pads with one arm lit on each — the code that got you there,
//! which is also the code that gets you back. Press any direction and the whole pad blooms
//! open: four clusters in a cross, four things in a cross on each cluster, laid out exactly
//! as [`crate::code`]'s ROSETTE is written. Press a second direction and it shuts on
//! whatever you landed on.
//!
//! **Not one word anywhere.** A thing is its picture ([`crate::glyph`]) in its own colour
//! from the registry; how many you have is a row of notches under it, and more than
//! [`MANY`] is one solid bar, because past a handful the answer a child wants is "lots".
//! What a thing *does* is not written here either — it is the shape of it, and the rig is
//! where what it costs is drawn.
//!
//! **The rig is behind the same pad.** The pad stays up while the crafting rig is open and
//! keeps meaning the same thing there, so the graph re-centres on whatever you just typed.
//! One selection, two views of it: the pad says which thing, the rig says what that thing
//! is made of and takes the payment.
//!
//! Everything here is laid out in the Deck's 1280x800 pixels and scaled to the panel it is
//! really on by [`crate::hud::scale_to_screen`], exactly as the rest of the overlay is.

use bevy::prelude::*;

use crate::code::{self, Code, Dir, Pad};
use crate::glyph;
use crate::input::Drum;
use crate::inventory::{Held, Inventory, Pocket};
use crate::registry::Item;

/// A key's picture. Everything else on the pad is sized off it.
const CELL: f32 = 32.0;
/// Centre to centre from a cluster's middle out to one of its four keys.
const KEY_STEP: f32 = 50.0;
/// Centre to centre from the middle of the pad out to one of its four clusters.
const ARM_STEP: f32 = 150.0;
/// Where the middle of the pad sits while it is open, and where the held thing sits while
/// it is shut. It moves because the shut pad is a badge at the bottom of the screen and the
/// open pad is a keyboard in the middle of it, and watching the badge fly into the middle
/// of the keyboard is what says they are the same object.
const PAD_AT: Vec2 = Vec2::new(640.0, 552.0);
const REST_AT: Vec2 = Vec2::new(640.0, 742.0);
/// The held thing, drawn twice the size of a key: the one thing on this surface a player
/// is looking at while they play.
const HUB: f32 = 56.0;
/// A little d-pad with one arm lit — the unit both halves of a code are drawn as.
const CHIP: f32 = 20.0;

/// The most notches drawn of anything. Past this the bar goes solid: the question a child
/// asks of a pile is "enough?" and never "how many exactly?", and the rig counts out the
/// exact beads of a recipe anyway.
const MANY: u32 = 8;
const NOTCH: f32 = 2.6;
const NOTCH_GAP: f32 = 1.2;

const PANEL: Color = Color::srgba(0.04, 0.05, 0.08, 0.62);
const KEY_PLATE: Color = Color::srgba(0.10, 0.11, 0.15, 0.88);
const SPARE: Color = Color::srgba(1.0, 1.0, 1.0, 0.09);
/// How far back the three clusters a half-typed code is *not* on are pushed. Dimmed rather
/// than hidden: pressing one key is how a child is shown the other three.
const ASIDE: f32 = 0.62;
/// How dark a thing you have none of is drawn. Darkened, never faded: fading it as well
/// would multiply the two dimmings together on a cluster that is also pushed aside, and
/// four of the fourteen things would be invisible whenever the pad was open anywhere else.
const UNOWNED: f32 = 0.55;
/// One cluster's plate, as a side. Everything that has to not overlap is derived from it.
const CLUSTER: f32 = 2.0 * KEY_STEP + CELL + 14.0;

/// Nothing on the pad may be drawn on top of anything else on it — two keys under one
/// another are one key as far as a thumb is concerned, and a key a thumb cannot single out
/// is a thing the player cannot hold.
///
/// Checked while the game is compiled, because these are all constants: a layout that
/// collides is a typo, and a typo should not need a test run to find.
const _: () = {
    assert!(ARM_STEP >= CLUSTER, "the clusters overlap each other");
    assert!(KEY_STEP > CELL + 16.0, "the keys of a cluster overlap");
    assert!(
        (HUB + CHIP + 22.0) * 0.5 < ARM_STEP - CLUSTER * 0.5,
        "the held thing is drawn inside a cluster"
    );
};

/// One coloured rectangle. The whole surface is a list of these, so what is on the screen
/// is a pure function of the pad and the pile — which is what makes the layout testable
/// without a window, and what lets the film draw the same pixels the game does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub round: f32,
    pub color: Color,
}

/// Everything the hotbar draws, in the Deck's pixels.
///
/// Kept separate from [`crate::hud::HudRoot`] on purpose: the crosshair and the game's
/// occasional line of words go away while the crafting rig is up, and this does not.
#[derive(Component)]
pub struct PadRoot;

/// Everything the surface is drawn from: the pad mid-code, the pile it counts, the thing
/// in hand, and how far open the whole thing is this frame.
///
/// One value threaded through, rather than five arguments arriving in a different order at
/// every level of the drawing — which is how a cluster ends up drawn with another key's
/// bloom.
struct Look<'a> {
    pad: &'a Pad,
    pile: &'a Inventory,
    held: Item,
    /// Where the middle of the pad is this frame, part way between its resting place and
    /// where it sits open.
    middle: Vec2,
    /// 0 shut, 1 wide open — what everything fades by.
    bloom: f32,
    /// The same, eased: what everything *moves* by, so the clusters arrive before they
    /// finish fading in.
    spring: f32,
}

/// The whole surface, as rectangles.
pub fn paint(pad: &Pad, pile: &Inventory, held: Item) -> Vec<Quad> {
    let mut out = Vec::new();
    let bloom = pad.bloom();
    // The clusters spring out faster than they fade in, so the pad reads as one thing
    // unfolding rather than four things appearing.
    let spring = 1.0 - (1.0 - bloom).powi(3);
    let look = Look {
        pad,
        pile,
        held,
        middle: REST_AT.lerp(PAD_AT, spring),
        bloom,
        spring,
    };

    if bloom > 0.01 {
        // The spokes first, under everything: four bars out of the middle, one per key, so
        // the whole surface reads as a d-pad with a d-pad on the end of each arm rather
        // than as four boxes that happen to be arranged in a cross.
        for arm in Dir::ALL {
            let reach = ARM_STEP * look.spring;
            let (w, h) = match arm {
                Dir::Left | Dir::Right => (reach, 8.0),
                Dir::Up | Dir::Down => (8.0, reach),
            };
            let fade = if pad.arm() == Some(arm) { 0.85 } else { 0.22 };
            quad(
                &mut out,
                look.middle + arm.unit() * (reach * 0.5),
                w,
                h,
                4.0,
                arm.tint().with_alpha(fade * bloom),
            );
        }
        for arm in Dir::ALL {
            cluster(&mut out, &look, arm);
        }
    }
    hub(&mut out, &look);
    out
}

/// One arm of the pad: a dark plate and the four things behind that direction.
fn cluster(out: &mut Vec<Quad>, look: &Look, arm: Dir) {
    // The arm a half-typed code is on is lit and the other three are pushed back — but
    // still drawn, because a child who pressed left and can see what is up, right and down
    // has been taught the whole pad by pressing one key.
    let bloom = look.bloom;
    let open = look.pad.arm();
    let mine = open == Some(arm);
    let lit = if mine || open.is_none() { 1.0 } else { ASIDE };
    let at = look.middle + arm.unit() * (ARM_STEP * look.spring);

    // The dark plate goes under every cluster, and the arm's colour goes on top of the
    // lit one. Tint alone was invisible: a cyan wash over a blue sky is a blue sky, and
    // the surface has to read the same over sand, stone and a forest.
    quad(
        out,
        at,
        CLUSTER,
        CLUSTER,
        24.0,
        PANEL.with_alpha(PANEL.alpha() * bloom),
    );
    if mine {
        quad(
            out,
            at,
            CLUSTER,
            CLUSTER,
            24.0,
            arm.tint().with_alpha(0.22 * bloom),
        );
        // A ring of the arm's own colour, so the lit cluster is the same colour as the key
        // that opened it, as the spoke leading to it, and as the first chip under the held
        // thing. Four places, one hue, one press.
        ring(
            out,
            at,
            CLUSTER,
            3.5,
            24.0,
            arm.tint().with_alpha(0.9 * bloom),
        );
    }

    for (key, item) in Dir::ALL.iter().zip(code::arm(arm)) {
        let spot = at + key.unit() * KEY_STEP;
        match item {
            Some(item) => cell(out, look, spot, item, Code { arm, key: *key }, lit * bloom),
            // A key with nothing behind it is drawn as a key with nothing behind it: the
            // pad has room, and a child who presses it and gets nothing has learnt where
            // the next thing is going to live.
            None => quad(out, spot, CELL * 0.5, CELL * 0.5, CELL * 0.25, {
                SPARE.with_alpha(SPARE.alpha() * lit * bloom)
            }),
        }
    }
}

/// One key: what is behind it, how many of them you have, and whether it is the one in
/// your hand.
fn cell(out: &mut Vec<Quad>, look: &Look, at: Vec2, item: Item, code: Code, lit: f32) {
    let n = look.pile.count(item);
    let w = CELL + 10.0;
    let h = CELL + 16.0;
    quad(
        out,
        at,
        w,
        h,
        8.0,
        KEY_PLATE.with_alpha(KEY_PLATE.alpha() * lit),
    );
    if item == look.held {
        ring(out, at, w, 2.5, 8.0, Color::WHITE.with_alpha(0.9 * lit));
    }
    // The key the code just landed on glows in its own colour while the pad shuts —
    // the eye's copy of the high note. The opening press has the whole cluster's ring.
    if let Some((struck, heat)) = look.pad.flash()
        && struck == code
    {
        quad(
            out,
            at,
            w,
            h,
            8.0,
            code.key.tint().with_alpha(0.45 * heat * lit),
        );
    }
    // A thing you have none of is still drawn, still in its place, just dark — that is how
    // you find out it exists and go and read its recipe.
    let dark = if n > 0 { 1.0 } else { UNOWNED };
    picture(out, at - Vec2::new(0.0, 6.0), CELL, item, dark, lit);
    notches(out, at + Vec2::new(0.0, CELL * 0.5 + 1.0), item, n, lit);
}

/// What you are holding, and the two presses that got you there.
fn hub(out: &mut Vec<Quad>, look: &Look) {
    let (held, middle, bloom) = (look.held, look.middle, look.bloom);
    let code = code::of(held);
    let w = HUB + 16.0;
    let h = HUB + CHIP + 22.0;
    // Solid at rest and see-through once the pad is open: with the whole rosette up, the
    // hub is a reminder and not the subject.
    quad(
        out,
        middle,
        w,
        h,
        12.0,
        PANEL.with_alpha(0.82 - 0.42 * bloom),
    );
    picture(
        out,
        middle - Vec2::new(0.0, CHIP * 0.5 + 2.0),
        HUB,
        held,
        1.0,
        1.0,
    );

    // At rest the two chips are a label: the code of the thing in your hand, which is the
    // code that gets it back. Mid-code they are a progress bar instead — the key you just
    // hit, and an unpressed pad waiting for the second one.
    let keys = match look.pad.arm() {
        Some(arm) => [Some(arm), None],
        None => [Some(code.arm), Some(code.key)],
    };
    let row = middle + Vec2::new(0.0, HUB * 0.5 - 1.0);
    for (i, key) in keys.into_iter().enumerate() {
        let x = (i as f32 - 0.5) * (CHIP + 6.0);
        chip(out, row + Vec2::new(x, 0.0), CHIP, key, 1.0);
    }
}

/// A d-pad the size of a thumbnail with one arm lit — the picture of one press.
///
/// The same cross as the pad it is a picture of, which is the whole trick: the thing a
/// player learns at the size of the screen is the thing they read at the size of a badge.
fn chip(out: &mut Vec<Quad>, at: Vec2, size: f32, lit: Option<Dir>, bright: f32) {
    let arm = size * 0.30;
    quad(
        out,
        at,
        arm,
        arm,
        arm * 0.2,
        Color::srgba(1.0, 1.0, 1.0, 0.22 * bright),
    );
    for dir in Dir::ALL {
        let on = Some(dir) == lit;
        let colour = if on {
            dir.tint().with_alpha(bright)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.16 * bright)
        };
        let (w, h) = match dir {
            Dir::Left | Dir::Right => (size * 0.32, arm),
            Dir::Up | Dir::Down => (arm, size * 0.32),
        };
        quad(
            out,
            at + dir.unit() * (size * 0.34),
            w,
            h,
            arm * 0.2,
            colour,
        );
    }
}

/// How many you have, as a row of notches under the picture. Past [`MANY`] it is one solid
/// bar: "more than you are going to count".
fn notches(out: &mut Vec<Quad>, at: Vec2, item: Item, n: u32, lit: f32) {
    let width = MANY as f32 * NOTCH + (MANY - 1) as f32 * NOTCH_GAP;
    if n > MANY {
        let [r, g, b] = item.color();
        quad(
            out,
            at,
            width,
            NOTCH * 1.6,
            NOTCH,
            Color::linear_rgb(r, g, b).lighter(0.35).with_alpha(lit),
        );
        return;
    }
    // Only the ones you have are drawn. An empty row of sockets under every one of
    // fourteen keys is a lot of dots that all mean nothing, and "none" reads better as
    // nothing at all than as eight empty holes.
    for i in 0..n {
        let x = (i as f32 - (n - 1) as f32 / 2.0) * (NOTCH + NOTCH_GAP);
        quad(
            out,
            at + Vec2::new(x, 0.0),
            NOTCH,
            NOTCH * 1.6,
            NOTCH * 0.5,
            Color::srgba(1.0, 1.0, 1.0, 0.92 * lit),
        );
    }
}

/// An item's picture, drawn in its own colour inside a square of `size`.
///
/// `dark` is how far toward black the colour is taken — what "you have none of these"
/// looks like. `alpha` is how far the whole thing has faded in — what the bloom and the
/// pushed-aside clusters do. Two knobs rather than one, because they mean different things
/// and multiplying them together is how a picture ends up drawn in no colour at all.
fn picture(out: &mut Vec<Quad>, at: Vec2, size: f32, item: Item, dark: f32, alpha: f32) {
    let base = item.color();
    let corner = at - Vec2::splat(size * 0.5);
    for part in glyph::of(item) {
        let short = part.w.min(part.h) * size;
        let c = glyph::shaded(base, part.tone).to_linear();
        out.push(Quad {
            x: corner.x + part.x * size,
            y: corner.y + part.y * size,
            w: part.w * size,
            h: part.h * size,
            round: part.round * short * 0.5,
            color: Color::linear_rgb(c.red * dark, c.green * dark, c.blue * dark)
                .with_alpha(alpha.min(1.0)),
        });
    }
}

/// A rectangle placed by its middle — everything on this surface is positioned that way,
/// because everything on it is arranged around the middle of a cross.
fn quad(out: &mut Vec<Quad>, at: Vec2, w: f32, h: f32, round: f32, color: Color) {
    out.push(Quad {
        x: at.x - w * 0.5,
        y: at.y - h * 0.5,
        w,
        h,
        round,
        color,
    });
}

/// A border, drawn as four bars rather than as a bigger rectangle behind — the things this
/// rings are see-through, and a plate behind them would show through as a wash.
fn ring(out: &mut Vec<Quad>, at: Vec2, side: f32, thick: f32, round: f32, color: Color) {
    let half = side * 0.5;
    for dir in Dir::ALL {
        let (w, h) = match dir {
            Dir::Left | Dir::Right => (thick, side),
            Dir::Up | Dir::Down => (side, thick),
        };
        quad(
            out,
            at + dir.unit() * (half - thick * 0.5),
            w,
            h,
            round * 0.3,
            color,
        );
    }
}

// ---------------------------------------------------------------------------
// The systems.
// ---------------------------------------------------------------------------

pub fn setup(mut commands: Commands) {
    commands.spawn((
        PadRoot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
    ));
}

/// The d-pad, typing. One system for the whole game: the pad means the same thing in the
/// world and inside the crafting rig, so there is one place a press turns into a choice.
pub fn drum(time: Res<Time>, drum: Res<Drum>, mut pad: ResMut<Pad>, mut held: ResMut<Held>) {
    pad.tick(time.delta_secs());
    if let Some(dir) = drum.press
        && let Some(item) = pad.press(dir)
    {
        held.0 = item;
    }
}

/// Shuts the pad on the way into a pause. The drum systems stop in Paused, so without
/// this a half-typed code freezes the open pad behind the menu and its stale arm lands on
/// the first press after resume.
pub fn rest(mut pad: ResMut<Pad>) {
    pad.shut();
}

/// Draws it, from scratch, every frame.
///
/// A few hundred coloured rectangles respawned each frame is nothing next to a chunk mesh,
/// and it is the reason there is no second copy of the pad's state living in the UI tree
/// to fall out of step with [`Pad`].
pub fn redraw(
    mut commands: Commands,
    pad: Res<Pad>,
    pocket: Res<Pocket>,
    held: Res<Held>,
    root: Query<Entity, With<PadRoot>>,
) {
    let Ok(root) = root.single() else {
        return;
    };
    commands.entity(root).despawn_children();
    let quads = paint(&pad, &pocket.0, held.0);
    commands.entity(root).with_children(|ui| {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(on: Dir) -> Pad {
        let mut pad = Pad::default();
        pad.press(on);
        for _ in 0..60 {
            pad.tick(1.0 / 60.0);
        }
        pad
    }

    fn bounds(quads: &[Quad]) -> (Vec2, Vec2) {
        quads.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(lo, hi), q| {
                (
                    lo.min(Vec2::new(q.x, q.y)),
                    hi.max(Vec2::new(q.x + q.w, q.y + q.h)),
                )
            },
        )
    }

    /// The open pad fits the Deck's panel. Wider or taller than the screen and some key is
    /// simply not there — and a key that is not there is a thing the player cannot reach,
    /// which on this surface means a thing they cannot hold.
    #[test]
    fn the_open_pad_fits_the_deck_panel() {
        for arm in Dir::ALL {
            let quads = paint(&opened(arm), &Inventory::default(), Item::Grass);
            let (lo, hi) = bounds(&quads);
            assert!(lo.x >= 0.0 && hi.x <= 1280.0, "{lo} to {hi} across");
            assert!(lo.y >= 0.0 && hi.y <= 800.0, "{lo} to {hi} down");
        }
    }

    /// Shut, it is a badge in the corner of the eye and nothing else. The player is looking
    /// at the world, and the surface that tells them what is in their hand may not be the
    /// thing in front of it.
    #[test]
    fn the_shut_pad_is_out_of_the_way() {
        let quads = paint(&Pad::default(), &Inventory::default(), Item::Car);
        let (lo, hi) = bounds(&quads);
        let area = (hi.x - lo.x) * (hi.y - lo.y);
        assert!(area < 0.02 * 1280.0 * 800.0, "the badge covers {area}px²");
        assert!(lo.y > 640.0, "it has crept up the screen to {}", lo.y);
    }

    /// Every key is drawn where its code says it is: press left, look left. If these two
    /// ever disagree the whole surface is a lie, because the only thing telling a
    /// non-reader what to press is where the thing is.
    #[test]
    fn a_thing_is_drawn_where_its_code_points() {
        let middle = PAD_AT;
        for item in Item::ALL {
            let c = code::of(*item);
            let want = middle + c.arm.unit() * ARM_STEP + c.key.unit() * KEY_STEP;
            let quads = paint(&opened(c.arm), &Inventory::default(), Item::Grass);
            let near = quads
                .iter()
                .filter(|q| {
                    let at = Vec2::new(q.x + q.w * 0.5, q.y + q.h * 0.5);
                    at.distance(want) < CELL * 0.5
                })
                .count();
            assert!(near > 3, "{item:?} is not drawn at {want}");
        }
    }

    /// The count is a shape and never a number, and the shape has a ceiling: eight
    /// notches at most, and one solid bar for anything past that. A hundred stone may not
    /// become a hundred rectangles, and four hundred may not cost more to draw than eight.
    #[test]
    fn a_big_pile_is_one_bar_not_a_hundred_notches() {
        let pile_of = |n: u32| {
            let mut pile = Inventory::default();
            for item in Item::ALL {
                pile.add(*item, n);
            }
            pile
        };
        let full = paint(&opened(Dir::Left), &pile_of(MANY), Item::Stone).len();
        let heaped = paint(&opened(Dir::Left), &pile_of(400), Item::Stone).len();
        assert!(
            heaped < full,
            "four hundred of everything drew {heaped} rectangles against {full} for eight"
        );
    }

    /// The ring lands on the key of the thing in your hand, and on no other key —
    /// wherever that key is, and whichever arm the pad happens to be open on.
    ///
    /// Drawn against an empty pile on purpose: with nothing owned, nothing else on the pad
    /// is solid white, so what this measures really is the ring.
    #[test]
    fn only_the_thing_in_your_hand_is_ringed() {
        let solid_white = |q: &&Quad| {
            let c = q.color.to_srgba();
            c.red > 0.95 && c.green > 0.95 && c.blue > 0.95 && c.alpha > 0.4
        };
        for held in Item::ALL {
            let c = code::of(*held);
            let want = PAD_AT + c.arm.unit() * ARM_STEP + c.key.unit() * KEY_STEP;
            let quads = paint(&opened(Dir::Left), &Inventory::default(), *held);
            let ringed: Vec<Vec2> = quads
                .iter()
                .filter(solid_white)
                .map(|q| Vec2::new(q.x + q.w * 0.5, q.y + q.h * 0.5))
                .collect();
            assert!(!ringed.is_empty(), "{held:?} wears no ring");
            for at in ringed {
                assert!(at.distance(want) < CELL, "{held:?} is ringed at {at}");
            }
        }
    }
}
