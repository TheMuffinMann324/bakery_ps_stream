//! Maps gamepads (via gilrs) and the keyboard to a PlayStation controller state.

use crate::ffi::PsController;
use eframe::egui::{self, Key};
use gilrs::{Axis, Button, GamepadId, Gilrs};

// ChiakiControllerButton bits.
const CROSS: u32 = 1 << 0;
const MOON: u32 = 1 << 1;
const BOX: u32 = 1 << 2;
const PYRAMID: u32 = 1 << 3;
const DPAD_LEFT: u32 = 1 << 4;
const DPAD_RIGHT: u32 = 1 << 5;
const DPAD_UP: u32 = 1 << 6;
const DPAD_DOWN: u32 = 1 << 7;
const L1: u32 = 1 << 8;
const R1: u32 = 1 << 9;
const L3: u32 = 1 << 10;
const R3: u32 = 1 << 11;
const OPTIONS: u32 = 1 << 12;
const SHARE: u32 = 1 << 13;
const TOUCHPAD: u32 = 1 << 14;
const PS: u32 = 1 << 15;

const GAMEPAD_BUTTONS: &[(Button, u32)] = &[
    (Button::South, CROSS),
    (Button::East, MOON),
    (Button::West, BOX),
    (Button::North, PYRAMID),
    (Button::DPadLeft, DPAD_LEFT),
    (Button::DPadRight, DPAD_RIGHT),
    (Button::DPadUp, DPAD_UP),
    (Button::DPadDown, DPAD_DOWN),
    (Button::LeftTrigger, L1),
    (Button::RightTrigger, R1),
    (Button::LeftThumb, L3),
    (Button::RightThumb, R3),
    (Button::Start, OPTIONS),
    (Button::Select, SHARE),
    (Button::Mode, PS),
];

/// Keyboard layout, shown in the UI as help text.
pub const KEYBOARD_HELP: &str = "Keyboard: WASD left stick · IJKL right stick · arrows D-pad · \
Enter Cross · Backspace Circle · Q Square · E Triangle · 1 L1 · 3 R1 · 2 L2 · 4 R2 · \
Z L3 · X R3 · O Options · P Share · T Touchpad · H PS button";

const KEYBOARD_BUTTONS: &[(Key, u32)] = &[
    (Key::Enter, CROSS),
    (Key::Backspace, MOON),
    (Key::Q, BOX),
    (Key::E, PYRAMID),
    (Key::ArrowLeft, DPAD_LEFT),
    (Key::ArrowRight, DPAD_RIGHT),
    (Key::ArrowUp, DPAD_UP),
    (Key::ArrowDown, DPAD_DOWN),
    (Key::Num1, L1),
    (Key::Num3, R1),
    (Key::Z, L3),
    (Key::X, R3),
    (Key::O, OPTIONS),
    (Key::P, SHARE),
    (Key::T, TOUCHPAD),
    (Key::H, PS),
];

pub struct Input {
    gilrs: Option<Gilrs>,
    active: Option<GamepadId>,
}

impl Input {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                crate::remote::log_line(format!("[W] Gamepad support unavailable: {e}"));
                None
            }
        };
        Self {
            gilrs,
            active: None,
        }
    }

    /// Name of the gamepad currently driving input, if any.
    pub fn active_gamepad_name(&self) -> Option<String> {
        let gilrs = self.gilrs.as_ref()?;
        let id = self.active?;
        Some(gilrs.gamepad(id).name().to_owned())
    }

    pub fn poll(&mut self, ctx: &egui::Context, keyboard_enabled: bool) -> PsController {
        let mut state = PsController::default();

        if let Some(gilrs) = self.gilrs.as_mut() {
            while let Some(ev) = gilrs.next_event() {
                self.active = Some(ev.id);
            }
            if self.active.is_none() {
                self.active = gilrs.gamepads().next().map(|(id, _)| id);
            }
            if let Some(id) = self.active {
                let pad = gilrs.gamepad(id);
                if pad.is_connected() {
                    for &(button, bit) in GAMEPAD_BUTTONS {
                        if pad.is_pressed(button) {
                            state.buttons |= bit;
                        }
                    }
                    let trigger = |b| pad.button_data(b).map_or(0.0, |d| d.value());
                    state.l2 = (trigger(Button::LeftTrigger2) * 255.0) as u8;
                    state.r2 = (trigger(Button::RightTrigger2) * 255.0) as u8;
                    // gilrs: up is positive; PlayStation: up is negative.
                    state.left_x = axis(pad.value(Axis::LeftStickX));
                    state.left_y = axis(-pad.value(Axis::LeftStickY));
                    state.right_x = axis(pad.value(Axis::RightStickX));
                    state.right_y = axis(-pad.value(Axis::RightStickY));
                } else {
                    self.active = None;
                }
            }
        }

        if keyboard_enabled {
            ctx.input(|i| {
                for &(key, bit) in KEYBOARD_BUTTONS {
                    if i.key_down(key) {
                        state.buttons |= bit;
                    }
                }
                if i.key_down(Key::Num2) {
                    state.l2 = 255;
                }
                if i.key_down(Key::Num4) {
                    state.r2 = 255;
                }
                let stick = |neg: Key, pos: Key| match (i.key_down(neg), i.key_down(pos)) {
                    (true, false) => -32767,
                    (false, true) => 32767,
                    _ => 0,
                };
                let (lx, ly) = (stick(Key::A, Key::D), stick(Key::W, Key::S));
                let (rx, ry) = (stick(Key::J, Key::L), stick(Key::I, Key::K));
                if lx != 0 {
                    state.left_x = lx;
                }
                if ly != 0 {
                    state.left_y = ly;
                }
                if rx != 0 {
                    state.right_x = rx;
                }
                if ry != 0 {
                    state.right_y = ry;
                }
            });
        }

        state
    }
}

fn axis(v: f32) -> i16 {
    // Small deadzone so idle sticks don't spam state updates.
    if v.abs() < 0.05 {
        0
    } else {
        (v.clamp(-1.0, 1.0) * 32767.0) as i16
    }
}
