//! Rebindable game keys: the bindings from `config.toml` (`[keys]`, edited in the
//! launcher), parsed into physical keys and modifiers.

use winit::keyboard::KeyCode;

use crate::input::Input;

/// A key and the modifiers that must be held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Binding {
    /// "A", "Ctrl+D", "Shift+Enter", "Period" (egui's key names, US layout).
    pub fn parse(text: &str) -> Option<Binding> {
        let mut b = Binding { key: KeyCode::Escape, ctrl: false, shift: false, alt: false };
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        // "Plus" names the key; a trailing "+" (as in "Ctrl++") does too.
        if parts.len() >= 2 && parts[parts.len() - 1].is_empty() {
            parts.pop();
            *parts.last_mut()? = "Plus";
        }
        let (key, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => b.ctrl = true,
                "shift" => b.shift = true,
                "alt" => b.alt = true,
                _ => return None,
            }
        }
        b.key = key_code(key)?;
        Some(b)
    }

    /// Whether it was pressed this frame. Ctrl and Alt must match exactly (so "A" is not
    /// "Ctrl+A"); Shift only counts when the binding asks for it (Shift queues orders).
    pub fn pressed(&self, input: &Input) -> bool {
        input.pressed(self.key) && self.modifiers_match(input.key_pressed_modifiers(self.key))
    }

    pub fn held(&self, input: &Input) -> bool {
        input.held(self.key) && self.modifiers_match(input.modifiers)
    }

    fn modifiers_match(&self, m: winit::keyboard::ModifiersState) -> bool {
        m.control_key() == self.ctrl && m.alt_key() == self.alt && (!self.shift || m.shift_key())
    }
}

/// Every action's binding.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    /// The configuration these were parsed from (to notice changes).
    source: ta_config::Keys,
    map: Vec<(&'static str, Option<Binding>)>,
    /// Keys the classic interface's page takes for TA's own shortcuts this frame: bindings on
    /// them without Ctrl or Alt stand down (F cycles the fire orders there, not Fight).
    pub claimed: Vec<KeyCode>,
}

impl Bindings {
    pub fn from_config(keys: &ta_config::Keys) -> Bindings {
        let map = ta_config::Keys::ACTIONS
            .iter()
            .map(|&(id, _, default)| {
                let text = keys.get(id);
                let parsed = Binding::parse(text).or_else(|| {
                    log::warn!("key binding {id} = {text:?} not understood; using {default}");
                    Binding::parse(default)
                });
                (id, parsed)
            })
            .collect();
        Bindings { source: keys.clone(), map, claimed: Vec::new() }
    }

    /// Whether an action's key is held, whatever the modifiers (camera scrolling keeps
    /// going with Shift for speed or Ctrl to orbit).
    pub fn key_held(&self, input: &Input, action: &str) -> bool {
        self.binding(action).is_some_and(|b| input.held(b.key))
    }

    /// Whether an action's key was pressed this frame, whatever the modifiers.
    pub fn key_pressed(&self, input: &Input, action: &str) -> bool {
        input.pad_action(action) || self.binding(action).is_some_and(|b| input.pressed(b.key))
    }

    /// Re-parses when the configuration changed.
    pub fn sync(&mut self, keys: &ta_config::Keys) {
        if self.map.is_empty() || self.source != *keys {
            *self = Bindings::from_config(keys);
        }
    }

    /// An action's key as configured ("F1", "Ctrl+S"), for hints.
    pub fn text(&self, action: &str) -> &str {
        self.source.get(action)
    }

    /// The configuration these were parsed from (for `fill`).
    pub fn config(&self) -> &ta_config::Keys {
        &self.source
    }

    fn binding(&self, action: &str) -> Option<Binding> {
        self.map.iter().find(|(id, _)| *id == action).and_then(|&(_, b)| b).filter(|b| b.ctrl || b.alt || !self.claimed.contains(&b.key))
    }

    pub fn pressed(&self, input: &Input, action: &str) -> bool {
        input.pad_action(action) || self.binding(action).is_some_and(|b| b.pressed(input))
    }

    pub fn held(&self, input: &Input, action: &str) -> bool {
        self.binding(action).is_some_and(|b| b.held(input))
    }

    /// Whether some action is bound to `key` with exactly these Ctrl and Alt.
    pub fn uses(&self, key: KeyCode, ctrl: bool, alt: bool) -> bool {
        self.map.iter().any(|(_, b)| b.is_some_and(|b| b.key == key && b.ctrl == ctrl && b.alt == alt))
    }
}

/// `template` translated, with every `{action}` replaced by its binding and the mouse buttons
/// by the ones that give orders and clear the selection (`{order_click}`, `{order_drag}`,
/// `{deselect}`): the controls reference's rows, tips and the tutorial's steps.
pub fn fill(template: &str, keys: &ta_config::Keys, order_button: ta_config::OrderButton) -> String {
    fill_by(template, keys, order_button, ta_ui::i18n::t)
}

/// [`fill`] in English whatever the language: what names the tutorial's spoken lines.
pub fn fill_english(template: &str, keys: &ta_config::Keys, order_button: ta_config::OrderButton) -> String {
    fn same(text: &str) -> &str {
        text
    }
    fill_by(template, keys, order_button, same)
}

fn fill_by(template: &str, keys: &ta_config::Keys, order_button: ta_config::OrderButton, t: impl for<'x> Fn(&'x str) -> &'x str) -> String {
    let left = order_button == ta_config::OrderButton::Left;
    let mut out = String::new();
    // (The mouse's words translated; the keys' names stay.)
    let mut rest = t(template);
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else { break };
        out.push_str(&rest[..start]);
        out.push_str(match &rest[start + 1..start + len] {
            "order_click" if left => t("Click"),
            "order_click" => t("Right-click"),
            "order_drag" if left => t("Drag"),
            "order_drag" => t("Right-drag"),
            "deselect" if left => t("Esc / Right-click"),
            "deselect" => "Esc",
            action => keys.get(action),
        });
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// The physical key of a letter (TA's GUI shortcuts).
pub fn letter_key(c: char) -> Option<KeyCode> {
    key_code(&c.to_string())
}

/// egui's key names → physical keys (US layout).
fn key_code(name: &str) -> Option<KeyCode> {
    use KeyCode::*;
    let letters = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ];
    let digits = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
    let f_keys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_uppercase() {
            return Some(letters[(c - b'A') as usize]);
        }
        if c.is_ascii_digit() {
            return Some(digits[(c - b'0') as usize]);
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<usize>().ok()) {
        return f_keys.get(n.checked_sub(1)?).copied();
    }
    Some(match upper.as_str() {
        "SPACE" => Space,
        "ENTER" => Enter,
        "TAB" => Tab,
        "BACKSPACE" => Backspace,
        "ESCAPE" => Escape,
        "PERIOD" => Period,
        "COMMA" => Comma,
        "MINUS" => Minus,
        "EQUALS" | "PLUS" => Equal,
        "SEMICOLON" | "COLON" => Semicolon,
        "SLASH" | "QUESTIONMARK" => Slash,
        "BACKSLASH" | "PIPE" => Backslash,
        "OPENBRACKET" | "OPENCURLYBRACKET" => BracketLeft,
        "CLOSEBRACKET" | "CLOSECURLYBRACKET" => BracketRight,
        "BACKTICK" => Backquote,
        "QUOTE" => Quote,
        "HOME" => Home,
        "END" => End,
        "INSERT" => Insert,
        "DELETE" => Delete,
        "PAGEUP" => PageUp,
        "PAGEDOWN" => PageDown,
        "UP" => ArrowUp,
        "DOWN" => ArrowDown,
        "LEFT" => ArrowLeft,
        "RIGHT" => ArrowRight,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_parse_keys_and_modifiers() {
        assert_eq!(Binding::parse("A"), Some(Binding { key: KeyCode::KeyA, ctrl: false, shift: false, alt: false }));
        assert_eq!(Binding::parse("Ctrl+D"), Some(Binding { key: KeyCode::KeyD, ctrl: true, shift: false, alt: false }));
        assert_eq!(Binding::parse("shift + alt + F7"), Some(Binding { key: KeyCode::F7, ctrl: false, shift: true, alt: true }));
        assert_eq!(Binding::parse("Period").map(|b| b.key), Some(KeyCode::Period));
        assert_eq!(Binding::parse("Ctrl++").map(|b| (b.key, b.ctrl)), Some((KeyCode::Equal, true)));
        assert_eq!(Binding::parse("7").map(|b| b.key), Some(KeyCode::Digit7));
        assert_eq!(Binding::parse("Hyper+A"), None);
        assert_eq!(Binding::parse("Blorp"), None);
        assert_eq!(Binding::parse("F13"), None);
    }

    #[test]
    fn every_default_binding_parses() {
        let b = Bindings::from_config(&ta_config::Keys::default());
        for &(id, _, default) in ta_config::Keys::ACTIONS {
            assert!(b.binding(id).is_some(), "{id} = {default}");
        }
    }

    #[test]
    fn chords_distinguish_ctrl_and_alt_while_shift_keeps_orders_queueable() {
        use winit::keyboard::ModifiersState as M;
        let (plain, control, shifted, alternate) = (
            Binding::parse("A").unwrap(), Binding::parse("Ctrl+A").unwrap(),
            Binding::parse("Shift+A").unwrap(), Binding::parse("Alt+A").unwrap(),
        );
        let mut input = Input::default();
        for (modifiers, expected) in [(M::empty(), [true, false, false, false]),
            (M::SHIFT, [true, false, true, false]), (M::CONTROL, [false, true, false, false]),
            (M::ALT, [false, false, false, true]), (M::CONTROL | M::ALT, [false; 4])]
        {
            input.modifiers = modifiers;
            input.press(KeyCode::KeyA);
            assert_eq!([plain, control, shifted, alternate].map(|b| b.pressed(&input)), expected);
            assert_eq!([plain, control, shifted, alternate].map(|b| b.held(&input)), expected);
        }
        input.modifiers = M::empty();
        input.end_frame();
        assert!(plain.held(&input) && !plain.pressed(&input), "a held scroll key is not a repeated command edge");
        input.claim_keys(&[KeyCode::KeyA]);
        assert!(!plain.held(&input), "the keyboard pointer reserves both holds and edges");
    }

    #[test]
    fn live_rebindings_and_classic_page_claims_keep_the_correct_input_owner() {
        use winit::keyboard::ModifiersState as M;
        let mut config = ta_config::Keys::default();
        config.set("attack", "not a key");
        let mut keys = Bindings::from_config(&config);
        let mut input = Input::default();
        input.press(KeyCode::KeyA);
        assert!(keys.pressed(&input, "attack"), "a malformed hand edit falls back to the default");
        config.set("attack", "B");
        keys.sync(&config);
        assert_eq!(keys.config(), &config);
        assert_eq!(keys.text("attack"), "B");
        assert!(!keys.pressed(&input, "attack"));
        input.press(KeyCode::KeyB);
        assert!(keys.pressed(&input, "attack") && keys.held(&input, "attack"));
        assert!(keys.key_pressed(&input, "attack") && keys.key_held(&input, "attack"));
        assert!(keys.uses(KeyCode::KeyB, false, false));
        keys.claimed.push(KeyCode::KeyB);
        keys.sync(&config);
        assert!(!keys.pressed(&input, "attack"), "unchanged settings must retain this frame's classic page claim");
        config.set("attack", "Ctrl+B");
        keys.sync(&config);
        keys.claimed.push(KeyCode::KeyB);
        input.modifiers = M::CONTROL;
        input.press(KeyCode::KeyB);
        assert!(keys.uses(KeyCode::KeyB, true, false));
        assert!(keys.pressed(&input, "attack"), "Ctrl and Alt chords remain available over classic build-page letters");
        input.apply_pad(&crate::gamepad::PadFrame { actions: vec!["attack"], ..Default::default() }, false);
        assert!(keys.pressed(&input, "attack") && keys.key_pressed(&input, "attack"));
        assert!(!keys.pressed(&input, "missing") && !keys.held(&input, "missing"));
    }

    #[test]
    fn control_hints_follow_rebindings_and_the_selected_order_button_scheme() {
        let mut keys = ta_config::Keys::default();
        keys.set("attack", "Ctrl+F7");
        let template = "{attack}: {order_click}; {order_drag}; {deselect}";
        assert_eq!(fill(template, &keys, ta_config::OrderButton::Right), "Ctrl+F7: Right-click; Right-drag; Esc");
        assert_eq!(fill(template, &keys, ta_config::OrderButton::Left), "Ctrl+F7: Click; Drag; Esc / Right-click");
        assert_eq!(fill("Unfinished {attack", &keys, ta_config::OrderButton::Right), "Unfinished {attack");
        assert_eq!(letter_key('z'), Some(KeyCode::KeyZ));
        assert_eq!(letter_key('é'), None);
    }
}
