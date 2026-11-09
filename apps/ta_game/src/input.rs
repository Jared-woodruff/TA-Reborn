//! Keyboard and mouse state accumulated from window events.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

#[derive(Default)]
pub struct Input {
    held: HashSet<KeyCode>,
    /// Keys reserved by the opt-in keyboard pointer for this frame.
    claimed: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    /// Chords use the modifiers at key-down, even if they change before redraw.
    key_modifiers: HashMap<KeyCode, ModifiersState>,
    pub mouse: Vec2,
    pub mouse_delta: Vec2,
    /// The mouse's own motion (device counts), also while the cursor is held in the window
    /// (Perspective looks round with it), and whether any has come (where it never does, a
    /// remote desktop, the cursor's motion stands in).
    pub raw_delta: Vec2,
    pub raw_seen: bool,
    pub buttons_held: HashSet<MouseButtonId>,
    pub buttons_pressed: HashSet<MouseButtonId>,
    pub buttons_released: HashSet<MouseButtonId>,
    /// Modifiers belong to each click edge, not the eventual redraw. A quick Shift+click
    /// must still queue when Shift is released before the frame processes that click.
    pressed_modifiers: HashMap<MouseButtonId, ModifiersState>,
    released_modifiers: HashMap<MouseButtonId, ModifiersState>,
    /// Accumulated wheel movement in "lines" (positive = away from the user).
    pub wheel: f32,
    /// The keyboard's modifiers and a gamepad's bumpers together.
    pub modifiers: winit::keyboard::ModifiersState,
    kb_modifiers: winit::keyboard::ModifiersState,
    pad_modifiers: winit::keyboard::ModifiersState,
    /// Gamepad: camera pan (-1..1 each way on screen), and the actions it struck this frame.
    pub pad_pan: Vec2,
    /// The cursor is over the window (edge scrolling stops when it leaves).
    pub cursor_inside: bool,
    pad_actions: HashSet<&'static str>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum MouseButtonId {
    Left,
    Right,
    Middle,
    Other(u16),
}

impl From<MouseButton> for MouseButtonId {
    fn from(b: MouseButton) -> Self {
        match b {
            MouseButton::Left => MouseButtonId::Left,
            MouseButton::Right => MouseButtonId::Right,
            MouseButton::Middle => MouseButtonId::Middle,
            MouseButton::Back => MouseButtonId::Other(3),
            MouseButton::Forward => MouseButtonId::Other(4),
            MouseButton::Other(n) => MouseButtonId::Other(n),
        }
    }
}

impl Input {
    /// A benchmark accepts keyboard input only for its cancellation keys. Consume pointer
    /// and text events before egui sees them too: an idle pointer must not hover the scene,
    /// click an overlay, or ask egui to restore the system cursor. Window lifecycle events
    /// still reach the normal handler, so resizing, focus changes and closing keep working.
    pub fn benchmark_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput { .. } | WindowEvent::ModifiersChanged(_) => {
                self.on_event(event, false, false);
                true
            }
            WindowEvent::Focused(false) => {
                self.on_event(event, false, false);
                false
            }
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::Ime(_)
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::RotationGesture { .. } => true,
            _ => false,
        }
    }

    /// Feeds a window event. `ui_wants_pointer`/`ui_wants_keyboard` suppress game input
    /// while the UI is being interacted with.
    pub fn on_event(&mut self, event: &WindowEvent, ui_wants_pointer: bool, ui_wants_keyboard: bool) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed if !ui_wants_keyboard => {
                            if self.held.insert(code) {
                                self.pressed.insert(code);
                                self.key_modifiers.insert(code, self.modifiers);
                            }
                        }
                        ElementState::Released => {
                            self.held.remove(&code);
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.kb_modifiers = m.state();
                self.modifiers = self.kb_modifiers | self.pad_modifiers;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = Vec2::new(position.x as f32, position.y as f32);
                self.mouse_delta += p - self.mouse;
                self.mouse = p;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = MouseButtonId::from(*button);
                match state {
                    ElementState::Pressed if !ui_wants_pointer => {
                        self.buttons_held.insert(b);
                        self.buttons_pressed.insert(b);
                        self.pressed_modifiers.insert(b, self.modifiers);
                    }
                    ElementState::Released => {
                        // Preserve UI-owned release metadata too, without exposing a game
                        // action. The HUD may finish a Shift+click after game input runs.
                        self.released_modifiers.insert(b, self.modifiers);
                        if self.buttons_held.remove(&b) {
                            self.buttons_released.insert(b);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !ui_wants_pointer => {
                self.wheel += match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
            }
            WindowEvent::Focused(false) => {
                // Releases can happen in another app. Retire both held state and pending
                // edges so returning cannot issue an old order or retain Ctrl/Shift.
                self.held.clear();
                self.buttons_held.clear();
                self.end_frame();
                self.modifiers = winit::keyboard::ModifiersState::empty();
                self.kb_modifiers = winit::keyboard::ModifiersState::empty();
                self.pad_modifiers = winit::keyboard::ModifiersState::empty();
                self.pad_pan = Vec2::ZERO;
            }
            _ => {}
        }
    }

    pub fn held(&self, key: KeyCode) -> bool {
        self.held.contains(&key) && !self.claimed.contains(&key)
    }

    pub fn pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key) && !self.claimed.contains(&key)
    }

    pub fn key_pressed_modifiers(&self, key: KeyCode) -> ModifiersState {
        self.key_modifiers.get(&key).copied().unwrap_or(self.modifiers)
    }

    /// Reserve keys without losing physical holds: movement continues next frame and
    /// normal battle hotkeys cannot run alongside the keyboard pointer's actions.
    pub fn claim_keys(&mut self, keys: &[KeyCode]) {
        self.claimed.extend(keys.iter().copied());
    }

    /// Presses a key for this frame (scripted checks).
    pub fn press(&mut self, key: KeyCode) {
        self.pressed.insert(key);
        self.held.insert(key);
        self.key_modifiers.insert(key, self.modifiers);
    }

    /// The keys pressed this frame.
    pub fn pressed_keys(&self) -> Vec<KeyCode> {
        self.pressed.iter().filter(|key| !self.claimed.contains(key)).copied().collect()
    }

    pub fn button_held(&self, b: MouseButtonId) -> bool {
        self.buttons_held.contains(&b)
    }

    pub fn button_pressed(&self, b: MouseButtonId) -> bool {
        self.buttons_pressed.contains(&b)
    }

    pub fn button_released(&self, b: MouseButtonId) -> bool {
        self.buttons_released.contains(&b)
    }

    /// Modifiers when this button was pressed/released, falling back to the current
    /// state for held gestures and scripted input that directly supplies button edges.
    pub fn button_pressed_modifiers(&self, b: MouseButtonId) -> ModifiersState {
        self.pressed_modifiers.get(&b).copied().unwrap_or(self.modifiers)
    }

    pub fn button_released_modifiers(&self, b: MouseButtonId) -> ModifiersState {
        self.released_modifiers.get(&b).copied().unwrap_or(self.modifiers)
    }

    /// Includes UI-owned releases for deciding when to retire a repeated tool. Unlike
    /// `button_released`, this must never be used to dispatch a battlefield command.
    pub fn pointer_released(&self, b: MouseButtonId) -> bool {
        self.buttons_released.contains(&b) || self.released_modifiers.contains_key(&b)
    }

    /// A gamepad's frame: its cursor, clicks (not over the interface), keys, actions and
    /// bumpers become this input's.
    pub fn apply_pad(&mut self, f: &crate::gamepad::PadFrame, ui_wants_pointer: bool) {
        let mut pad = ModifiersState::empty();
        if f.shift {
            pad |= ModifiersState::SHIFT;
        }
        if f.ctrl {
            pad |= ModifiersState::CONTROL;
        }
        self.pad_modifiers = pad;
        self.modifiers = self.kb_modifiers | pad;
        self.pad_pan = f.pan;
        if !ui_wants_pointer { self.wheel += f.zoom; }
        if let Some(p) = f.cursor {
            self.mouse_delta += p - self.mouse;
            self.mouse = p;
        }
        for (b, state) in [(MouseButtonId::Left, f.left), (MouseButtonId::Right, f.right)] {
            match state {
                Some(true) if !ui_wants_pointer => {
                    if self.buttons_held.insert(b) {
                        self.buttons_pressed.insert(b);
                        self.pressed_modifiers.insert(b, self.modifiers);
                    }
                }
                Some(false) => {
                    self.released_modifiers.insert(b, self.modifiers);
                    if self.buttons_held.remove(&b) {
                        self.buttons_released.insert(b);
                    }
                }
                _ => {}
            }
        }
        for &key in &f.keys {
            self.pressed.insert(key);
            self.key_modifiers.entry(key).or_insert(self.modifiers);
        }
        self.pad_actions.extend(f.actions.iter().copied());
    }

    /// A gamepad struck this action this frame.
    pub fn pad_action(&self, action: &str) -> bool {
        self.pad_actions.contains(action)
    }

    /// Nothing pressed or held this frame, and the pointer away (the benchmark's battles take no
    /// input). Keep the keyboard modifiers: cancellation can be a chord whose modifier was
    /// pressed in an earlier frame.
    pub fn ignore(&mut self) {
        self.end_frame();
        self.held.clear();
        self.buttons_held.clear();
        self.pad_pan = Vec2::ZERO;
        self.cursor_inside = false;
    }

    /// Clears per-frame edge state; call at the end of each frame.
    pub fn end_frame(&mut self) {
        self.claimed.clear();
        self.pad_actions.clear();
        self.pressed.clear();
        self.key_modifiers.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.pressed_modifiers.clear();
        self.released_modifiers.clear();
        self.mouse_delta = Vec2::ZERO;
        self.raw_delta = Vec2::ZERO;
        self.wheel = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{DeviceId, Ime, Touch, TouchPhase};
    use winit::keyboard::ModifiersState;

    #[test]
    fn ui_pointer_capture_blocks_controller_zoom_as_it_blocks_the_mouse_wheel() {
        let mut input = Input::default();
        let wheel = WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(), delta: MouseScrollDelta::LineDelta(0.0, 2.0), phase: TouchPhase::Moved,
        };
        let pad = crate::gamepad::PadFrame { zoom: 2.0, left: Some(true), ..Default::default() };
        input.on_event(&wheel, true, false);
        input.apply_pad(&pad, true);
        assert_eq!(input.wheel, 0.0, "scrolling over a HUD control must not zoom the battlefield");
        assert!(!input.button_pressed(MouseButtonId::Left));
        input.on_event(&wheel, false, false);
        input.apply_pad(&pad, false);
        assert_eq!(input.wheel, 4.0, "both input devices still zoom over the battlefield");
        assert!(input.button_pressed(MouseButtonId::Left));
    }

    #[test]
    fn modal_input_discard_clears_newly_polled_controller_actions() {
        let mut input = Input::default();
        let pad = crate::gamepad::PadFrame {
            keys: vec![KeyCode::Escape, KeyCode::Digit1], actions: vec!["stop", "fight"],
            pan: Vec2::ONE, zoom: 1.0, left: Some(true), ..Default::default()
        };
        input.apply_pad(&pad, false);
        input.ignore();
        assert!(!input.pressed(KeyCode::Escape) && !input.pressed(KeyCode::Digit1));
        assert!(!input.pad_action("stop") && !input.pad_action("fight"));
        assert_eq!(input.pad_pan, Vec2::ZERO);
        assert_eq!(input.wheel, 0.0);
        assert!(!input.button_held(MouseButtonId::Left) && !input.button_pressed(MouseButtonId::Left));
        // The frame remains intact for the separate egui input consumer.
        assert!(!crate::gamepad::Gamepad::egui_events(&pad, Vec2::ZERO, 1.0, egui::Modifiers::NONE).is_empty());
    }

    fn mouse_event(input: &mut Input, button: MouseButton, state: ElementState, over_ui: bool) {
        input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state, button }, over_ui, false);
    }

    fn modifiers(input: &mut Input, modifiers: ModifiersState) {
        input.on_event(&WindowEvent::ModifiersChanged(modifiers.into()), false, false);
    }

    #[test]
    fn quick_keyboard_chords_keep_the_modifiers_of_the_press() {
        let plain = crate::keys::Binding::parse("A").unwrap();
        let chord = crate::keys::Binding::parse("Ctrl+A").unwrap();
        let mut input = Input::default();
        modifiers(&mut input, ModifiersState::CONTROL);
        input.press(KeyCode::KeyA);
        modifiers(&mut input, ModifiersState::empty());
        input.apply_pad(&crate::gamepad::PadFrame::default(), false);
        assert!(chord.pressed(&input), "releasing Ctrl before redraw must not lose Ctrl+A");
        assert!(!plain.pressed(&input), "Ctrl+A must not become a plain A command");
        assert!(plain.held(&input) && !chord.held(&input), "holds follow the current modifiers");
        input.end_frame();
        input.press(KeyCode::KeyA);
        modifiers(&mut input, ModifiersState::CONTROL);
        assert!(plain.pressed(&input), "a later Ctrl press must not change an earlier A");
        assert!(!chord.pressed(&input));
    }

    #[test]
    fn completed_shift_click_keeps_queue_modifier_until_the_frame_handles_it() {
        for button in [MouseButton::Left, MouseButton::Right] {
            let mut input = Input::default();
            modifiers(&mut input, ModifiersState::SHIFT);
            mouse_event(&mut input, button, ElementState::Pressed, false);
            input.end_frame(); // mouse-down and mouse-up can fall in different frames
            mouse_event(&mut input, button, ElementState::Released, false);
            modifiers(&mut input, ModifiersState::empty());
            input.apply_pad(&crate::gamepad::PadFrame::default(), false);
            let button = MouseButtonId::from(button);
            assert!(!input.modifiers.shift_key(), "the live keyboard state has moved on");
            assert!(input.button_released(button));
            assert!(input.button_released_modifiers(button).shift_key(), "the completed Shift+click still queues its order");
            input.end_frame();
            assert!(!input.button_released_modifiers(button).shift_key(), "the following plain click does not inherit Shift");
        }
    }

    #[test]
    fn mouse_modifier_edges_are_independent_and_ui_clicks_do_not_leak() {
        let mut input = Input::default();
        modifiers(&mut input, ModifiersState::CONTROL | ModifiersState::SHIFT);
        mouse_event(&mut input, MouseButton::Left, ElementState::Pressed, false);
        mouse_event(&mut input, MouseButton::Left, ElementState::Released, false);
        modifiers(&mut input, ModifiersState::empty());
        mouse_event(&mut input, MouseButton::Right, ElementState::Pressed, false);
        mouse_event(&mut input, MouseButton::Right, ElementState::Released, false);
        modifiers(&mut input, ModifiersState::SHIFT);
        assert_eq!(input.button_pressed_modifiers(MouseButtonId::Left), ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(input.button_released_modifiers(MouseButtonId::Left), ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert!(input.button_released_modifiers(MouseButtonId::Right).is_empty(), "a later Shift press must not queue an earlier plain click");
        input.on_event(&WindowEvent::Focused(false), false, false);
        assert!(input.pressed_modifiers.is_empty() && input.released_modifiers.is_empty());
        modifiers(&mut input, ModifiersState::SHIFT);
        mouse_event(&mut input, MouseButton::Left, ElementState::Pressed, true);
        mouse_event(&mut input, MouseButton::Left, ElementState::Released, false);
        assert!(!input.button_released(MouseButtonId::Left));
        assert!(input.pointer_released(MouseButtonId::Left), "a HUD release delays repeat-mode cleanup until the HUD handles its own click");
        assert!(input.button_released_modifiers(MouseButtonId::Left).shift_key());
    }

    #[test]
    fn releasing_keyboard_shift_preserves_a_held_gamepad_queue_bumper() {
        let mut input = Input::default();
        modifiers(&mut input, ModifiersState::SHIFT);
        input.apply_pad(&crate::gamepad::PadFrame { shift: true, ..Default::default() }, false);
        mouse_event(&mut input, MouseButton::Right, ElementState::Pressed, false);
        modifiers(&mut input, ModifiersState::empty());
        mouse_event(&mut input, MouseButton::Right, ElementState::Released, false);
        assert!(input.button_released_modifiers(MouseButtonId::Right).shift_key(), "keyboard and gamepad holds are independent even between pad polls");
        input.apply_pad(&crate::gamepad::PadFrame::default(), false);
        assert!(!input.modifiers.shift_key());
    }

    #[test]
    fn gamepad_click_edges_use_the_same_frames_bumpers() {
        let mut input = Input::default();
        input.apply_pad(&crate::gamepad::PadFrame { left: Some(true), shift: true, ctrl: true, ..Default::default() }, false);
        assert_eq!(input.button_pressed_modifiers(MouseButtonId::Left), ModifiersState::SHIFT | ModifiersState::CONTROL);
        input.end_frame();
        input.apply_pad(&crate::gamepad::PadFrame { left: Some(false), shift: true, ..Default::default() }, false);
        input.apply_pad(&crate::gamepad::PadFrame::default(), false);
        assert_eq!(input.button_released_modifiers(MouseButtonId::Left), ModifiersState::SHIFT);
        assert!(input.modifiers.is_empty());
    }

    #[test]
    fn gameplay_focus_loss_releases_modifiers_and_pending_actions() {
        let mut input = Input::default();
        input.on_event(&WindowEvent::ModifiersChanged((ModifiersState::CONTROL | ModifiersState::SHIFT).into()), false, false);
        input.press(KeyCode::KeyD);
        input.buttons_held.insert(MouseButtonId::Right);
        input.buttons_pressed.insert(MouseButtonId::Right);
        input.wheel = 2.0;
        input.on_event(&WindowEvent::Focused(false), false, false);
        input.on_event(&WindowEvent::Focused(true), false, false);
        assert!(!input.held(KeyCode::KeyD) && !input.pressed(KeyCode::KeyD), "unhandled commands must not survive leaving the window");
        assert!(input.modifiers.is_empty(), "Ctrl/Shift released in another app must not force fire or queue orders on return");
        assert!(!input.button_held(MouseButtonId::Right) && !input.button_pressed(MouseButtonId::Right));
        assert_eq!(input.wheel, 0.0);
        input.apply_pad(&crate::gamepad::PadFrame::default(), false);
        assert!(input.modifiers.is_empty(), "gamepad input must not restore stale keyboard modifiers");
        input.on_event(&WindowEvent::ModifiersChanged(ModifiersState::SHIFT.into()), false, false);
        assert!(input.modifiers.shift_key(), "fresh modifiers still work");
    }

    #[test]
    fn benchmark_pointer_events_never_reach_game_or_ui() {
        let device_id = DeviceId::dummy();
        let mut input = Input { mouse: Vec2::new(40.0, 80.0), ..Default::default() };
        let events = [
            WindowEvent::CursorEntered { device_id },
            WindowEvent::CursorMoved { device_id, position: PhysicalPosition::new(400.0, 600.0) },
            WindowEvent::MouseInput { device_id, state: ElementState::Pressed, button: MouseButton::Left },
            WindowEvent::MouseWheel { device_id, delta: MouseScrollDelta::LineDelta(0.0, 3.0), phase: TouchPhase::Moved },
            WindowEvent::Touch(Touch { device_id, phase: TouchPhase::Started, location: PhysicalPosition::new(400.0, 600.0), force: None, id: 0 }),
            WindowEvent::Ime(Ime::Commit("q".into())),
            WindowEvent::CursorLeft { device_id },
            WindowEvent::CursorEntered { device_id },
        ];
        for event in events {
            assert!(input.benchmark_event(&event), "must stop before egui: {event:?}");
            assert_eq!(input.mouse, Vec2::new(40.0, 80.0));
            assert_eq!(input.mouse_delta, Vec2::ZERO);
            assert_eq!(input.wheel, 0.0);
            assert!(input.buttons_held.is_empty());
            assert!(input.buttons_pressed.is_empty());
            assert!(!input.cursor_inside);
        }
        for event in [WindowEvent::Resized(PhysicalSize::new(1920, 1080)), WindowEvent::Focused(false), WindowEvent::RedrawRequested, WindowEvent::CloseRequested] {
            assert!(!input.benchmark_event(&event), "lifecycle events still work: {event:?}");
        }
    }

    #[test]
    fn benchmark_ignores_actions_but_keeps_cancellation_chords() {
        let mut input = Input::default();
        assert!(input.benchmark_event(&WindowEvent::ModifiersChanged(ModifiersState::CONTROL.into())));
        input.press(KeyCode::KeyQ);
        input.buttons_held.insert(MouseButtonId::Left);
        input.buttons_pressed.insert(MouseButtonId::Left);
        input.mouse_delta = Vec2::ONE;
        input.raw_delta = Vec2::ONE;
        input.pad_pan = Vec2::ONE;
        input.pad_actions.insert("attack");
        input.wheel = 1.0;
        input.cursor_inside = true;
        assert!(input.modifiers.control_key() && input.pressed(KeyCode::KeyQ), "quit is checked before the frame's input is discarded");
        input.ignore();
        assert!(!input.held(KeyCode::KeyQ) && !input.pressed(KeyCode::KeyQ));
        assert!(input.buttons_held.is_empty() && input.buttons_pressed.is_empty());
        assert_eq!(input.mouse_delta, Vec2::ZERO);
        assert_eq!(input.raw_delta, Vec2::ZERO);
        assert_eq!(input.pad_pan, Vec2::ZERO);
        assert_eq!(input.wheel, 0.0);
        assert!(!input.cursor_inside && !input.pad_action("attack"));
        input.press(KeyCode::KeyQ);
        assert!(input.modifiers.control_key() && input.pressed(KeyCode::KeyQ), "a held modifier still works in a later frame");
        assert!(input.benchmark_event(&WindowEvent::ModifiersChanged(ModifiersState::empty().into())));
        assert!(!input.modifiers.control_key());
    }

    #[test]
    fn loading_cancellation_uses_fresh_edges_and_drops_unfocused_keys() {
        let quit = crate::keys::Binding::parse("Ctrl+Q").unwrap();
        let mut input = Input::default();
        input.press(KeyCode::KeyQ);
        assert!(!quit.pressed(&input));
        input.end_frame(); // loading checks and retires edges after every window event
        input.held.remove(&KeyCode::KeyQ); // Q released before Ctrl
        input.benchmark_event(&WindowEvent::ModifiersChanged(ModifiersState::CONTROL.into()));
        assert!(!quit.pressed(&input), "a later modifier must not revive an old key press");
        input.end_frame();
        input.press(KeyCode::KeyQ);
        assert!(quit.pressed(&input), "a fresh quit key still cancels with Ctrl held");

        assert!(!input.benchmark_event(&WindowEvent::Focused(false)), "focus still reaches the window handler");
        assert!(!input.held(KeyCode::KeyQ) && !input.pressed(KeyCode::KeyQ));
        assert!(input.modifiers.is_empty());
        assert!(!input.benchmark_event(&WindowEvent::Focused(true)));
        input.press(KeyCode::KeyQ);
        assert!(!quit.pressed(&input), "Ctrl released outside the window must not stick");
        input.end_frame();
        input.apply_pad(&crate::gamepad::PadFrame::default(), false);
        assert!(input.modifiers.is_empty(), "the saved keyboard modifiers were cleared too");
    }
}
