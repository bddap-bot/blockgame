//! Input: keyboard + mouse and gamepad, folded into one [`Intent`].
//!
//! The Steam Deck is the primary target, so the gamepad path is not a courtesy — it is
//! the main one, and every action including quitting is reachable without a keyboard.
//! Everything downstream reads [`Intent`] and never asks which device the player used, so
//! remapping is a change to [`KEYS`] / [`PAD`] and nothing else.

use crate::code::Dir;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

/// Radians of turn per pixel of mouse movement.
const MOUSE_SENSITIVITY: f32 = 0.0022;
/// Radians per second at full right-stick deflection.
const STICK_LOOK_SPEED: f32 = 3.4;
/// Sticks report small values at rest; below this they read as zero.
const STICK_DEADZONE: f32 = 0.18;
/// Pitch stops just shy of straight up/down, where the camera basis degenerates.
pub const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;
const _: () = assert!(PITCH_LIMIT < std::f32::consts::FRAC_PI_2 && PITCH_LIMIT > 1.5);

/// Keyboard and mouse bindings. One row per action — rebinding is a one-line diff.
pub struct KeyBinds {
    pub forward: KeyCode,
    pub back: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub jump: KeyCode,
    pub sprint: KeyCode,
    pub descend: KeyCode,
    pub toggle_fly: KeyCode,
    /// Opens the pause menu, which is where quitting and sharing the world's ticket
    /// live. There is no key that quits outright: an unconfirmed quit is a lost world.
    pub pause: KeyCode,
    /// Held down to swing, drill or fire — see [`crate::registry::Use`]. Held rather than
    /// tapped because breaking a block takes time now, and how long it takes is the whole
    /// difference between a fist and a drill.
    pub use_item: MouseButton,
    /// Tapped: a held place button sprays a wall of blocks nobody asked for.
    pub place_block: MouseButton,
    /// Makes one of whatever you are holding.
    pub craft: KeyCode,
    /// Gets into the car in front of you, and back out of it again.
    pub ride: KeyCode,
    /// The four keys a code is typed on, in [`Dir::ALL`] order. There is no row of number
    /// keys any more: a number key reaches nine cells and then stops, and what replaced it
    /// reaches everything in two presses and never runs out.
    pub dpad: [KeyCode; 4],
}

pub const KEYS: KeyBinds = KeyBinds {
    forward: KeyCode::KeyW,
    back: KeyCode::KeyS,
    left: KeyCode::KeyA,
    right: KeyCode::KeyD,
    jump: KeyCode::Space,
    sprint: KeyCode::ShiftLeft,
    descend: KeyCode::ControlLeft,
    toggle_fly: KeyCode::KeyF,
    pause: KeyCode::Escape,
    use_item: MouseButton::Left,
    place_block: MouseButton::Right,
    craft: KeyCode::KeyC,
    ride: KeyCode::KeyR,
    dpad: [
        KeyCode::ArrowLeft,
        KeyCode::ArrowUp,
        KeyCode::ArrowRight,
        KeyCode::ArrowDown,
    ],
};

/// Gamepad bindings, named for the Deck's face labels.
pub struct PadBinds {
    /// A
    pub jump: GamepadButton,
    /// Y
    pub toggle_fly: GamepadButton,
    /// X — makes one of whatever you are holding. The only face button no
    /// movement action uses, so crafting never fights with jumping.
    pub craft: GamepadButton,
    /// B — get in the car, get out of the car. The last free face button, and the one a
    /// thumb is already resting on.
    pub ride: GamepadButton,
    /// R2 — the trigger, held: swing, drill or fire whatever is in hand.
    pub use_item: GamepadButton,
    /// L2 — the other trigger, tapped: put a block down.
    pub place_block: GamepadButton,
    /// R1 — hold to move faster
    pub sprint: GamepadButton,
    /// L1 — hold to descend while flying
    pub descend: GamepadButton,
    /// The pad, in [`Dir::ALL`] order — the four keys every code is typed on. Two presses
    /// reach anything the game has, and go on reaching it as the registry grows.
    pub dpad: [GamepadButton; 4],
    /// Start — opens the pause menu, where quitting and sharing this world's ticket
    /// live. It used to take a two-button chord to quit, which nobody who had not read
    /// the source could find; a menu you can see is worth more than a chord you cannot
    /// hit by accident.
    pub pause: GamepadButton,
}

pub const PAD: PadBinds = PadBinds {
    jump: GamepadButton::South,
    toggle_fly: GamepadButton::North,
    craft: GamepadButton::West,
    ride: GamepadButton::East,
    use_item: GamepadButton::RightTrigger2,
    place_block: GamepadButton::LeftTrigger2,
    sprint: GamepadButton::RightTrigger,
    descend: GamepadButton::LeftTrigger,
    dpad: [
        GamepadButton::DPadLeft,
        GamepadButton::DPadUp,
        GamepadButton::DPadRight,
        GamepadButton::DPadDown,
    ],
    pause: GamepadButton::Start,
};

/// What the player asked for this frame, device-independent.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Intent {
    /// `x` strafes right, `y` walks forward. Length is clamped to 1.
    pub walk: Vec2,
    /// Radians to add to yaw (`x`) and pitch (`y`) this frame.
    pub look: Vec2,
    pub jump: bool,
    pub sprint: bool,
    /// While flying: `+1` up, `-1` down.
    pub vertical: f32,
    pub toggle_fly: bool,
    /// Held, not tapped: this drives [`crate::registry::Use`], and a block takes time to
    /// break.
    pub use_item: bool,
    pub place_block: bool,
    /// Make one of whatever the cursor is on.
    pub craft: bool,
    /// Get into your car, or out of it. Tapped: held, it would be a door flapping.
    pub ride: bool,
    /// Open the pause menu.
    pub pause: bool,
}

/// What the player asked of a menu.
///
/// Separate from [`Intent`] because a menu is asked different questions than a world is,
/// and folding "which row" into "which way am I walking" would put a cursor on every
/// gameplay system's plate. The *bindings* are shared where they overlap — confirm is
/// [`PadBinds::jump`], the same A that jumps — so a button means one thing everywhere.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct MenuIntent {
    /// Rows to move the cursor, this frame.
    pub step: i32,
    pub confirm: bool,
    /// Back out: leave the pause menu, or leave the game from the title.
    pub back: bool,
    /// Whether the stick was already pushed last frame. A held stick steps once, not
    /// once per frame — a list that scrolls at 60 rows a second is unusable.
    stick_pushed: bool,
}

/// The d-pad, read on its own.
///
/// Not folded into [`Intent`] with the walking and the shooting, because the pad is the one
/// control that means the same thing in every state the game has: in the world it picks
/// what you hold, inside the crafting rig it picks what the rig is showing, and it is the
/// same two presses either way. One resource, gathered once, and no state gets to decide
/// that left means something else today.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Drum {
    pub press: Option<Dir>,
}

fn deadzoned(v: Vec2) -> Vec2 {
    if v.length() < STICK_DEADZONE {
        Vec2::ZERO
    } else {
        v
    }
}

/// One controller can arrive as several pads: on the TV, Steam Input publishes a virtual
/// pad beside the physical device it wraps, and both report the same thumb. So every read
/// below folds the connected pads into one — a press *any* pad reports is one press, and a
/// stick takes the strongest reading rather than the sum.
///
/// Summing is what made the TV feel broken: one d-pad press counted as two,
/// and one stick push turned twice as fast as on the Deck.
fn tapped(pads: &Query<&Gamepad>, button: GamepadButton) -> bool {
    pads.iter().any(|pad| pad.just_pressed(button))
}

fn held(pads: &Query<&Gamepad>, button: GamepadButton) -> bool {
    pads.iter().any(|pad| pad.pressed(button))
}

fn stick(pads: &Query<&Gamepad>, which: fn(&Gamepad) -> Vec2) -> Vec2 {
    pads.iter()
        .map(|pad| deadzoned(which(pad)))
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .unwrap_or(Vec2::ZERO)
}

pub fn gather_intent(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    pads: Query<&Gamepad>,
    time: Res<Time>,
    mut intent: ResMut<Intent>,
) {
    let dt = time.delta_secs();
    let mut out = Intent::default();

    let axis =
        |neg: KeyCode, pos: KeyCode| (keys.pressed(pos) as i32 - keys.pressed(neg) as i32) as f32;
    out.walk = Vec2::new(axis(KEYS.left, KEYS.right), axis(KEYS.back, KEYS.forward));
    out.look = Vec2::new(-motion.delta.x, -motion.delta.y) * MOUSE_SENSITIVITY;
    out.jump = keys.pressed(KEYS.jump);
    out.sprint = keys.pressed(KEYS.sprint);
    out.vertical = keys.pressed(KEYS.jump) as i32 as f32 - keys.pressed(KEYS.descend) as i32 as f32;
    out.toggle_fly = keys.just_pressed(KEYS.toggle_fly);
    out.use_item = mouse.pressed(KEYS.use_item);
    out.place_block = mouse.just_pressed(KEYS.place_block);
    out.craft = keys.just_pressed(KEYS.craft);
    out.ride = keys.just_pressed(KEYS.ride);
    out.pause = keys.just_pressed(KEYS.pause);

    // The pads, folded into one — see [`tapped`]. The Deck's built-in controls are one of
    // them.
    out.walk += stick(&pads, Gamepad::left_stick);
    let look = stick(&pads, Gamepad::right_stick);
    out.look += Vec2::new(-look.x, look.y) * STICK_LOOK_SPEED * dt;
    out.jump |= held(&pads, PAD.jump);
    out.sprint |= held(&pads, PAD.sprint);
    out.vertical += held(&pads, PAD.jump) as i32 as f32 - held(&pads, PAD.descend) as i32 as f32;
    out.toggle_fly |= tapped(&pads, PAD.toggle_fly);
    out.use_item |= held(&pads, PAD.use_item);
    out.place_block |= tapped(&pads, PAD.place_block);
    out.craft |= tapped(&pads, PAD.craft);
    out.ride |= tapped(&pads, PAD.ride);
    out.pause |= tapped(&pads, PAD.pause);

    out.walk = out.walk.clamp_length_max(1.0);
    out.vertical = out.vertical.clamp(-1.0, 1.0);
    *intent = out;
}

/// The menu's own read of the same devices. Runs while a menu is up and nowhere else, so
/// a d-pad press cannot both drum the pad and move a menu cursor.
pub fn gather_menu_intent(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut menu: ResMut<MenuIntent>,
) {
    let key_step =
        |up: KeyCode, down: KeyCode| keys.just_pressed(down) as i32 - keys.just_pressed(up) as i32;
    let mut out = MenuIntent {
        step: key_step(KeyCode::ArrowUp, KeyCode::ArrowDown) + key_step(KEYS.forward, KEYS.back),
        confirm: keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space),
        back: keys.just_pressed(KEYS.pause),
        stick_pushed: false,
    };

    // Folded across the pads, exactly as gameplay input is — a menu that moved two rows
    // per press on the TV would be the same bug wearing a different hat.
    out.step += tapped(&pads, PAD.dpad[Dir::Down.index()]) as i32
        - tapped(&pads, PAD.dpad[Dir::Up.index()]) as i32;
    out.confirm |= tapped(&pads, PAD.jump);
    // B backs out, and so does Start — whichever button opened this, the same one and the
    // obvious one both close it.
    out.back |= tapped(&pads, PAD.ride) || tapped(&pads, PAD.pause);
    // The stick reads as a d-pad: pushed past the deadzone is one step, and it has to come
    // back to centre before it gives another.
    let pushed = stick(&pads, Gamepad::left_stick).y;
    if pushed != 0.0 {
        out.stick_pushed = true;
        if !menu.stick_pushed {
            out.step += if pushed < 0.0 { 1 } else { -1 };
        }
    }

    *menu = out;
}

/// The pad, typed. One press a frame at most: a code is a rhythm, and two directions
/// reported in the same frame is one of them lost.
///
/// Read every frame in every state, which is what makes the code mean one thing
/// everywhere. What acts on it is [`crate::hotbar::drum`], and that runs only where a
/// selection makes sense.
pub fn gather_drum(keys: Res<ButtonInput<KeyCode>>, pads: Query<&Gamepad>, mut drum: ResMut<Drum>) {
    drum.press = Dir::ALL.into_iter().find(|dir| {
        keys.just_pressed(KEYS.dpad[dir.index()]) || tapped(&pads, PAD.dpad[dir.index()])
    });
}

/// What the crafting rig is asked, beyond the pad it shares with the world.
///
/// The rig has no navigation of its own any more: the d-pad picks a thing, the same two
/// presses it picks one with anywhere else, and the graph re-centres on whatever was
/// picked. What is left here is the two buttons the rig does own — make one, and leave.
pub fn gather_forge_nav(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut nav: ResMut<crate::forge::Nav>,
) {
    let mut out = crate::forge::Nav {
        craft: keys.just_pressed(KEYS.craft),
        leave: keys.just_pressed(KEYS.pause) || keys.just_pressed(KEYS.ride),
    };

    for pad in &pads {
        out.craft |= pad.just_pressed(PAD.craft);
        out.leave |= pad.just_pressed(PAD.ride) || pad.just_pressed(PAD.pause);
    }

    *nav = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::InputPlugin;
    use bevy::input::gamepad::{
        GamepadConnection, GamepadConnectionEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
    };

    /// An app with `count` pads plugged in and nothing else — the TV's shape at `count` of
    /// 2, where one controller arrives as two devices.
    fn with_pads(count: usize) -> (App, Vec<Entity>) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .init_resource::<Drum>()
            .add_systems(Update, gather_drum);

        let pads: Vec<Entity> = (0..count)
            .map(|_| {
                let pad = app.world_mut().spawn_empty().id();
                app.world_mut().write_message(GamepadConnectionEvent::new(
                    pad,
                    GamepadConnection::Connected {
                        name: "test pad".to_string(),
                        vendor_id: None,
                        product_id: None,
                    },
                ));
                pad
            })
            .collect();
        app.update();
        (app, pads)
    }

    /// Presses `button` on every pad at once, as two devices wrapping one controller do,
    /// and reports the one key the game heard.
    fn press(app: &mut App, pads: &[Entity], button: GamepadButton) -> Option<Dir> {
        for pad in pads {
            app.world_mut().write_message(RawGamepadEvent::Button(
                RawGamepadButtonChangedEvent::new(*pad, button, 1.0),
            ));
        }
        app.update();
        app.world().resource::<Drum>().press
    }

    /// The TV bug, in the shape it takes now: Steam Input's virtual pad and the controller
    /// it wraps both report the press, and a code typed on the TV used to eat two keys per
    /// thumb — which on a two-press code means the wrong thing every time. One press is one
    /// key, however many devices saw it.
    #[test]
    fn one_thumb_is_one_key_however_many_pads_report_it() {
        for pads in [1, 2, 3] {
            for dir in Dir::ALL {
                let (mut app, ids) = with_pads(pads);
                assert_eq!(
                    press(&mut app, &ids, PAD.dpad[dir.index()]),
                    Some(dir),
                    "{pads} pad(s) reporting one {dir:?}"
                );
                // And it is over: a held key is one key, not a key a frame.
                app.update();
                assert_eq!(app.world().resource::<Drum>().press, None);
            }
        }
    }

    #[test]
    fn deadzone_kills_stick_drift_but_not_real_input() {
        assert_eq!(deadzoned(Vec2::new(0.1, 0.05)), Vec2::ZERO);
        assert_eq!(deadzoned(Vec2::new(0.9, 0.0)), Vec2::new(0.9, 0.0));
    }
}
