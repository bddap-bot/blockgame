//! Input: keyboard + mouse and gamepad, folded into one [`Intent`].
//!
//! The Steam Deck is the primary target, so the gamepad path is not a courtesy — it is
//! the main one, and every action including quitting is reachable without a keyboard.
//! Everything downstream reads [`Intent`] and never asks which device the player used, so
//! remapping is a change to [`KEYS`] / [`PAD`] and nothing else.

use crate::registry::HOTBAR_COLUMNS;
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
    pub quit: KeyCode,
    /// Held down to swing, drill or fire — see [`crate::registry::Use`]. Held rather than
    /// tapped because breaking a block takes time now, and how long it takes is the whole
    /// difference between a fist and a drill.
    pub use_item: MouseButton,
    /// Tapped: a held place button sprays a wall of blocks nobody asked for.
    pub place_block: MouseButton,
    /// Makes one of whatever the hotbar cursor is on.
    pub craft: KeyCode,
    /// Gets into the car in front of you, and back out of it again.
    pub ride: KeyCode,
    /// Step the hotbar cursor one cell.
    pub prev_item: KeyCode,
    pub next_item: KeyCode,
    /// Picks a hotbar cell outright, the first cell first. The number row runs out long
    /// before the hotbar does, which is why stepping exists: a new item needs a table row,
    /// never a free key.
    pub slots: [KeyCode; 9],
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
    quit: KeyCode::Escape,
    use_item: MouseButton::Left,
    place_block: MouseButton::Right,
    craft: KeyCode::KeyC,
    ride: KeyCode::KeyR,
    prev_item: KeyCode::KeyQ,
    next_item: KeyCode::KeyE,
    slots: [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ],
};

/// Gamepad bindings, named for the Deck's face labels.
pub struct PadBinds {
    /// A
    pub jump: GamepadButton,
    /// Y
    pub toggle_fly: GamepadButton,
    /// X — makes one of whatever the hotbar cursor is on. The only face button no
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
    /// The hotbar cursor: left and right walk it a cell at a time, up and down jump a
    /// whole row. Two rows of seven are reachable in at most four presses, which is what
    /// keeps a growing item table from turning into a long press-and-hold.
    pub next_item: GamepadButton,
    pub prev_item: GamepadButton,
    pub next_row: GamepadButton,
    pub prev_row: GamepadButton,
    /// Held together to quit. A chord because quitting is instant and unconfirmed, and
    /// these two are the only buttons no gameplay action uses — a thumb cannot land on
    /// both mid-build.
    pub quit: [GamepadButton; 2],
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
    next_item: GamepadButton::DPadRight,
    prev_item: GamepadButton::DPadLeft,
    next_row: GamepadButton::DPadDown,
    prev_row: GamepadButton::DPadUp,
    quit: [GamepadButton::Select, GamepadButton::Start],
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
    /// Hotbar cursor movement, in cells. A row is [`HOTBAR_COLUMNS`] of them.
    pub item_delta: i32,
    /// An absolute hotbar cell, from the number-row keys.
    pub item_pick: Option<usize>,
    /// Make one of whatever the cursor is on.
    pub craft: bool,
    /// Get into your car, or out of it. Tapped: held, it would be a door flapping.
    pub ride: bool,
    pub quit: bool,
}

fn deadzoned(v: Vec2) -> Vec2 {
    if v.length() < STICK_DEADZONE {
        Vec2::ZERO
    } else {
        v
    }
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
    out.quit = keys.just_pressed(KEYS.quit);
    out.item_delta =
        keys.just_pressed(KEYS.next_item) as i32 - keys.just_pressed(KEYS.prev_item) as i32;
    for (slot, key) in KEYS.slots.iter().enumerate() {
        if keys.just_pressed(*key) {
            out.item_pick = Some(slot);
        }
    }

    // Every connected pad contributes; the Deck's built-in controls are one of them.
    for pad in &pads {
        let walk = deadzoned(pad.left_stick());
        if walk != Vec2::ZERO {
            out.walk += walk;
        }
        let look = deadzoned(pad.right_stick());
        if look != Vec2::ZERO {
            out.look += Vec2::new(-look.x, look.y) * STICK_LOOK_SPEED * dt;
        }
        out.jump |= pad.pressed(PAD.jump);
        out.sprint |= pad.pressed(PAD.sprint);
        out.vertical +=
            pad.pressed(PAD.jump) as i32 as f32 - pad.pressed(PAD.descend) as i32 as f32;
        out.toggle_fly |= pad.just_pressed(PAD.toggle_fly);
        out.use_item |= pad.pressed(PAD.use_item);
        out.place_block |= pad.just_pressed(PAD.place_block);
        out.craft |= pad.just_pressed(PAD.craft);
        out.ride |= pad.just_pressed(PAD.ride);
        out.item_delta +=
            pad.just_pressed(PAD.next_item) as i32 - pad.just_pressed(PAD.prev_item) as i32;
        out.item_delta += (pad.just_pressed(PAD.next_row) as i32
            - pad.just_pressed(PAD.prev_row) as i32)
            * HOTBAR_COLUMNS as i32;
        out.quit |= PAD.quit.iter().all(|&b| pad.pressed(b));
    }

    out.walk = out.walk.clamp_length_max(1.0);
    out.vertical = out.vertical.clamp(-1.0, 1.0);
    *intent = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_kills_stick_drift_but_not_real_input() {
        assert_eq!(deadzoned(Vec2::new(0.1, 0.05)), Vec2::ZERO);
        assert_eq!(deadzoned(Vec2::new(0.9, 0.0)), Vec2::new(0.9, 0.0));
    }
}
