//! Gamepads (an Xbox-style layout; the Steam Deck's controls arrive through Steam Input as
//! one). The left stick pans the camera, the right stick moves the cursor, A and B click
//! like the mouse's left and right buttons, the triggers zoom, the bumpers hold Shift and
//! Ctrl, and the other buttons stand in for the keys used most. Everything becomes the same
//! input the mouse and keyboard give (the game's `Input`, egui's events), so every page and
//! dialog works with a pad too; on the menus and a battle's dialogs the d-pad also moves the
//! keyboard focus, A works what has it, and B and Start go back ([`Nav`]).

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use glam::Vec2;
use ta_ui::i18n::n;
use winit::keyboard::KeyCode;

/// One frame of pad input, in the terms of mouse and keyboard.
#[derive(Default)]
pub struct PadFrame {
    /// Camera pan, -1..1 each way on screen (x right, y down).
    pub pan: Vec2,
    /// Mouse-wheel lines this frame (positive zooms in).
    pub zoom: f32,
    /// Where the cursor went (window pixels), when it moved.
    pub cursor: Option<Vec2>,
    /// The left and right buttons: pressed (true) or released (false) this frame.
    pub left: Option<bool>,
    pub right: Option<bool>,
    /// Keys struck this frame (the menu key, control groups).
    pub keys: Vec<KeyCode>,
    /// Rebindable actions struck this frame, by name.
    pub actions: Vec<&'static str>,
    /// The interface's moves struck this frame (the app keeps them on the menus and a battle's
    /// dialogs only).
    pub nav: Vec<Nav>,
    pub shift: bool,
    pub ctrl: bool,
}

/// The pad's buttons as the interface takes them, as keys: the d-pad moves the keyboard focus,
/// A works what has it, B (and on the menus Start) goes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    Activate,
    /// B.
    Back,
    /// Start (in a battle, its own Escape: the game menu).
    Menu,
}

/// What a button does to the interface, if anything.
fn nav_of(button: Button) -> Option<Nav> {
    Some(match button {
        Button::DPadUp => Nav::Up,
        Button::DPadDown => Nav::Down,
        Button::DPadLeft => Nav::Left,
        Button::DPadRight => Nav::Right,
        Button::South => Nav::Activate,
        Button::East => Nav::Back,
        Button::Start => Nav::Menu,
        _ => return None,
    })
}

pub struct Gamepad {
    gilrs: Option<Gilrs>,
    /// The pad last used.
    current: Option<GamepadId>,
    /// The cursor's position to sub-pixel precision.
    cursor: Option<Vec2>,
    /// Pointer buttons last sent to the game and egui, released when a pad disappears.
    pointer: PointerButtons,
    focus: FocusGate,
    /// A pad is in use (set by any pad input; the app clears it when the mouse moves).
    pub active: bool,
}

/// A held controller button must not become a fresh click when focus returns. Polling
/// continues while inactive, but input resumes only after a neutral focused frame.
#[derive(Default)]
struct FocusGate { waiting_for_neutral: bool }

impl FocusGate {
    fn accepts(&mut self, focused: bool, neutral: bool) -> bool {
        if !focused {
            self.waiting_for_neutral = true;
            false
        } else if self.waiting_for_neutral {
            if neutral { self.waiting_for_neutral = false; }
            false
        } else {
            true
        }
    }
}

#[derive(Default)]
struct PointerButtons([bool; 2]);

impl PointerButtons {
    fn update(&mut self, held: [bool; 2], pressed: [bool; 2]) -> (Option<bool>, Option<bool>) {
        // A complete tap can arrive between polls. Deliver its press now and its release
        // on the next poll, so both the game and egui see a click rather than no input.
        let held = std::array::from_fn(|i| held[i] || (!self.0[i] && pressed[i]));
        let edge = |i| (self.0[i] != held[i]).then_some(held[i]);
        let events = (edge(0), edge(1));
        self.0 = held;
        events
    }
}

/// Radial dead zone: small deflections are ignored and the rest rescaled to 0..1.
fn stick(x: f32, y: f32, deadzone: f32) -> Vec2 {
    let dead = if deadzone.is_finite() { deadzone.clamp(0.0, 0.5) } else { 0.18 };
    let v = Vec2::new(x, -y);
    let len = v.length();
    if !len.is_finite() || len <= dead { Vec2::ZERO } else { v / len * ((len - dead) / (1.0 - dead)).min(1.0) }
}

fn focus_neutral(x: f32, y: f32, deadzone: f32) -> bool {
    let tolerance = if deadzone.is_finite() { deadzone.clamp(0.05, 0.5) } else { 0.18 };
    stick(x, y, tolerance) == Vec2::ZERO
}

impl Gamepad {
    pub fn new(enabled: bool) -> Gamepad {
        let gilrs = if enabled {
            match Gilrs::new() {
                Ok(g) => {
                    for (_, pad) in g.gamepads() {
                        log::info!("gamepad: {}", pad.name());
                    }
                    Some(g)
                }
                Err(e) => {
                    log::warn!("gamepads unavailable: {e}");
                    None
                }
            }
        } else {
            None
        };
        Gamepad { gilrs, current: None, cursor: None, pointer: PointerButtons::default(), focus: FocusGate::default(), active: false }
    }

    /// Reads the pads: `mouse` is where the cursor is now (the pad's cursor carries on from
    /// there), `window` the window's size in pixels.
    pub fn update(&mut self, dt: f32, mouse: Vec2, window: Vec2, deadzone: f32, sensitivity: f32, focused: bool) -> PadFrame {
        let mut f = PadFrame::default();
        let Some(gilrs) = self.gilrs.as_mut() else { return self.release_pointer() };
        let mut struck: Vec<(GamepadId, Button)> = Vec::new();
        let mut lifted: Vec<Button> = Vec::new();
        while let Some(ev) = gilrs.next_event() {
            match ev.event {
                EventType::ButtonPressed(b, _) => {
                    self.current = Some(ev.id);
                    struck.push((ev.id, b));
                }
                EventType::ButtonReleased(b, _) => lifted.push(b),
                EventType::AxisChanged(..) | EventType::ButtonChanged(..) => self.current = Some(ev.id),
                EventType::Connected => log::info!("gamepad connected: {}", gilrs.gamepad(ev.id).name()),
                EventType::Disconnected
                    if self.current == Some(ev.id) => {
                        self.current = None;
                    }
                _ => {}
            }
        }
        let Some(pad) = self.current.map(|id| gilrs.gamepad(id)).filter(|p| p.is_connected()) else { return self.release_pointer() };
        let held = |b: Button| pad.is_pressed(b);
        let analog = |b: Button| pad.button_data(b).map_or(0.0, |d| d.value());

        let pan = stick(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY), deadzone);
        let aim = stick(pad.value(Axis::RightStickX), pad.value(Axis::RightStickY), deadzone);
        // Rearming after focus loss tolerates a tiny physical drift even when the player
        // deliberately sets the active dead zone to zero.
        let neutral = focus_neutral(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY), deadzone)
            && focus_neutral(pad.value(Axis::RightStickX), pad.value(Axis::RightStickY), deadzone)
            && analog(Button::RightTrigger2) < 0.05 && analog(Button::LeftTrigger2) < 0.05
            && pad.state().buttons().all(|(_, data)| !data.is_pressed());
        if !self.focus.accepts(focused, neutral) { return self.release_pointer(); }
        f.pan = pan;
        f.zoom = (analog(Button::RightTrigger2) - analog(Button::LeftTrigger2)) * 5.0 * dt;
        f.shift = held(Button::LeftTrigger);
        f.ctrl = held(Button::RightTrigger);

        // The cursor: faster the further the stick is pushed (a curve keeps small
        // movements precise), scaled to the window.
        if aim != Vec2::ZERO {
            let sensitivity = if sensitivity.is_finite() { sensitivity.clamp(0.25, 3.0) } else { 1.0 };
            let speed = 1250.0 * sensitivity * (window.y / 1080.0).max(0.5);
            let v = aim.normalize() * aim.length().powf(1.7) * speed * dt;
            let from = match self.cursor {
                Some(c) if c.distance(mouse) < 2.0 => c,
                _ => mouse,
            };
            let to = (from + v).clamp(Vec2::ZERO, window - Vec2::ONE);
            self.cursor = Some(to);
            f.cursor = Some(to);
        }

        // Combine the current pad's state with its press edges: changing pads releases the
        // old pad's drag while short taps still reach both input consumers.
        let pressed = |button| struck.iter().any(|(id, b)| Some(*id) == self.current && *b == button);
        (f.left, f.right) = self.pointer.update([held(Button::South), held(Button::East)], [pressed(Button::South), pressed(Button::East)]);
        for (_, b) in struck.iter().filter(|(id, _)| Some(*id) == self.current) {
            match b {
                Button::West => f.actions.push("fight"),
                Button::North => f.actions.push("stop"),
                Button::Select => f.actions.push("idle_builder"),
                Button::LeftThumb => f.actions.push("follow"),
                Button::RightThumb => f.actions.push("select_type"),
                Button::Start => f.keys.push(KeyCode::Escape),
                Button::DPadUp => f.keys.push(KeyCode::Digit1),
                Button::DPadRight => f.keys.push(KeyCode::Digit2),
                Button::DPadDown => f.keys.push(KeyCode::Digit3),
                Button::DPadLeft => f.keys.push(KeyCode::Digit4),
                _ => {}
            }
            f.nav.extend(nav_of(*b));
        }
        let used = f.pan != Vec2::ZERO || f.cursor.is_some() || f.zoom != 0.0 || !struck.is_empty() || !lifted.is_empty();
        self.active |= used;
        f
    }

    fn release_pointer(&mut self) -> PadFrame {
        let (left, right) = self.pointer.update([false, false], [false, false]);
        self.cursor = None;
        self.active = false;
        PadFrame { left, right, ..Default::default() }
    }

    /// The pad's pointer events for egui (positions in points).
    pub fn egui_events(f: &PadFrame, mouse: Vec2, pixels_per_point: f32, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        let at = |p: Vec2| egui::pos2(p.x / pixels_per_point, p.y / pixels_per_point);
        let pos = f.cursor.unwrap_or(mouse);
        let mut out = Vec::new();
        if let Some(c) = f.cursor {
            out.push(egui::Event::PointerMoved(at(c)));
        }
        for (button, state) in [(egui::PointerButton::Primary, f.left), (egui::PointerButton::Secondary, f.right)] {
            if let Some(pressed) = state {
                out.push(egui::Event::PointerButton { pos: at(pos), button, pressed, modifiers });
            }
        }
        out
    }

    /// The pad's moves on the interface as egui's keys: the d-pad's arrows move the keyboard
    /// focus (Tab the first time, nothing having it: to the first button that takes it), A is
    /// Enter on what has it (nothing having it, A is the pointer's click alone), B and Start
    /// are Escape.
    pub fn nav_events(nav: &[Nav], focused: bool) -> Vec<egui::Event> {
        let mut focused = focused;
        let mut out = Vec::new();
        for &n in nav {
            let key = match n {
                Nav::Up | Nav::Down | Nav::Left | Nav::Right if !focused => {
                    focused = true;
                    egui::Key::Tab
                }
                Nav::Up => egui::Key::ArrowUp,
                Nav::Down => egui::Key::ArrowDown,
                Nav::Left => egui::Key::ArrowLeft,
                Nav::Right => egui::Key::ArrowRight,
                Nav::Activate if focused => egui::Key::Enter,
                Nav::Activate => continue,
                Nav::Back | Nav::Menu => egui::Key::Escape,
            };
            out.extend([true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }));
        }
        out
    }
}

/// The two mouse-button emulations must describe the selected battle scheme. A still
/// operates UI controls; with left-button orders, it also issues contextual orders.
pub fn button_description(button: &str, description: &'static str, order_button: ta_config::OrderButton) -> &'static str {
    if order_button == ta_config::OrderButton::Left {
        match button {
            "A" => n("Select, place, confirm or order (by context)"),
            "B" => n("Cancel / clear the selection; on one of your units, its menu"),
            _ => description,
        }
    } else {
        description
    }
}

/// Buttons as the controls reference shows them.
pub const LAYOUT: &[(&str, &str)] = &[
    (n("Left stick"), n("Scroll the map")),
    (n("Right stick"), n("Move the cursor")),
    ("A", n("Select, place, confirm (hold and move: box select)")),
    ("B", n("Order (move, attack, build...), cancel; on one of your units, its menu")),
    ("X", n("Fight (attack-move)")),
    ("Y", n("Stop")),
    ("LT / RT", n("Zoom out / in")),
    (n("LB (hold)"), n("Shift: queue orders, add to the selection")),
    (n("RB (hold)"), n("Ctrl: with the d-pad, make a control group")),
    (n("D-pad"), n("Control groups 1 to 4")),
    (n("Left stick click"), n("Follow the selected unit")),
    (n("Right stick click"), n("Every unit of the selected types")),
    ("View / Back", n("Next idle constructor")),
    ("Menu / Start", n("Game menu")),
    ("D-pad / A / B", n("On menus and dialogs: move between buttons, choose, go back")),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_ignore_small_deflections_and_point_down_screen() {
        assert_eq!(stick(0.1, 0.1, 0.18), Vec2::ZERO);
        let up = stick(0.0, 1.0, 0.18);
        assert!(up.y < -0.99 && up.x.abs() < 1e-6, "pushed up moves up the screen");
        assert!((stick(0.6, 0.0, 0.18).x - (0.6 - 0.18) / 0.82).abs() < 1e-5);
        assert!(stick(1.0, 1.0, 0.18).length() <= 1.0 + 1e-6);
    }

    #[test]
    fn adjustable_deadzone_handles_zero_extremes_and_invalid_values() {
        assert_eq!(stick(0.0, 0.0, 0.0), Vec2::ZERO);
        assert!(focus_neutral(0.02, 0.02, 0.0), "small drift must not lock out a zero-deadzone controller after Alt+Tab");
        assert!(!focus_neutral(0.2, 0.0, 0.0));
        assert_ne!(stick(0.02, 0.02, 0.0), Vec2::ZERO, "active controls still honour zero deadzone");
        assert_eq!(stick(0.4, 0.0, 0.5), Vec2::ZERO);
        assert!(stick(0.1, 0.0, 0.0).x > 0.09);
        assert_eq!(stick(0.0, 1.0, f32::NAN), Vec2::new(0.0, -1.0));
        assert_eq!(stick(f32::NAN, 0.0, 0.18), Vec2::ZERO);
        assert!(stick(1.0, 1.0, 0.5).length() <= 1.00001);
    }

    #[test]
    fn controller_reference_follows_the_order_scheme() {
        use ta_config::OrderButton::{Left, Right};
        assert_eq!(button_description("B", "Order", Right), "Order");
        assert_eq!(button_description("B", "Order", Left), "Cancel / clear the selection; on one of your units, its menu");
        assert_eq!(button_description("X", "Fight", Left), "Fight");
        assert!(button_description("A", "Select", Left).contains("order"));
    }

    #[test]
    fn lost_gamepad_releases_both_game_and_interface_drags_once() {
        use crate::input::{Input, MouseButtonId};
        let mut pad = Gamepad::new(false);
        let (left, right) = pad.pointer.update([true, true], [true, true]);
        let mut input = Input::default();
        input.apply_pad(&PadFrame { left, right, shift: true, ctrl: true, ..Default::default() }, false);
        assert!(input.button_held(MouseButtonId::Left) && input.button_held(MouseButtonId::Right));
        let released = pad.update(0.016, Vec2::ZERO, Vec2::splat(100.0), 0.18, 1.0, true);
        assert_eq!((released.left, released.right), (Some(false), Some(false)));
        input.apply_pad(&released, true);
        assert!(!input.button_held(MouseButtonId::Left) && !input.button_held(MouseButtonId::Right));
        assert!(input.modifiers.is_empty());
        let events = Gamepad::egui_events(&released, Vec2::ZERO, 1.0, egui::Modifiers::NONE);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event, egui::Event::PointerButton { pressed: false, .. })));
        let idle = pad.update(0.016, Vec2::ZERO, Vec2::splat(100.0), 0.18, 1.0, true);
        assert_eq!((idle.left, idle.right), (None, None), "idle frames must not release the physical mouse repeatedly");
    }

    #[test]
    fn focus_loss_waits_for_release_before_a_fresh_action() {
        let mut gate = FocusGate::default();
        let mut buttons = PointerButtons::default();
        assert!(gate.accepts(true, true));
        assert_eq!(buttons.update([true, false], [true, false]).0, Some(true));
        assert!(!gate.accepts(false, false));
        assert_eq!(buttons.update([false, false], [false, false]).0, Some(false));
        assert!(!gate.accepts(false, true), "background taps must stay suppressed");
        assert!(!gate.accepts(true, false), "held button on return is not a fresh click");
        assert!(!gate.accepts(true, true), "neutral frame drains old events");
        assert!(gate.accepts(true, false), "a new input after release works");
        assert_eq!(buttons.update([true, false], [true, false]).0, Some(true));
    }

    #[test]
    fn changing_pads_releases_buttons_the_new_pad_is_not_holding() {
        let mut buttons = PointerButtons::default();
        assert_eq!(buttons.update([true, false], [true, false]), (Some(true), None));
        assert_eq!(buttons.update([true, false], [false, false]), (None, None));
        assert_eq!(buttons.update([false, true], [false, true]), (Some(false), Some(true)));
        assert_eq!(buttons.update([false, false], [false, false]), (None, Some(false)));
    }

    #[test]
    fn the_pad_works_menus_as_keys_do() {
        use egui::Key;
        let keys = |nav: &[Nav], focused| -> Vec<(Key, bool)> {
            Gamepad::nav_events(nav, focused).into_iter().filter_map(|e| match e { egui::Event::Key { key, pressed, .. } => Some((key, pressed)), _ => None }).collect()
        };
        assert_eq!(nav_of(Button::DPadDown), Some(Nav::Down));
        assert_eq!(nav_of(Button::South), Some(Nav::Activate));
        assert_eq!((nav_of(Button::East), nav_of(Button::Start)), (Some(Nav::Back), Some(Nav::Menu)));
        assert_eq!(nav_of(Button::West), None, "X is the battle's alone");
        // Nothing focused: the first move focuses the first button, the next ones move on.
        assert_eq!(keys(&[Nav::Down, Nav::Down], false), [(Key::Tab, true), (Key::Tab, false), (Key::ArrowDown, true), (Key::ArrowDown, false)]);
        assert_eq!(keys(&[Nav::Left], true), [(Key::ArrowLeft, true), (Key::ArrowLeft, false)]);
        // A works what has the focus, else it is the pointer's click alone.
        assert_eq!(keys(&[Nav::Activate], true), [(Key::Enter, true), (Key::Enter, false)]);
        assert!(keys(&[Nav::Activate], false).is_empty());
        assert_eq!(keys(&[Nav::Back], false), [(Key::Escape, true), (Key::Escape, false)]);
        assert_eq!(keys(&[Nav::Menu], true), [(Key::Escape, true), (Key::Escape, false)]);
    }

    #[test]
    fn quick_taps_between_polls_deliver_a_press_then_a_release() {
        let mut buttons = PointerButtons::default();
        assert_eq!(buttons.update([false, false], [true, true]), (Some(true), Some(true)));
        assert_eq!(buttons.update([false, false], [false, false]), (Some(false), Some(false)));
        assert_eq!(buttons.update([false, false], [false, false]), (None, None));
    }
}
