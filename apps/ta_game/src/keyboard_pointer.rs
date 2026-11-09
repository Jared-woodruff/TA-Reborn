//! Opt-in keyboard access to the battle's existing pointer controls. The same click path
//! selects units, places buildings, operates the build menu and moves the minimap view.
//! This is pointer emulation with unit traversal, not a screen-reader navigation model.

use glam::{Vec2, Vec3};
use ta_render::Camera;
use ta_sim::{Seen, UnitId};
use winit::keyboard::KeyCode;

use crate::{game::Game, gamepad::PadFrame, input::Input};

pub const RESERVED: &[KeyCode] = &[
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::Enter,
    KeyCode::NumpadEnter,
    KeyCode::End,
    KeyCode::Home,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Tab,
    KeyCode::Backspace,
];

pub fn reserved_event(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key {
            key: egui::Key::ArrowUp
                | egui::Key::ArrowDown
                | egui::Key::ArrowLeft
                | egui::Key::ArrowRight
                | egui::Key::Enter
                | egui::Key::End
                | egui::Key::Home
                | egui::Key::PageUp
                | egui::Key::PageDown
                | egui::Key::Tab
                | egui::Key::Backspace,
            ..
        }
    )
}

#[derive(Default)]
pub struct KeyboardPointer {
    /// A tap presses for one frame and releases on the next; holding Enter never drags.
    buttons: [bool; 2],
    initialized: bool,
    ours: Option<UnitId>,
    enemy: Option<UnitId>,
    /// Position the pointer after the requested camera jump has been applied.
    target: Option<UnitId>,
}

impl KeyboardPointer {
    /// Emit a complete tap even when the physical key stays held. When suspended, release
    /// pending taps without creating a new action. Focus loss separately cancels gestures.
    pub fn update(&mut self, input: &mut Input, dt: f32, size: Vec2, active: bool, order: ta_config::OrderButton) -> PadFrame {
        let mut frame = PadFrame::default();
        (frame.left, frame.right) = self.taps(if active {
            // A traversal first jumps the camera; do not click the old location in that frame.
            let traversing = input.pressed(KeyCode::Tab) || input.pressed(KeyCode::Backspace);
            let select = !traversing && (input.pressed(KeyCode::Enter) || input.pressed(KeyCode::NumpadEnter));
            let order_key = !traversing && input.pressed(KeyCode::End);
            [select || order_key && order == ta_config::OrderButton::Left, order_key && order == ta_config::OrderButton::Right]
        } else {
            [false; 2]
        });
        if !active {
            self.target = None;
            return frame;
        }
        if !std::mem::replace(&mut self.initialized, true) {
            frame.cursor = Some(size * 0.5);
        }
        // A tap has one location. Moving during its release would turn a click into a drag
        // (or a right-button camera orbit), defeating the no-hold alternative.
        if frame.left.is_some() || frame.right.is_some() {
            return frame;
        }
        let axis = |positive, negative| (u8::from(input.held(positive)) as f32) - (u8::from(input.held(negative)) as f32);
        let movement = Vec2::new(axis(KeyCode::ArrowRight, KeyCode::ArrowLeft), axis(KeyCode::ArrowDown, KeyCode::ArrowUp));
        if movement != Vec2::ZERO {
            let speed = if input.modifiers.shift_key() { 90.0 } else { 600.0 };
            let to = frame.cursor.unwrap_or(input.mouse) + movement.normalize() * speed * dt.clamp(0.0, 0.1);
            frame.cursor = Some(to.clamp(Vec2::ZERO, (size - Vec2::ONE).max(Vec2::ZERO)));
            self.target = None;
        }
        if input.pressed(KeyCode::Home) {
            frame.cursor = Some(size * 0.5);
            self.target = None;
        }
        frame.zoom = axis(KeyCode::PageUp, KeyCode::PageDown) * dt.clamp(0.0, 0.1) * 5.0;
        // Read traversal before claiming these keys in the host.
        frame
    }

    pub fn has_pending_click(&self) -> bool {
        self.buttons.iter().any(|&held| held)
    }

    fn taps(&mut self, pressed: [bool; 2]) -> (Option<bool>, Option<bool>) {
        let mut out = [None; 2];
        for i in 0..2 {
            if self.buttons[i] {
                out[i] = Some(false);
                self.buttons[i] = false;
            } else if pressed[i] {
                out[i] = Some(true);
                self.buttons[i] = true;
            }
        }
        (out[0], out[1])
    }

    /// Stable traversal never includes radar-only, hidden, dead or transported units. Enemy
    /// traversal aims the cursor and leaves the player's order-giving selection intact.
    pub fn traverse(&mut self, game: &mut Game, input: &Input) {
        let enemy = input.pressed(KeyCode::Backspace);
        if !enemy && !input.pressed(KeyCode::Tab) {
            return;
        }
        if game.spectator || game.playback.is_some() {
            return;
        }
        let reverse = input.modifiers.shift_key();
        let last = if enemy { self.enemy } else { self.ours };
        let mut candidates: Vec<_> = game
            .sim
            .units()
            .filter(|u| eligible(u.dead, u.carried_by.is_some(), game.seen(u), if enemy { game.sim.is_enemy(u.owner, game.local_player) } else { u.owner == game.local_player }))
            .map(|u| u.id)
            .collect();
        candidates.sort_unstable();
        let Some(id) = cycle(&candidates, last, reverse) else {
            game.message(ta_ui::i18n::t(if enemy { ta_ui::i18n::n("No visible enemy units") } else { ta_ui::i18n::n("No available units") }));
            return;
        };
        if enemy {
            self.enemy = Some(id);
            if let Some(u) = game.sim.unit(id) {
                game.focus_request = Some(Vec3::from_array(u.pos.to_f32()));
            }
        } else {
            self.ours = Some(id);
            game.placement = None;
            game.command_mode = None;
            game.select_and_look(id);
        }
        self.target = Some(id);
        if let Some(u) = game.sim.unit(id) {
            let name = game.sim.unit_type(u).def.display_name.clone();
            game.message(if enemy { ta_ui::i18n::tf("Target: {name}", &[("name", &name)]) } else { ta_ui::i18n::tf("Selected: {name}", &[("name", &name)]) });
        }
    }

    pub fn snap_target(&mut self, game: &Game, camera: &Camera) -> Option<Vec2> {
        let id = self.target.take()?;
        let u = game.sim.unit(id)?;
        if u.dead || u.carried_by.is_some() || game.seen(u) != Seen::Visible {
            return None;
        }
        let base = game.scene.unit_position(id, game.alpha()).unwrap_or(Vec3::from_array(u.pos.to_f32()));
        let height = game.sim.unit_type(u).model.height.to_f32().max(8.0);
        Game::screen_of(camera, base + Vec3::Y * height * 0.5).filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x < camera.viewport.x && p.y < camera.viewport.y)
    }
}

fn eligible(dead: bool, transported: bool, seen: Seen, relationship: bool) -> bool {
    relationship && !dead && !transported && seen == Seen::Visible
}

fn cycle<T: Copy + Ord>(sorted: &[T], last: Option<T>, reverse: bool) -> Option<T> {
    if reverse {
        sorted.iter().rev().find(|&&id| last.is_none_or(|last| id < last)).or(sorted.last()).copied()
    } else {
        sorted.iter().find(|&&id| last.is_none_or(|last| id > last)).or(sorted.first()).copied()
    }
}

pub fn hint(ctx: &egui::Context, view: egui::Rect, keys: &ta_config::Keys) {
    let text = ta_ui::i18n::tf("Keyboard pointer · {key}: off", &[("key", &keys.get("keyboard_pointer"))]);
    let width = view.width().min(350.0).max(160.0) - 20.0;
    // The HUD's free view excludes both the modern command deck and the classic bottom
    // bar. Keep the persistent hint below the transient notices at the top of the battle.
    egui::Area::new(egui::Id::new("keyboard_pointer_hint")).order(egui::Order::Foreground).interactable(false).pivot(egui::Align2::LEFT_BOTTOM).fixed_pos(egui::pos2(view.left() + 10.0, view.bottom() - 10.0)).show(
        ctx,
        |ui| {
            egui::Frame::new().fill(egui::Color32::from_black_alpha(225)).inner_margin(8.0).show(ui, |ui| {
                ui.set_max_width(width);
                ui.add(egui::Label::new(egui::RichText::new(text).color(egui::Color32::WHITE)).wrap());
                let help = ta_ui::i18n::tf("Arrows: pointer · Enter: select · End: order · {help}: help", &[("help", &keys.get("help"))]);
                ui.add(egui::Label::new(help).wrap());
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_wraps_both_ways_and_survives_removed_units() {
        assert_eq!(cycle(&[1, 3, 5], None, false), Some(1));
        assert_eq!(cycle(&[1, 3, 5], Some(5), false), Some(1));
        assert_eq!(cycle(&[1, 3, 5], Some(4), false), Some(5));
        assert_eq!(cycle(&[1, 3, 5], Some(1), true), Some(5));
        assert_eq!(cycle::<u32>(&[], None, false), None);
    }

    #[test]
    fn traversal_does_not_disclose_hidden_radar_or_carried_units() {
        assert!(eligible(false, false, Seen::Visible, true));
        assert!(!eligible(false, false, Seen::Hidden, true));
        assert!(!eligible(false, false, Seen::Radar, true));
        assert!(!eligible(false, true, Seen::Visible, true));
        assert!(!eligible(true, false, Seen::Visible, true));
        assert!(!eligible(false, false, Seen::Visible, false));
    }

    #[test]
    fn keyboard_clicks_release_without_repeating_or_dragging() {
        let mut pointer = KeyboardPointer::default();
        let mut input = Input::default();
        input.press(KeyCode::Enter);
        let first = pointer.update(&mut input, 0.016, Vec2::splat(100.0), true, ta_config::OrderButton::Right);
        assert_eq!(first.left, Some(true));
        input.end_frame();
        let release = pointer.update(&mut input, 0.016, Vec2::splat(100.0), true, ta_config::OrderButton::Right);
        assert_eq!(release.left, Some(false));
        assert_eq!(pointer.update(&mut input, 0.016, Vec2::splat(100.0), true, ta_config::OrderButton::Right).left, None);
        input.press(KeyCode::End);
        assert_eq!(pointer.update(&mut input, 0.016, Vec2::splat(100.0), true, ta_config::OrderButton::Right).right, Some(true));
        assert_eq!(pointer.update(&mut input, 0.016, Vec2::splat(100.0), false, ta_config::OrderButton::Right).right, Some(false));
    }

    #[test]
    fn traversal_does_not_click_before_the_camera_has_moved() {
        let mut pointer = KeyboardPointer::default();
        let mut input = Input::default();
        input.press(KeyCode::Tab);
        input.press(KeyCode::End);
        let frame = pointer.update(&mut input, 0.016, Vec2::splat(100.0), true, ta_config::OrderButton::Right);
        assert_eq!((frame.left, frame.right), (None, None));
    }

    #[test]
    fn click_release_keeps_the_pointer_in_place() {
        let mut pointer = KeyboardPointer::default();
        let mut input = Input::default();
        input.press(KeyCode::End);
        pointer.update(&mut input, 0.016, Vec2::splat(100.0), true, ta_config::OrderButton::Right);
        input.end_frame();
        input.press(KeyCode::ArrowRight);
        let release = pointer.update(&mut input, 0.1, Vec2::splat(100.0), true, ta_config::OrderButton::Right);
        assert_eq!(release.right, Some(false));
        assert_eq!(release.cursor, None, "an order click must not become a drag");
    }

    #[test]
    fn claimed_arrows_remain_held_for_pointer_but_not_battle_hotkeys() {
        let mut pointer = KeyboardPointer::default();
        let mut input = Input::default();
        input.mouse = Vec2::splat(50.0);
        input.press(KeyCode::ArrowRight);
        let frame = pointer.update(&mut input, 0.1, Vec2::splat(100.0), true, ta_config::OrderButton::Left);
        assert_eq!(frame.cursor, Some(Vec2::new(99.0, 50.0)));
        input.claim_keys(RESERVED);
        assert!(!input.held(KeyCode::ArrowRight));
        assert!(!input.pressed_keys().contains(&KeyCode::ArrowRight));
        input.end_frame();
        assert!(input.held(KeyCode::ArrowRight));
        let suspended = pointer.update(&mut input, 0.1, Vec2::splat(100.0), false, ta_config::OrderButton::Left);
        assert_eq!(suspended.cursor, None);
    }
}
