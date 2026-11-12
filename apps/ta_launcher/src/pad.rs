//! The gamepad in the launcher: a Steam Deck's controls (Steam Input gives them as an Xbox
//! pad) or any pad of that layout. The moves of the game's menus (`ta_ui::pad`): the d-pad and
//! the left stick move the keyboard focus (held, they repeat), A works what has it, B and Start
//! go back; the control reached is scrolled into view. And the launcher's own: the bumpers turn
//! the pages, the right stick moves the pointer (A then clicks there, until the d-pad moves the
//! focus again: A follows the way the pad last moved), the triggers scroll, X
//! opens the chosen card's menu (Shift+F10), Y the page's search (Ctrl+F) and View the list of
//! keys and buttons (F1).
//!
//! A thread reads the pads (gilrs, waiting for them) and wakes the window as one is used: an idle
//! launcher draws nothing until something happens.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use egui::{Event, Id, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, TouchPhase, Vec2};
use gilrs::{Axis, Button, EventType};
use ta_ui::pad::{FocusFollow, Nav, tap};

/// A pad's reading, from the thread that reads them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    Pressed(Button),
    Released(Button),
    /// A stick's axis, -1..1 (up and right positive).
    Axis(Axis, f32),
    /// An analogue trigger, 0..1.
    Trigger(Button, f32),
    /// The pad in use went away, or another took over: everything held is let go.
    Gone,
}

/// A held direction's first repeat, and those after it (seconds).
const REPEAT_DELAY: f64 = 0.42;
const REPEAT_EVERY: f64 = 0.11;
/// How far the left stick goes before it is a direction, and how far back before it is not.
const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;
/// The pointer's speed with the right stick pushed all the way (points a second, on a window 720
/// points high), and the page's with a trigger pressed all the way.
const POINTER_SPEED: f32 = 900.0;
const SCROLL_SPEED: f32 = 1100.0;

/// The d-pad's directions.
const DIRECTIONS: [(Button, Nav); 4] = [(Button::DPadUp, Nav::Up), (Button::DPadDown, Nav::Down), (Button::DPadLeft, Nav::Left), (Button::DPadRight, Nav::Right)];

/// What the pad holds, and what it does each frame ([`State::frame`]).
#[derive(Debug, Default)]
pub struct State {
    /// The d-pad's directions held, in [`DIRECTIONS`]' order.
    dpad: [bool; 4],
    /// The sticks (x right, y up) and the triggers.
    left: Vec2,
    right: Vec2,
    triggers: [f32; 2],
    /// The direction the left stick is pushed, as the d-pad's.
    stick: Option<Nav>,
    /// The direction held, and when it next repeats.
    repeat: Option<(Nav, f64)>,
    /// A was pressed to click where the pointer is: the pointer's button is down.
    clicking: bool,
    /// The pointer as the right stick moved it (points), until the mouse takes it back.
    pointer: Option<Pos2>,
    /// The right stick moved the pointer since the d-pad (or the left stick) last moved the
    /// focus: A clicks where the pointer is, not on the control with the focus.
    pointing: bool,
    follow: FocusFollow,
    last: Option<f64>,
}

/// What the pad did in a frame.
#[derive(Debug, Default)]
pub struct Frame {
    /// For egui's raw input.
    pub events: Vec<Event>,
    /// Pages to turn (the bumpers): back is negative.
    pub pages: i32,
    /// Where the system's pointer goes (points), as the right stick moves it.
    pub cursor: Option<Pos2>,
    /// More frames are wanted: a direction held, a stick or trigger pushed, the focus followed.
    pub busy: bool,
}

/// What the input of a frame already has from the mouse: where it moved last.
pub fn mouse_moved(events: &[Event]) -> Option<Pos2> {
    events.iter().rev().find_map(|e| match e {
        Event::PointerMoved(at) => Some(*at),
        _ => None,
    })
}

/// A stick's push with its dead zone taken out, rescaled to 0..1 (screen axes: y down).
fn dead(stick: Vec2, deadzone: f32) -> Vec2 {
    let dead = if deadzone.is_finite() { deadzone.clamp(0.0, 0.5) } else { 0.18 };
    let v = Vec2::new(stick.x, -stick.y);
    let len = v.length();
    if !len.is_finite() || len <= dead { Vec2::ZERO } else { v / len * ((len - dead) / (1.0 - dead)).min(1.0) }
}

impl State {
    /// A frame at `time` (seconds) with the pad's `readings` since the last: `mouse` is where the
    /// pointer is (points; `moved`, where the mouse itself moved it this frame), `screen` the
    /// window's size (points), `focused` the control with the keyboard's focus (`typing`: a text
    /// field, left with Tab: `ta_ui::pad::nav_events`).
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, readings: &[Reading], time: f64, mouse: Option<Pos2>, moved: Option<Pos2>, screen: Vec2, focused: Option<Id>, typing: bool, deadzone: f32, sensitivity: f32) -> Frame {
        let dt = self.last.map_or(0.0, |last| (time - last).clamp(0.0, 0.1)) as f32;
        self.last = Some(time);
        // The mouse moved the pointer (not to where the stick put it): it is the mouse's again.
        if let (Some(moved), Some(ours)) = (moved, self.pointer)
            && moved.distance(ours) > 2.0
        {
            self.pointer = None;
            self.pointing = false;
        }
        let mut f = Frame::default();
        let mut nav = Vec::new();
        let mut keys = Vec::new();
        let at = self.pointer.or(mouse);
        for &reading in readings {
            match reading {
                Reading::Pressed(button) => {
                    if let Some(i) = DIRECTIONS.iter().position(|&(b, _)| b == button) {
                        self.dpad[i] = true;
                        nav.push(DIRECTIONS[i].1);
                        self.repeat = Some((DIRECTIONS[i].1, time + REPEAT_DELAY));
                        continue;
                    }
                    match button {
                        // A: a click where the right stick put the pointer, when it moved since
                        // the d-pad last moved the focus (or nothing has the focus); else what has
                        // the focus; else the focus goes to the first control.
                        Button::South => match self.pointer.filter(|_| self.pointing || focused.is_none()) {
                            Some(pos) => {
                                self.clicking = true;
                                f.events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                            }
                            None if focused.is_some() => nav.push(Nav::Activate),
                            None => nav.push(Nav::Down),
                        },
                        Button::East => nav.push(Nav::Back),
                        Button::Start => nav.push(Nav::Menu),
                        Button::Select => keys.push((Key::F1, Modifiers::NONE)),
                        Button::West => keys.push((Key::F10, Modifiers::SHIFT)),
                        Button::North => keys.push((Key::F, Modifiers::COMMAND)),
                        Button::LeftTrigger => f.pages -= 1,
                        Button::RightTrigger => f.pages += 1,
                        _ => {}
                    }
                }
                Reading::Released(button) => {
                    if let Some(i) = DIRECTIONS.iter().position(|&(b, _)| b == button) {
                        self.dpad[i] = false;
                    } else if button == Button::South {
                        self.let_go(&mut f, at);
                    }
                }
                Reading::Axis(axis, value) => match axis {
                    Axis::LeftStickX => self.left.x = value,
                    Axis::LeftStickY => self.left.y = value,
                    Axis::RightStickX => self.right.x = value,
                    Axis::RightStickY => self.right.y = value,
                    _ => {}
                },
                Reading::Trigger(Button::LeftTrigger2, value) => self.triggers[0] = value,
                Reading::Trigger(Button::RightTrigger2, value) => self.triggers[1] = value,
                Reading::Trigger(..) => {}
                Reading::Gone => {
                    self.let_go(&mut f, at);
                    (self.dpad, self.left, self.right, self.triggers, self.stick, self.repeat) = Default::default();
                }
            }
        }
        // The left stick as a d-pad: a direction once pushed past STICK_ON along its main axis,
        // until it comes back under STICK_OFF.
        let (x, y) = (self.left.x, self.left.y);
        let reach = x.abs().max(y.abs());
        let main = if x.abs() > y.abs() {
            if x > 0.0 { Nav::Right } else { Nav::Left }
        } else if y > 0.0 {
            Nav::Up
        } else {
            Nav::Down
        };
        let pushed = (reach.is_finite() && reach >= STICK_ON).then_some(main);
        match (self.stick.filter(|_| reach >= STICK_OFF), pushed) {
            (Some(held), Some(pushed)) if held != pushed => self.push_stick(pushed, time, &mut nav),
            (Some(_), _) => {}
            (None, Some(pushed)) => self.push_stick(pushed, time, &mut nav),
            (None, None) => self.stick = None,
        }
        // A direction held repeats.
        if let Some((direction, next)) = self.repeat {
            let held = DIRECTIONS.iter().zip(self.dpad).any(|(&(_, d), held)| held && d == direction) || self.stick == Some(direction);
            if !held {
                self.repeat = None;
            } else if time >= next {
                nav.push(direction);
                self.repeat = Some((direction, time + REPEAT_EVERY));
            }
        }
        // The d-pad moved the focus: A works what has it again.
        if nav.iter().any(|n| n.moves()) {
            self.pointing = false;
        }
        // The right stick moves the pointer: faster the further it is pushed (a curve keeps small
        // moves precise), scaled to the window.
        let aim = dead(self.right, deadzone);
        if aim != Vec2::ZERO {
            self.pointing = true;
            let sensitivity = if sensitivity.is_finite() { sensitivity.clamp(0.25, 3.0) } else { 1.0 };
            let speed = POINTER_SPEED * sensitivity * (screen.y / 720.0).max(0.5);
            let from = at.unwrap_or((screen / 2.0).to_pos2());
            let to = (from + aim.normalized() * aim.length().powf(1.7) * speed * dt).clamp(Pos2::ZERO, (screen - Vec2::splat(1.0)).max(Vec2::ZERO).to_pos2());
            self.pointer = Some(to);
            f.cursor = Some(to);
            f.events.push(Event::PointerMoved(to));
        }
        // The triggers scroll what is under the pointer: the right one down the page.
        let scroll = self.triggers[1] - self.triggers[0];
        if scroll.abs() > 0.05 {
            f.events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, -scroll * SCROLL_SPEED * dt), phase: TouchPhase::Move, modifiers: Modifiers::NONE });
        }
        let (events, following) = self.follow.events(&nav, focused, typing);
        f.events.extend(events);
        for (key, modifiers) in keys {
            f.events.extend(tap(key, modifiers));
        }
        f.busy = following || self.repeat.is_some() || aim != Vec2::ZERO || scroll.abs() > 0.05;
        f
    }

    fn push_stick(&mut self, direction: Nav, time: f64, nav: &mut Vec<Nav>) {
        self.stick = Some(direction);
        nav.push(direction);
        self.repeat = Some((direction, time + REPEAT_DELAY));
    }

    /// A's click, if down, let go.
    fn let_go(&mut self, f: &mut Frame, at: Option<Pos2>) {
        if std::mem::take(&mut self.clicking)
            && let Some(pos) = self.pointer.or(at)
        {
            f.events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        }
    }
}

/// The pads, read on a thread of their own.
pub struct Pad {
    readings: Receiver<Reading>,
    stop: Arc<AtomicBool>,
    /// The launcher takes the pads' input (its window has the focus, no game is running): else
    /// the thread reads them without waking it.
    listening: Arc<AtomicBool>,
    pub state: State,
}

impl Pad {
    /// Starts reading the pads, waking `ctx` as one is used. (Where they cannot be read, it is
    /// logged and nothing comes.)
    pub fn start(ctx: &egui::Context) -> Pad {
        let (sender, readings) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let listening = Arc::new(AtomicBool::new(true));
        let (ctx, stopping, listen) = (ctx.clone(), stop.clone(), listening.clone());
        let spawned = std::thread::Builder::new().name("gamepads".into()).spawn(move || {
            let mut gilrs = match gilrs::Gilrs::new() {
                Ok(gilrs) => gilrs,
                Err(e) => {
                    log::warn!("gamepads unavailable: {e}");
                    return;
                }
            };
            for (_, pad) in gilrs.gamepads() {
                log::info!("gamepad: {}", pad.name());
            }
            // The pad last used is the one read: another's buttons let go of the first's.
            let mut current = None;
            while !stopping.load(Ordering::Relaxed) {
                let Some(event) = gilrs.next_event_blocking(Some(Duration::from_millis(250))) else { continue };
                let ours = current == Some(event.id);
                let mut readings = Vec::new();
                match event.event {
                    EventType::ButtonPressed(button, _) => {
                        if !ours {
                            if current.is_some() {
                                readings.push(Reading::Gone);
                            }
                            current = Some(event.id);
                        }
                        readings.push(Reading::Pressed(button));
                    }
                    EventType::ButtonReleased(button, _) if ours => readings.push(Reading::Released(button)),
                    EventType::ButtonChanged(button @ (Button::LeftTrigger2 | Button::RightTrigger2), value, _) if ours || current.is_none() => {
                        current = Some(event.id);
                        readings.push(Reading::Trigger(button, value));
                    }
                    EventType::AxisChanged(axis, value, _) if ours || current.is_none() => {
                        current = Some(event.id);
                        readings.push(Reading::Axis(axis, value));
                    }
                    EventType::Connected => log::info!("gamepad connected: {}", gilrs.gamepad(event.id).name()),
                    EventType::Disconnected if ours => {
                        current = None;
                        readings.push(Reading::Gone);
                    }
                    _ => {}
                }
                if readings.is_empty() || !listen.load(Ordering::Relaxed) {
                    continue;
                }
                if readings.into_iter().any(|reading| sender.send(reading).is_err()) {
                    break;
                }
                ctx.request_repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("gamepads unavailable: {e}");
        }
        Pad { readings, stop, listening, state: State::default() }
    }

    /// The readings since the last frame.
    pub fn take(&self) -> Vec<Reading> {
        self.readings.try_iter().collect()
    }

    /// Whether the launcher takes the pads' input now. Not taking it, what is held is let go and
    /// what comes is dropped (nothing wakes the window for it).
    pub fn listen(&mut self, on: bool) {
        if !on && self.listening.load(Ordering::Relaxed) {
            let _ = self.take();
            self.state = State::default();
        }
        self.listening.store(on, Ordering::Relaxed);
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // (The thread sees it within a quarter of a second, and ends.)
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Vec2 = Vec2::new(1120.0, 720.0);

    fn keys(events: &[Event]) -> Vec<(Key, bool, Modifiers)> {
        events.iter().filter_map(|e| match e { Event::Key { key, pressed, modifiers, .. } => Some((*key, *pressed, *modifiers)), _ => None }).collect()
    }

    /// A frame with nothing focused at `time`.
    fn frame(state: &mut State, readings: &[Reading], time: f64, focused: Option<Id>) -> Frame {
        state.frame(readings, time, Some(Pos2::new(500.0, 300.0)), None, SCREEN, focused, false, 0.18, 1.0)
    }

    #[test]
    fn the_buttons_work_the_launcher_as_its_keys_do() {
        let mut state = State::default();
        let focused = Some(Id::new("card"));
        let pressed = |state: &mut State, button| keys(&frame(state, &[Reading::Pressed(button), Reading::Released(button)], 0.0, focused).events);
        let none = Modifiers::NONE;
        assert_eq!(pressed(&mut state, Button::DPadRight), [(Key::ArrowRight, true, none), (Key::ArrowRight, false, none)]);
        assert_eq!(pressed(&mut state, Button::South), [(Key::Enter, true, none), (Key::Enter, false, none)]);
        assert_eq!(pressed(&mut state, Button::East), [(Key::Escape, true, none), (Key::Escape, false, none)]);
        assert_eq!(pressed(&mut state, Button::Start), [(Key::Escape, true, none), (Key::Escape, false, none)]);
        assert_eq!(pressed(&mut state, Button::Select), [(Key::F1, true, none), (Key::F1, false, none)]);
        assert_eq!(pressed(&mut state, Button::West), [(Key::F10, true, Modifiers::SHIFT), (Key::F10, false, Modifiers::SHIFT)]);
        assert_eq!(pressed(&mut state, Button::North), [(Key::F, true, Modifiers::COMMAND), (Key::F, false, Modifiers::COMMAND)]);
        // The bumpers turn the pages.
        assert_eq!(frame(&mut state, &[Reading::Pressed(Button::RightTrigger)], 0.0, focused).pages, 1);
        assert_eq!(frame(&mut state, &[Reading::Pressed(Button::LeftTrigger), Reading::Pressed(Button::LeftTrigger)], 0.0, focused).pages, -2);
        // Nothing focused: A takes the focus to the first control.
        assert_eq!(keys(&frame(&mut state, &[Reading::Pressed(Button::South)], 0.0, None).events)[0].0, Key::Tab);
    }

    /// A direction held repeats, after a pause; let go, it stops.
    #[test]
    fn a_held_direction_repeats() {
        let mut state = State::default();
        let focused = Some(Id::new("row"));
        let downs = |f: &Frame| keys(&f.events).iter().filter(|(k, pressed, _)| *k == Key::ArrowDown && *pressed).count();
        assert_eq!(downs(&frame(&mut state, &[Reading::Pressed(Button::DPadDown)], 0.0, focused)), 1);
        assert_eq!(downs(&frame(&mut state, &[], 0.3, focused)), 0, "a pause first");
        let f = frame(&mut state, &[], 0.45, focused);
        assert!(downs(&f) == 1 && f.busy);
        assert_eq!(downs(&frame(&mut state, &[], 0.5, focused)), 0);
        assert_eq!(downs(&frame(&mut state, &[], 0.57, focused)), 1);
        assert_eq!(downs(&frame(&mut state, &[Reading::Released(Button::DPadDown)], 0.7, focused)), 0);
        assert_eq!(downs(&frame(&mut state, &[], 2.0, focused)), 0, "let go, it stops");
    }

    /// The left stick is a d-pad: a direction once pushed well over, held until it comes back.
    #[test]
    fn the_left_stick_moves_the_focus_as_the_d_pad_does() {
        let mut state = State::default();
        let focused = Some(Id::new("row"));
        let moves = |f: &Frame| keys(&f.events).into_iter().filter(|(_, pressed, _)| *pressed).map(|(k, ..)| k).collect::<Vec<_>>();
        assert!(moves(&frame(&mut state, &[Reading::Axis(Axis::LeftStickX, 0.4)], 0.0, focused)).is_empty(), "not far enough");
        assert_eq!(moves(&frame(&mut state, &[Reading::Axis(Axis::LeftStickX, 0.9)], 0.1, focused)), [Key::ArrowRight]);
        assert!(moves(&frame(&mut state, &[Reading::Axis(Axis::LeftStickX, 0.45)], 0.2, focused)).is_empty(), "still held");
        assert_eq!(moves(&frame(&mut state, &[Reading::Axis(Axis::LeftStickX, 0.0), Reading::Axis(Axis::LeftStickY, -0.8)], 0.3, focused)), [Key::ArrowDown]);
        assert!(moves(&frame(&mut state, &[Reading::Axis(Axis::LeftStickY, 0.1)], 0.4, focused)).is_empty());
        assert!(state.repeat.is_none(), "back in the middle: no repeat");
        assert_eq!(moves(&frame(&mut state, &[Reading::Axis(Axis::LeftStickY, 0.9)], 0.5, focused)), [Key::ArrowUp]);
    }

    /// The right stick moves the pointer, and then A clicks where it is, pressed and let go; the
    /// mouse moving takes the pointer back.
    #[test]
    fn the_right_stick_moves_the_pointer_and_a_clicks_there() {
        let mut state = State::default();
        frame(&mut state, &[Reading::Axis(Axis::RightStickX, 1.0)], 0.0, None);
        let f = frame(&mut state, &[], 0.05, None);
        let to = f.cursor.expect("the pointer moved");
        assert!(to.x > 530.0 && (to.y - 300.0).abs() < 0.01, "right, from the mouse's place: {to:?}");
        assert!(f.busy);
        let f = frame(&mut state, &[Reading::Axis(Axis::RightStickX, 0.0), Reading::Pressed(Button::South)], 0.1, None);
        assert!(f.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, pos, .. } if *pos == to)), "{:?}", f.events);
        let f = frame(&mut state, &[Reading::Released(Button::South)], 0.15, None);
        assert!(f.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: false, .. })));
        // The mouse moves: A no longer clicks where the stick left the pointer.
        state.frame(&[], 0.2, Some(Pos2::new(10.0, 10.0)), Some(Pos2::new(10.0, 10.0)), SCREEN, None, false, 0.18, 1.0);
        let f = frame(&mut state, &[Reading::Pressed(Button::South)], 0.25, None);
        assert!(!f.events.iter().any(|e| matches!(e, Event::PointerButton { .. })));
        // A pad gone lets go of a click held.
        frame(&mut state, &[Reading::Axis(Axis::RightStickY, 1.0)], 0.3, None);
        frame(&mut state, &[Reading::Axis(Axis::RightStickY, 0.0), Reading::Pressed(Button::South)], 0.35, None);
        let f = frame(&mut state, &[Reading::Gone], 0.4, None);
        assert!(f.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: false, .. })));
    }

    /// A works what the pad last moved to: the control the d-pad focused; the right stick having
    /// moved the pointer since, what is under the pointer (the focus still on the control); the
    /// d-pad again, the control.
    #[test]
    fn a_follows_the_way_the_pad_last_moved() {
        let mut state = State::default();
        let focused = Some(Id::new("button"));
        let clicks = |f: &Frame| f.events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. }));
        let enters = |f: &Frame| keys(&f.events).contains(&(Key::Enter, true, Modifiers::NONE));
        let tap = |button| [Reading::Pressed(button), Reading::Released(button)];
        frame(&mut state, &tap(Button::DPadDown), 0.0, focused);
        let f = frame(&mut state, &tap(Button::South), 0.05, focused);
        assert!(enters(&f) && !clicks(&f), "the focused control: {:?}", f.events);
        frame(&mut state, &[Reading::Axis(Axis::RightStickX, 1.0)], 0.1, focused);
        frame(&mut state, &[Reading::Axis(Axis::RightStickX, 0.0)], 0.15, focused);
        let f = frame(&mut state, &tap(Button::South), 0.2, focused);
        assert!(clicks(&f) && !enters(&f), "under the pointer: {:?}", f.events);
        frame(&mut state, &tap(Button::DPadUp), 0.25, focused);
        let f = frame(&mut state, &tap(Button::South), 0.3, focused);
        assert!(enters(&f) && !clicks(&f), "the focused control again: {:?}", f.events);
    }

    /// The triggers scroll: the right one down the page, the left one up.
    #[test]
    fn the_triggers_scroll() {
        let mut state = State::default();
        frame(&mut state, &[Reading::Trigger(Button::RightTrigger2, 1.0)], 0.0, None);
        let wheel = |f: &Frame| f.events.iter().find_map(|e| match e { Event::MouseWheel { delta, .. } => Some(delta.y), _ => None });
        let f = frame(&mut state, &[], 0.05, None);
        assert!(wheel(&f).is_some_and(|y| y < -50.0) && f.busy, "{:?}", wheel(&f));
        let f = frame(&mut state, &[Reading::Trigger(Button::RightTrigger2, 0.0), Reading::Trigger(Button::LeftTrigger2, 0.5)], 0.1, None);
        assert!(wheel(&f).is_some_and(|y| y > 0.0));
        assert!(wheel(&frame(&mut state, &[Reading::Trigger(Button::LeftTrigger2, 0.0)], 0.15, None)).is_none());
    }
}
