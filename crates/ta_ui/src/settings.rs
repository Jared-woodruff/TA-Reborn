//! The settings screen the game's options and the launcher share: the categories down the
//! side under a search over every setting, and each setting on a row of its own (its name
//! and what it does on the left, the control on the right). Changes are made to the
//! [`Config`] at once; the host applies what [`SettingsView::show`] says changed and saves.

use crate::tooltip::TooltipExt as _;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use egui::emath::TSTransform;
use egui::{
    Color32, CornerRadius, DragValue, Frame, Id, Key, Label, Margin, Modifiers, RichText, ScrollArea, Sense, Shape,
    Slider, Stroke, TextEdit, Ui, vec2,
};
use ta_config::{
    ArtStyle, Audio, CameraStyle, Config, Gameplay, Graphics, HealthBars, Keys, Launcher, MenuBackdrop, OnPlay, OrderButton, Preset, Quality, RadialOpens,
    Reflections, SeasonalTheme, TimeOfDay, UserMusic, WindowMode, limits,
};

use crate::i18n::{n, t, tf};
use crate::metal;
use crate::theme::*;
use crate::widgets::{self, ActionButton, NavIcon, caption};

/// The settings' categories, in the order they are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Category {
    #[default]
    Gameplay,
    Controls,
    Interface,
    Accessibility,
    Graphics,
    Display,
    Sound,
    /// The launcher's own (shown in the launcher only).
    Launcher,
}

impl Category {
    pub const ALL: [Category; 8] =
        [Category::Gameplay, Category::Controls, Category::Interface, Category::Accessibility, Category::Graphics, Category::Display, Category::Sound, Category::Launcher];

    /// The categories shown: the launcher's own only in the launcher.
    pub fn shown(in_launcher: bool) -> impl Iterator<Item = Category> {
        Self::ALL.into_iter().filter(move |c| in_launcher || *c != Category::Launcher)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Gameplay => n("GAMEPLAY"),
            Category::Controls => n("CONTROLS"),
            Category::Interface => n("INTERFACE"),
            Category::Accessibility => n("ACCESSIBILITY"),
            Category::Graphics => n("GRAPHICS"),
            Category::Display => n("DISPLAY"),
            Category::Sound => n("SOUND"),
            Category::Launcher => n("LAUNCHER"),
        }
    }

    /// What is in it, under its title.
    fn blurb(self) -> &'static str {
        match self {
            Category::Gameplay => n("Your name, new games' unit limit and speed, saving"),
            Category::Controls => n("Mouse buttons, scrolling, the gamepad and every key"),
            Category::Interface => n("Language, health bars, icons, the classic interface and the seasons"),
            Category::Accessibility => n("Readability, motion, flashes, cursors and message timing"),
            Category::Graphics => n("Detail, antialiasing, effects, lighting and the camera"),
            Category::Display => n("Resolution, window and frame rate"),
            Category::Sound => n("Volumes, units' replies, and how many sounds play at once"),
            Category::Launcher => n("When the game starts, the launcher's music and sounds, updates and its pointers"),
        }
    }

    fn icon(self) -> NavIcon {
        match self {
            Category::Gameplay => NavIcon::Gameplay,
            Category::Controls => NavIcon::Controls,
            Category::Interface => NavIcon::Interface,
            Category::Accessibility => NavIcon::Accessibility,
            Category::Graphics => NavIcon::Graphics,
            Category::Display => NavIcon::Display,
            Category::Sound => NavIcon::Sound,
            Category::Launcher => NavIcon::Play,
        }
    }
}

/// Where the settings are shown, and what they need from there.
pub struct Host<'a> {
    /// In the game changes take effect at once (antialiasing when it next starts); in the
    /// launcher, when the game next starts.
    pub in_game: bool,
    /// In the game: the antialiasing it started with.
    pub msaa_running: u32,
    /// In the game: whether it traces rays (the device was opened for it).
    pub ray_tracing_running: bool,
    /// The monitor's resolution, when known (offered among the resolutions).
    pub native: Option<(u32, u32)>,
    /// Interface languages, (code, name), and the folders their files are in.
    pub languages: &'a [(String, String)],
    pub language_dirs: &'a [PathBuf],
    /// The height the list of settings scrolls in; None: as tall as it needs to be.
    pub height: Option<f32>,
}

/// What changed, for the host to apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Volumes and voices.
    pub audio: bool,
    /// Graphics options (effects, shadows, lighting...).
    pub graphics: bool,
    /// The window: its size and mode, vertical sync, the frame rate limit.
    pub display: bool,
    /// Modern or classic camera.
    pub camera: bool,
}

/// Width of the categories' column.
const SIDEBAR: f32 = 204.0;
/// Below this width categories become a selector above the settings. Hosts then scroll the
/// complete page, so increasing the UI scale never leaves a fixed sidebar over the controls.
pub fn compact(width: f32) -> bool {
    width < 760.0
}
/// How long the list takes to come in after the category or the search changes.
const SWITCH_TIME: f32 = 0.2;

/// The settings screen's own state: the category shown, the search, a key being changed.
#[derive(Default)]
pub struct SettingsView {
    category: Category,
    search: String,
    /// The action whose new key is awaited.
    capturing: Option<&'static str>,
    /// Editing a resolution of one's own.
    custom_resolution: bool,
    /// "Restore defaults" clicked once, for this category.
    confirm_reset: Option<Category>,
    /// When the list last changed (the category or the search), for its entrance.
    switched_at: f64,
    /// The search box had the keyboard last frame (Escape then empties it).
    search_focused: bool,
    /// Shown in the launcher (its own category with the rest).
    in_launcher: bool,
    /// RESTORE DEFAULT KEYS was clicked once (a second click confirms).
    confirm_keys: bool,
}

impl SettingsView {
    /// The category shown.
    pub fn category(&self) -> Category {
        self.category
    }

    /// Shows a category (e.g. Controls from the controls reference), without a search.
    pub fn open(&mut self, category: Category) {
        self.category = category;
        self.search.clear();
        self.capturing = None;
        self.confirm_reset = None;
    }

    /// Searches for `text` (captures).
    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_string();
    }

    /// A key is awaited for an action: the keyboard is the settings' until it comes.
    pub fn capturing(&self) -> bool {
        self.capturing.is_some()
    }

    /// Awaits a key for `action` (tests).
    pub fn capture(&mut self, action: &'static str) {
        self.capturing = Some(action);
    }

    /// Draws the screen; returns what changed.
    pub fn show(&mut self, ui: &mut Ui, config: &mut Config, host: &Host) -> Changes {
        let before = config.clone();
        self.in_launcher = !host.in_game;
        if host.in_game && self.category == Category::Launcher {
            self.category = Category::default();
        }
        self.keyboard(ui, &mut config.keys);
        let gap = ui.spacing().item_spacing.x;
        if compact(ui.available_width()) {
            self.compact_navigation(ui);
            ui.add_space(space::L);
            self.list(ui, config, host);
            if !self.searching() {
                ui.add_space(space::L);
                self.restore_controls(ui, config, ui.available_width());
            }
        } else {
            let row = ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                self.sidebar(ui, config, host.height);
                ui.add_space(space::XXL);
                ui.vertical(|ui| {
                    // (The controls in a row keep their gaps.)
                    ui.spacing_mut().item_spacing.x = gap;
                    self.list(ui, config, host)
                });
            });
            // A seam between the categories and the list.
            let rect = row.response.rect;
            let x = rect.min.x + SIDEBAR + space::XXL / 2.0;
            ui.painter().vline(x - 0.5, rect.y_range(), Stroke::new(1.0, Color32::from_black_alpha(150)));
            ui.painter().vline(x + 0.5, rect.y_range(), Stroke::new(1.0, Color32::from_white_alpha(14)));
        }
        let (g0, g1) = (&before.graphics, &config.graphics);
        Changes {
            audio: before.audio != config.audio,
            graphics: g0 != g1,
            display: (g0.width, g0.height, g0.window_mode, g0.vsync, g0.fps_limit) != (g1.width, g1.height, g1.window_mode, g1.vsync, g1.fps_limit),
            camera: g0.camera != g1.camera,
        }
    }

    /// Keys before anything is drawn: the key awaited for an action (Escape cancels), Escape
    /// in the search box (it empties the search and does no more), Ctrl+F (search).
    fn keyboard(&mut self, ui: &mut Ui, keys: &mut Keys) {
        let ctx = ui.ctx().clone();
        if !ui.is_enabled() || !ctx.memory(|memory| memory.is_above_modal_layer(ui.layer_id())) { return; }
        if let Some(action) = self.capturing {
            let pressed = ctx.input_mut(|i| {
                let at = i.events.iter().position(|e| matches!(e, egui::Event::Key { pressed: true, .. }))?;
                match i.events.remove(at) {
                    egui::Event::Key { key, modifiers, .. } => Some((key, modifiers)),
                    _ => None,
                }
            });
            if let Some((key, m)) = pressed {
                if key != Key::Escape {
                    let mut text = String::new();
                    if m.ctrl || m.command {
                        text.push_str("Ctrl+");
                    }
                    if m.alt {
                        text.push_str("Alt+");
                    }
                    if m.shift {
                        text.push_str("Shift+");
                    }
                    text.push_str(key.name());
                    keys.set(action, &text);
                }
                self.capturing = None;
            }
        }
        if self.search_focused && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.search.clear();
            self.switched(&ctx);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            ctx.memory_mut(|m| m.request_focus(search_id()));
        }
    }

    fn switched(&mut self, ctx: &egui::Context) {
        self.switched_at = ctx.input(|i| i.time);
        self.confirm_reset = None;
    }

    fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    fn compact_navigation(&mut self, ui: &mut Ui) {
        let search = widgets::search_field(ui, &mut self.search, t("Search settings  (Ctrl+F)"), ui.available_width(), Some(search_id()));
        if search.changed() {
            self.switched(ui.ctx());
        }
        self.search_focused = search.has_focus();
        let mut category = self.category;
        egui::ComboBox::from_id_salt("ta_ui::settings::category")
            .selected_text(t(category.label()))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for choice in Category::shown(self.in_launcher) {
                    if ui.selectable_value(&mut category, choice, t(choice.label())).clicked() {
                        self.open(choice);
                        self.switched(ui.ctx());
                    }
                }
            });
    }

    /// The search box, the categories, and restoring the shown category's defaults; the
    /// categories closer together when the list is short (a small window), so that the column
    /// is no taller than the list beside it.
    fn sidebar(&mut self, ui: &mut Ui, config: &mut Config, list_height: Option<f32>) {
        // (Beside the list: its title and line of text, about 64 points, and the list.)
        let count = Category::shown(self.in_launcher).count() as f32;
        let item = list_height.map_or(widgets::NAV_ITEM_HEIGHT, |h| ((h + 64.0 - 34.0 - 10.0 - 18.0 - 30.0) / count - 4.0).clamp(30.0, widgets::NAV_ITEM_HEIGHT));
        ui.vertical(|ui| {
            ui.set_width(SIDEBAR);
            ui.spacing_mut().item_spacing.y = 4.0;
            let search = widgets::search_field(ui, &mut self.search, t("Search settings  (Ctrl+F)"), SIDEBAR, Some(search_id()));
            if search.changed() {
                self.switched(ui.ctx());
            }
            self.search_focused = search.has_focus();
            ui.add_space(10.0);
            let mut chosen = None;
            for category in Category::shown(self.in_launcher) {
                let selected = !self.searching() && self.category == category;
                let item = widgets::nav_item_sized(ui, category.icon(), category.label(), None, selected, item);
                if selected {
                    chosen = Some(item.rect);
                }
                if item.clicked() && !selected {
                    self.open(category);
                    self.switched(ui.ctx());
                }
            }
            if let Some(at) = chosen {
                widgets::nav_marker(ui, "categories", at);
            }
            if self.searching() {
                return;
            }
            ui.add_space(18.0);
            self.restore_controls(ui, config, SIDEBAR);
        });
    }

    fn restore_controls(&mut self, ui: &mut Ui, config: &mut Config, width: f32) {
        let category = self.category;
        if self.confirm_reset == Some(category) {
            let kept = match category {
                Category::Gameplay => t("Your name is kept."),
                Category::Interface => t("The language is kept."),
                _ => "",
            };
            ui.add(Label::new(RichText::new(tf("Restore every {category} setting to its default?", &[("category", &t(category.label()))])).font(body(12.5)).color(TEXT)).wrap());
            if !kept.is_empty() {
                ui.label(RichText::new(kept).font(body(12.0)).color(STEEL_DIM));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui.add(ActionButton::danger("RESTORE")).clicked() {
                    restore(category, config);
                    self.capturing = None;
                    self.confirm_reset = None;
                }
                if ui.add(ActionButton::secondary("KEEP")).clicked() {
                    self.confirm_reset = None;
                }
            });
        } else if ui.add(ActionButton::secondary("RESTORE DEFAULTS").size(width, 30.0)).hover_tip(t("This category's settings back to how they came")).clicked() {
            self.confirm_reset = Some(category);
        }
    }

    /// The shown category's title and its settings, or the search's results.
    fn list(&mut self, ui: &mut Ui, config: &mut Config, host: &Host) {
        let searching = self.searching();
        if searching {
            metal::embossed_label(ui, RichText::new(t("SEARCH")).font(display(24.0)).extra_letter_spacing(crate::i18n::tracking(1.4)), TEXT_BRIGHT);
            ui.label(RichText::new(tf("Settings about “{text}”", &[("text", &self.search.trim())])).font(body(13.0)).color(STEEL));
        } else if !compact(ui.available_width()) {
            metal::embossed_label(ui, RichText::new(t(self.category.label())).font(display(24.0)).extra_letter_spacing(crate::i18n::tracking(1.4)), TEXT_BRIGHT);
            ui.label(RichText::new(t(self.category.blurb())).font(body(13.0)).color(STEEL));
        }
        if searching || !compact(ui.available_width()) {
            ui.add_space(space::M);
        }
        // The list comes in (fades, rises) whenever it changes.
        let now = ui.input(|i| i.time);
        let t = if crate::motion::reduced(ui.ctx()) { 1.0 } else { (((now - self.switched_at) as f32) / SWITCH_TIME).clamp(0.0, 1.0) };
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        let t = egui::emath::easing::cubic_out(t);
        // (Each category keeps its own scroll position.)
        let list_id = ("ta_ui::settings::list", self.category, searching);
        let mut draw = |ui: &mut Ui| {
            ui.multiply_opacity(t);
            ui.with_visual_transform(TSTransform::from_translation(vec2(0.0, 10.0 * (1.0 - t))), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.rows(ui, config, host);
            });
        };
        match host.height {
            Some(height) => {
                let area = ScrollArea::vertical().id_salt(list_id).max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
                crate::scroll::show(ui, area, crate::scroll::Behind::Plate, draw);
            }
            None => draw(ui),
        }
    }

    /// The rows of the shown category, or of every category matching the search.
    fn rows(&mut self, ui: &mut Ui, config: &mut Config, host: &Host) {
        let words: Vec<String> = self.search.to_lowercase().split_whitespace().map(str::to_owned).collect();
        // Every slider's value in a box of one width, wide enough for the widest value.
        let value_box = widgets::value_box_width(ui, &[t("Unlimited"), "480 fps", "20000", "100%", "0.00×"]).min(control_width(ui.available_width() - 24.0) * 0.4);
        let mut rows = Rows { words, pending: None, shown: 0, jump: None, value_box };
        let categories: Vec<Category> = if rows.searching() { Category::shown(!host.in_game).collect() } else { vec![self.category] };
        let graphics_before = config.graphics.clone();
        let mut level_picked = false;
        for category in categories {
            rows.begin(category);
            match category {
                Category::Gameplay => gameplay(ui, &mut rows, &mut config.gameplay),
                Category::Controls => self.controls(ui, &mut rows, &mut config.gameplay, &mut config.keys),
                Category::Interface => interface(ui, &mut rows, &mut config.gameplay, &mut config.launcher, host),
                Category::Accessibility => accessibility(ui, &mut rows, &mut config.gameplay),
                Category::Graphics => level_picked = graphics(ui, &mut rows, &mut config.graphics, host),
                Category::Display => self.display(ui, &mut rows, &mut config.graphics, host),
                Category::Sound => sound(ui, &mut rows, &mut config.audio),
                Category::Launcher => launcher(ui, &mut rows, &mut config.launcher, &mut config.audio, &mut config.updates),
            }
        }
        if rows.searching() && rows.shown == 0 {
            ui.add_space(24.0);
            ui.label(RichText::new(t("Nothing matches. Try another word, or pick a category.")).font(body(13.5)).color(STEEL));
        }
        if let Some(category) = rows.jump {
            self.open(category);
            self.switched(ui.ctx());
        }
        // An option of the graphics level set on its own makes the level Custom (or the one
        // the options now match).
        if !level_picked && config.graphics.level_options_differ(&graphics_before) {
            config.graphics.preset = config.graphics.matching_preset();
        }
    }

    fn controls(&mut self, ui: &mut Ui, rows: &mut Rows, p: &mut Gameplay, keys: &mut Keys) {
        let orders = match p.order_button {
            OrderButton::Right => n("The left button selects; the right one moves, attacks, repairs and more, by what is under the cursor"),
            OrderButton::Left => n("TA's original: the left button selects and gives orders; the right one clears the selection"),
        };
        setting_row(ui, rows, "GIVE ORDERS WITH", orders, "mouse button click swap left right move attack classic original", |ui| {
            widgets::segmented_fill(ui, &mut p.order_button, &[(OrderButton::Right, "RIGHT BUTTON"), (OrderButton::Left, "LEFT BUTTON")]);
        });
        toggle_row(ui, rows, "RADIAL MENU", "A ring of a unit's orders round the pointer, opened on one of your units by the right button (as set below), a middle click or a controller's B. Off: the buttons keep their other uses.", "right middle click hold unit context menu orders ring pie radial controller gamepad", &mut p.radial_menu);
        let opens = match (p.radial_opens, p.order_button) {
            (RadialOpens::ClickOrHold, OrderButton::Right) => n("A right-click on one of your units opens it where it would give no order: with nothing selected, or on a selected unit. Holding the button on the unit opens it at any time."),
            (RadialOpens::ClickOrHold, OrderButton::Left) => n("A right-click on one of your units opens it (elsewhere it clears the selection), and so does holding the button on the unit."),
            (RadialOpens::Hold, OrderButton::Right) => n("The right button opens it only when held on one of your units: a right-click always gives its order."),
            (RadialOpens::Hold, OrderButton::Left) => n("The right button opens it only when held on one of your units: a right-click always clears the selection, as in TA."),
        };
        setting_row(ui, rows, "RADIAL MENU OPENS WITH", opens, "radial ring pie context menu right click hold long press open", |ui| {
            ui.add_enabled_ui(p.radial_menu, |ui| {
                widgets::segmented_fill(ui, &mut p.radial_opens, &[(RadialOpens::ClickOrHold, n("CLICK OR HOLD")), (RadialOpens::Hold, n("HOLD ONLY"))]);
            });
        });
        let value_box = rows.value_box;
        setting_row(ui, rows, "SCROLL SPEED", "How fast the arrow keys and the screen edges move the view", "camera pan move screen", |ui| {
            widgets::value_slider(ui, Slider::new(&mut p.scroll_speed, limits::SCROLL_SPEED).step_by(0.05).custom_formatter(|v, _| format!("{v:.2}×")).custom_parser(times), value_box);
        });
        toggle_row(ui, rows, "EDGE SCROLLING", "Scroll the map when the cursor touches the edge of the screen", "camera pan mouse border", &mut p.edge_scroll);
        toggle_row(ui, rows, "GAMEPAD", "Play with a controller (the Steam Deck's included): sticks scroll and aim, A and B click. Read when the game starts.", "controller joystick xbox steam deck", &mut p.gamepad);

        rows.group(ui, n("KEYS"));
        if !rows.searching() {
            ui.label(RichText::new(t("Click a key, then press the new one (Esc cancels). Control groups (0-9) and camera positions (F6-F9) stay as they are.")).font(body(12.5)).color(STEEL_DIM));
            ui.add_space(6.0);
        }
        for &(id, what, _) in Keys::ACTIONS {
            let bound = keys.get(id).to_string();
            if !rows.show(ui, &[what, &bound, "key shortcut hotkey"]) {
                continue;
            }
            let capturing = self.capturing == Some(id);
            let clash = keys.conflicts(id);
            row_with(ui, RichText::new(t(what)).font(body(13.0)).color(TEXT), "", 5, |ui| {
                ui.horizontal(|ui| {
                    let button = widgets::key_binding(ui, &bound, capturing, 150.0);
                    if button.clicked() {
                        self.capturing = if capturing { None } else { Some(id) };
                        // Space or Enter must not "click" the button again while it has focus.
                        button.surrender_focus();
                    }
                    if !clash.is_empty() {
                        let others = clash.iter().map(|c| t(c)).collect::<Vec<_>>().join(", ");
                        ui.label(RichText::new(tf("also: {actions}", &[("actions", &others)])).font(body(12.0)).color(AMBER));
                    }
                });
            });
        }
        if !rows.searching() && !keys.bindings.is_empty() {
            ui.add_space(10.0);
            let armed = self.confirm_keys;
            let label = if armed { n("REALLY RESTORE DEFAULT KEYS?") } else { n("RESTORE DEFAULT KEYS") };
            let button = if armed { ActionButton::danger(label) } else { ActionButton::secondary(label) };
            if ui.add(button).on_hover_text(t("Every key you changed goes back to TA's own")).clicked() {
                if armed {
                    keys.bindings.clear();
                    self.capturing = None;
                }
                self.confirm_keys = !armed;
            }
        }
    }

    fn display(&mut self, ui: &mut Ui, rows: &mut Rows, g: &mut Graphics, host: &Host) {
        let hint = if host.in_game { n("The window's size (full screen uses the screen's own)") } else { "" };
        setting_row(ui, rows, "RESOLUTION", hint, "size window pixels width height 1080p 1440p 4k", |ui| {
            resolution(ui, g, &mut self.custom_resolution, host.native);
        });
        setting_row(ui, rows, "WINDOW MODE", "", "fullscreen full screen borderless windowed", |ui| {
            widgets::segmented_fill(ui, &mut g.window_mode, &[(WindowMode::Windowed, "WINDOWED"), (WindowMode::Borderless, "BORDERLESS"), (WindowMode::Fullscreen, "FULLSCREEN")]);
        });
        toggle_row(ui, rows, "VERTICAL SYNC", "Smooth, tear-free frames at the display's refresh rate", "vsync tearing fps refresh", &mut g.vsync);
        let limit = if g.vsync { n("Only used while vertical sync is off") } else { n("Fewer frames keep a laptop cooler and quieter") };
        let value_box = rows.value_box;
        setting_row(ui, rows, "FRAME RATE LIMIT", limit, "fps cap frames per second", |ui| {
            // Unlimited is the notch past the highest cap, at the right: more to the right is
            // more frames. (0, unlimited, as the settings keep it.)
            const STEP: f64 = 10.0;
            let top = *limits::FPS_LIMIT.end() as f64;
            let unlimited = top + STEP;
            let fps = &mut g.fps_limit;
            let slider = Slider::from_get_set(STEP..=unlimited, move |v| {
                if let Some(v) = v {
                    *fps = if v > top { 0 } else { v as u32 };
                }
                if *fps == 0 { unlimited } else { *fps as f64 }
            })
            .step_by(STEP)
            .custom_formatter(move |v, _| if v > top { t("Unlimited").to_string() } else { format!("{v:.0} fps") })
            .custom_parser(move |text| {
                let text = text.trim().to_ascii_lowercase();
                if text.starts_with("unl") || text == "0" { Some(unlimited) } else { text.trim_end_matches("fps").trim().parse().ok() }
            });
            ui.add_enabled_ui(!g.vsync, |ui| widgets::value_slider(ui, slider, value_box));
        });
        toggle_row(ui, rows, "FRAME RATE COUNTER", "Frames a second, and how long each frame takes, in the corner of the battlefield", "fps counter performance frames per second overlay show", &mut g.show_frame_rate);
        toggle_row(ui, rows, "SAVE POWER IN THE BACKGROUND", "With another window in front, the game draws 30 frames a second", "background power battery unfocused alt-tab fps frame rate", &mut g.save_power_in_background);
    }
}

fn search_id() -> Id {
    Id::new("ta_ui::settings::search")
}

/// The rows being drawn: every row of the category, or with a search those that match it,
/// under the heading of their category.
struct Rows {
    /// The search, in lower-case words; empty: no search.
    words: Vec<String>,
    /// The width of every slider's value box.
    value_box: f32,
    /// Searching: the category whose heading is due before its first row that matches.
    pending: Option<Category>,
    /// Rows shown.
    shown: usize,
    /// A category heading clicked: open that category.
    jump: Option<Category>,
}

impl Rows {
    fn searching(&self) -> bool {
        !self.words.is_empty()
    }

    fn begin(&mut self, category: Category) {
        if self.searching() {
            self.pending = Some(category);
        }
    }

    /// Whether a row with these texts (English, looked up translated too) shows; the first
    /// found in a category brings its heading.
    fn show(&mut self, ui: &mut Ui, texts: &[&str]) -> bool {
        if self.searching() {
            let hay = texts.iter().flat_map(|s| [s.to_lowercase(), t(s).to_lowercase()]).collect::<Vec<_>>().join(" ");
            if !self.words.iter().all(|w| hay.contains(w.as_str())) {
                return false;
            }
            if let Some(category) = self.pending.take()
                && search_heading(ui, category, self.shown == 0)
            {
                self.jump = Some(category);
            }
        }
        self.shown += 1;
        true
    }

    /// A heading within a category (not while searching).
    fn group(&self, ui: &mut Ui, title: &str) {
        if !self.searching() {
            ui.add_space(18.0);
            widgets::card_title(ui, title);
        }
    }
}

/// A category's heading among search results: clicking it opens the category.
fn search_heading(ui: &mut Ui, category: Category, first: bool) -> bool {
    if !first {
        ui.add_space(14.0);
    }
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let (mark, _) = ui.allocate_exact_size(vec2(8.0, 15.0), Sense::hover());
            ui.painter().add(metal::rivet(mark.center(), 3.2));
            let label = ui.add(Label::new(RichText::new(format!("{}  ›", t(category.label()))).font(strong(13.5)).color(AMBER_BRIGHT).extra_letter_spacing(crate::i18n::tracking(1.6))).sense(Sense::click()));
            label.hover_tip(t("Show everything in this category")).on_hover_cursor(egui::CursorIcon::PointingHand)
        })
        .inner;
    ui.add_space(4.0);
    // (Never the last thing in sight: kept with the top of the row that follows.)
    crate::scroll::keep_together(ui, egui::Rect::from_x_y_ranges(response.rect.x_range(), response.rect.min.y..=ui.cursor().min.y + 12.0));
    response.clicked()
}

/// A setting: its name and what it does on the left, the control on the right. Nothing is
/// drawn when a search leaves it out; `keywords` are other words it is found by.
fn setting_row(ui: &mut Ui, rows: &mut Rows, label: &str, hint: &str, keywords: &str, add: impl FnOnce(&mut Ui)) {
    if rows.show(ui, &[label, hint, keywords]) {
        row_with(ui, caption(label), hint, 9, add);
    }
}

/// A setting that is on or off.
fn toggle_row(ui: &mut Ui, rows: &mut Rows, label: &str, hint: &str, keywords: &str, value: &mut bool) {
    setting_row(ui, rows, label, hint, keywords, |ui| toggle_control(ui, value, label));
}

/// The switch of a [`toggle_row`], and its ON or OFF (dimmed with the rest of a disabled row).
fn toggle_control(ui: &mut Ui, value: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        widgets::toggle_named(ui, value, label);
        let colour = if !ui.is_enabled() { STEEL_DIM } else if *value { GREEN } else { STEEL };
        ui.label(RichText::new(t(if *value { "ON" } else { "OFF" })).font(strong(12.0)).color(colour));
    });
}

/// Width of a settings row's control column in a list `width` wide: enough for a slider and
/// its value, and for a row of five choices in equal segments; the rest for the name and what
/// it does.
pub fn control_width(width: f32) -> f32 {
    (width * 0.48).clamp(210.0, 400.0)
}

/// Below this width a row's control goes under its name instead of beside it (the name and
/// what it does would otherwise wrap into a narrow column).
const SIDE_BY_SIDE: f32 = 460.0;

/// A row: `label` and `hint` on the left, the control on the right (or below them, when
/// narrow); it lights up under the pointer and a seam runs under it.
fn row_with(ui: &mut Ui, label: RichText, hint: &str, pad: i8, add: impl FnOnce(&mut Ui)) {
    let background = ui.painter().add(Shape::Noop);
    let width = ui.available_width();
    let inside = width - 24.0;
    let control = control_width(inside);
    let describe = |ui: &mut Ui, label: RichText| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(label);
        if !hint.is_empty() {
            widgets::pretty_label(ui, RichText::new(t(hint)).font(body(12.5)).color(STEEL_DIM));
        }
    };
    let inner = Frame::new().inner_margin(Margin::symmetric(12, pad)).show(ui, |ui| {
        ui.set_width(inside);
        if inside < SIDE_BY_SIDE {
            ui.vertical(|ui| {
                describe(ui, label);
                ui.add_space(space::S);
                ui.spacing_mut().item_spacing.y = 4.0;
                let control = inside.min(330.0);
                ui.scope(|ui| {
                    ui.set_max_width(control);
                    add(ui);
                });
            });
            return;
        }
        // The name and what it does centred on the control, or the control on them, whichever
        // is shorter (by the heights they took last frame: the first is hidden by the list's
        // fade).
        let id = ui.id().with(("ta_ui::settings::row", label.text().to_owned()));
        let (text_height, control_height) = ui.data(|d| d.get_temp::<(f32, f32)>(id)).unwrap_or((widgets::CONTROL_HEIGHT, widgets::CONTROL_HEIGHT));
        let tall = text_height.max(control_height);
        let mut measured = (text_height, control_height);
        ui.horizontal_top(|ui| {
            // (No gaps between the columns but the one put in; the control's own keep theirs.)
            let gap = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 0.0;
            let text = (ui.available_width() - control - space::L).max(120.0);
            ui.vertical(|ui| {
                ui.set_width(text);
                let pad = ((tall - text_height) / 2.0).floor();
                ui.add_space(pad);
                let top = ui.cursor().min.y;
                describe(ui, label);
                measured.0 = ui.min_rect().max.y - top;
            });
            ui.add_space(space::L);
            ui.vertical(|ui| {
                ui.set_width(control);
                ui.spacing_mut().item_spacing = vec2(gap, 4.0);
                let pad = ((tall - control_height) / 2.0).floor();
                ui.add_space(pad);
                let top = ui.cursor().min.y;
                add(ui);
                measured.1 = ui.min_rect().max.y - top;
            });
        });
        if (measured.0 - text_height).abs() > 0.5 || (measured.1 - control_height).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(id, measured));
            ui.ctx().request_repaint();
        }
    });
    let rect = inner.response.rect;
    crate::scroll::keep_together(ui, rect);
    let painter = ui.painter();
    if ui.rect_contains_pointer(rect) {
        painter.set(background, Shape::rect_filled(rect, CornerRadius::same(2), Color32::from_rgba_unmultiplied(255, 236, 190, 7)));
    }
    painter.add(metal::seam(egui::Rangef::new(rect.min.x + 8.0, rect.max.x - 8.0), rect.max.y - 1.0));
}

/// A volume or similar: a slider in percent.
fn percent(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>, value_box: f32) {
    let slider = Slider::new(value, range)
        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
        .custom_parser(|text| text.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0));
    widgets::value_slider(ui, slider, value_box);
}

/// Reads "1.5x" (or "1.5×", or "1.5") as 1.5.
fn times(text: &str) -> Option<f64> {
    text.trim().trim_end_matches(['x', 'X', '×']).trim().parse().ok()
}

/// Where the settings keep why the last picture chosen could not be used.
fn picture_error_id() -> Id {
    Id::new("ta_ui::settings::picture_error")
}

fn gameplay(ui: &mut Ui, rows: &mut Rows, p: &mut Gameplay) {
    setting_row(ui, rows, "COMMANDER NAME", "Your name in multiplayer games", "player name multiplayer nickname", |ui| {
        ui.add(TextEdit::singleline(&mut p.player_name).char_limit(limits::PLAYER_NAME_CHARS).hint_text(t("Commander")).margin(Margin::symmetric(10, 6)).min_size(vec2(0.0, widgets::CONTROL_HEIGHT)).vertical_align(egui::Align::Center).desired_width(ui.available_width()));
    });
    setting_row(ui, rows, "YOUR PICTURE", "Beside your lines on the chat and in the players panel (the computer players show their side's Commander)", "portrait avatar profile picture image photo", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::M;
            // The picture in a slot as tall as the buttons beside it; none yet: a figure's
            // outline in its place.
            let picture = crate::portrait::texture(ui.ctx());
            let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), egui::Sense::hover());
            ui.painter().add(metal::slab(ui.ctx(), rect, metal::Slab::Slot, 0.0, false));
            let inside = rect.shrink(2.0);
            match &picture {
                Some(tex) => {
                    ui.painter().image(tex.id(), inside, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    let painter = ui.painter().with_clip_rect(inside);
                    let c = inside.center();
                    painter.circle_filled(c - vec2(0.0, 4.0), 5.0, STEEL_DIM);
                    painter.circle_filled(c + vec2(0.0, 14.0), 11.0, STEEL_DIM);
                }
            }
            if let Some(profile) = crate::profiles::current(ui.ctx()) {
                crate::profiles::badge(ui.painter(), rect, profile.allegiance);
            }
            if ui.add(widgets::ActionButton::secondary("CHOOSE…")).clicked()
                && let Some(file) = rfd::FileDialog::new().set_title(crate::i18n::t("Your picture")).add_filter(crate::i18n::t("Pictures"), &["png", "jpg", "jpeg"]).pick_file()
            {
                match crate::portrait::set_from(&file) {
                    Ok(()) => ui.ctx().data_mut(|d| d.remove::<String>(picture_error_id())),
                    Err(e) => {
                        log::warn!("picture {}: {e}", file.display());
                        ui.ctx().data_mut(|d| d.insert_temp(picture_error_id(), tf("That picture could not be used: {error}", &[("error", &e)])));
                    }
                }
            }
            if picture.is_some() && ui.add(widgets::ActionButton::secondary("CLEAR")).clicked() {
                crate::portrait::clear();
                ui.ctx().data_mut(|d| d.remove::<String>(picture_error_id()));
            }
        });
        // (Why the last picture chosen could not be used, until another is.)
        if let Some(error) = ui.ctx().data(|d| d.get_temp::<String>(picture_error_id())) {
            ui.add(Label::new(RichText::new(error).font(body(12.0)).color(RED)).wrap());
        }
    });
    let value_box = rows.value_box;
    setting_row(ui, rows, "UNIT LIMIT", "Units per player that new skirmishes start with. The 1997 original allowed 250.", "cap maximum units population", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.unit_limit, limits::UNIT_LIMIT).logarithmic(true).step_by(50.0), value_box);
    });
    setting_row(ui, rows, "GAME SPEED", "The speed single-player battles start at: 1.0× is TA's normal 30 simulation ticks a second", "fast slow tempo pace", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.speed, limits::SPEED).step_by(0.25).custom_formatter(|v, _| format!("{v:.2}×")).custom_parser(times), value_box);
    });
    toggle_row(ui, rows, "AUTOSAVE", "Single-player games save themselves every five minutes", "save load continue", &mut p.autosave);
    toggle_row(ui, rows, "COMPUTER PLAYERS TALK", "In single-player battles the enemy's computer players greet you, answer your messages and gloat or grumble as the battle turns", "chat chatter ai messages taunts", &mut p.ai_chatter);
    toggle_row(ui, rows, "BATTLE TIPS", "A tip at the start of each battle: a key, a trick or a rule of thumb", "tips hints start loading advice help", &mut p.start_tips);
    setting_row(ui, rows, "COACH", "A word of advice in battle when a store runs dry, a factory waits or your Commander is in danger", "advice help hints tips beginner new", |ui| {
        widgets::segmented_fill(ui, &mut p.coach, &[(ta_config::Coach::FirstBattles, "FIRST BATTLES"), (ta_config::Coach::Always, "ALWAYS"), (ta_config::Coach::Off, "OFF")]);
    });
    setting_row(ui, rows, "HIDDEN ADVICE", "Advice from the coach that you chose not to see again", "coach advice hints hidden reset restore", |ui| {
        let hidden = p.coach_muted.len();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if ui.add_enabled(hidden > 0, widgets::ActionButton::secondary("SHOW AGAIN")).clicked() {
                p.coach_muted.clear();
            }
            ui.label(RichText::new(hidden.to_string()).font(strong(12.0)).color(if hidden > 0 { TEXT_BRIGHT } else { STEEL }));
        });
    });
}

fn interface(ui: &mut Ui, rows: &mut Rows, p: &mut Gameplay, launcher: &mut Launcher, host: &Host) {
    if host.languages.len() > 1 {
        setting_row(ui, rows, "LANGUAGE", "The game's and the launcher's menus and messages", "translation locale", |ui| {
            if crate::i18n::picker(ui, "language", &mut p.language, host.languages)
                && let Err(e) = crate::i18n::set_language(&p.language, host.language_dirs)
            {
                log::warn!("{e}");
            }
        });
    }
    setting_row(ui, rows, "HEALTH BARS", "Selected and hovered units always show theirs", "hp damage life", |ui| {
        widgets::segmented_fill(ui, &mut p.health_bars, &[(HealthBars::Auto, "AUTO"), (HealthBars::Damaged, "DAMAGED"), (HealthBars::Selected, "SELECTED")]);
    });
    toggle_row(ui, rows, "STRATEGIC ICONS", "Far out, where units are a few pixels across, each one shows as an icon in its owner's colour, its shape saying what it is. Off: the units themselves at every height.", "zoom out icons strategic overview symbols far units", &mut p.strategic_icons);
    toggle_row(ui, rows, "CLASSIC INTERFACE", "TA's own 1997 side panel, resource bar and build menus in battle", "hud old original panel", &mut p.classic_interface);
    setting_row(ui, rows, "ORIGINAL PIXELS", "The classic interface's art and lettering in TA's own square pixels, enlarged as they are. Off: smoothed for your screen, sharp at any size", "classic interface hud pixels pixelated pixel art sharp smooth blurry retro scaling filter", |ui| {
        let classic = p.classic_interface;
        ui.add_enabled_ui(classic, |ui| toggle_control(ui, &mut p.classic_pixels, "ORIGINAL PIXELS"));
    });
    toggle_row(ui, rows, "COMPANION SCREEN", "Every battle opens the companion screen on your second display: the map and intel beside the battle. Its key (Alt+C) opens and closes it at any time.", "second monitor display screen map intel window dual", &mut p.companion);
    setting_row(ui, rows, "MENU BACKDROP", "Behind the game's title menu: a film of short scenes staged on TA Reborn's worlds (the season's at Halloween, Christmas and the New Year), or a battle between computer players as it plays", "title main menu background scene film cinematic attract battle", |ui| {
        widgets::segmented_fill(ui, &mut p.menu_backdrop, &[(MenuBackdrop::Film, n("FILM")), (MenuBackdrop::Battle, n("BATTLE"))]);
    });
    // (The seasons are the launcher's settings, kept in its section, but they deck the game's
    // loading screens and title menu too: both show them here.)
    toggle_row(ui, rows, "SEASONAL EFFECTS", "Celebrate the season with festive launcher and loading screen decorations, the season's own field notes while loading, and its own scenes behind the game's title menu. Reduced motion keeps them still.", "launcher loading christmas new year halloween snow confetti balloons pumpkins bats spooky festive holiday animation tips notes menu film", &mut launcher.seasonal_effects);
    setting_row(ui, rows, "SEASONAL THEME", "Automatic celebrates Halloween from 24 October to 1 November, and Christmas from 1 December to 6 January, with New Year on 31 December and 1 January. Choose a theme to enjoy it at any time.", "launcher christmas new year halloween snow confetti pumpkins bats spooky festive holiday calendar automatic preview", |ui| {
        ui.add_enabled_ui(launcher.seasonal_effects, |ui| {
            widgets::segmented_fill(ui, &mut launcher.seasonal_theme, &[(SeasonalTheme::Automatic, n("AUTOMATIC")), (SeasonalTheme::Christmas, n("CHRISTMAS")), (SeasonalTheme::NewYear, n("NEW YEAR")), (SeasonalTheme::Halloween, n("HALLOWEEN"))]);
        });
    });
}

/// The launcher's own settings (its category is shown in the launcher only).
fn launcher(ui: &mut Ui, rows: &mut Rows, launcher: &mut Launcher, a: &mut Audio, updates: &mut ta_config::Updates) {
    let value_box = rows.value_box;
    setting_row(ui, rows, "WHEN THE GAME STARTS", "What the launcher does once you press Play (minimised, it comes back when the game ends)", "launcher minimise minimize close hide stay open taskbar", |ui| {
        widgets::segmented_fill(ui, &mut launcher.on_play, &[(OnPlay::Stay, n("STAY OPEN")), (OnPlay::Minimise, n("MINIMISE")), (OnPlay::Close, n("CLOSE"))]);
    });
    toggle_row(ui, rows, "LAUNCHER MUSIC", "TA's calmer music, quietly, while the launcher is open; it fades out while you play. The note at the top of the launcher turns it on and off", "launcher background music soundtrack quiet ambient", &mut a.launcher_music);
    setting_row(ui, rows, "LAUNCHER MUSIC VOLUME", "A share of the music volume", "launcher background music loudness quiet", |ui| percent(ui, &mut a.launcher_music_volume, limits::VOLUME, value_box));
    toggle_row(ui, rows, "LAUNCHER SOUNDS", "TA's own button sounds in the launcher, from your install", "launcher clicks ui sound effects", &mut a.launcher_sounds);
    toggle_row(ui, rows, "LOOK FOR UPDATES", "Ask for a newer version of TA Reborn each time the launcher starts (About has CHECK NOW)", "update version new release download automatic check", &mut updates.check);
    toggle_row(ui, rows, "THEMED LAUNCHER CURSORS", "Use Reborn cursors in the launcher. Turn off to use your system pointer.", "launcher mouse pointer cursor accessibility", &mut launcher.themed_cursor);
}

fn accessibility(ui: &mut Ui, rows: &mut Rows, p: &mut Gameplay) {
    let value_box = rows.value_box;
    setting_row(ui, rows, "INTERFACE SCALE", "The size of the menus and the battle's panels (for high-resolution screens)", "ui size zoom dpi text big small accessibility", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.ui_scale, limits::UI_SCALE).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)), value_box);
    });
    toggle_row(ui, rows, "COLOUR-BLIND COLOURS", "Player colours (units included), health bars and map marks in colours that stay apart with colour blindness", "color accessibility deuteranopia protanopia", &mut p.colour_blind);
    toggle_row(ui, rows, "PLAYER MARKERS", "Show player numbers and relation symbols on visible units and the minimap", "accessibility colour color team ally enemy identity", &mut p.team_markers);
    toggle_row(ui, rows, "KEYBOARD POINTER", "Use arrow keys to move the pointer, Enter to select and End to order. Alt+K toggles this mode by default.", "accessibility keyboard mouse pointer navigation", &mut p.keyboard_pointer);
    toggle_row(ui, rows, "SCREEN-READER ANNOUNCEMENTS", "Announce selection changes and battle notices when a screen reader is active", "accessibility screen reader speech alerts", &mut p.screen_reader_announcements);
    toggle_row(ui, rows, "SPOKEN TUTORIAL", "The tutorial reads its instructions and hints aloud, at the unit voices' volume", "accessibility tutorial voice speech narration read aloud", &mut p.tutorial_voice);
    setting_row(ui, rows, "CONTROLLER DEAD ZONE", "Ignore small stick movements; lower values need less movement", "accessibility gamepad controller stick deadzone", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.gamepad_deadzone, limits::GAMEPAD_DEADZONE).step_by(0.01).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)), value_box);
    });
    setting_row(ui, rows, "CONTROLLER POINTER SPEED", "Adjust how quickly the controller moves the pointer", "accessibility gamepad controller sensitivity speed", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.gamepad_sensitivity, limits::GAMEPAD_SENSITIVITY).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)), value_box);
    });
    toggle_row(ui, rows, "REDUCED MOTION", "Reduce menu animation, camera shake and cockpit movement", "accessibility motion shake recoil sway animation", &mut p.reduced_motion);
    // (Reduced motion leaves the shake out whatever this says: the row is dimmed then.)
    let reduced = p.reduced_motion;
    setting_row(ui, rows, "SCREEN SHAKE", "Big blasts shake the view (TA's +noshake stops it for one battle). Reduced motion leaves it out too.", "camera shake explosion blast noshake motion sickness", |ui| {
        ui.add_enabled_ui(!reduced, |ui| toggle_control(ui, &mut p.screen_shake, "SCREEN SHAKE"));
    });
    toggle_row(ui, rows, "REDUCED FLASHES", "Reduce bright combat effects and pulsing warnings. Some flashes remain.", "accessibility flashes brightness flicker effects", &mut p.reduced_flashes);
    toggle_row(ui, rows, "TA CURSORS", "The original animated cursors in battle (off: the system cursor)", "mouse pointer accessibility", &mut p.ta_cursors);
    setting_row(ui, rows, "CURSOR SIZE", "Enlarge the game and launcher cursors. The system cursor uses your Windows pointer settings.", "accessibility mouse pointer large size", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.cursor_scale, limits::CURSOR_SCALE).step_by(0.25).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)), value_box);
    });
    toggle_row(ui, rows, "HIGH-CONTRAST CURSOR", "Add a black-and-white marker around the game and launcher cursors", "accessibility mouse pointer visibility contrast", &mut p.high_contrast_cursor);
    setting_row(ui, rows, "MESSAGE DURATION", "Keep battle notices visible for longer. Pausing also pauses their timers.", "accessibility reading time alerts notices", |ui| {
        widgets::value_slider(ui, Slider::new(&mut p.message_duration, limits::MESSAGE_DURATION).step_by(0.5).custom_formatter(|v, _| format!("{v:.1}×")).custom_parser(times), value_box);
    });
}

/// Returns whether a graphics level was picked (it sets the options together).
fn graphics(ui: &mut Ui, rows: &mut Rows, g: &mut Graphics, host: &Host) -> bool {
    const QUALITY: [(Quality, &str); 4] = [(Quality::Low, n("LOW")), (Quality::Medium, n("MEDIUM")), (Quality::High, n("HIGH")), (Quality::Ultra, n("ULTRA"))];
    let mut picked = false;
    // How the battle is seen, first: it changes how the whole game looks and plays.
    let camera = match g.camera {
        CameraStyle::Modern => n("A free 3D camera: turn, tilt and zoom over the battle, with 3D trees, the sky and the time of day (F5 switches)"),
        CameraStyle::Classic => n("TA's own top-down view, as in 1997: the maps as they were painted, never turned or tilted; the wheel still zooms (F5 switches)"),
    };
    setting_row(ui, rows, "CAMERA", camera, "view perspective classic 3d oblique top down original 1997", |ui| {
        widgets::segmented_fill(ui, &mut g.camera, &[(CameraStyle::Modern, n("MODERN 3D")), (CameraStyle::Classic, n("CLASSIC TOP-DOWN"))]);
    });
    setting_row(ui, rows, "ART STYLE", "Reborn rebuilds TA's units with armour plates, bolts, road wheels and repainted metal, rounded edges catching the light; Enhanced Classic keeps TA's own models and paint, smoothed and with finer edges", "reborn enhanced classic art style models textures units buildings worlds palette", |ui| {
        widgets::segmented_fill(ui, &mut g.art_style, &[(ArtStyle::Reborn, n("REBORN")), (ArtStyle::Classic, n("ENHANCED CLASSIC"))]);
    });
    // Auto until the game has picked one for the graphics card, then the level the options
    // match (Custom when none does).
    let current = if g.preset == Preset::Auto { Preset::Auto } else { g.matching_preset() };
    let hint = match current {
        Preset::Auto => t("Picked for your graphics card when the game next starts").to_string(),
        _ if !g.gpu.is_empty() => tf("Graphics card: {gpu}. AUTO picks the level for it again.", &[("gpu", &g.gpu)]),
        _ => t("Low suits integrated graphics").to_string(),
    };
    setting_row(ui, rows, "GRAPHICS LEVEL", &hint, "quality preset detail performance fps low medium high ultra", |ui| {
        let mut options = vec![(Preset::Auto, n("AUTO"))];
        options.extend(Preset::LEVELS.iter().map(|&p| (p, p.label())));
        if current == Preset::Custom {
            options.push((Preset::Custom, n("CUSTOM")));
        }
        let mut choice = current;
        if widgets::segmented_fill(ui, &mut choice, &options).changed() && choice != current {
            match choice {
                // Picked again when the game next starts.
                Preset::Auto => g.preset = Preset::Auto,
                Preset::Custom => {}
                level => {
                    level.apply(g);
                    g.preset = level;
                }
            }
            picked = true;
        }
    });
    let msaa = if host.in_game && g.msaa != host.msaa_running { n("Changes when the game next starts") } else { n("Smooth edges; the largest the graphics card supports up to this") };
    setting_row(ui, rows, "ANTIALIASING", msaa, "msaa aa jaggies smooth edges", |ui| {
        widgets::segmented_fill(ui, &mut g.msaa, &[(1, "OFF"), (2, "2x"), (4, "4x"), (8, "8x")]);
    });
    let value_box = rows.value_box;
    setting_row(ui, rows, "RENDER SCALE", "The battlefield drawn at a lower resolution and scaled up; menus and panels stay sharp", "resolution performance fps upscale", |ui| {
        widgets::value_slider(ui, Slider::new(&mut g.render_scale, limits::RENDER_SCALE).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)), value_box);
    });
    setting_row(ui, rows, "SHADOWS", "Low turns them off", "shadow quality sun", |ui| {
        widgets::segmented_fill(ui, &mut g.shadows, &QUALITY);
    });
    setting_row(ui, rows, "EFFECTS", "Explosions, smoke, debris and particles in big battles, tread marks", "particles explosions debris smoke quality", |ui| {
        widgets::segmented_fill(ui, &mut g.effects, &QUALITY);
    });
    setting_row(ui, rows, "WATER REFLECTIONS", "Shores, units, ships and explosions mirrored in the sea; Full is sharper and costs more", "sea mirror reflection", |ui| {
        widgets::segmented_fill(ui, &mut g.water_reflections, &[(Reflections::Off, "OFF"), (Reflections::Half, "HALF"), (Reflections::Full, "FULL")]);
    });
    setting_row(ui, rows, "TERRAIN RELIGHTING", "0 keeps the 1997 map artwork as painted; more adds light and shade on slopes", "lighting ground relight original art", |ui| {
        widgets::value_slider(ui, Slider::new(&mut g.terrain_relight, limits::TERRAIN_RELIGHT).step_by(0.05).fixed_decimals(2), value_box);
    });
    toggle_row(ui, rows, "HD SURFACES", "Up close: smooth ground with fine grain and relief; units with worn edges, battle damage, shining metal and glowing lights", "texture detail high definition damage wear", &mut g.hd_surfaces);
    // (It is drawn only with HD surfaces and High or Ultra effects: the row is dimmed without.)
    let can_move = g.hd_surfaces && matches!(g.effects, Quality::High | Quality::Ultra);
    setting_row(ui, rows, "GROUND IN THE WIND", "Up close, grass waves, sand streams and snow drifts in the wind (modern camera, with HD surfaces and High or Ultra effects; reduced motion keeps it still)", "grass sand snow wind animated terrain ground moving", |ui| {
        ui.add_enabled_ui(can_move, |ui| toggle_control(ui, &mut g.ground_life, "GROUND IN THE WIND"));
    });
    toggle_row(ui, rows, "3D TREES AND ROCKS", "Close to the ground, trees, bushes and rocks are modelled in 3D, swaying in the wind and casting shadows (modern camera)", "trees forest foliage bushes rocks models 3d", &mut g.trees_3d);
    let ray_tracing = if !g.gpu.is_empty() && !g.gpu_ray_tracing {
        n("This graphics card has no hardware ray tracing")
    } else if host.in_game && g.ray_tracing && !host.ray_tracing_running {
        n("Starts when the game next starts")
    } else {
        n("Units and buildings ray traced: their shadows from the sun and from fires and explosions, ambient occlusion, reflections in metal and puddles (modern camera; graphics cards with hardware ray tracing)")
    };
    toggle_row(ui, rows, "RAY TRACING", ray_tracing, "rtx raytracing ray traced shadows reflections ambient occlusion", &mut g.ray_tracing);
    toggle_row(ui, rows, "BLOOM", "Glow around bright lasers, explosions and lights", "glow hdr", &mut g.bloom);
    toggle_row(ui, rows, "AMBIENT OCCLUSION", "Soft shading where units, buildings and trees meet the ground", "ao ssao shading", &mut g.ambient_occlusion);
    toggle_row(ui, rows, "CLOUD SHADOWS", "Shadows of the drifting clouds on the battlefield", "clouds", &mut g.cloud_shadows);
    toggle_row(ui, rows, "HEAT HAZE", "Hot air shimmering over explosions, fires and lava", "distortion shimmer", &mut g.heat_haze);
    toggle_row(ui, rows, "WEATHER", "Snow on ice worlds, embers on lava, showers and lightning on green and water worlds, dust on deserts", "rain snow storm lightning", &mut g.weather);
    toggle_row(ui, rows, "INTRO ROOM", "TA's intro plays on an old monitor in a bedroom at night, as it did in 1997; off: full screen", "intro movie video crt monitor nostalgia cutscene", &mut g.intro_room);
    setting_row(ui, rows, "TIME OF DAY", "TA's own light, dawn, dusk, a moonlit night, or a day and a night every twenty minutes of play (modern camera)", "night dusk dawn sunset sun moon stars cycle lighting", |ui| {
        let options = [
            (TimeOfDay::TaDay, n("TA's day")),
            (TimeOfDay::Dawn, n("Dawn")),
            (TimeOfDay::Dusk, n("Dusk")),
            (TimeOfDay::Night, n("Night")),
            (TimeOfDay::Cycle, n("Day and night")),
        ];
        let current = options.iter().find(|(v, _)| *v == g.time_of_day).map_or("", |(_, label)| label);
        egui::ComboBox::from_id_salt("ta_ui::settings::time_of_day").selected_text(t(current)).width(ui.available_width()).show_ui(ui, |ui| {
            for (value, label) in options {
                ui.selectable_value(&mut g.time_of_day, value, t(label));
            }
        });
    });
    picked
}

fn sound(ui: &mut Ui, rows: &mut Rows, a: &mut Audio) {
    let value_box = rows.value_box;
    setting_row(ui, rows, "MASTER VOLUME", "", "sound audio loudness mute", |ui| percent(ui, &mut a.master, limits::VOLUME, value_box));
    setting_row(ui, rows, "MUSIC", "", "soundtrack volume", |ui| percent(ui, &mut a.music, limits::VOLUME, value_box));
    setting_row(ui, rows, "SOUND EFFECTS", "Weapons, explosions, machines and the interface", "sfx volume", |ui| percent(ui, &mut a.effects, limits::VOLUME, value_box));
    setting_row(ui, rows, "UNIT VOICES", "Units answering orders, and briefings", "speech acknowledgements volume", |ui| percent(ui, &mut a.voice, limits::VOLUME, value_box));
    toggle_row(ui, rows, "UNIT REPLIES", "Units answer when selected and given orders. Off: they keep quiet; alerts, the self-destruct count, briefings and the tutorial still speak.", "acknowledgements voices speech select order response chatter quiet", &mut a.unit_replies);
    toggle_row(ui, rows, "MUTE IN THE BACKGROUND", "The game is silent while another window is in front", "mute background unfocused alt-tab focus quiet sound", &mut a.mute_in_background);
    setting_row(ui, rows, "SIMULTANEOUS SOUNDS", "More voices keep big battles fuller, at some CPU cost. Takes effect the next time the game starts", "channels voices polyphony", |ui| {
        widgets::value_slider(ui, Slider::new(&mut a.max_voices, limits::MAX_VOICES).step_by(8.0), value_box);
    });
    setting_row(ui, rows, "YOUR MUSIC", "Your own music (the folders below) with TA's soundtrack or in its place; TA's victory and defeat stay. The jukebox (J) plays any track", "jukebox own songs playlist mp3 ogg flac soundtrack", |ui| {
        widgets::segmented_fill(ui, &mut a.user_music, &[(UserMusic::Off, "OFF"), (UserMusic::Mixed, "WITH TA'S"), (UserMusic::Only, "INSTEAD OF TA'S")]);
    });
    setting_row(ui, rows, "MUSIC FOLDER", "MP3, Ogg, FLAC, WAV and M4A files in this folder (and in the settings folder's music folder)", "jukebox own music directory songs", |ui| {
        // The folder from the column's left edge, its buttons at its right.
        let shown = a.music_folder.as_ref().map_or_else(|| t("Not set").to_string(), |p| p.display().to_string());
        egui::Sides::new().height(32.0).shrink_left().truncate().show(
            ui,
            |ui| ui.add(Label::new(RichText::new(shown).color(Color32::from_gray(200))).truncate()),
            |ui| {
                if a.music_folder.is_some() && ui.add(ActionButton::secondary("CLEAR")).clicked() {
                    a.music_folder = None;
                }
                if ui.add(ActionButton::secondary("CHOOSE…")).clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_title(t("Your music folder")).pick_folder()
                {
                    a.music_folder = Some(dir);
                }
            },
        );
    });
}

/// Common 16:9, 16:10, 21:9 and 4:3 resolutions, and the monitor's own.
const RESOLUTIONS: &[(u32, u32)] = &[
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1440, 900),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2560, 1080),
    (2560, 1440),
    (2560, 1600),
    (3440, 1440),
    (3840, 2160),
    (1024, 768),
];

fn resolution(ui: &mut Ui, g: &mut Graphics, custom: &mut bool, native: Option<(u32, u32)>) {
    let mut options: Vec<(u32, u32)> = RESOLUTIONS.to_vec();
    if let Some(native) = native.filter(|n| !options.contains(n)) {
        options.push(native);
    }
    options.sort_by_key(|&(w, h)| (w * h, w));
    let current = (g.width, g.height);
    let is_preset = options.contains(&current);
    let selected = if *custom || !is_preset { t("Custom").to_string() } else { resolution_text(g.width, g.height, native) };
    ui.horizontal(|ui| {
        // (Across the column; beside the width and height boxes when they show.)
        let fields = if *custom || !is_preset { 2.0 * ui.spacing().interact_size.x + 3.0 * ui.spacing().item_spacing.x + 12.0 } else { 0.0 };
        egui::ComboBox::from_id_salt("ta_ui::settings::resolution").selected_text(selected).width(ui.available_width() - fields).show_ui(ui, |ui| {
            for &(w, h) in &options {
                if ui.selectable_label(!*custom && current == (w, h), resolution_text(w, h, native)).clicked() {
                    g.width = w;
                    g.height = h;
                    *custom = false;
                }
            }
            if ui.selectable_label(*custom || !is_preset, t("Custom…")).clicked() {
                *custom = true;
            }
        });
        if *custom || !is_preset {
            ui.add(DragValue::new(&mut g.width).range(limits::WIDTH).speed(4));
            ui.label(RichText::new("×").color(STEEL));
            ui.add(DragValue::new(&mut g.height).range(limits::HEIGHT).speed(4));
        }
    });
}

/// "16:9" and friends, when the ratio is a common one.
pub fn aspect_label(width: u32, height: u32) -> Option<&'static str> {
    let ratio = width as f32 / height.max(1) as f32;
    [(16.0 / 9.0, "16:9"), (16.0 / 10.0, "16:10"), (4.0 / 3.0, "4:3"), (5.0 / 4.0, "5:4"), (21.0 / 9.0, "21:9"), (32.0 / 9.0, "32:9")]
        .into_iter()
        .find(|(r, _)| (ratio - r).abs() < 0.03 || (*r == 21.0 / 9.0 && (ratio - 2.37).abs() < 0.05))
        .map(|(_, label)| label)
}

fn resolution_text(width: u32, height: u32, native: Option<(u32, u32)>) -> String {
    let mut text = format!("{width} × {height}");
    if let Some(aspect) = aspect_label(width, height) {
        text.push_str(&format!("   {aspect}"));
    }
    if native == Some((width, height)) {
        text.push_str(&format!("   · {}", t("native")));
    }
    text
}

/// Every category's settings back to their defaults (the launcher's RESET ALL SETTINGS): what each
/// category's RESTORE DEFAULTS puts back, the keys included, and nothing else. The player's name and
/// language stay, and so does what is not a setting: the game folder, the mods, the graphics card
/// found, the multiplayer games joined lately, the launcher's window.
pub fn restore_all(config: &mut Config) {
    for category in Category::ALL {
        restore(category, config);
    }
}

/// Puts a category's settings back to their defaults (the player's name and the language
/// stay).
fn restore(category: Category, config: &mut Config) {
    let d = Config::default();
    match category {
        Category::Gameplay => {
            let p = &mut config.gameplay;
            p.unit_limit = d.gameplay.unit_limit;
            p.speed = d.gameplay.speed;
            p.autosave = d.gameplay.autosave;
            p.ai_chatter = d.gameplay.ai_chatter;
            p.coach = d.gameplay.coach;
            p.start_tips = d.gameplay.start_tips;
            p.coach_muted = d.gameplay.coach_muted.clone();
        }
        Category::Controls => {
            let p = &mut config.gameplay;
            p.order_button = d.gameplay.order_button;
            p.radial_menu = d.gameplay.radial_menu;
            p.radial_opens = d.gameplay.radial_opens;
            p.scroll_speed = d.gameplay.scroll_speed;
            p.edge_scroll = d.gameplay.edge_scroll;
            p.gamepad = d.gameplay.gamepad;
            config.keys = d.keys;
        }
        Category::Interface => {
            let p = &mut config.gameplay;
            p.health_bars = d.gameplay.health_bars;
            p.strategic_icons = d.gameplay.strategic_icons;
            p.classic_interface = d.gameplay.classic_interface;
            p.classic_pixels = d.gameplay.classic_pixels;
            p.companion = d.gameplay.companion;
            p.menu_backdrop = d.gameplay.menu_backdrop;
            config.launcher.seasonal_effects = d.launcher.seasonal_effects;
            config.launcher.seasonal_theme = d.launcher.seasonal_theme;
        }
        Category::Accessibility => {
            let p = &mut config.gameplay;
            p.ui_scale = d.gameplay.ui_scale;
            p.colour_blind = d.gameplay.colour_blind;
            p.team_markers = d.gameplay.team_markers;
            p.keyboard_pointer = d.gameplay.keyboard_pointer;
            p.screen_reader_announcements = d.gameplay.screen_reader_announcements;
            p.tutorial_voice = d.gameplay.tutorial_voice;
            p.gamepad_deadzone = d.gameplay.gamepad_deadzone;
            p.gamepad_sensitivity = d.gameplay.gamepad_sensitivity;
            p.reduced_motion = d.gameplay.reduced_motion;
            p.reduced_flashes = d.gameplay.reduced_flashes;
            p.screen_shake = d.gameplay.screen_shake;
            p.ta_cursors = d.gameplay.ta_cursors;
            p.cursor_scale = d.gameplay.cursor_scale;
            p.high_contrast_cursor = d.gameplay.high_contrast_cursor;
            p.message_duration = d.gameplay.message_duration;
        }
        Category::Graphics => {
            // (The display's settings are Display's, though kept with the graphics.)
            let g = &config.graphics;
            config.graphics = Graphics {
                gpu: g.gpu.clone(),
                width: g.width,
                height: g.height,
                window_mode: g.window_mode,
                vsync: g.vsync,
                fps_limit: g.fps_limit,
                save_power_in_background: g.save_power_in_background,
                show_frame_rate: g.show_frame_rate,
                ..d.graphics
            };
        }
        Category::Display => {
            let g = &mut config.graphics;
            g.width = d.graphics.width;
            g.height = d.graphics.height;
            g.window_mode = d.graphics.window_mode;
            g.vsync = d.graphics.vsync;
            g.fps_limit = d.graphics.fps_limit;
            g.save_power_in_background = d.graphics.save_power_in_background;
            g.show_frame_rate = d.graphics.show_frame_rate;
        }
        Category::Sound => {
            // (The launcher's music and sounds are its own category's.)
            let a = &config.audio;
            let launcher = (a.launcher_sounds, a.launcher_music, a.launcher_music_volume);
            config.audio = d.audio;
            (config.audio.launcher_sounds, config.audio.launcher_music, config.audio.launcher_music_volume) = launcher;
        }
        Category::Launcher => {
            // (The seasons are Interface's: both the game and the launcher show them.)
            config.launcher.on_play = d.launcher.on_play;
            config.launcher.themed_cursor = d.launcher.themed_cursor;
            config.audio.launcher_sounds = d.audio.launcher_sounds;
            config.audio.launcher_music = d.audio.launcher_music;
            config.audio.launcher_music_volume = d.audio.launcher_music_volume;
            config.updates.check = d.updates.check;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of the screen: what changed, and every text it drew.
    fn frame(view: &mut SettingsView, config: &mut Config, host: &Host) -> (Changes, Vec<String>) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1400.0, 900.0))), time: Some(10.0), ..Default::default() };
        let mut changes = Changes::default();
        let mut output = ctx.run_ui(raw, |ui| changes = view.show(ui, config, host));
        output.textures_delta.clear();
        fn collect(shape: &Shape, out: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => out.push(text.galley.text().to_string()),
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        (changes, texts)
    }

    fn host() -> Host<'static> {
        Host { in_game: true, msaa_running: 4, ray_tracing_running: false, native: Some((2560, 1440)), languages: &[], language_dirs: &[], height: Some(600.0) }
    }

    #[test]
    fn every_category_draws() {
        let mut config = Config::default();
        for category in Category::ALL {
            let mut view = SettingsView::default();
            view.open(category);
            // (The launcher's own category in the launcher.)
            let mut host = host();
            host.in_game = category != Category::Launcher;
            let (changes, texts) = frame(&mut view, &mut config, &host);
            assert_eq!(changes, Changes::default(), "showing {category:?} changes nothing");
            assert!(texts.iter().any(|t| t == category.label()), "{category:?} has its title: {texts:?}");
        }
        assert_eq!(config, Config::default());
    }

    #[test]
    fn settings_reflow_at_large_scale_without_widening_the_viewport() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        // 1280 x 720 pixels at Windows 150% and UI 200% leaves 427 x 240 points.
        // Options' frame leaves 330 points for its content; all categories must keep it.
        for width in [330.0, 414.0, 720.0] {
            for category in Category::ALL {
                let mut config = Config::default();
                let mut view = SettingsView::default();
                view.open(category);
                let mut host = host();
                host.height = None;
                let mut content_width = 0.0;
                for _ in 0..3 {
                    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 240.0))), time: Some(10.0), ..Default::default() };
                    ctx.run_ui(raw, |ui| {
                        crate::scroll::show(ui, ScrollArea::vertical().max_height(220.0).auto_shrink([false, false]), crate::scroll::Behind::Plate, |ui| {
                            view.show(ui, &mut config, &host);
                            content_width = ui.min_rect().width();
                        });
                    }).drop_without_applying_deltas();
                }
                assert!(content_width <= width + 1.0, "{category:?} at {width}: content widened to {content_width}");
                assert_eq!(config, Config::default());
            }
        }
    }

    #[test]
    fn compact_options_scroll_instead_of_scaling_the_whole_panel_down() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let mut config = Config::default();
        let mut view = SettingsView::default();
        view.open(Category::Accessibility);
        let mut host = host();
        host.height = None;
        for _ in 0..4 {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(427.0, 240.0))), time: Some(10.0), ..Default::default() };
            ctx.run_ui(raw, |_| {
                let panel = crate::panel::Panel::new("OPTIONS").subtitle("CHANGES TAKE EFFECT AT ONCE")
                    .exit(crate::panel::Exit::Close).width(330.0).max_height(220.0);
                let response = crate::panel::modal(&ctx, "compact_options_test", panel, true, |ui| {
                    view.show(ui, &mut config, &host);
                });
                assert!(response.rect.width() <= 407.0, "options stays inside the viewport: {:?}", response.rect);
                assert!(response.rect.height() <= 221.0, "body must scroll: {:?}", response.rect);
            }).drop_without_applying_deltas();
        }
        let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("compact_options_test"));
        let scale = ctx.layer_transform_to_global(layer).map_or(1.0, |transform| transform.scaling);
        assert!((scale - 1.0).abs() < 0.001, "requested UI scale must not be reduced to fit: {scale}");
    }

    #[test]
    fn search_finds_settings_in_any_category() {
        let mut config = Config::default();
        let mut view = SettingsView { search: "mouse".into(), ..Default::default() };
        let (_, texts) = frame(&mut view, &mut config, &host());
        assert!(texts.iter().any(|t| t == "GIVE ORDERS WITH"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "TA CURSORS"), "interface's cursors are about the mouse too");
        assert!(!texts.iter().any(|t| t == "MUSIC"));

        view.search = "vsync".into();
        let (_, texts) = frame(&mut view, &mut config, &host());
        assert!(texts.iter().any(|t| t == "VERTICAL SYNC"));
        // Keys are found by what they do and by the key.
        view.search = "patrol".into();
        let (_, texts) = frame(&mut view, &mut config, &host());
        assert!(texts.iter().any(|t| t == "Patrol"));
        view.search = "zzzz".into();
        let (_, texts) = frame(&mut view, &mut config, &host());
        assert!(texts.iter().any(|t| t.starts_with("Nothing matches")));
    }

    /// The radial menu's two rows, in both hosts, found by searching; Controls' defaults put it
    /// back on, opening on a click or a hold.
    #[test]
    fn the_radial_menu_has_its_rows_and_its_defaults() {
        for in_game in [false, true] {
            let mut host = host();
            host.in_game = in_game;
            let mut config = Config::default();
            let mut view = SettingsView::default();
            view.open(Category::Controls);
            let (_, texts) = frame(&mut view, &mut config, &host);
            for row in ["RADIAL MENU", "RADIAL MENU OPENS WITH", "CLICK OR HOLD", "HOLD ONLY"] {
                assert!(texts.iter().any(|text| text == row), "{row}: {texts:?}");
            }
            for words in ["radial", "middle click", "hold"] {
                view.search = words.into();
                let (_, texts) = frame(&mut view, &mut config, &host);
                assert!(texts.iter().any(|text| text == "RADIAL MENU"), "{words}: {texts:?}");
            }
            assert_eq!(config, Config::default(), "browsing changes nothing");
        }
        let mut config = Config::default();
        config.gameplay.radial_menu = false;
        config.gameplay.radial_opens = RadialOpens::Hold;
        restore(Category::Interface, &mut config);
        assert!(!config.gameplay.radial_menu, "another category's defaults leave it");
        restore(Category::Controls, &mut config);
        assert!(config.gameplay.radial_menu);
        assert_eq!(config.gameplay.radial_opens, RadialOpens::ClickOrHold);
    }

    #[test]
    fn art_style_is_available_in_game_and_launcher_settings() {
        for in_game in [false, true] {
            let mut host = host();
            host.in_game = in_game;
            let mut config = Config::default();
            config.graphics.art_style = ArtStyle::Classic;
            let before = config.clone();
            let mut view = SettingsView { search: "art style".into(), ..Default::default() };
            let (changes, texts) = frame(&mut view, &mut config, &host);
            assert_eq!(changes, Changes::default());
            assert_eq!(config, before);
            assert!(texts.iter().any(|t| t == "ART STYLE"), "{texts:?}");
            assert!(texts.iter().any(|t| t == "ENHANCED CLASSIC"), "{texts:?}");
        }
    }

    #[test]
    fn accessibility_is_available_in_both_hosts_and_search() {
        for in_game in [false, true] {
            let mut host = host();
            host.in_game = in_game;
            host.height = Some(1600.0);
            let mut config = Config::default();
            let mut view = SettingsView::default();
            view.open(Category::Accessibility);
            let (_, texts) = frame(&mut view, &mut config, &host);
            for label in ["ACCESSIBILITY", "REDUCED MOTION", "REDUCED FLASHES", "CURSOR SIZE", "HIGH-CONTRAST CURSOR", "MESSAGE DURATION"] {
                assert!(texts.iter().any(|text| text == label), "missing {label}: {texts:?}");
            }
            view.search = "accessibility".into();
            let (_, texts) = frame(&mut view, &mut config, &host);
            assert!(texts.iter().any(|text| text == "REDUCED MOTION"));
            view.search = "message duration".into();
            let (_, texts) = frame(&mut view, &mut config, &host);
            assert!(texts.iter().any(|text| text == "MESSAGE DURATION"));
            assert_eq!(config, Config::default(), "browsing must not change settings");
        }
    }

    #[test]
    fn restoring_accessibility_preserves_other_preferences() {
        let mut config = Config::default();
        config.gameplay.reduced_motion = true;
        config.gameplay.reduced_flashes = true;
        config.gameplay.high_contrast_cursor = true;
        config.gameplay.cursor_scale = 2.0;
        config.gameplay.message_duration = 3.0;
        config.gameplay.ui_scale = 1.5;
        config.gameplay.colour_blind = true;
        config.gameplay.ta_cursors = false;
        config.gameplay.player_name = "Tester".into();
        config.gameplay.order_button = OrderButton::Left;
        config.audio.music = 0.2;
        config.graphics.bloom = false;
        let mut expected = config.clone();
        restore(Category::Interface, &mut config);
        assert_eq!(config, expected, "interface reset leaves accessibility intact");
        let defaults = Gameplay::default();
        expected.gameplay = Gameplay { player_name: "Tester".into(), order_button: OrderButton::Left, ..defaults };
        restore(Category::Accessibility, &mut config);
        assert_eq!(config, expected);
    }

    #[test]
    fn restoring_a_category_leaves_the_others() {
        let mut config = Config::default();
        config.audio.music = 0.1;
        config.gameplay.order_button = OrderButton::Left;
        config.gameplay.player_name = "Jim".into();
        config.graphics.bloom = false;
        restore(Category::Controls, &mut config);
        assert_eq!(config.gameplay.order_button, OrderButton::Right);
        assert_eq!(config.audio.music, 0.1);
        restore(Category::Gameplay, &mut config);
        assert_eq!(config.gameplay.player_name, "Jim", "the name stays");
        restore(Category::Graphics, &mut config);
        assert!(config.graphics.bloom);
        restore(Category::Sound, &mut config);
        assert_eq!(config.audio, Audio::default());
    }

    #[test]
    fn the_launchers_own_category_is_the_launchers_alone() {
        let mut config = Config::default();
        // In the game: never shown, not even asked for.
        let mut view = SettingsView::default();
        view.open(Category::Launcher);
        let (_, texts) = frame(&mut view, &mut config, &host());
        assert!(!texts.iter().any(|text| text == "LAUNCHER" || text == "WHEN THE GAME STARTS"), "{texts:?}");
        // In the launcher: its rows, and RESTORE DEFAULTS puts back only its own.
        let mut launcher = host();
        launcher.in_game = false;
        let mut view = SettingsView::default();
        view.open(Category::Launcher);
        let (_, texts) = frame(&mut view, &mut config, &launcher);
        for row in ["WHEN THE GAME STARTS", "LAUNCHER MUSIC", "LOOK FOR UPDATES", "THEMED LAUNCHER CURSORS"] {
            assert!(texts.iter().any(|text| text == row), "{row}: {texts:?}");
        }
        config.audio.launcher_music = !config.audio.launcher_music;
        config.audio.master = 0.25;
        config.updates.check = !config.updates.check;
        restore(Category::Sound, &mut config);
        assert_eq!(config.audio.master, Config::default().audio.master);
        assert_ne!(config.audio.launcher_music, Config::default().audio.launcher_music, "the Sound category's defaults leave the launcher's music");
        restore(Category::Launcher, &mut config);
        assert_eq!(config.audio.launcher_music, Config::default().audio.launcher_music);
        assert_eq!(config.updates.check, Config::default().updates.check);
    }

    /// The seasons deck the game's loading screens and title menu as well as the launcher, so
    /// both show them, under Interface, and Interface's defaults put them back (not the
    /// launcher's own category's).
    #[test]
    fn seasonal_settings_are_in_both_hosts_and_reset_with_interface() {
        let mut config = Config::default();
        for in_game in [false, true] {
            let mut host = host();
            host.in_game = in_game;
            for search in ["seasonal", "christmas"] {
                let mut view = SettingsView { search: search.into(), ..Default::default() };
                let (_, texts) = frame(&mut view, &mut config, &host);
                assert!(texts.iter().any(|text| text == "SEASONAL EFFECTS"), "{search} in_game={in_game}: {texts:?}");
                assert!(texts.iter().any(|text| text == "SEASONAL THEME"), "{search} in_game={in_game}");
            }
            let mut view = SettingsView::default();
            view.open(Category::Interface);
            host.height = Some(1600.0);
            let (_, texts) = frame(&mut view, &mut config, &host);
            assert!(texts.iter().any(|text| text == "SEASONAL EFFECTS"), "in_game={in_game}: {texts:?}");
        }
        config.launcher.seasonal_effects = false;
        config.launcher.seasonal_theme = SeasonalTheme::NewYear;
        restore(Category::Launcher, &mut config);
        assert!(!config.launcher.seasonal_effects);
        assert_eq!(config.launcher.seasonal_theme, SeasonalTheme::NewYear);
        restore(Category::Interface, &mut config);
        assert!(config.launcher.seasonal_effects);
        assert_eq!(config.launcher.seasonal_theme, SeasonalTheme::Automatic);
    }

    /// The switches for what used to be fixed: each in its category in both hosts, found by
    /// searching, and put back by its own category's defaults (the frame rate counter by
    /// Display's, though it is kept with the graphics).
    #[test]
    fn the_new_switches_have_rows_search_and_defaults() {
        let rows = [
            (Category::Interface, "STRATEGIC ICONS", "icons"),
            (Category::Accessibility, "SCREEN SHAKE", "shake"),
            (Category::Graphics, "GROUND IN THE WIND", "grass"),
            (Category::Sound, "UNIT REPLIES", "acknowledgements"),
            (Category::Display, "FRAME RATE COUNTER", "fps"),
        ];
        for in_game in [false, true] {
            let mut host = host();
            host.in_game = in_game;
            host.height = Some(2400.0);
            let mut config = Config::default();
            for (category, label, word) in rows {
                let mut view = SettingsView::default();
                view.open(category);
                let (_, texts) = frame(&mut view, &mut config, &host);
                assert!(texts.iter().any(|text| text == label), "{label} in {category:?}: {texts:?}");
                let mut view = SettingsView { search: word.into(), ..Default::default() };
                let (_, texts) = frame(&mut view, &mut config, &host);
                assert!(texts.iter().any(|text| text == label), "{word} finds {label}: {texts:?}");
            }
            assert_eq!(config, Config::default(), "browsing changes nothing");
        }
        let mut config = Config::default();
        config.gameplay.strategic_icons = false;
        config.gameplay.screen_shake = false;
        config.graphics.ground_life = false;
        config.graphics.show_frame_rate = true;
        config.audio.unit_replies = false;
        restore(Category::Graphics, &mut config);
        assert!(config.graphics.ground_life && config.graphics.show_frame_rate, "Graphics' defaults leave Display's counter");
        restore(Category::Display, &mut config);
        assert!(!config.graphics.show_frame_rate);
        for category in [Category::Interface, Category::Accessibility, Category::Sound] {
            restore(category, &mut config);
        }
        assert_eq!(config, Config::default());
    }

    #[test]
    fn themed_launcher_cursor_is_searchable_only_in_launcher_and_resets_with_launcher() {
        let mut config = Config::default();
        for in_game in [false, true] {
            let mut host = host();
            host.in_game = in_game;
            let mut view = SettingsView { search: "themed launcher cursors".into(), ..Default::default() };
            let (_, texts) = frame(&mut view, &mut config, &host);
            assert_eq!(texts.iter().any(|text| text == "THEMED LAUNCHER CURSORS"), !in_game);
        }
        config.launcher.themed_cursor = false;
        restore(Category::Interface, &mut config);
        assert!(!config.launcher.themed_cursor);
        restore(Category::Launcher, &mut config);
        assert!(config.launcher.themed_cursor);
    }

    #[test]
    fn aspect_labels() {
        assert_eq!(aspect_label(1920, 1080), Some("16:9"));
        assert_eq!(aspect_label(1366, 768), Some("16:9"));
        assert_eq!(aspect_label(1920, 1200), Some("16:10"));
        assert_eq!(aspect_label(3440, 1440), Some("21:9"));
        assert_eq!(aspect_label(1024, 768), Some("4:3"));
        assert_eq!(aspect_label(1000, 100), None);
    }
}
