//! The code — every thing you can hold is two presses of the d-pad, and every code is a
//! tune.
//!
//! A hotbar you *walk* costs one press per cell and grows a press longer every time the
//! registry grows a row. A hotbar you *type* costs two presses for ever: four keys pick
//! one of four arms, four keys pick one of four things on that arm, and sixteen keys is
//! more than the game has to hold. Two presses reach anything, from anything, with no
//! travel in between — so what a player learns is not where a thing sits in a line, it is
//! a pair of thumb movements, and a pair of thumb movements is a thing a four-year-old
//! learns the way they learn a dance step.
//!
//! **The picture is the pad.** [`ROSETTE`] is laid out on screen exactly as it is written
//! here: four arms in a cross, four keys in a cross on each arm. So the shape a player
//! looks at is the shape of the thing under their thumb, twice, and the position of a
//! thing on the screen *is* its code. Nothing has to be read to work that out.
//!
//! **The code is also a sound.** Each direction is a note — down lowest, up highest, and
//! left below right, which is the arrangement of a piano and of every stairway. The arm
//! press sounds it low and the key press sounds it an octave up, so each of the sixteen
//! keys has its own two-note tune. Drum the same code often enough and you hear the thing
//! you picked before you have looked at it, which is the point: the hand learns the tune
//! and the eyes go back to the world.

use bevy::prelude::*;

use crate::registry::Item;

/// Keys on a d-pad, and so the width of every level of a code.
pub const KEYS: usize = 4;

/// One press of the pad.
///
/// Ordered as it is drawn and as it sounds: this is the order [`ROSETTE`]'s rows and
/// columns are in, so a row of the table below is what a player sees on the screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Dir {
    Left,
    Up,
    Right,
    Down,
}

impl Dir {
    pub const ALL: [Dir; KEYS] = [Dir::Left, Dir::Up, Dir::Right, Dir::Down];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Which way this points on the screen, `y` running *down* as the UI's does.
    pub fn unit(self) -> Vec2 {
        match self {
            Dir::Left => Vec2::new(-1.0, 0.0),
            Dir::Up => Vec2::new(0.0, -1.0),
            Dir::Right => Vec2::new(1.0, 0.0),
            Dir::Down => Vec2::new(0.0, 1.0),
        }
    }

    /// The colour this direction is drawn in, everywhere it is drawn.
    ///
    /// One hue per key, held to across the whole surface — the big pad, the little code
    /// chips under a held thing, the flash a press makes. A child who has not worked out
    /// that the gold key is *right* has still worked out that the gold thing and the gold
    /// key go together, which is the same fact arriving by a different road.
    pub fn tint(self) -> Color {
        match self {
            Dir::Left => Color::srgb(0.45, 0.88, 0.62),
            Dir::Up => Color::srgb(0.45, 0.76, 1.00),
            Dir::Right => Color::srgb(1.00, 0.78, 0.32),
            Dir::Down => Color::srgb(1.00, 0.48, 0.70),
        }
    }

    /// Semitones above the root of the key's octave — a major pentatonic, so no two keys
    /// pressed in any order can sound wrong together. There is no dissonant code, which
    /// matters when the instrument is also the inventory and it gets drummed all day.
    fn semitone(self) -> i32 {
        match self {
            Dir::Down => 0,
            Dir::Left => 2,
            Dir::Right => 4,
            Dir::Up => 7,
        }
    }

    /// What this key sounds like. The arm press is the low octave and the key press the
    /// one above it, so a code is always a low note answered by a high one and the two
    /// halves of it can never be mistaken for each other.
    pub fn hz(self, high: bool) -> f32 {
        let root: f32 = if high { 523.25 } else { 261.63 };
        root * 2f32.powf(self.semitone() as f32 / 12.0)
    }
}

/// Two presses: which arm, then which key on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Code {
    pub arm: Dir,
    pub key: Dir,
}

/// Where every item lives on the pad, and so what its code is. **The single source** —
/// the screen is drawn from it, the presses are read against it, and the tune comes out
/// of the same two directions.
///
/// The arms are families a child already has: the ground you dig, the forest and the soft
/// things it becomes, the four things you swing or fire, and the nail with what eight of
/// them turn into. A family is not the mechanism — the mechanism is that a thing is always
/// in the same place — but a family is what makes the first guess right.
///
/// Empty keys are deliberate: the pad has room, and an item added to the registry gets one
/// of them. The seventeenth item is the one that has no key left, and [`FITS`] is what
/// refuses to compile it rather than leaving it unreachable.
const ROSETTE: [[Option<Item>; KEYS]; KEYS] = [
    // Left — the ground, in the order a hole goes down through it.
    [
        Some(Item::Grass),
        Some(Item::Dirt),
        Some(Item::Stone),
        Some(Item::Sand),
    ],
    // Up — the forest, and the two soft things made out of it.
    [
        Some(Item::Wood),
        Some(Item::Leaves),
        Some(Item::Cushion),
        Some(Item::Parachute),
    ],
    // Right — everything with a trigger or a swing on it.
    [
        Some(Item::Hammer),
        Some(Item::Drill),
        Some(Item::Handgun),
        Some(Item::Rifle),
    ],
    // Down — the nail, and the eight-nail thing you drive away in.
    [Some(Item::Nail), Some(Item::Car), None, None],
];

/// Every item has exactly one key, and no key answers to two items — checked while the
/// game is compiled, not while it is played.
///
/// This is the whole safety of splitting "what exists" (the registry) from "where you
/// press for it" (the rosette). They are genuinely two decisions — a thing existing does
/// not say where a thumb should go for it — but a thing that exists with nowhere to press
/// is an item no player can ever hold, and that has to be a build error.
const FITS: () = {
    let mut seen = [false; Item::COUNT];
    let mut arm = 0;
    while arm < KEYS {
        let mut key = 0;
        while key < KEYS {
            if let Some(item) = ROSETTE[arm][key] {
                let i = item as usize;
                assert!(!seen[i], "two keys of the rosette answer to the same item");
                seen[i] = true;
            }
            key += 1;
        }
        arm += 1;
    }
    let mut i = 0;
    while i < Item::COUNT {
        assert!(seen[i], "an item has no key on the rosette — give it one");
        i += 1;
    }
};
const _: () = FITS;

/// What is behind a code, or nothing for one of the pad's spare keys.
pub fn at(code: Code) -> Option<Item> {
    ROSETTE[code.arm.index()][code.key.index()]
}

/// The four things on one arm, in key order. What the pad draws, and what a player sees
/// the moment they press one direction.
pub fn arm(arm: Dir) -> [Option<Item>; KEYS] {
    ROSETTE[arm.index()]
}

/// An item's code. Total: [`FITS`] is what makes it so.
pub fn of(item: Item) -> Code {
    for a in Dir::ALL {
        for k in Dir::ALL {
            if ROSETTE[a.index()][k.index()] == Some(item) {
                return Code { arm: a, key: k };
            }
        }
    }
    unreachable!("every item has a key — see FITS")
}

/// A note the pad has struck and nobody has played yet.
///
/// Pushed rather than played, for the reason the rig pushes crafts: which speaker this
/// comes out of, and whether there is one at all, is not the pad's business. It also means
/// the film drives the pad, watches it strike, and never has to own an audio device.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Note {
    pub dir: Dir,
    /// The key press, an octave up. The arm press is the low one.
    pub high: bool,
}

/// The pad, mid-code.
///
/// **Exactly two presses, always.** The first opens the pad on an arm; the second closes
/// it, whatever it lands on. A press onto one of the spare keys picks nothing and the pad
/// shuts anyway — so there is no cancel button to teach, no timeout to wait out, and no
/// way to be left half-way through a code wondering which press the game is still holding.
#[derive(Resource, Default)]
pub struct Pad {
    arm: Option<Dir>,
    /// How far the pad has bloomed open, 0 to 1. The drawing reads it; nothing else does.
    bloom: f32,
    /// Seconds since the last press, for the flash that goes with it.
    since: f32,
    struck: Option<Code>,
    /// Notes struck and not yet sounded, drained by whoever owns the speaker.
    pub sounded: Vec<Note>,
}

/// How long a key stays flashed after it is hit.
const FLASH: f32 = 0.22;
/// How fast the pad opens and shuts, in fractions of the way there per second.
const BLOOM_RATE: f32 = 7.5;

impl Pad {
    /// Feeds one press in. Returns the item once a code completes — and nothing at all on
    /// the first press of the two, or on a code that lands on a spare key.
    pub fn press(&mut self, dir: Dir) -> Option<Item> {
        self.since = 0.0;
        match self.arm.take() {
            None => {
                // No key flash for the opening press: it lights a whole cluster, and the
                // flash is the landing key's alone — one mark per fact on the screen.
                self.struck = None;
                self.arm = Some(dir);
                self.sounded.push(Note { dir, high: false });
                None
            }
            Some(arm) => {
                let code = Code { arm, key: dir };
                self.struck = Some(code);
                self.sounded.push(Note { dir, high: true });
                at(code)
            }
        }
    }

    /// The arm the pad is open on, if a code is half typed.
    pub fn arm(&self) -> Option<Dir> {
        self.arm
    }

    /// How far open the pad is drawn, 0 shut and 1 wide open.
    pub fn bloom(&self) -> f32 {
        self.bloom
    }

    /// The key the last code landed on and how lit it still is, 1 at the moment of the
    /// press and 0 once the flash has died.
    pub fn flash(&self) -> Option<(Code, f32)> {
        self.struck
            .map(|c| (c, (1.0 - self.since / FLASH).clamp(0.0, 1.0)))
    }

    /// Shuts the pad and forgets a half-typed code. For the one state the two-press rule
    /// cannot cover: a pause mid-code would otherwise freeze the open pad behind the menu
    /// and land the stale arm on the first press after resume.
    pub fn shut(&mut self) {
        self.arm = None;
        self.bloom = 0.0;
        self.struck = None;
    }

    /// Moves the drawing on. The pad is open for exactly as long as a code is half typed,
    /// so nothing outside decides when it shows.
    pub fn tick(&mut self, dt: f32) {
        let want = if self.arm.is_some() { 1.0 } else { 0.0 };
        self.bloom += (want - self.bloom) * (dt * BLOOM_RATE).min(1.0);
        self.since += dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round trip: what a code is drawn as is what pressing it gets you. This is the whole
    /// contract between the picture and the pad.
    #[test]
    fn a_code_gets_the_item_it_is_drawn_under() {
        for item in Item::ALL {
            assert_eq!(at(of(*item)), Some(*item), "{item:?}");
        }
    }

    /// Two presses, every time, for everything — no item is further away than any other,
    /// which is the property a walked hotbar cannot have.
    #[test]
    fn everything_is_two_presses_away() {
        for item in Item::ALL {
            let code = of(*item);
            let mut pad = Pad::default();
            assert_eq!(pad.press(code.arm), None, "the first press picks nothing");
            assert_eq!(pad.press(code.key), Some(*item));
            assert!(pad.arm().is_none(), "the pad shuts on the second press");
        }
    }

    /// Every code sounds different, so the tune identifies the thing on its own. Compared
    /// as the pair of notes it is: two codes that shared both notes would be one tune for
    /// two things, and the ear would have nothing to go on.
    #[test]
    fn no_two_items_share_a_tune() {
        let tune = |item: Item| {
            let c = of(item);
            let cents = |d: Dir, high: bool| (d.hz(high) * 100.0) as i64;
            (cents(c.arm, false), cents(c.key, true))
        };
        for a in Item::ALL {
            for b in Item::ALL {
                assert!(
                    a == b || tune(*a) != tune(*b),
                    "{a:?} and {b:?} sound alike"
                );
            }
        }
    }

    /// The high half of a code is always above the low half, whichever keys they are — so
    /// a code is heard as one gesture with a direction to it rather than as two beeps.
    #[test]
    fn the_second_note_is_always_above_the_first() {
        for low in Dir::ALL {
            for high in Dir::ALL {
                assert!(high.hz(true) > low.hz(false), "{high:?} over {low:?}");
            }
        }
    }

    /// Pitch goes up as the thumb goes up, and left is under right. The one thing about
    /// this instrument that is not arbitrary, and the thing that lets a player guess a
    /// tune they have not heard.
    #[test]
    fn pitch_follows_the_thumb() {
        let hz = |d: Dir| d.hz(false);
        assert!(hz(Dir::Down) < hz(Dir::Left));
        assert!(hz(Dir::Left) < hz(Dir::Right));
        assert!(hz(Dir::Right) < hz(Dir::Up));
    }

    /// A press onto one of the pad's spare keys picks nothing and still shuts the pad.
    /// Every code is two presses, including the ones that turn out to be for nothing.
    #[test]
    fn a_spare_key_picks_nothing_and_still_closes() {
        let mut pad = Pad::default();
        pad.press(Dir::Down);
        assert_eq!(pad.press(Dir::Right), None, "nothing is down-right yet");
        assert!(pad.arm().is_none());
    }

    /// The flash belongs to the completing press alone: it marks the exact key a code
    /// landed on, and an opening press — whose mark is the cluster lighting — leaves no
    /// flash at all. The gate the drawing uses could once never see a second press, so
    /// this is the property that keeps that regression out.
    #[test]
    fn the_completing_press_flashes_its_landing_key() {
        let mut pad = Pad::default();
        pad.press(Dir::Left);
        assert_eq!(pad.flash(), None, "an opening press does not flash a key");
        pad.press(Dir::Up);
        let (struck, heat) = pad.flash().expect("the landing key flashes");
        assert_eq!(
            struck,
            Code {
                arm: Dir::Left,
                key: Dir::Up
            }
        );
        assert_eq!(heat, 1.0, "brightest at the moment of the press");
    }

    /// A pause mid-code must not leave the pad open behind the menu, and must not land
    /// the stale arm on the first press after resume.
    #[test]
    fn shut_forgets_a_half_typed_code() {
        let mut pad = Pad::default();
        pad.press(Dir::Left);
        pad.tick(0.1);
        pad.shut();
        assert_eq!(pad.arm(), None);
        assert_eq!(pad.bloom(), 0.0);
        assert_eq!(pad.flash(), None);
        assert_eq!(
            pad.press(Dir::Up),
            None,
            "the press after a pause opens an arm, it does not finish the old code"
        );
    }

    /// The pad opens while a code is half typed and shuts itself after. Nothing else gets
    /// a say, so it cannot be left open over the world.
    #[test]
    fn the_pad_opens_on_the_first_press_and_shuts_on_the_second() {
        let mut pad = Pad::default();
        assert_eq!(pad.bloom(), 0.0);
        pad.press(Dir::Left);
        for _ in 0..40 {
            pad.tick(1.0 / 60.0);
        }
        assert!(pad.bloom() > 0.9, "half a code leaves the pad open");
        pad.press(Dir::Up);
        for _ in 0..40 {
            pad.tick(1.0 / 60.0);
        }
        assert!(pad.bloom() < 0.1, "a whole code shuts it again");
    }

    /// Both presses sound, low then high, and the pad hands them on rather than playing
    /// them.
    #[test]
    fn both_presses_strike_a_note() {
        let mut pad = Pad::default();
        pad.press(Dir::Right);
        pad.press(Dir::Up);
        assert_eq!(
            pad.sounded,
            vec![
                Note {
                    dir: Dir::Right,
                    high: false
                },
                Note {
                    dir: Dir::Up,
                    high: true
                }
            ]
        );
    }

    /// Four keys of four arms is sixteen, and the game holds fourteen things. The margin
    /// is the point: it is what says the next two items are free, and the one after that
    /// is a decision somebody has to make.
    #[test]
    fn the_pad_has_room_and_says_how_much() {
        let spare = Dir::ALL
            .iter()
            .flat_map(|a| arm(*a))
            .filter(Option::is_none)
            .count();
        assert_eq!(spare + Item::COUNT, KEYS * KEYS);
        assert_eq!(spare, 2, "two keys left before the pad needs a third press");
    }
}
