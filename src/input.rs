//! Input: keyboard + mouse and gamepad, folded into one [`Intent`].
//!
//! The Steam Deck is the primary target, so the gamepad path is not a courtesy — it is
//! the main one. Everything downstream reads [`Intent`] and never asks which device the
//! player used, so remapping is a change to [`KEYS`] / [`PAD`] and nothing else.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

/// Radians of turn per pixel of mouse movement.
const MOUSE_SENSITIVITY: f32 = 0.0022;
/// Radians per second at full right-stick deflection.
const STICK_LOOK_SPEED: f32 = 3.4;
/// Sticks report small values at rest; below this they read as zero.
const STICK_DEADZONE: f32 = 0.18;
/// Analog triggers count as pressed past this.
const TRIGGER_THRESHOLD: f32 = 0.5;
/// Pitch stops just shy of straight up/down, where the camera basis degenerates.
pub const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;
const _: () = assert!(PITCH_LIMIT < std::f32::consts::FRAC_PI_2 && PITCH_LIMIT > 1.5);

/// Keyboard bindings. One row per action — rebinding is a one-line diff.
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
};

/// Gamepad bindings, named for the Deck's face labels.
pub struct PadBinds {
    /// A
    pub jump: GamepadButton,
    /// Y
    pub toggle_fly: GamepadButton,
    /// R2
    pub break_block: GamepadButton,
    /// L2
    pub place_block: GamepadButton,
    /// R1 — hold to move faster
    pub sprint: GamepadButton,
    /// L1 — hold to descend while flying
    pub descend: GamepadButton,
    pub next_item: GamepadButton,
    pub prev_item: GamepadButton,
}

pub const PAD: PadBinds = PadBinds {
    jump: GamepadButton::South,
    toggle_fly: GamepadButton::North,
    break_block: GamepadButton::RightTrigger2,
    place_block: GamepadButton::LeftTrigger2,
    sprint: GamepadButton::RightTrigger,
    descend: GamepadButton::LeftTrigger,
    next_item: GamepadButton::DPadRight,
    prev_item: GamepadButton::DPadLeft,
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
    pub break_block: bool,
    pub place_block: bool,
    /// Hotbar movement, in slots (d-pad).
    pub item_delta: i32,
    /// An absolute hotbar slot, from the number-row keys.
    pub item_pick: Option<usize>,
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
    out.break_block = mouse.just_pressed(MouseButton::Left);
    out.place_block = mouse.just_pressed(MouseButton::Right);
    out.quit = keys.just_pressed(KEYS.quit);
    for (i, key) in [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ]
    .into_iter()
    .enumerate()
    {
        if keys.just_pressed(key) {
            out.item_pick = Some(i);
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
        out.sprint |= pad.pressed(PAD.sprint) || trigger_held(pad, PAD.sprint);
        out.vertical += pad.pressed(PAD.jump) as i32 as f32
            - (pad.pressed(PAD.descend) || trigger_held(pad, PAD.descend)) as i32 as f32;
        out.toggle_fly |= pad.just_pressed(PAD.toggle_fly);
        out.break_block |= pad.just_pressed(PAD.break_block);
        out.place_block |= pad.just_pressed(PAD.place_block);
        out.item_delta +=
            pad.just_pressed(PAD.next_item) as i32 - pad.just_pressed(PAD.prev_item) as i32;
    }

    out.walk = out.walk.clamp_length_max(1.0);
    out.vertical = out.vertical.clamp(-1.0, 1.0);
    *intent = out;
}

/// Analog shoulder triggers report a value rather than a press on some pads, so check
/// both.
fn trigger_held(pad: &Gamepad, button: GamepadButton) -> bool {
    pad.get(button).is_some_and(|v| v >= TRIGGER_THRESHOLD)
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
