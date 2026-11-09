//! The in-game controls reference (F1, or Controls in the pause menu). Rebindable keys
//! appear as `{action}` and show whatever they are bound to; `{order_click}`,
//! `{order_drag}` and `{deselect}` follow the button that gives orders.

use egui::RichText;
use ta_ui::i18n::n;
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets as w;

use crate::theme::*;

/// The radial menu's rows (see [`radial_rows`]): a right-click on one of our units, a middle
/// click, and holding the button in the right-click's place when the menu is kept to a hold.
const RADIAL_CLICK: &str = n("Right-click on your unit");
const RADIAL_MIDDLE: &str = n("Middle-click on your unit");
const RADIAL_HOLD: (&str, &str) = (n("Hold the right button on your unit"), n("Its menu: orders, stance, build, self-destruct (flick towards a choice and let go)"));

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        n("SELECTING"),
        &[
            (n("Click / drag"), n("Select a unit / box-select (Shift adds or removes)")),
            (n("Double-click"), n("Every unit of that type on screen")),
            ("{select_type}", n("Every unit of the selected types, map-wide")),
            ("{select_on_screen}", n("Every unit on the screen")),
            ("{select_all} / {select_commander}", n("Select everything / the Commander (and look at it)")),
            ("{select_builders} / {select_factories} / {select_armed} / {select_aircraft}", n("Constructors (not the Commander) / factories / armed units that move / aircraft")),
            ("{select_armed_aircraft} / {select_radar} / {select_mines}", n("Armed aircraft / radar, jammers and sonar / mines")),
            ("Ctrl+0-9", n("Assign a control group")),
            ("0-9", n("Recall a group (Shift adds; tap twice to look at it)")),
            ("{idle_builder} / {idle_factory}", n("Next idle constructor / idle factory")),
            ("{deselect}", n("Cancel / clear the selection")),
        ],
    ),
    (
        n("ORDERS"),
        &[
            ("{order_click}", n("Move, attack, repair, guard, load, reclaim (by context)")),
            (RADIAL_CLICK, n("Its menu: orders, stance, build, self-destruct (hold the button and flick towards a choice)")),
            (RADIAL_MIDDLE, n("Its menu, however the right button is set to open it")),
            ("Ctrl+{order_click}", n("Force fire: attack whatever is there, yours too, or the ground")),
            ("Ctrl+{order_drag}", n("Formation: spread along the line")),
            (n("Shift+order"), n("Queue it after the current orders")),
            ("{move} / {attack} / {patrol} / {guard}", n("Move, attack, patrol, guard")),
            ("{repair} / {load} / {unload} / {capture} / {reclaim}", n("Repair, load, unload, capture, reclaim")),
            ("{fight} / {resurrect}", n("Fight (attack-move), resurrect")),
            ("{fire_orders} / {move_orders} / {on_off}", n("Fire orders, move orders, on and off")),
            ("{duty}", n("Constructors on duty: they work by themselves (economy, repair, reclaim, defence; any order takes them off duty)")),
            (n("Drag (those four)"), n("Attack, capture, resurrect, reclaim over an area")),
            ("{dgun}", n("D-gun / launch (manual fire)")),
            ("{area_mex}", n("Extractors on every metal spot in the circle you drag (a click: around the cursor)")),
            ("{stop}", n("Stop")),
            ("{cloak}", n("Cloak or decloak")),
            ("{self_destruct}", n("Self-destruct (again to cancel)")),
        ],
    ),
    (
        n("BUILDING"),
        &[
            (n("Build menu"), n("Click to place; drag to lay out a row")),
            ("Shift", n("Keep placing / queue in factories x5")),
            (n("Alt+drag"), n("Fill a rectangle with the building")),
            (n("Click a queued unit or building"), n("Build it next (right-click: remove it)")),
            (n("Ctrl (extractors)"), n("Place exactly (no snapping onto the metal)")),
            (n("Right-click (menu)"), n("Remove from a factory queue")),
            (n("Factory + orders"), n("Move, patrol, fight and guard orders set where its units go (Shift: waypoints)")),
        ],
    ),
    (
        n("CAMERA"),
        &[
            (n("{scroll_up}/{scroll_left}/{scroll_down}/{scroll_right}, screen edges"), n("Scroll (Shift: faster)")),
            (n("Mouse wheel"), n("Zoom towards the cursor")),
            (n("Middle-drag"), n("Pan")),
            (n("Right-drag / Ctrl+scroll keys"), n("Orbit and tilt (modern camera)")),
            ("{tilt_up} / {tilt_down}", n("Tilt the camera")),
            ("{reset_view}", n("Reset the view")),
            ("{camera_style}", n("Modern / classic camera")),
            ("Ctrl+F6-F9 / F6-F9", n("Store / recall a camera position")),
            ("{last_alert} / Backspace", n("Look at the last attack")),
            ("{next_unit}", n("Look at the next of your units off the screen (of the selection, if any)")),
            ("{overlay_path} / {overlay_height} / {overlay_metal} / {overlay_sight}", n("Map: movement, heights, metal, sight and radar")),
            ("{follow}", n("Follow the selected unit (scroll to let go)")),
        ],
    ),
    (
        n("GAME"),
        &[
            ("{pause} / Pause", n("Pause (for everyone, in multiplayer)")),
            ("{faster} / {slower}", n("Game speed (likewise)")),
            (n("{chat} (Shift: allies)"), n("Chat to everyone / to your allies; a line starting with + is a command, such as +sharemetal, or a cheat code (+help)")),
            ("{cheats}", n("Cheats (single player)")),
            (n("{players} (hold)"), n("Players: teams, and your side's units and income")),
            ("{share}", n("Share resources and units with an ally")),
            ("{jukebox}", n("Jukebox: TA's music and your own")),
            ("{health_bars}", n("Health bars: our damaged units, every damaged unit, or the selection")),
            (n("Shift over a unit"), n("A constructor: everyone's planned buildings; one that cloaks: how near an enemy may come")),
            ("{quick_save}", n("Quick save (Continue in the main menu loads it)")),
            (n("Alt+click"), n("Ping the map for your team")),
            (n("Alt+drag"), n("Signal your team: attack, defend, on my way, metal or energy needed (computer allies act on it)")),
            ("{help}", n("Information on the selected unit, or the one under the pointer; with none, this reference")),
            ("{clear_messages}", n("Clear the messages and the chat")),
            ("{event_history}", n("Battle event history: review and filter earlier notices")),
            ("{quit}", n("Quit")),
            ("{screenshot}", n("Screenshot (saved in your Pictures folder)")),
            ("{photo_mode}", n("Photo mode: free camera, time of day, focus; F12 takes a photo")),
            ("{perspective}", n("Perspective: see through the eyes of the selected unit, or the one under the pointer, as it goes on with its orders (Escape leaves)")),
            (n("{game_menu} / Esc (nothing selected)"), n("Menu: save, options, statistics")),
        ],
    ),
];

/// A row's keys, the alternatives: " / " between them, or ", " in a list ("Up/Left/Down/Right,
/// screen edges": the scroll keys, or the screen's edges), each on a cap of its own.
fn keys_of(keys: &str) -> Vec<&str> {
    keys.split(" / ").flat_map(|alternative| alternative.split(", ")).collect()
}

/// A key cap ([`panel::keycap`]) no wider than `width`: the lettering of a longer one (a mouse
/// action in words, a translation) runs on over more lines, so that it keeps to the keys' column.
fn keycap(ui: &mut egui::Ui, text: &str, width: f32) {
    let lettering = || RichText::new(text).font(crate::theme::strong(12.0));
    let line = ta_ui::widgets::galley(ui, lettering());
    if line.size().x + 14.0 <= width {
        panel::keycap(ui, text);
        return;
    }
    let galley = egui::WidgetText::from(lettering()).into_galley(ui, Some(egui::TextWrapMode::Wrap), (width - 14.0).max(24.0), egui::TextStyle::Body);
    // (As tall as a key, 22, and a line more for each line more.)
    let size = egui::vec2(galley.size().x + 14.0, galley.size().y + 22.0 - line.size().y);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().add(ta_ui::metal::slab(ui.ctx(), rect, ta_ui::metal::Slab::Steel, 0.0, false));
    ta_ui::metal::slab_text(ui.painter(), rect.center() - egui::vec2(0.0, 0.5), galley, true, CREAM, false);
}

/// A row: the keys (" / " between alternatives) in a column of their own, the description
/// wrapping beside them, its first line level with the first row of keys. Returns the room it
/// takes.
fn row(ui: &mut egui::Ui, keys: &str, what: &str) -> egui::Rect {
    // (A row of keys is as tall as a key, 22, and a little.)
    const KEYS_ROW: f32 = 24.0;
    let key_width = (ui.available_width() * 0.4).clamp(150.0, 220.0);
    let rect = ui
        .horizontal_top(|ui| {
            let keys_layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
            ui.allocate_ui_with_layout(egui::vec2(key_width, KEYS_ROW), keys_layout, |ui| {
                ui.set_width(key_width);
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                let slash = ui.painter().layout_no_wrap("/".to_string(), egui::TextStyle::Body.resolve(ui.style()), STEEL_DIM);
                let parts = keys_of(keys);
                for (i, part) in parts.iter().enumerate() {
                    let part = ta_ui::i18n::t(part);
                    // The "/" goes with the key before it (laid out as one, as wide as both), so
                    // that a long list wraps after it, as text does, rather than leaving it at the
                    // head of a line.
                    let separated = i + 1 < parts.len();
                    let after = if separated { 4.0 + slash.size().x } else { 0.0 };
                    let room = key_width - after;
                    let key = ta_ui::widgets::galley(ui, RichText::new(part).font(crate::theme::strong(12.0)));
                    let size = egui::vec2((key.size().x + 14.0).min(room) + after, 22.0);
                    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                        keycap(ui, part, room);
                        if separated {
                            ui.label(RichText::new("/").color(STEEL_DIM));
                        }
                    });
                }
            });
            ui.vertical(|ui| {
                ui.add_space(((KEYS_ROW - ui.text_style_height(&egui::TextStyle::Body)) / 2.0).max(0.0));
                ui.add(egui::Label::new(RichText::new(ta_ui::i18n::t(what)).color(TEXT)).wrap());
            });
        })
        .response
        .rect;
    ui.add_space(3.0);
    rect
}

/// A list's rows as laid out, their tops and feet down from the list's top, so that a view onto
/// it can end between two of them rather than through a line of text. A heading keeps with the
/// row after it.
#[derive(Clone, Default)]
pub(crate) struct Rows {
    origin: f32,
    spans: Vec<(f32, f32)>,
    open: Option<f32>,
}

impl Rows {
    /// Rows laid out in `ui` from where its cursor is now, the list's top.
    pub fn new(ui: &egui::Ui) -> Rows {
        Rows { origin: ui.cursor().top(), ..Default::default() }
    }

    /// A row starts where `ui`'s cursor is (a heading's row, the heading's).
    pub fn start(&mut self, ui: &egui::Ui) {
        self.open.get_or_insert(ui.cursor().top());
    }

    /// The row started ends at `bottom`.
    pub fn end_at(&mut self, bottom: f32) {
        if let Some(top) = self.open.take() {
            self.spans.push((top - self.origin, bottom - self.origin));
        }
    }

    /// The row started ends with what `ui` holds now.
    pub fn end(&mut self, ui: &egui::Ui) {
        self.end_at(ui.min_rect().bottom());
    }

    /// How far down a view onto the rows `room` deep may reach, at most, for its edge to fall
    /// between two rows (midway across the gap between them): `room` itself when it does already
    /// or the rows end before it, or when the first row is deeper than it.
    pub fn cut(&self, room: f32) -> f32 {
        let mut spans = self.spans.clone();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        // (Rows over one another taken as one; rows meeting, with no gap, still part there.)
        let mut taken: Vec<(f32, f32)> = Vec::new();
        for (top, bottom) in spans {
            match taken.last_mut() {
                Some(last) if top < last.1 - 0.5 => last.1 = last.1.max(bottom),
                _ => taken.push((top, bottom)),
            }
        }
        match taken.iter().position(|&(top, bottom)| top < room && room < bottom) {
            Some(i) if i > 0 => (taken[i - 1].1 + taken[i].0) / 2.0,
            _ => room,
        }
    }
}

/// The keyboard pointer's keys (Accessibility).
const POINTER: &[(&str, &str)] = &[
    ("{keyboard_pointer}", n("Turn keyboard pointer controls on or off (also in Accessibility settings)")),
    (n("Arrow keys"), n("Move the pointer across the battlefield, build menu and minimap (Shift: precise movement)")),
    ("Enter / End", n("Select, place or activate a control / issue a contextual order. Each press is a click; no hold or drag is needed.")),
    ("Tab / Shift+Tab", n("Select and locate the next / previous unit of yours")),
    ("Backspace / Shift+Backspace", n("Aim at the next / previous visible enemy, keeping your selection")),
    ("PageUp / PageDown / Home", n("Zoom in / out / centre the pointer")),
    ("Esc", n("Cancel an order or placement; clear the selection; open the menu")),
    (n("While enabled"), n("These navigation keys replace their usual battle shortcuts. Other order keys still work. Text fields and dialogs use normal keyboard navigation.")),
];

/// A section of the reference: its title, and its rows (the keys as bound, and what they do).
type Section = (&'static str, Vec<(String, &'static str)>);

/// `rows` with the radial menu's as the player's controls open it: holding the right button in
/// the right-click's place when the menu is kept to a hold, and none of them with it off.
fn radial_rows(rows: &[(&'static str, &'static str)], controls: &ta_config::Gameplay) -> Vec<(&'static str, &'static str)> {
    rows.iter()
        .filter_map(|&(keys, what)| match keys {
            RADIAL_CLICK | RADIAL_MIDDLE if !controls.radial_menu => None,
            RADIAL_CLICK if controls.radial_opens == ta_config::RadialOpens::Hold => Some(RADIAL_HOLD),
            _ => Some((keys, what)),
        })
        .collect()
}

/// Every section, the keys filled in as bound, the mouse's words for the button that gives
/// orders and the radial menu's as it opens; the gamepad's buttons and the keyboard pointer's
/// keys last.
fn sections(keys: &ta_config::Keys, controls: &ta_config::Gameplay) -> Vec<Section> {
    let order_button = controls.order_button;
    let fill = |rows: &[(&str, &'static str)]| rows.iter().map(|&(key, what)| (crate::keys::fill(key, keys, order_button), what)).collect::<Vec<_>>();
    let mut all: Vec<Section> = SECTIONS.iter().map(|&(title, rows)| (title, fill(&radial_rows(rows, controls)))).collect();
    let pad = crate::gamepad::LAYOUT.iter().map(|&(button, what)| (button.to_owned(), crate::gamepad::button_description(button, what, order_button)));
    all.push((n("GAMEPAD"), pad.collect()));
    all.push((n("KEYBOARD POINTER"), fill(POINTER)));
    all
}

/// The rows of `sections` in which `words` (lower case) are found, in their keys or in what
/// they do, in English or in the language in use (all of a section's, found in its title); the
/// sections left with none, left out.
fn found(sections: Vec<Section>, words: &str) -> Vec<Section> {
    if words.is_empty() {
        return sections;
    }
    let has = |text: &str| text.to_lowercase().contains(words) || ta_ui::i18n::t(text).to_lowercase().contains(words);
    sections
        .into_iter()
        .filter_map(|(title, rows)| {
            let rows: Vec<_> = if has(title) { rows } else { rows.into_iter().filter(|(keys, what)| has(keys) || has(what)).collect() };
            (!rows.is_empty()).then_some((title, rows))
        })
        .collect()
}

/// An entry of the reference's columns.
#[derive(Debug)]
enum Entry<'a> {
    Heading(&'static str),
    Row(&'a str, &'static str),
}

/// `sections` in two columns about as long as each other (a heading counted as a row and a
/// half, a row as the lines its words take, about); a section running on into the second column
/// has its heading again at its top; the first column never ends with a heading, and holds a
/// section too short to share.
fn two_columns(sections: &[Section]) -> [Vec<Entry<'_>>; 2] {
    let mut first: Vec<Entry> = Vec::new();
    for (title, rows) in sections {
        first.push(Entry::Heading(title));
        first.extend(rows.iter().map(|(keys, what)| Entry::Row(keys, what)));
    }
    let weight = |entry: &Entry| match entry {
        Entry::Heading(_) => 1.5,
        Entry::Row(_, what) => 1.0 + (ta_ui::i18n::t(what).chars().count() / 70) as f32 * 0.6,
    };
    let half = first.iter().map(weight).sum::<f32>() / 2.0;
    let mut split = first.len();
    let mut sum = 0.0;
    for (i, entry) in first.iter().enumerate() {
        if sum + weight(entry) / 2.0 > half {
            split = i;
            break;
        }
        sum += weight(entry);
    }
    // (A heading goes with its first row.)
    if split > 0 && matches!(first[split - 1], Entry::Heading(_)) {
        split -= 1;
    }
    if split == 0 {
        split = first.len();
    }
    let mut second = Vec::new();
    if matches!(first.get(split), Some(Entry::Row(..))) {
        let title = first[..split].iter().rev().find_map(|entry| match entry { Entry::Heading(title) => Some(*title), Entry::Row(..) => None });
        second.extend(title.map(Entry::Heading));
    }
    second.extend(first.drain(split..));
    [first, second]
}

/// What the reference shows: everything, or the section of that title.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Shown {
    #[default]
    All,
    Section(&'static str),
}

/// Draws the reference (with the current key bindings and mouse buttons) in the middle of
/// `room`, the part of the screen the battle's interface leaves free, and no bigger than it
/// (it covered the resource bar): a search over it, and a tab for each section (there was
/// room for five rows of all of them); `open` turns false once closed. Returns true when the
/// player asks to change the keys.
pub fn window(ctx: &egui::Context, open: &mut bool, keys: &ta_config::Keys, controls: &ta_config::Gameplay, room: egui::Rect) -> bool {
    // (Its frame, header and tools round the list, as laid out last frame; and 10 clear of the
    // room's edges, where a dialog is kept, or it is shown smaller to fit, its lettering blurred.)
    let chrome_id = egui::Id::new("controls_chrome");
    let chrome = ctx.data(|d| d.get_temp::<egui::Vec2>(chrome_id)).unwrap_or(egui::vec2(48.0, 230.0)) + egui::vec2(20.0, 20.0);
    let page = Panel::new("CONTROLS").width((room.width() - chrome.x).clamp(320.0, 1300.0)).exit(Exit::Close);
    let max_height = (room.height() - chrome.y).clamp(240.0, 640.0);
    // What it shows, and what is searched for (kept for the next time it opens).
    let view_id = egui::Id::new("controls_view");
    let (mut shown, mut search): (Shown, String) = ctx.data(|d| d.get_temp(view_id)).unwrap_or_default();
    // Each column's rows as laid out last frame, for what it showed then: a column shows only
    // the rows wholly above the view's foot (through one, the foot showed the tops of a line of
    // text). The two columns' rows seldom part at the same height: cut short to where they did,
    // the list lost a good part of its height.
    let rows_id = egui::Id::new("controls_rows");
    let (rows_for, shown_rows): ((Shown, String), Vec<Rows>) = ctx.data(|d| d.get_temp(rows_id)).unwrap_or_default();
    let all = sections(keys, controls);
    let mut change = false;
    let mut list = egui::Rect::NOTHING;
    let mut laid_out: Vec<Rows> = Vec::new();
    let mut laid_out_for = (shown, search.clone());
    let response = panel::modal_within(ctx, "controls", page, true, room, |ui| {
        // The search, and the sections shown: every one, or one (a drop-down list where their
        // tabs do not fit).
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if w::search_field(ui, &mut search, ta_ui::i18n::t("Search the controls…"), 240.0, None).changed() && !search.trim().is_empty() {
                // (Found in every section, as the tabs then say.)
                shown = Shown::All;
            }
            let mut tabs = vec![(Shown::All, n("ALL"))];
            tabs.extend(all.iter().map(|&(title, _)| (Shown::Section(title), title)));
            w::segmented(ui, &mut shown, &tabs);
        });
        ui.add_space(8.0);
        let picked = all.iter().filter(|(title, _)| shown == Shown::All || shown == Shown::Section(title)).cloned().collect();
        let matching = found(picked, &search.trim().to_lowercase());
        laid_out_for = (shown, search.clone());
        let fresh = rows_for == laid_out_for;
        // (As tall whatever it shows: the search over it stays where it is as words are typed.)
        let area = egui::ScrollArea::vertical().id_salt(("controls_list", shown)).max_height(max_height).min_scrolled_height(max_height).auto_shrink([false, false]);
        list = ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
            if matching.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(ta_ui::i18n::t("No control matches. Try another word, or clear the search.")).color(STEEL));
                ui.add_space(4.0);
                if ui.add(w::ActionButton::secondary("CLEAR")).clicked() {
                    search.clear();
                }
                return;
            }
            // (The content's clip is the view.)
            let foot = ui.clip_rect().bottom();
            let parts = two_columns(&matching);
            w::columns(ui, 2, |columns| {
                let mut column_rows: Vec<Rows> = columns.iter().map(Rows::new).collect();
                if fresh {
                    for (ui, (last, rows)) in columns.iter_mut().zip(shown_rows.iter().zip(&column_rows)) {
                        let mut clip = ui.clip_rect();
                        clip.max.y = clip.max.y.min(rows.origin + last.cut(foot - rows.origin));
                        ui.set_clip_rect(clip);
                    }
                }
                for ((ui, entries), rows_here) in columns.iter_mut().zip(&parts).zip(column_rows.iter_mut()) {
                    for (i, entry) in entries.iter().enumerate() {
                        match *entry {
                            Entry::Heading(title) => {
                                if i > 0 {
                                    ui.add_space(12.0);
                                }
                                rows_here.start(ui);
                                panel::section(ui, title);
                            }
                            Entry::Row(keys, what) => {
                                rows_here.start(ui);
                                let r = row(ui, keys, what);
                                rows_here.end_at(r.bottom());
                            }
                        }
                    }
                }
                laid_out = column_rows;
            });
        })
        .inner_rect;
        panel::rule(ui);
        // (A row as tall as the button: laid out right to left, it took all the height there
        // was, and the sheet ran down the screen whatever its list.)
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 34.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            change = ui.add(ta_ui::widgets::ActionButton::secondary("CHANGE KEYS AND BUTTONS").size(240.0, 34.0)).clicked();
        });
    });
    ctx.data_mut(|d| d.insert_temp(view_id, (shown, search)));
    if list.is_positive() {
        ctx.data_mut(|d| d.insert_temp(chrome_id, response.rect.size() - list.size()));
        ctx.data_mut(|d| d.insert_temp(rows_id, (laid_out_for, laid_out)));
    }
    if response.exit || change {
        *open = false;
    }
    change
}

#[cfg(test)]
mod tests {
    #[test]
    fn rows_show_the_bound_keys() {
        use ta_config::OrderButton::Right;
        let mut keys = ta_config::Keys::default();
        assert_eq!(crate::keys::fill("{attack} / {fight}", &keys, Right), "A / Alt+F");
        keys.set("attack", "Q");
        assert_eq!(crate::keys::fill("{attack} / {fight} (x)", &keys, Right), "Q / Alt+F (x)");
        assert_eq!(crate::keys::fill("Esc", &keys, Right), "Esc");
    }

    /// TA's order keys (its GUI files' quick keys) and selections are the defaults; every
    /// default binding parses, and no two actions share one.
    #[test]
    fn defaults_are_tas_keys() {
        let keys = ta_config::Keys::default();
        for (action, key) in [
            ("move", "M"),
            ("attack", "A"),
            ("repair", "R"),
            ("load", "L"),
            ("fire_orders", "F"),
            ("move_orders", "V"),
            ("on_off", "X"),
            ("select_builders", "Ctrl+B"),
            ("select_factories", "Ctrl+F"),
            ("select_armed", "Ctrl+W"),
            ("select_on_screen", "Ctrl+S"),
        ] {
            assert_eq!(keys.get(action), key, "{action}");
        }
        for &(action, _, default) in ta_config::Keys::ACTIONS {
            assert!(crate::keys::Binding::parse(default).is_some(), "{action}: {default}");
            assert!(keys.conflicts(action).is_empty(), "{action} shares {default} with {:?}", keys.conflicts(action));
        }
    }

    #[test]
    fn lists_end_between_rows() {
        let rows = |spans: &[(f32, f32)]| super::Rows { spans: spans.to_vec(), ..Default::default() };
        // Rows 30 high, 10 apart.
        let one = rows(&[(0.0, 30.0), (40.0, 70.0), (80.0, 110.0)]);
        assert_eq!(one.cut(75.0), 75.0, "already between two rows");
        assert_eq!(one.cut(90.0), 75.0, "midway across the gap over the row it would cut through");
        assert_eq!(one.cut(200.0), 200.0, "past the last row");
        assert_eq!(one.cut(20.0), 20.0, "a first row taller than the room");
        // Rows over one another are one; rows meeting still part.
        let overlapping = rows(&[(0.0, 10.0), (20.0, 40.0), (30.0, 50.0), (60.0, 90.0)]);
        assert_eq!(overlapping.cut(45.0), 15.0);
        assert_eq!(overlapping.cut(70.0), 55.0);
        assert_eq!(rows(&[(0.0, 20.0), (20.0, 40.0)]).cut(30.0), 20.0);
    }

    #[test]
    fn rows_list_their_keys_apart() {
        assert_eq!(super::keys_of("Click / drag"), vec!["Click", "drag"]);
        assert_eq!(super::keys_of("Up/Left/Down/Right, screen edges"), vec!["Up/Left/Down/Right", "screen edges"]);
        assert_eq!(super::keys_of("F3 / Backspace, x"), vec!["F3", "Backspace", "x"]);
        assert_eq!(super::keys_of("Home"), vec!["Home"]);
    }

    #[test]
    fn a_search_keeps_the_rows_it_finds() {
        let keys = ta_config::Keys::default();
        let all = super::sections(&keys, &ta_config::Gameplay::default());
        let zoom = super::found(all.clone(), "zoom");
        assert!(zoom.iter().any(|(title, _)| *title == "CAMERA"), "{zoom:?}");
        for (_, rows) in &zoom {
            assert!(rows.iter().all(|(keys, what)| keys.to_lowercase().contains("zoom") || what.to_lowercase().contains("zoom")), "{rows:?}");
        }
        // A key as bound finds its row; a section's title, all its rows.
        assert!(super::found(all.clone(), "ctrl+b").iter().flat_map(|(_, rows)| rows).any(|(keys, _)| keys.contains("Ctrl+B")));
        let pad = super::found(all.clone(), "gamepad");
        assert_eq!(pad.iter().find(|(title, _)| *title == "GAMEPAD").map(|(_, rows)| rows.len()), Some(crate::gamepad::LAYOUT.len()));
        assert!(super::found(all.clone(), "no such words").is_empty());
        assert_eq!(super::found(all.clone(), "").len(), all.len());
    }

    #[test]
    fn columns_share_the_rows() {
        use super::Entry::{Heading, Row};
        let part = |title, rows: usize| (title, (0..rows).map(|i| (format!("K{i}"), "x")).collect::<Vec<_>>());
        // One section: its heading again over the second column.
        let one = [part("A", 8)];
        let [first, second] = super::two_columns(&one);
        assert!(matches!(first[0], Heading("A")) && matches!(second[0], Heading("A")), "{first:?} {second:?}");
        assert_eq!(first.len() + second.len(), 10);
        // A row alone stays in the first.
        let alone = [part("A", 1)];
        let [first, second] = super::two_columns(&alone);
        assert_eq!((first.len(), second.len()), (2, 0));
        // Never a heading last in the first column; every row once.
        let two = [part("A", 3), part("B", 3)];
        let [first, second] = super::two_columns(&two);
        assert!(matches!(first.last(), Some(Row(..))), "{first:?}");
        assert_eq!(first.iter().chain(&second).filter(|e| matches!(e, Row(..))).count(), 6);
    }

    #[test]
    fn mouse_rows_follow_the_order_button() {
        use ta_config::OrderButton::{Left, Right};
        let keys = ta_config::Keys::default();
        assert_eq!(crate::keys::fill("{order_click}", &keys, Right), "Right-click");
        assert_eq!(crate::keys::fill("Ctrl+{order_drag}", &keys, Left), "Ctrl+Drag");
        assert_eq!(crate::keys::fill("{deselect}", &keys, Left), "Esc / Right-click");
    }

    /// The radial menu's rows say how the player's setting opens it: a right-click (or a hold)
    /// and a middle click; holding the button when it is kept to a hold; nothing with it off.
    #[test]
    fn the_radial_menus_rows_follow_its_setting() {
        use ta_config::RadialOpens::{ClickOrHold, Hold};
        let keys = ta_config::Keys::default();
        let mouse = |radial_menu, radial_opens| {
            let controls = ta_config::Gameplay { radial_menu, radial_opens, ..Default::default() };
            let all = super::sections(&keys, &controls);
            all.into_iter().find(|(title, _)| *title == "ORDERS").unwrap().1.into_iter().map(|(keys, _)| keys).filter(|k| k.ends_with("on your unit")).collect::<Vec<_>>()
        };
        assert_eq!(mouse(true, ClickOrHold), ["Right-click on your unit", "Middle-click on your unit"]);
        assert_eq!(mouse(true, Hold), ["Hold the right button on your unit", "Middle-click on your unit"]);
        assert!(mouse(false, ClickOrHold).is_empty() && mouse(false, Hold).is_empty());
    }
}
