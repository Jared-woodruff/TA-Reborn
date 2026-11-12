//! A gamepad on the interface, the game's menus and the launcher's pages alike: the d-pad moves
//! the keyboard focus, A works what has it, B (and Start) go back, as their keys do
//! ([`nav_events`]); and the control the pad moved to is scrolled into view
//! ([`FocusFollow`]), which egui leaves to whoever moved the focus.

use egui::{Event, Id, Key, Modifiers};

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

impl Nav {
    /// Whether it moves the focus.
    pub fn moves(self) -> bool {
        matches!(self, Nav::Up | Nav::Down | Nav::Left | Nav::Right)
    }
}

/// The pad's moves on the interface as egui's keys: the d-pad's arrows move the keyboard focus
/// (Tab the first time, nothing having it: to the first control that takes it), A is Enter on
/// what has it (nothing having it, A is the pointer's click alone), B and Start are Escape.
/// `typing`: a text field has the focus, which keeps the arrows for its cursor: the d-pad leaves
/// it with Tab (down and right) and Shift+Tab (up and left), the next control and the one before.
pub fn nav_events(nav: &[Nav], focused: bool, typing: bool) -> Vec<Event> {
    let mut focused = focused;
    let mut typing = typing && focused;
    let mut out = Vec::new();
    for &n in nav {
        let (key, modifiers) = match n {
            Nav::Up | Nav::Down | Nav::Left | Nav::Right if !focused => {
                focused = true;
                (Key::Tab, Modifiers::NONE)
            }
            Nav::Down | Nav::Right if typing => (Key::Tab, Modifiers::NONE),
            Nav::Up | Nav::Left if typing => (Key::Tab, Modifiers::SHIFT),
            Nav::Up => (Key::ArrowUp, Modifiers::NONE),
            Nav::Down => (Key::ArrowDown, Modifiers::NONE),
            Nav::Left => (Key::ArrowLeft, Modifiers::NONE),
            Nav::Right => (Key::ArrowRight, Modifiers::NONE),
            Nav::Activate if focused => (Key::Enter, Modifiers::NONE),
            Nav::Activate => continue,
            Nav::Back | Nav::Menu => (Key::Escape, Modifiers::NONE),
        };
        // (Off the text field, the next moves are arrows again.)
        typing &= !n.moves();
        out.extend(tap(key, modifiers));
    }
    out
}

/// A key struck and let go.
pub fn tap(key: Key, modifiers: Modifiers) -> [Event; 2] {
    [true, false].map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers })
}

/// The request that scrolls every scroll area round the control `id` until it is in view, as a
/// screen reader's does (egui takes it as the control is laid out).
pub fn scroll_into_view(id: Id) -> Event {
    Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::ScrollIntoView,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: id.accesskit_id(),
        data: None,
    })
}

/// Follows the keyboard focus the pad moves: once it has gone to another control, that control
/// is scrolled into view. egui moves the focus as the frame with the keys is laid out and hands
/// it over as the next one begins, so the new control is known a frame or two later.
#[derive(Debug, Default)]
pub struct FocusFollow {
    /// The control that had the focus when the pad last moved it, and the frames left to see it go.
    moved: Option<(Option<Id>, u8)>,
    /// Frames still wanted after a scroll was asked for: a list scrolls as the frame after the
    /// request ends, and is drawn so in the one after that.
    settling: u8,
}

impl FocusFollow {
    /// The events for this frame (to go into egui's raw input): the keys for the pad's moves
    /// `nav`, and the scroll to the control the moves before them reached. `focused` is the
    /// control that has the focus now (`typing`: a text field, [`nav_events`]). True in the
    /// second: frames are still wanted (to see the focus go and scroll to it), however still the
    /// pad is.
    pub fn events(&mut self, nav: &[Nav], focused: Option<Id>, typing: bool) -> (Vec<Event>, bool) {
        let mut out = Vec::new();
        if let Some((from, frames)) = self.moved.take() {
            match focused {
                Some(id) if focused != from => out.push(scroll_into_view(id)),
                _ if frames > 1 => self.moved = Some((from, frames - 1)),
                _ => {}
            }
        }
        self.settling = if out.is_empty() { self.settling.saturating_sub(1) } else { 2 };
        out.extend(nav_events(nav, focused.is_some(), typing));
        if nav.iter().any(|n| n.moves()) {
            self.moved = Some((focused, 3));
        }
        let waiting = self.moved.is_some() || self.settling > 0;
        (out, waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(events: &[Event]) -> Vec<(Key, bool)> {
        events.iter().filter_map(|e| match e { Event::Key { key, pressed, .. } => Some((*key, *pressed)), _ => None }).collect()
    }

    #[test]
    fn the_pad_works_the_interface_as_keys_do() {
        // Nothing focused: the first move focuses the first control, the next ones move on.
        assert_eq!(keys(&nav_events(&[Nav::Down, Nav::Down], false, false)), [(Key::Tab, true), (Key::Tab, false), (Key::ArrowDown, true), (Key::ArrowDown, false)]);
        assert_eq!(keys(&nav_events(&[Nav::Left], true, false)), [(Key::ArrowLeft, true), (Key::ArrowLeft, false)]);
        // A works what has the focus, else it is the pointer's click alone.
        assert_eq!(keys(&nav_events(&[Nav::Activate], true, false)), [(Key::Enter, true), (Key::Enter, false)]);
        assert!(nav_events(&[Nav::Activate], false, false).is_empty());
        assert_eq!(keys(&nav_events(&[Nav::Back], false, false)), [(Key::Escape, true), (Key::Escape, false)]);
        assert_eq!(keys(&nav_events(&[Nav::Menu], true, false)), [(Key::Escape, true), (Key::Escape, false)]);
    }

    /// On a text field (which keeps the arrows for its cursor) the d-pad goes on with Tab, down
    /// and right to the next control, up and left to the one before; off it, arrows again.
    #[test]
    fn the_pad_leaves_a_text_field_with_tab() {
        assert_eq!(keys(&nav_events(&[Nav::Down], true, true)), [(Key::Tab, true), (Key::Tab, false)]);
        let shift_tab: Vec<_> = nav_events(&[Nav::Left], true, true).into_iter().filter_map(|e| match e { Event::Key { key, modifiers, pressed: true, .. } => Some((key, modifiers)), _ => None }).collect();
        assert_eq!(shift_tab, [(Key::Tab, Modifiers::SHIFT)]);
        assert_eq!(keys(&nav_events(&[Nav::Right, Nav::Down], true, true)), [(Key::Tab, true), (Key::Tab, false), (Key::ArrowDown, true), (Key::ArrowDown, false)]);
        assert_eq!(keys(&nav_events(&[Nav::Activate], true, true)), [(Key::Enter, true), (Key::Enter, false)]);
    }

    /// The control the pad moves to is scrolled to once it has the focus (which egui hands over
    /// a frame after the keys); not when the focus stays put, nor when it moves another way.
    #[test]
    fn the_control_the_pad_moves_to_is_scrolled_into_view() {
        let (a, b) = (Id::new("a"), Id::new("b"));
        let scrolled = |events: &[Event]| events.iter().find_map(|e| match e { Event::AccessKitActionRequest(r) => Some(r.target_node), _ => None });
        let mut follow = FocusFollow::default();
        let (events, waiting) = follow.events(&[Nav::Down], Some(a), false);
        assert_eq!(keys(&events), [(Key::ArrowDown, true), (Key::ArrowDown, false)]);
        assert!(waiting, "frames are wanted to see the focus go");
        // (A frame on, egui has not handed it over yet.)
        let (events, waiting) = follow.events(&[], Some(a), false);
        assert!(events.is_empty() && waiting);
        let (events, waiting) = follow.events(&[], Some(b), false);
        assert_eq!(scrolled(&events), Some(b.accesskit_id()));
        assert!(waiting, "frames more, to draw the list scrolled");
        assert!(follow.events(&[], Some(b), false).1);
        assert!(!follow.events(&[], Some(b), false).1);
        assert!(follow.events(&[], Some(a), false).0.is_empty(), "a focus moved by the mouse is left where it is");
        // At the end of a list the focus stays: nothing scrolls, and the wait ends.
        follow.events(&[Nav::Down], Some(b), false);
        for _ in 0..3 {
            assert!(scrolled(&follow.events(&[], Some(b), false).0).is_none());
        }
        assert!(!follow.events(&[], Some(b), false).1);
        // A and B do not move it.
        assert!(!follow.events(&[Nav::Activate, Nav::Back], Some(b), false).1);
    }

    /// A frame of a list of 60 buttons in a scroll area 200 points high: how far it is scrolled,
    /// and each button's id and place.
    fn list(ctx: &egui::Context, events: Vec<Event>) -> (f32, Vec<(Id, egui::Rect)>) {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))), events, ..Default::default() };
        let (mut offset, mut buttons) = (0.0, Vec::new());
        ctx.run_ui(input, |ui| {
            offset = egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    for i in 0..60 {
                        let button = ui.button(format!("row {i}"));
                        buttons.push((button.id, button.rect));
                    }
                })
                .state
                .offset
                .y;
        })
        .drop_without_applying_deltas();
        (offset, buttons)
    }

    /// The request reaches the control: a list at its top scrolls down to a control far below it.
    #[test]
    fn the_request_scrolls_a_list_to_the_control() {
        let ctx = egui::Context::default();
        let (offset, buttons) = list(&ctx, Vec::new());
        assert_eq!(offset, 0.0);
        let (far, _) = buttons[50];
        list(&ctx, vec![scroll_into_view(far)]);
        // (Drawn scrolled two frames on, as `FocusFollow` waits for.)
        list(&ctx, Vec::new());
        let (offset, buttons) = list(&ctx, Vec::new());
        assert!(offset > 0.0, "scrolled down");
        let rect = buttons[50].1;
        assert!(rect.top() >= 0.0 && rect.bottom() <= 200.0, "row 50 in view: {rect:?}");
    }
}
