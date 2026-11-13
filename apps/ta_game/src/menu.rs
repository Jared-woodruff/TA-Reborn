//! Front end: title menu and skirmish setup, drawn over the live attract-mode battle.

use egui::{Align2, Color32, CornerRadius, Margin, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::scroll::{self, Behind};
use ta_ui::sound::{self, UiSound};
use ta_ui::widgets::{self as w, ActionButton};

use crate::palette::team_color;
use crate::setup::{Assets, Controller, GameSetup, SlotSetup};
use crate::theme::*;

mod record_prompt;
mod scenarios;

#[cfg(test)]
mod tests;

pub use record_prompt::installed_now;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Title,
    Tutorial,
    Conquest,
    Skirmish,
    Scenarios,
    Campaign,
    Multiplayer,
    Lobby,
    Records,
    Credits,
    /// The unit encyclopaedia.
    Units,
    /// The Galactic War: the TA universe's story as a timeline.
    Timeline,
}

impl Page {
    /// How deep in the menus the page is: going to a shallower one is going back.
    fn depth(self) -> u8 {
        match self {
            Page::Title => 0,
            Page::Lobby => 2,
            _ => 1,
        }
    }
}

/// What the title menu lists: its few main entries, or one of its two groups in their place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TitleMenu {
    #[default]
    Main,
    /// Playing alone: a skirmish, the campaigns, Conquest, the scenarios, the tutorial and the
    /// saved games.
    SinglePlayer,
    /// The units, the galactic war, replays, the map editor, the credits and the intro.
    Extras,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotKind {
    Closed,
    Human,
    AiEasy,
    AiNormal,
    AiHard,
}

impl SlotKind {
    /// Its name in `skirmish.toml`.
    fn key(self) -> &'static str {
        match self {
            SlotKind::Closed => "closed",
            SlotKind::Human => "player",
            SlotKind::AiEasy => "easy",
            SlotKind::AiNormal => "normal",
            SlotKind::AiHard => "hard",
        }
    }

    fn from_key(key: &str) -> Option<SlotKind> {
        [SlotKind::Closed, SlotKind::Human, SlotKind::AiEasy, SlotKind::AiNormal, SlotKind::AiHard].into_iter().find(|k| k.key() == key)
    }

    fn label(self) -> &'static str {
        match self {
            SlotKind::Closed => t("Closed"),
            SlotKind::Human => t("You"),
            SlotKind::AiEasy => t("Computer — Easy"),
            SlotKind::AiNormal => t("Computer — Normal"),
            SlotKind::AiHard => t("Computer — Hard"),
        }
    }
}

struct Slot {
    kind: SlotKind,
    side: usize,
    color: u8,
    team: u8,
}

/// Gives every open seat of `seats` a colour of its own: two players of one colour cannot be
/// told apart. A seat sharing its colour with an open seat before it takes the next free one;
/// `opened`, a seat just opened from its drop-down, is the one to move when it clashes, whatever
/// its place, so that the seats already playing keep their colours. (A closed seat keeps the
/// colour it had while another took it; opening it, or a map with more starts bringing a seat
/// back into view, gave two players one colour.)
fn settle_colours(seats: &mut [Slot], opened: Option<usize>) {
    let mut order: Vec<usize> = (0..seats.len()).filter(|&i| Some(i) != opened).collect();
    order.extend(opened.filter(|&n| n < seats.len()));
    let mut settled: Vec<usize> = Vec::new();
    for i in order {
        if seats[i].kind == SlotKind::Closed {
            continue;
        }
        if settled.iter().any(|&k| seats[k].color == seats[i].color) {
            let taken: Vec<u8> = seats.iter().enumerate().filter(|&(j, s)| j != i && s.kind != SlotKind::Closed).map(|(_, s)| s.color).collect();
            seats[i].color = crate::palette::next_free_colour(seats[i].color, |c| taken.contains(&c));
        }
        settled.push(i);
    }
}

/// The skirmish last set up, kept beside the settings (`Menu::keep_skirmish`).
const KEPT_SKIRMISH: &str = "skirmish.toml";

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct KeptSkirmish {
    map: String,
    slots: Vec<KeptSlot>,
    rules: Option<ta_sim::GameRules>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct KeptSlot {
    kind: String,
    /// The side's key (ARM, CORE, a mod's).
    side: String,
    color: u8,
    team: u8,
}

// (One action a frame at most: the size difference between its variants costs nothing.)
#[allow(clippy::large_enum_variant)]
pub enum MenuAction {
    None,
    Start(GameSetup),
    PlayIntro,
    Quit,
    /// Play (Some) or stop (None) a briefing narration.
    Narrate(Option<String>),
    /// A multiplayer game starts with this lockstep session.
    StartNet(GameSetup, ta_net::Session),
    /// Continue a saved game or watch a replay.
    Play(crate::replay::Record),
    /// Turn these mods on (and only these, in this order), and start the game again on the
    /// saved game or replay at `path` (`replay`: to watch).
    RestartWithMods { path: std::path::PathBuf, replay: bool, order: Vec<String> },
    /// Open the launcher's Mods page on this mod (one a record needs that is not installed).
    GetModInLauncher(String),
    /// Open the options panel.
    Options,
    /// Play a tutorial lesson.
    StartTutorial(usize),
    /// Open the map editor.
    Editor,
    /// The encyclopaedia's showroom: show this unit (a type's name) in this world (map) at
    /// these hours of the day, doing this.
    Show { unit: String, map: String, hours: f32, act: crate::showroom::Act },
    /// The Units page closed: back to the battle behind the menu.
    LeaveShowroom,
    /// A wallpaper of the unit on show, this size, its words on it or not.
    Wallpaper { width: u32, height: u32, words: bool },
    /// Open or close the jukebox.
    Jukebox,
    /// The film behind the title menu plays on its own, the menu stepped aside.
    Watch,
}

/// The title menu's margin on the left, and its column's width (narrower in a window under 1000
/// points tall, which gets the compact lockup and menu).
const TITLE_LEFT: f32 = 64.0;
fn title_width(compact: bool) -> f32 {
    if compact { 320.0 } else { 360.0 }
}

/// How many times its own size the title screen's lockup, menu and captions are drawn in a
/// window `height` points tall: in proportion from 1080 points up, so a window of 1440 or 2160
/// lines at the screen's own scale is composed as one of 1080 is (the lockup, the menu and the
/// film's subject in the same places), at most half again; under that, their own size (the
/// compact lockup and menu under 1000 points).
pub fn title_scale(height: f32) -> f32 {
    if height < 1000.0 { 1.0 } else { (height / 1080.0).clamp(1.0, 1.5) }
}

/// Where the title menu's column ends, a gap after it (points from the window's left, in a
/// window `height` points tall): the film behind it stands right of there.
pub fn title_column_right(height: f32) -> f32 {
    (TITLE_LEFT + title_width(height < 1000.0) + 24.0) * title_scale(height)
}

/// The title menu's entries as they are laid out: each fades and slides in a moment after
/// the one above it.
struct MenuItems {
    since: f64,
    /// Where each entry slides in from.
    from: egui::Vec2,
    compact: bool,
    /// How many times their own size they are drawn ([`title_scale`]).
    scale: f32,
    /// Not room for every entry's line under its name: only the essential ones keep theirs.
    dense: bool,
    count: usize,
    /// The first entry (the arrow keys start there).
    first: Option<Response>,
}

impl MenuItems {
    fn menu_item(&mut self, ui: &mut Ui, label: &str, detail: Option<&str>, enabled: bool) -> Response {
        let detail = detail.filter(|_| !self.dense);
        self.menu_item_beacon(ui, label, detail, enabled, false)
    }

    /// [`Self::menu_item`] whose line is kept however little room there is (the save to
    /// continue), lit as the place to start when `beacon`.
    fn menu_item_beacon(&mut self, ui: &mut Ui, label: &str, detail: Option<&str>, enabled: bool, beacon: bool) -> Response {
        let (compact, scale) = (self.compact, self.scale);
        let response = panel::stagger(ui, self.since, self.count, 0.035, self.from * scale, |ui| title_entry(ui, label, detail, enabled, compact, beacon, scale));
        self.count += 1;
        if enabled && self.first.is_none() {
            self.first = Some(response.clone());
        }
        response
    }

    /// A group's last entry, back to the main ones.
    fn back(&mut self, ui: &mut Ui) -> Response {
        let (compact, scale) = (self.compact, self.scale);
        let response = panel::stagger(ui, self.since, self.count, 0.035, self.from * scale, |ui| title_back_entry(ui, n("BACK"), compact, scale));
        self.count += 1;
        response
    }

    /// A group's name over its entries, in brass, with a chevron before it pointing back: a
    /// click on it goes back too.
    fn group_heading(&mut self, ui: &mut Ui, label: &str) -> Response {
        let (compact, s) = (self.compact, self.scale);
        let response = panel::stagger(ui, self.since, self.count, 0.035, self.from * s, |ui| {
            let name = w::galley(ui, RichText::new(t(label)).font(display(s * if compact { 18.0 } else { 21.0 })).extra_letter_spacing(ta_ui::i18n::tracking(5.0 * s)));
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), name.size().y + s * if compact { 4.0 } else { 8.0 }), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t("BACK")));
            if response.clicked() {
                sound::play(ui.ctx(), UiSound::Back);
            }
            let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
            let painter = ui.painter();
            let c = pos2(rect.min.x + (6.0 - 3.0 * hover) * s, rect.center().y);
            let points = |d: f32| vec![pos2(c.x + 3.5 * s + d, c.y - 7.0 * s + d), pos2(c.x - 3.5 * s + d, c.y + d), pos2(c.x + 3.5 * s + d, c.y + 7.0 * s + d)];
            painter.add(Shape::line(points(1.0), Stroke::new(2.6 * s, Color32::from_black_alpha(170))));
            painter.add(Shape::line(points(0.0), Stroke::new(2.3 * s, AMBER_DIM.lerp_to_gamma(AMBER_BRIGHT, hover))));
            ta_ui::metal::embossed(painter, pos2(rect.min.x + 22.0 * s, rect.center().y - name.size().y / 2.0), name, AMBER.lerp_to_gamma(AMBER_BRIGHT, hover));
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
        });
        self.count += 1;
        response
    }
}

/// The dark slot of a title menu entry, rimmed in steel (TA's menu slots), `hover` (0..1) lighting
/// it: pointed at, its rim brightens and a warm light rises in it from the lamp's side.
fn title_slot(ui: &Ui, rect: Rect, hover: f32, s: f32) -> Rect {
    let painter = ui.painter();
    painter.add(ta_ui::metal::slab(ui.ctx(), rect, ta_ui::metal::Slab::Steel, hover * 0.6, false));
    let slot = rect.shrink((3.0 * s).round());
    painter.add(ta_ui::metal::well(slot, Color32::from_rgb(20, 19, 16).lerp_to_gamma(Color32::from_rgb(38, 35, 29), hover)));
    if hover > 0.0 {
        let warm = Color32::from_rgba_unmultiplied(232, 178, 84, (30.0 * hover) as u8);
        let lit = Rect::from_min_max(slot.min + vec2(1.0, 1.0), pos2(slot.min.x + slot.width() * 0.62, slot.max.y - 1.0));
        w::gradient_h(painter, lit, warm, Color32::TRANSPARENT);
    }
    slot
}

/// An entry of the title menu (as `ta_ui::panel::menu_entry_beacon` draws one, at `s` times its
/// size: see [`title_scale`]): a dark slot in a steel rim, a lamp that lights when it is pointed at
/// or focused, the label sliding right, `detail` a second, smaller line; with `beacon` the one to
/// start with, its lamp breathing amber, its rim edged in brass and its line in brass.
fn title_entry(ui: &mut Ui, label: &str, detail: Option<&str>, enabled: bool, compact: bool, beacon: bool, s: f32) -> Response {
    let (label, detail) = (t(label), detail.map(t));
    let height = s * match (detail.is_some(), compact) {
        (true, false) => 62.0,
        (false, false) => 48.0,
        (true, true) => 50.0,
        (false, true) => 38.0,
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height.round()), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    w::reveal_focus(ui, &response);
    if response.clicked() {
        sound::play(ui.ctx(), UiSound::BigButton);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    // (Keyboard focus lights an entry up like the pointer does.)
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), enabled && (response.hovered() || response.has_focus()), 0.12);
    let down = enabled && response.is_pointer_button_down_on();
    let slot = title_slot(ui, rect, hover, s);
    let painter = ui.painter();
    let lamp = pos2(slot.min.x + 15.0 * s, rect.center().y);
    let lamp_size = s * if compact { 3.5 } else { 4.0 };
    if beacon {
        let time = ui.input(|i| i.time);
        let pulse = 0.55 + 0.45 * (time * 2.2).sin() as f32;
        painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(2), Stroke::new(1.5 * s, AMBER.gamma_multiply(0.55 + 0.45 * pulse)), StrokeKind::Inside);
        painter.add(ta_ui::metal::lamp(lamp, lamp_size, AMBER, if down { 1.0 } else { pulse.max(hover) }));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
    } else {
        painter.add(ta_ui::metal::lamp(lamp, lamp_size, GREEN, if down { 1.0 } else { hover * 0.9 }));
    }
    let text_color = if !enabled { STEEL_DIM } else { TEXT_BRIGHT.lerp_to_gamma(Color32::from_rgb(255, 246, 206), hover) };
    let x = slot.min.x + (32.0 + 6.0 * hover) * s;
    // (Up to the chevron; longer words shrink, then are cut short.)
    let room = slot.max.x - 40.0 * s - x;
    let sink = if down { 1.0 } else { 0.0 };
    let title = w::fitted_galley(ui, |size| RichText::new(label).font(display(size)).extra_letter_spacing(ta_ui::i18n::tracking(2.0 * s)), s * if compact { 21.0 } else { 25.0 }, room, 0.7);
    let title_y = if detail.is_some() { rect.min.y + s * if compact { 6.0 } else { 9.0 } } else { rect.center().y - title.size().y / 2.0 };
    ta_ui::metal::embossed(painter, pos2(x, title_y + sink).round(), title, text_color);
    if let Some(detail) = detail {
        let line = w::fitted_galley(ui, |size| RichText::new(detail).font(body(size)), s * if compact { 11.5 } else { 12.5 }, room, 0.8);
        let tone = if !enabled { STEEL_DIM } else if beacon { AMBER_BRIGHT } else { STEEL.lerp_to_gamma(TEXT, hover * 0.6) };
        painter.galley(pos2(x + s, rect.max.y - s * if compact { 7.0 } else { 10.0 } - line.size().y + sink).round(), line, tone);
    }
    if enabled {
        // A chevron on the right, brass when pointed at.
        let c = pos2(rect.max.x - (22.0 - 4.0 * hover) * s, rect.center().y);
        let chevron = STEEL_DIM.lerp_to_gamma(AMBER_BRIGHT, hover);
        let points = |d: f32| vec![pos2(c.x - 4.0 * s + d, c.y - 7.0 * s + d), pos2(c.x + 3.0 * s + d, c.y + d), pos2(c.x - 4.0 * s + d, c.y + 7.0 * s + d)];
        painter.add(Shape::line(points(1.0), Stroke::new(2.5 * s, Color32::from_black_alpha(170))));
        painter.add(Shape::line(points(0.0), Stroke::new(2.2 * s, chevron)));
    }
    w::focus_ring(ui, &response, rect);
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// The last entry of one of the title menu's groups, TA's "previous menu" (as
/// `ta_ui::panel::menu_back_entry` draws it, at `s` times its size): the same slot as
/// [`title_entry`], lower, its chevron on the left pointing back and its label in steel.
fn title_back_entry(ui: &mut Ui, label: &str, compact: bool, s: f32) -> Response {
    let label = t(label);
    let height = s * if compact { 34.0 } else { 40.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height.round()), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    w::reveal_focus(ui, &response);
    if response.clicked() {
        sound::play(ui.ctx(), UiSound::Back);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() || response.has_focus(), 0.12);
    let slot = title_slot(ui, rect, hover, s);
    let painter = ui.painter();
    // The chevron where the other entries have their lamp, moving back when pointed at.
    let c = pos2(slot.min.x + (16.0 - 3.0 * hover) * s, rect.center().y);
    let chevron = STEEL.lerp_to_gamma(AMBER_BRIGHT, hover);
    let points = |d: f32| vec![pos2(c.x + 3.0 * s + d, c.y - 6.0 * s + d), pos2(c.x - 3.0 * s + d, c.y + d), pos2(c.x + 3.0 * s + d, c.y + 6.0 * s + d)];
    painter.add(Shape::line(points(1.0), Stroke::new(2.4 * s, Color32::from_black_alpha(170))));
    painter.add(Shape::line(points(0.0), Stroke::new(2.1 * s, chevron)));
    let x = slot.min.x + (32.0 + 4.0 * hover) * s;
    let title = w::fitted_galley(ui, |size| RichText::new(label).font(display(size)).extra_letter_spacing(ta_ui::i18n::tracking(2.0 * s)), s * if compact { 17.0 } else { 19.0 }, slot.max.x - 16.0 * s - x, 0.7);
    let color = STEEL.lerp_to_gamma(TEXT_BRIGHT, hover);
    ta_ui::metal::embossed(painter, pos2(x, rect.center().y - title.size().y / 2.0).round(), title, color);
    w::focus_ring(ui, &response, rect);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub struct Menu {
    pub page: Page,
    pub campaign: crate::campaign::CampaignPage,
    pub conquest: crate::conquest_ui::ConquestPage,
    pub connect: crate::lobby::ConnectForm,
    pub lobby: Option<crate::lobby::NetLobby>,
    /// Saved games and replays on disk (refreshed when the page opens).
    records: Vec<(std::path::PathBuf, crate::replay::RecordSummary)>,
    records_loading: Option<std::sync::mpsc::Receiver<(Vec<(std::path::PathBuf, crate::replay::RecordSummary)>, Vec<ta_config::records::RecordMod>)>>,
    record_loading: Option<std::sync::mpsc::Receiver<(std::path::PathBuf, anyhow::Result<crate::replay::Record>, Vec<ta_config::records::RecordMod>)>>,
    pub record_error: Option<String>,
    /// A record made with other mods or game data, asked about before it is opened.
    pub record_prompt: Option<record_prompt::RecordPrompt>,
    /// Something that went wrong out of sight (a replay not kept, progress or settings not
    /// saved, the map editor or a scenario not starting): a dialog's title and words.
    pub failure: Option<(&'static str, String)>,
    /// The files of the scenarios folder that could not be read, and why.
    scenarios_unread: Vec<(String, String)>,
    /// The scenarios (read again when their page opens), the one picked and the difficulty.
    scenarios: Vec<crate::scenario::Entry>,
    scenario_selected: usize,
    scenario_difficulty: ta_ai::Difficulty,
    selected_map: usize,
    /// The maps the list shows: words, origins, order.
    map_choice: crate::map_list::Choice,
    slots: Vec<Slot>,
    seed: u64,
    pub rules: ta_sim::GameRules,
    /// The newest saved game, for "Continue": its file, when it was written, and its title
    /// (re-read only when a newer file appears).
    latest_save: Option<(std::path::PathBuf, std::time::SystemTime, String)>,
    /// When the saves folder was last looked at.
    saves_checked: Option<std::time::Instant>,
    latest_cache: crate::replay::LatestSaveCache,
    latest_loading: Option<std::sync::mpsc::Receiver<(Option<(std::path::PathBuf, std::time::SystemTime, String)>, crate::replay::LatestSaveCache)>>,
    /// A record whose delete button was clicked once (the second click deletes it).
    delete_armed: Option<std::path::PathBuf>,
    /// The page drawn last (a new one slides in from the side the player went).
    shown: Page,
    /// A page has just come into view: its list scrolls to the entry chosen (once).
    reveal: bool,
    /// The title menu: the frame it was last drawn, and when it came into view (its entries
    /// build up from then).
    title_pass: u64,
    title_since: f64,
    /// The title menu's height last frame, and when last shown in full (whether every entry
    /// keeps the line under its name).
    title_menu_height: f32,
    title_full_height: f32,
    /// What the title menu lists, and where its entries slide in from (the side the player
    /// went: the right going into a group, the left coming back).
    pub title_menu: TitleMenu,
    title_from: f32,
    /// Saved games or replays: which the records page lists.
    records_kind: crate::replay::RecordKind,
    /// A gamepad is in use (the title's footer says how it works the menus).
    pub pad: bool,
    /// The records page's search (by name or map) and its list's order.
    records_filter: String,
    records_sort: RecordSort,
    /// DELETE ALL clicked: the files it would remove (those shown), while it asks.
    delete_all_asked: Option<Vec<std::path::PathBuf>>,
    /// What the credits' film shows over the picture this frame (`credits_show.rs`), when it plays.
    pub credits: Option<crate::credits_show::Showing>,
    /// FROM THE TOP clicked: the film starts again (the app takes it); and the right arrow pressed:
    /// it goes on to the next act.
    pub credits_restart: bool,
    pub credits_skip: bool,
    /// TA Reborn's closing words, which the film shows over its emblem.
    pub credits_disclaimer: Option<String>,
    /// The unit encyclopaedia.
    pub encyclopaedia: crate::encyclopaedia::Encyclopaedia,
    /// Units' portraits for the pages (the Archives', Conquest's veterans).
    pub portraits: crate::encyclopaedia::Portraits,
    /// The timeline of the TA universe.
    timeline: crate::timeline::Timeline,
    /// What the film behind the menu shows this frame (`backdrop.rs`), when one plays.
    pub film: Option<crate::backdrop::Showing>,
    /// How far the title screen is in (0..1): back from a page, its lockup, menu and footer come
    /// in as the page goes.
    title_fade: f32,
}

/// The black bars the film plays between on its own (2.39:1, none in a window as wide), coming in
/// from the window's edges as it steps forward (`watch`, 0..1; with reduced motion, fading in).
fn film_bars(ui: &Ui, screen: Rect, watch: f32) {
    let full = ((screen.height() - screen.width() / 2.39) / 2.0).max(0.0);
    if full < 1.0 {
        return;
    }
    let ease = egui::emath::easing::cubic_in_out(watch.clamp(0.0, 1.0));
    let still = ta_ui::motion::still(ui.ctx());
    let (height, alpha) = if still { (full, ease) } else { (full * ease, 1.0) };
    let black = Color32::from_black_alpha((alpha * 255.0).round() as u8);
    let painter = ui.painter();
    painter.rect_filled(Rect::from_min_max(screen.min, pos2(screen.max.x, screen.min.y + height)), 0.0, black);
    painter.rect_filled(Rect::from_min_max(pos2(screen.min.x, screen.max.y - height), screen.max), 0.0, black);
}

/// The film's caption at the window's foot, on the right: the world's name in brass capitals,
/// a rule before it, over the moment, rising a little as it fades in (`alpha`); lower while the
/// film plays on its own (`watch`), in the bar under the picture, the footer gone. `s` times its
/// own size ([`title_scale`]).
fn film_caption(ui: &Ui, screen: Rect, world: &str, moment: &str, alpha: f32, watch: f32, s: f32) {
    let still = ta_ui::motion::still(ui.ctx());
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(alpha.clamp(0.0, 1.0));
    let name = w::galley(ui, RichText::new(world.to_uppercase()).font(display(15.0 * s)).extra_letter_spacing(ta_ui::i18n::tracking(4.0 * s)));
    let line = w::galley(ui, RichText::new(t(moment)).font(body(13.5 * s)));
    let right = screen.max.x - 26.0 * s;
    let rise = if still { 0.0 } else { (1.0 - alpha) * 6.0 * s };
    let foot = screen.max.y - egui::lerp(52.0..=30.0, watch.clamp(0.0, 1.0)) * s + rise;
    let line_at = pos2((right - line.size().x).round(), (foot - line.size().y).round());
    let name_at = pos2((right - name.size().x).round(), (line_at.y - name.size().y - s).round());
    let left = line_at.x.min(name_at.x - 40.0 * s);
    painter.add(panel::scrim(Rect::from_min_max(pos2(left - 6.0, name_at.y - 2.0), pos2(right + 6.0, foot + 2.0))));
    let rule_at = pos2(name_at.x - 38.0 * s, name_at.y + name.size().y * 0.5 - 1.5 * s).round();
    let rule = Rect::from_min_size(rule_at, (vec2(28.0, 3.0) * s).round());
    painter.rect_filled(rule.expand(1.0), CornerRadius::same(1), ta_ui::metal::STEEL_EDGE);
    painter.add(ta_ui::metal::vgrad(rule, AMBER_BRIGHT, AMBER_DIM));
    ta_ui::metal::outlined(&painter, name_at, name, AMBER);
    ta_ui::metal::outlined(&painter, line_at, line, TEXT);
}

/// While the film plays on its own, at first: how to bring the menu back, in the bar over the
/// picture's foot, on the left.
fn film_hint(ui: &Ui, screen: Rect, alpha: f32, s: f32) {
    film_hint_with(ui, screen, t("Any key, click or move brings the menu back"), alpha, s);
}

/// A hint in the bar over the picture's foot, on the left.
fn film_hint_with(ui: &Ui, screen: Rect, words: &str, alpha: f32, s: f32) {
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(alpha.clamp(0.0, 1.0));
    let text = w::galley(ui, RichText::new(words).font(body(12.5 * s)));
    let at = pos2(screen.min.x + TITLE_LEFT * s, (screen.max.y - 30.0 * s - text.size().y).round());
    ta_ui::metal::outlined(&painter, at, text, STEEL);
}

/// The lockup's shine: once every this many seconds (the first this long after the title comes
/// into view), a band of light this long crossing the name.
const SHINE_EVERY: f32 = 12.0;
const SHINE_FIRST: f32 = 2.5;
const SHINE_LENGTH: f32 = 1.6;

/// A soft band of light leaning a little, `t` (0..1) of the way across `name` (drawn at `at`), as
/// light catches polished metal: the name drawn again in white through narrow clips, brightest in
/// the band's middle, on the steel's own letters only.
fn shine(painter: &egui::Painter, at: egui::Pos2, name: &std::sync::Arc<egui::Galley>, t: f32) {
    const ACROSS: usize = 12;
    const DOWN: usize = 4;
    let rect = Rect::from_min_size(at, name.size());
    let width = rect.height() * 0.7;
    let middle = rect.min.x - width + (rect.width() + 2.0 * width) * t;
    let (strip, row) = (width / ACROSS as f32, rect.height() / DOWN as f32);
    for r in 0..DOWN {
        // (Leaning right: higher rows a little further on.)
        let lean = (DOWN as f32 / 2.0 - r as f32 - 0.5) * row * 0.35;
        for i in 0..ACROSS {
            let u = (i as f32 + 0.5) / ACROSS as f32 * 2.0 - 1.0;
            let a = (1.0 - u * u).powi(2);
            let x = middle + lean + u * width * 0.5 - strip * 0.5;
            let clip = Rect::from_min_max(pos2(x, rect.min.y + r as f32 * row), pos2(x + strip + 0.5, rect.min.y + (r + 1) as f32 * row + 0.5));
            painter.with_clip_rect(clip.intersect(painter.clip_rect())).galley_with_override_text_color(at, name.clone(), Color32::from_white_alpha((a * 170.0) as u8));
        }
    }
}

/// The version line at the end of the title's footer: the version in steel and the build's
/// commit after it, dimmer, straight on the battle (no box: it is small print, not a control).
fn version_line(ui: &mut Ui) -> Response {
    let label = crate::version_label();
    let (version, commit) = match label.split_once(" (") {
        Some((v, c)) => (v.to_string(), Some(c.trim_end_matches(')').to_string())),
        None => (label.clone(), None),
    };
    let version = w::galley(ui, RichText::new(format!("v{version}")).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.6)));
    let commit = commit.map(|c| w::galley(ui, RichText::new(c).font(body(11.0))));
    let gap = 6.0;
    let width = 14.0 + version.size().x + commit.as_ref().map_or(0.0, |c| gap + c.size().x);
    let height = version.size().y.max(commit.as_ref().map_or(0.0, |c| c.size().y));
    let (rect, response) = ui.allocate_exact_size(vec2(width, height + 2.0), Sense::hover());
    let painter = ui.painter();
    // (A thin seam between the links before it and the line.)
    painter.vline(rect.min.x + 3.0, (rect.min.y + 3.0)..=(rect.max.y - 3.0), Stroke::new(1.0, STEEL_DIM.gamma_multiply(0.5)));
    let version_at = pos2(rect.min.x + 14.0, rect.center().y - version.size().y / 2.0).round();
    let after = version_at.x + version.size().x + gap;
    ta_ui::metal::outlined(painter, version_at, version, STEEL);
    if let Some(commit) = commit {
        ta_ui::metal::outlined(painter, pos2(after, rect.center().y - commit.size().y / 2.0).round(), commit, STEEL_DIM);
    }
    response.on_hover_text(t("The version of TA Reborn and the build it was made from: give both when reporting a problem"))
}

/// The numerals of the acts.
fn roman(n: usize) -> String {
    const NUMERALS: [(usize, &str); 5] = [(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")];
    let (mut n, mut out) = (n, String::new());
    for (value, text) in NUMERALS {
        while n >= value {
            out.push_str(text);
            n -= value;
        }
    }
    out
}

/// The card between acts, on the black: the act's number in brass, the world's name in steel, the
/// moment under it.
fn chapter_card(ui: &Ui, screen: Rect, chapter: &crate::credits_show::Chapter) {
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(chapter.alpha.clamp(0.0, 1.0));
    let still = ta_ui::motion::still(ui.ctx());
    let rise = if still { 0.0 } else { (1.0 - chapter.alpha) * 10.0 };
    let middle = screen.center().x;
    let top = screen.min.y + screen.height() * 0.4 + rise;
    let number = w::galley(ui, RichText::new(roman(chapter.number)).font(display(26.0)).extra_letter_spacing(ta_ui::i18n::tracking(8.0)));
    let world = painter.layout_no_wrap(chapter.world.to_uppercase(), display(if screen.height() < 700.0 { 40.0 } else { 56.0 }), Color32::WHITE);
    let moment = w::galley(ui, RichText::new(t(chapter.moment)).font(body(18.0)));
    let number_at = pos2((middle - number.size().x / 2.0).round(), top.round());
    ta_ui::metal::outlined(&painter, number_at, number, AMBER);
    let rule = Rect::from_center_size(pos2(middle, top + 44.0), vec2(56.0, 3.0));
    painter.rect_filled(rule.expand(1.0), CornerRadius::same(1), ta_ui::metal::STEEL_EDGE);
    painter.add(ta_ui::metal::vgrad(rule, AMBER_BRIGHT, AMBER_DIM));
    let (name_top, name_foot) = ta_ui::metal::cap_band(&world);
    let at = pos2((middle - world.size().x / 2.0).round(), (top + 66.0 - name_top).round());
    let name_height = name_foot - name_top;
    ta_ui::metal::chrome(&painter, at, world);
    ta_ui::metal::outlined(&painter, pos2((middle - moment.size().x / 2.0).round(), (top + 66.0 + name_height + 24.0).round()), moment, TEXT);
}

/// TA Reborn's closing words over the finale's emblem: what it needs, and what it is not.
fn credits_disclaimer(ui: &Ui, screen: Rect, text: Option<&str>, alpha: f32) {
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(alpha.clamp(0.0, 1.0));
    let width = (screen.width() - 160.0).clamp(320.0, 820.0);
    let first = painter.layout(t("TA Reborn needs your own copy of Total Annihilation to play.").to_string(), strong(24.0), AMBER_BRIGHT, width);
    let second = text.map(|text| painter.layout(text.to_string(), body(18.5), TEXT_BRIGHT, width));
    let height = first.size().y + second.as_ref().map_or(0.0, |s| s.size().y + 8.0);
    let top = screen.max.y - 84.0 - height;
    let panel = Rect::from_min_size(pos2(screen.center().x - width / 2.0 - 26.0, top - 16.0), vec2(width + 52.0, height + 32.0));
    painter.add(panel::scrim(panel));
    let first_height = first.size().y;
    painter.galley(pos2(screen.center().x - first.size().x / 2.0, top), first, AMBER_BRIGHT);
    if let Some(second) = second {
        painter.galley(pos2(screen.center().x - second.size().x / 2.0, top + first_height + 8.0), second, TEXT_BRIGHT);
    }
}

/// TA's own last word, on the black at the end.
fn credits_last_word(ui: &Ui, screen: Rect, alpha: f32) {
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(alpha.clamp(0.0, 1.0));
    let thanks = painter.layout_no_wrap(t("Thank you for playing.").to_string(), body(24.0), TEXT_BRIGHT);
    let nice = painter.layout_no_wrap(t("Have a nice day.").to_string(), strong(18.0), AMBER);
    let top = screen.center().y - 34.0;
    let thanks_height = thanks.size().y;
    painter.galley(pos2(screen.center().x - thanks.size().x / 2.0, top), thanks, TEXT_BRIGHT);
    painter.galley(pos2(screen.center().x - nice.size().x / 2.0, top + thanks_height + 12.0), nice, AMBER);
}

impl Menu {
    pub fn new(assets: &Assets) -> Menu {
        let default_map = assets.selectable_maps.iter().position(|m| m.name.eq_ignore_ascii_case("Painted Desert")).unwrap_or(0);
        Menu {
            page: Page::Title,
            campaign: Default::default(),
            conquest: crate::conquest_ui::ConquestPage::loaded(),
            connect: Default::default(),
            lobby: None,
            records: Vec::new(),
            records_loading: None,
            record_loading: None,
            record_error: None,
            record_prompt: None,
            failure: None,
            scenarios_unread: Vec::new(),
            scenarios: crate::scenario::catalogue(assets),
            scenario_selected: 0,
            scenario_difficulty: ta_ai::Difficulty::Normal,
            selected_map: default_map,
            map_choice: Default::default(),
            slots: vec![
                Slot { kind: SlotKind::Human, side: 0, color: 0, team: 0 },
                Slot { kind: SlotKind::AiNormal, side: 1, color: 1, team: 1 },
                Slot { kind: SlotKind::Closed, side: 0, color: 2, team: 2 },
                Slot { kind: SlotKind::Closed, side: 1, color: 3, team: 3 },
            ],
            // A new series of games every session (the computer players draw from it).
            seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64),
            rules: Default::default(),
            latest_save: None,
            saves_checked: None,
            latest_loading: None,
            latest_cache: Default::default(),
            delete_armed: None,
            shown: Page::Title,
            reveal: false,
            title_pass: 0,
            title_since: 0.0,
            title_menu_height: 0.0,
            title_full_height: 0.0,
            title_menu: TitleMenu::Main,
            title_from: -28.0,
            records_kind: crate::replay::RecordKind::Save,
            pad: false,
            records_filter: String::new(),
            records_sort: RecordSort::Newest,
            delete_all_asked: None,
            credits: None,
            credits_restart: false,
            credits_skip: false,
            credits_disclaimer: None,
            encyclopaedia: crate::encyclopaedia::Encyclopaedia::new(assets),
            portraits: Default::default(),
            timeline: Default::default(),
            film: None,
            title_fade: 1.0,
        }
    }

    /// Looks for a newer saved game every few seconds (the folder listing only).
    fn refresh_latest_save(&mut self, assets: &Assets) {
        if let Some(receiver) = &self.latest_loading {
            match receiver.try_recv() {
                Ok((latest, cache)) => { self.latest_save = latest; self.latest_cache = cache; self.latest_loading = None; }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.latest_loading = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
            }
        }
        if self.saves_checked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)) {
            return;
        }
        self.saves_checked = Some(std::time::Instant::now());
        let (send, receive) = std::sync::mpsc::channel();
        let mut cache = std::mem::take(&mut self.latest_cache);
        let assets = assets.clone();
        self.latest_loading = Some(receive);
        std::thread::spawn(move || {
            let latest = crate::replay::Record::latest_save(&assets, &mut cache);
            let _ = send.send((latest, cache));
        });
    }

    fn records_page(&mut self, ui: &mut egui::Ui) -> MenuAction {
        let action = MenuAction::None;
        let ctx = ui.ctx().clone();
        let mut delete = None;
        let mut delete_all = None;
        let mut arm = None;
        let mut skirmish = false;
        let mut open = None;
        let saves = self.records.iter().filter(|(_, r)| r.kind == crate::replay::RecordKind::Save).count();
        let replays = self.records.len() - saves;
        let subtitle = tf("SAVED GAMES: {saves} · REPLAYS: {replays}", &[("saves", &saves), ("replays", &replays)]);
        let page = Panel::new("SAVED GAMES & REPLAYS").subtitle(&subtitle).width(860.0).exit(Exit::Back);
        let response = panel::page(&ctx, "records", page, |ui| {
            // Saved games or replays (the entry that opened the page picks which).
            ui.horizontal(|ui| {
                w::segmented(ui, &mut self.records_kind, &[(crate::replay::RecordKind::Save, "SAVED GAMES"), (crate::replay::RecordKind::Replay, "REPLAYS")]);
            });
            ui.add_space(8.0);
            if self.records_loading.is_some() {
                ui.horizontal(|ui| { ui.spinner(); ui.label(t("Reading saved games and replays…")); });
                return;
            }
            let kind = self.records_kind;
            if !self.records.iter().any(|(_, r)| r.kind == kind) {
                // None yet: what this list is for, and a way to start making some.
                let save = kind == crate::replay::RecordKind::Save;
                let bodies = [ta_ui::i18n::t("Save a battle from its pause menu to carry it on from here later."), ta_ui::i18n::t("Every battle you play is kept here as a replay: watch it again, and jump through it with the bar.")];
                let (heading, body_text) = if save { (ta_ui::i18n::t("NO SAVED GAMES YET"), bodies[0]) } else { (ta_ui::i18n::t("NO REPLAYS YET"), bodies[1]) };
                panel::well(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add_space(22.0);
                    ui.vertical_centered(|ui| {
                        let (emblem, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                        records_emblem(ui.painter(), emblem, save, STEEL_DIM);
                        ui.add_space(10.0);
                        ui.label(RichText::new(heading).font(strong(14.0)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.5)));
                        // (A column of its own: a centred layout's maximum width is no cap.) As
                        // tall as the longer of the two kinds' sentences, so the page keeps its
                        // height when the kind is switched; its lines even.
                        let width = ui.available_width().min(560.0);
                        let text = |body: &str| RichText::new(body).color(STEEL_DIM);
                        let tallest = bodies.iter().map(|b| w::wrapped_galley(ui, text(b), width).size().y).fold(0.0, f32::max);
                        ui.allocate_ui_with_layout(vec2(width, tallest), egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.set_min_height(tallest);
                            w::balanced_label(ui, text(body_text));
                        });
                        ui.add_space(14.0);
                        if ui.add(ActionButton::secondary("START A SKIRMISH").size(210.0, 34.0)).clicked() {
                            skirmish = true;
                        }
                    });
                    ui.add_space(22.0);
                });
                return;
            }
            // Those the search finds, in the order chosen; over them the search, the order
            // and DELETE ALL (those shown).
            let words = self.records_filter.trim().to_lowercase();
            let mut shown: Vec<usize> = self.records.iter().enumerate()
                .filter(|(_, (_, r))| r.kind == kind && (words.is_empty() || r.title.to_lowercase().contains(&words) || r.setup.map.to_lowercase().contains(&words)))
                .map(|(i, _)| i)
                .collect();
            match self.records_sort {
                // (As read.)
                RecordSort::Newest => {}
                RecordSort::Oldest => shown.reverse(),
                RecordSort::Longest => shown.sort_by_key(|&i| std::cmp::Reverse(self.records[i].1.end_tick)),
                RecordSort::Name => shown.sort_by_cached_key(|&i| self.records[i].1.title.to_lowercase()),
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let search = w::search_field(ui, &mut self.records_filter, t("Search by name or map…"), 300.0, None);
                let width = w::combo_width(ui, RecordSort::ALL.iter().map(|s| t(s.label())), 150.0);
                egui::ComboBox::from_id_salt("records_sort").selected_text(t(self.records_sort.label())).width(width).show_ui(ui, |ui| {
                    for sort in RecordSort::ALL {
                        ui.selectable_value(&mut self.records_sort, sort, t(sort.label()));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let hover = if words.is_empty() { t("Delete every one of them") } else { t("Delete every one the search finds") };
                    if ui.add_enabled(!shown.is_empty(), ActionButton::secondary("DELETE ALL").height(search.rect.height())).on_hover_text(hover).clicked() {
                        delete_all = Some(shown.iter().map(|&i| self.records[i].0.clone()).collect::<Vec<_>>());
                    }
                });
            });
            ui.add_space(8.0);
            if shown.is_empty() {
                ui.add(egui::Label::new(RichText::new(t("No battle matches. Try another word, or clear the search.")).color(STEEL)).wrap());
                ui.add_space(4.0);
                if ui.add(ActionButton::secondary("CLEAR")).clicked() {
                    self.records_filter.clear();
                }
                return;
            }
            // (As tall as the window lets it be, whatever the page's height was last frame: an
            // area's contents have that much room, and a list given what is left shrank with it.)
            let height = panel::fit_height(&ctx, 260.0, 160.0, 540.0);
            scroll::show(ui, egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height), Behind::Plate, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (k, &i) in shown.iter().enumerate() {
                    let (path, r) = &self.records[i];
                    let save = r.kind == crate::replay::RecordKind::Save;
                    let Some(verdict) = &r.verdict else { continue };
                    let (hash_matches, simulation_matches) = (verdict.data, verdict.simulation);
                    // (A saved game of another simulation cannot carry on; one of other data or
                    // mods is asked about as it is opened.)
                    let openable = !save || simulation_matches;
                    let compatible = !save || hash_matches && simulation_matches;
                    let approximate = !save && (!hash_matches || !simulation_matches
                        || r.metadata.as_ref().is_none_or(|m| m.engine_build != crate::version_label()));
                    // Its mods: which, when they are as they were; else what is not.
                    let mods_line = match &verdict.mods {
                        Some(check) if hash_matches => (!check.needed.is_empty()).then(|| ta_ui::record_mods::Line {
                            tone: ta_ui::record_mods::Tone::Fine,
                            text: tf("Mods: {mods}", &[("mods", &ta_ui::record_mods::names(&check.needed.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>()))]),
                        }),
                        Some(check) => ta_ui::record_mods::summary(check),
                        None => None,
                    };
                    let armed = self.delete_armed.as_ref() == Some(path);
                    // (The rows come in one after another as the page does.)
                    panel::list_entry(ui, k, |ui| egui::Frame::new()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0, if armed { RED.gamma_multiply(0.6) } else { LINE }))
                        .corner_radius(CornerRadius::same(2))
                        .inner_margin(Margin { left: 12, right: 10, top: 8, bottom: 8 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (label, color) = if save { (ta_ui::i18n::n("SAVED"), GREEN) } else { (ta_ui::i18n::n("REPLAY"), SKY) };
                                // (As wide as the wider of the two tags, in the language in use.)
                                let slot = ["SAVED", "REPLAY"].iter().map(|tag| w::galley(ui, RichText::new(t(tag)).font(strong(10.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 12.0).fold(62.0, f32::max);
                                ui.allocate_ui(vec2(slot, 22.0), |ui| w::tag(ui, label, color));
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&r.title).font(strong(15.0)).color(if compatible { TEXT_BRIGHT } else { STEEL }));
                                    let mut line = tf("{length} long · {age}", &[("length", &r.duration()), ("age", &format_age(r.saved_at))]);
                                    if !openable {
                                        // (Said below.)
                                    } else if approximate && (hash_matches || verdict.mods.is_none()) {
                                        line = format!("{line} · {}", t("Compatibility playback"));
                                    } else if !compatible && mods_line.as_ref().is_none_or(|l| l.tone == ta_ui::record_mods::Tone::Fine) {
                                        line = format!("{line} · {}", t("made with different game data or mods"));
                                    }
                                    ui.label(RichText::new(line).font(body(12.5)).color(STEEL));
                                    if !openable {
                                        ui.add(egui::Label::new(RichText::new(t("Made by an earlier version of the game: it no longer plays")).font(body(12.5)).color(AMBER)).truncate());
                                    } else if let Some(mods) = &mods_line {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            if mods.tone != ta_ui::record_mods::Tone::Fine {
                                                let (rect, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                                                ui.painter().add(ta_ui::metal::lamp(rect.center(), 3.5, mods.tone.color(), 1.0));
                                            }
                                            let color = if mods.tone == ta_ui::record_mods::Tone::Fine { STEEL_DIM } else { mods.tone.color() };
                                            ui.add(egui::Label::new(RichText::new(&mods.text).font(body(12.5)).color(color)).truncate());
                                        });
                                    }
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if armed {
                                        if ui.add(ActionButton::danger("DELETE")).on_hover_text(t("To the Recycle Bin, if this drive has one")).clicked() {
                                            delete = Some(path.clone());
                                        }
                                        if ui.add(ActionButton::secondary("KEEP")).clicked() {
                                            arm = Some(None);
                                        }
                                    } else {
                                        if w::icon_button(ui, w::Glyph::Trash, true).on_hover_text(ta_ui::i18n::t("Delete")).clicked() {
                                            arm = Some(Some(path.clone()));
                                        }
                                        let verb = if save { ta_ui::i18n::n("LOAD") } else { ta_ui::i18n::n("WATCH") };
                                        let button = if save { ActionButton::primary(verb) } else { ActionButton::secondary(verb) };
                                        let hover = if !hash_matches {
                                            t("Made with other mods or game data: you are told what that means before it opens")
                                        } else if approximate {
                                            t("Recorded with different game data or an older engine. AI, physics and outcomes may differ.")
                                        } else { "" };
                                        if ui.add_enabled(openable && self.record_loading.is_none(), button.size(96.0, 32.0))
                                            .on_disabled_hover_text(t("Made by an earlier version of the game: it no longer plays"))
                                            .on_hover_text(hover).clicked() {
                                            open = Some(path.clone());
                                        }
                                    }
                                });
                            });
                        }));
                }
            });
        });
        if response.exit {
            self.delete_armed = None;
            self.delete_all_asked = None;
            self.page = Page::Title;
        }
        if let Some(armed) = arm {
            self.delete_armed = armed;
        }
        if skirmish {
            sound::play(&ctx, UiSound::Skirmish);
            self.delete_armed = None;
            self.page = Page::Skirmish;
        }
        if let Some(path) = delete {
            self.recycle_records(&[path]);
        }
        if let Some(paths) = delete_all {
            self.delete_armed = None;
            self.delete_all_asked = Some(paths);
        }
        if let Some(paths) = self.delete_all_asked.clone() {
            // (How many, and where they go.)
            let save = self.records_kind == crate::replay::RecordKind::Save;
            let (mut go, mut keep) = (false, false);
            let title = if save { n("DELETE THESE SAVED GAMES?") } else { n("DELETE THESE REPLAYS?") };
            let asked = panel::modal(&ctx, "records_delete_all", Panel::new(title).width(480.0).exit(Exit::Close).accent(RED), true, |ui| {
                let count = if save { tf("Saved games to delete: {n}", &[("n", &paths.len())]) } else { tf("Replays to delete: {n}", &[("n", &paths.len())]) };
                ui.label(RichText::new(count).font(strong(14.0)).color(TEXT_BRIGHT));
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(t("They go to the Recycle Bin, if this drive has one.")).color(TEXT)).wrap());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    go = ui.add(ActionButton::danger("DELETE")).clicked();
                    keep = ui.add(ActionButton::secondary("KEEP THEM")).clicked();
                });
            });
            if go {
                self.recycle_records(&paths);
            }
            if go || keep || asked.exit {
                self.delete_all_asked = None;
            }
        }
        if let Some(path) = open { self.load_record(path); }
        action
    }

    /// Saved games or replays to the Recycle Bin (where they can go), and out of the list;
    /// why not, in a dialog.
    fn recycle_records(&mut self, paths: &[std::path::PathBuf]) {
        match ta_ui::recycle::recycle_all(paths) {
            Ok(outcome) => log::info!("{} saved games or replays removed ({outcome:?})", paths.len()),
            Err(error) => self.record_error = Some(tf("Could not delete: {error}", &[("error", &error)])),
        }
        // (Those gone leave the list, whatever was said: some may have gone before a failure.)
        self.records.retain(|(path, _)| path.exists());
        self.delete_armed = None;
        self.saves_checked = None;
    }

    fn load_record(&mut self, path: std::path::PathBuf) {
        if self.record_loading.is_some() { return; }
        let (send, receive) = std::sync::mpsc::channel();
        self.record_loading = Some(receive);
        // (With the mods installed now: what it needs is told against them.)
        std::thread::spawn(move || { let record = crate::replay::Record::read(&path); let _ = send.send((path, record, installed_now())); });
    }

    /// Opens the Conquest page (its state read from disk again).
    pub fn open_conquest(&mut self) {
        self.conquest.open();
        self.page = Page::Conquest;
    }

    /// A profile's allegiance is the initial choice, not a restriction on playable factions.
    pub fn use_profile(&mut self, assets: &Assets, profile: &ta_config::profiles::Profile) {
        self.connect.name = profile.name.clone();
        let allegiance = match profile.allegiance { ta_config::profiles::Allegiance::Arm => "ARM", ta_config::profiles::Allegiance::Core => "CORE" };
        if let Some(side) = crate::setup::side_choices(&assets.data).iter().position(|s| s.key.eq_ignore_ascii_case(allegiance)) {
            if let Some(slot) = self.slots.first_mut() { slot.side = side; }
        }
        self.restore_skirmish(assets);
    }

    /// Jumps straight to the skirmish setup page.
    pub fn open_skirmish(&mut self) {
        self.page = Page::Skirmish;
    }

    /// The skirmish's computer players made Easy (a first battle after the lessons).
    pub fn easy_opponents(&mut self) {
        for slot in &mut self.slots {
            if matches!(slot.kind, SlotKind::AiNormal | SlotKind::AiHard) {
                slot.kind = SlotKind::AiEasy;
            }
        }
    }

    /// Chooses `map` (its name without extension, any case) on the skirmish setup page, if it
    /// is one of the maps on offer; its row is brought into view.
    pub fn choose_skirmish_map(&mut self, assets: &Assets, map: &str) {
        if let Some(index) = assets.selectable_maps.iter().position(|m| m.name.eq_ignore_ascii_case(map.trim())) {
            self.selected_map = index;
            self.reveal = true;
            // (The list shows it: all of them, if it did not.)
            if !self.map_choice.accepts(&assets.selectable_maps[index]) {
                self.map_choice.clear();
            }
        }
    }

    /// Opens the saved games and replays page (read from disk now) on one kind of the two.
    pub fn open_records(&mut self, kind: crate::replay::RecordKind) {
        if self.records_loading.is_none() {
            let (send, receive) = std::sync::mpsc::channel();
            self.records_loading = Some(receive);
            std::thread::spawn(move || {
                let mut records = crate::replay::Record::list(crate::replay::RecordKind::Save);
                records.extend(crate::replay::Record::list(crate::replay::RecordKind::Replay));
                let _ = send.send((records, installed_now()));
            });
        }
        self.records_kind = kind;
        self.page = Page::Records;
    }

    /// Shows the title menu with one of its lists: its entries build up again, sliding in from
    /// the side the player went.
    pub fn show_title_menu(&mut self, menu: TitleMenu) {
        if menu != self.title_menu {
            self.title_from = if menu == TitleMenu::Main { -28.0 } else { 28.0 };
            self.title_menu = menu;
            // (The build-up starts over at the next frame.)
            self.title_pass = 0;
        }
        self.page = Page::Title;
    }

    /// The unit encyclopaedia, from the Archives.
    pub fn open_units(&mut self) {
        self.encyclopaedia.opened();
        self.page = Page::Units;
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets, progress: &ta_config::CampaignProgress, game: &crate::game::Game) -> MenuAction {
        if let Some(receiver) = &self.records_loading {
            match receiver.try_recv() {
                Ok((mut records, installed)) => {
                    for (_, record) in &mut records {
                        record.verdict = Some(record.verdict_now(assets, &installed));
                    }
                    self.records = records;
                    self.records_loading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.records_loading = None;
                    self.record_error = Some(t("Could not read saved games and replays").to_string());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)),
            }
        }
        if let Some(receiver) = &self.record_loading {
            match receiver.try_recv() {
                Ok((path, result, installed)) => {
                    self.record_loading = None;
                    match result {
                        Ok(record) => if let Some(action) = self.offer(path, record, assets, &installed) { return action },
                        Err(error) => self.record_error = Some(tf("This battle could not be opened: {error}", &[("error", &format!("{error:#}"))])),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.record_loading = None;
                    self.record_error = Some(t("Could not read saved games and replays").to_string());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(50)),
            }
        }
        if let Some(error) = &self.record_error {
            let mut close = false;
            let response = panel::modal(ui.ctx(), "record_error", Panel::new("SAVED GAMES AND REPLAYS").width(520.0).exit(Exit::Close).accent(RED), true, |ui| {
                ui.label(error);
                close = ui.add(ActionButton::primary("CLOSE")).clicked();
            });
            if close || response.exit { self.record_error = None; }
        } else if let Some((title, text)) = &self.failure {
            let mut close = false;
            let response = panel::modal(ui.ctx(), "menu_failure", Panel::new(title).width(520.0).exit(Exit::Close).accent(RED), true, |ui| {
                ui.add(egui::Label::new(RichText::new(text.as_str()).color(TEXT)).wrap());
                ui.add_space(8.0);
                close = ui.add(ActionButton::primary("CLOSE")).clicked();
            });
            if close || response.exit { self.failure = None; }
        } else if self.record_loading.is_some() {
            panel::modal(ui.ctx(), "reading_record", Panel::new("Reading battle…").width(360.0), false, |ui| {
                ui.horizontal(|ui| { ui.spinner(); ui.label(t("Reading the battle…")); });
            });
        }
        // A record made with other mods or game data, asked about before it opens.
        let asked = if self.record_error.is_none() && self.failure.is_none() { self.record_prompt_dialog(ui.ctx(), assets) } else { MenuAction::None };
        // The battle behind: shaded where the title's menu stands, dimmed behind a page (not the
        // showroom's world, which is the page's subject; the credits and Conquest's space cover
        // it), each fading into the other as the page comes in.
        let screen = ui.max_rect();
        let ctx = ui.ctx().clone();
        let ease = egui::emath::easing::cubic_out;
        let title = ctx.animate_bool_with_time_and_easing(egui::Id::new("menu_title_shade"), self.page == Page::Title, panel::PAGE_TIME, ease);
        self.title_fade = if ta_ui::motion::reduced(&ctx) { 1.0 } else { title };
        let behind_page = !matches!(self.page, Page::Title | Page::Units | Page::Credits | Page::Conquest);
        let dim = ctx.animate_bool_with_time_and_easing(egui::Id::new("menu_page_dim"), behind_page, panel::PAGE_TIME, ease);
        let watch = self.film.as_ref().map_or(0.0, |f| f.watch);
        if title * (1.0 - watch) > 0.0 {
            panel::vignette(ui.painter(), screen, title * (1.0 - watch));
        }
        if dim > 0.0 {
            panel::page_backdrop(ui.painter(), screen, dim);
        }
        // The film's cut from scene to scene, through black (under the menus).
        if let Some(black) = self.film.as_ref().map(|f| f.black).filter(|&b| b > 0.0) {
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha((black * 255.0).round() as u8));
        }
        // A new page slides in from the right going deeper in, from the left coming back.
        if self.page != self.shown {
            let back = self.page.depth() < self.shown.depth();
            panel::set_direction(ui.ctx(), if back { panel::Direction::Back } else { panel::Direction::Forward });
            self.shown = self.page;
            self.reveal = true;
            self.campaign.reveal = true;
        }
        let action = match self.page {
            Page::Title => self.title(ui, assets, progress, game),
            Page::Conquest => match self.conquest.draw(ui, assets, &mut self.portraits) {
                crate::conquest_ui::ConquestAction::None => MenuAction::None,
                crate::conquest_ui::ConquestAction::Back => {
                    self.page = Page::Title;
                    MenuAction::None
                }
                crate::conquest_ui::ConquestAction::Start(setup) => MenuAction::Start(setup),
            },
            Page::Tutorial => match crate::tutorial::page(ui.ctx(), &progress.tutorials) {
                crate::tutorial::PageAction::None => MenuAction::None,
                crate::tutorial::PageAction::Back => {
                    self.page = Page::Title;
                    MenuAction::None
                }
                crate::tutorial::PageAction::Start(n) => MenuAction::StartTutorial(n),
            },
            Page::Skirmish => self.skirmish(ui, assets),
            Page::Scenarios => self.scenarios_page(ui, assets, progress),
            Page::Campaign => match self.campaign.draw(ui, assets, progress) {
                crate::campaign::CampaignAction::None => MenuAction::None,
                crate::campaign::CampaignAction::Back => {
                    self.page = Page::Title;
                    MenuAction::Narrate(None)
                }
                crate::campaign::CampaignAction::Start(setup) => MenuAction::Start(setup),
                crate::campaign::CampaignAction::Narrate(n) => MenuAction::Narrate(n),
            },
            Page::Multiplayer => match self.connect.draw(ui, assets) {
                crate::lobby::ConnectAction::None => MenuAction::None,
                crate::lobby::ConnectAction::Back => {
                    self.page = Page::Title;
                    MenuAction::None
                }
                crate::lobby::ConnectAction::Lobby(l) => {
                    self.lobby = Some(l);
                    self.page = Page::Lobby;
                    MenuAction::None
                }
            },
            Page::Records => self.records_page(ui),
            Page::Credits => self.credits_page(ui),
            Page::Timeline => match self.timeline.draw(ui, assets, progress) {
                crate::timeline::TimelineAction::None => MenuAction::None,
                crate::timeline::TimelineAction::Back => {
                    self.show_title_menu(TitleMenu::Extras);
                    MenuAction::None
                }
                crate::timeline::TimelineAction::Play { campaign, mission } => {
                    self.campaign.select(assets, &campaign, mission);
                    self.campaign.set_difficulty(&progress.difficulty);
                    self.page = Page::Campaign;
                    MenuAction::None
                }
            },
            Page::Units => match self.encyclopaedia.draw(ui, assets, game, &mut self.portraits) {
                crate::encyclopaedia::Action::None => MenuAction::None,
                crate::encyclopaedia::Action::Back => {
                    self.show_title_menu(TitleMenu::Extras);
                    MenuAction::LeaveShowroom
                }
                crate::encyclopaedia::Action::Show { unit, map, hours, act } => MenuAction::Show { unit, map, hours, act },
                crate::encyclopaedia::Action::Wallpaper { width, height, words } => MenuAction::Wallpaper { width, height, words },
            },
            Page::Lobby => {
                let Some(lobby) = self.lobby.as_mut() else {
                    self.page = Page::Multiplayer;
                    return MenuAction::None;
                };
                match lobby.draw(ui, assets) {
                    crate::lobby::LobbyAction::None => MenuAction::None,
                    crate::lobby::LobbyAction::Leave => {
                        self.lobby = None;
                        self.page = Page::Multiplayer;
                        MenuAction::None
                    }
                    crate::lobby::LobbyAction::Closed(why) => {
                        self.lobby = None;
                        self.connect.tell(why);
                        self.page = Page::Multiplayer;
                        MenuAction::None
                    }
                    crate::lobby::LobbyAction::Start(setup, session) => {
                        self.lobby = None;
                        self.page = Page::Title;
                        MenuAction::StartNet(setup, session)
                    }
                }
            }
        };
        if matches!(asked, MenuAction::None) { action } else { asked }
    }

    /// The credits' page: the film plays behind it (`credits_show.rs`); this is what it shows over
    /// the picture, and how it is left (Escape, BACK).
    fn credits_page(&mut self, ui: &mut Ui) -> MenuAction {
        let screen = ui.max_rect();
        let leave_key = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.credits_skip = true;
        }
        match self.credits.clone() {
            Some(s) => {
                let painter = ui.painter().clone();
                if s.black > 0.0 {
                    painter.rect_filled(screen, 0.0, Color32::from_black_alpha((s.black.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
                if let Some(chapter) = &s.chapter {
                    chapter_card(ui, screen, chapter);
                }
                if let Some((world, moment, alpha)) = &s.caption {
                    film_caption(ui, screen, world, moment, *alpha, 0.0, 1.0);
                }
                if s.disclaimer > 0.0 {
                    credits_disclaimer(ui, screen, self.credits_disclaimer.as_deref(), s.disclaimer);
                }
                if s.last_word > 0.0 {
                    credits_last_word(ui, screen, s.last_word);
                }
                if s.hint > 0.0 {
                    film_hint_with(ui, screen, t("Esc leaves the credits, the right arrow skips ahead"), s.hint, 1.0);
                }
                // How far the film has got: a thin line along the window's foot.
                if s.progress > 0.0 {
                    let line = Rect::from_min_size(pos2(screen.min.x, screen.max.y - 3.0), vec2(screen.width() * s.progress, 3.0));
                    painter.rect_filled(line, 0.0, AMBER_DIM.gamma_multiply(0.7));
                }
                if s.buttons {
                    let again = egui::Area::new(egui::Id::new("credits_again")).anchor(Align2::CENTER_BOTTOM, [0.0, -56.0]).show(ui.ctx(), |ui| ui.add(ActionButton::secondary("FROM THE TOP")).clicked()).inner;
                    if again {
                        self.credits_restart = true;
                    }
                }
            }
            None => {
                ui.painter().rect_filled(screen, 0.0, Color32::BLACK);
            }
        }
        // Back, top right; Escape too.
        let back = egui::Area::new(egui::Id::new("credits_back")).anchor(Align2::RIGHT_TOP, [-28.0, 24.0]).show(ui.ctx(), |ui| ta_ui::panel::back_button(ui, "BACK").on_hover_text(t("Back (Esc)")).clicked()).inner;
        if back || leave_key {
            self.page = Page::Title;
            self.credits = None;
        }
        MenuAction::None
    }

    fn title(&mut self, ui: &mut egui::Ui, assets: &Assets, progress: &ta_config::CampaignProgress, game: &crate::game::Game) -> MenuAction {
        let mut action = MenuAction::None;
        let screen = ui.max_rect();
        // The film playing on its own: the menu stepped aside (`shown` of it left), black bars
        // above and below the picture, its caption in the lower one.
        let film = self.film.clone();
        let watch = film.as_ref().map_or(0.0, |f| f.watch);
        // (The menu gone before the bars are in: no ghost of it over the picture.)
        let shown = 1.0 - egui::emath::easing::cubic_out((watch / 0.6).clamp(0.0, 1.0));
        if watch > 0.0 {
            film_bars(ui, screen, watch);
        }
        // Short windows get a smaller lockup and menu (so they never meet); tall ones a larger
        // (in proportion: see `title_scale`).
        let compact = screen.height() < 1000.0;
        let s = title_scale(screen.height());
        if let Some((world, moment, alpha)) = film.as_ref().and_then(|f| f.caption) {
            film_caption(ui, screen, world, moment, alpha, watch, s);
        }
        if let Some(hint) = film.as_ref().map(|f| f.hint).filter(|&h| h > 0.0) {
            film_hint(ui, screen, hint, s);
        }
        if shown < 0.02 {
            // (Nothing of the menu to click while it is away: the first key or click brings it
            // back.)
            self.title_pass = 0;
            return action;
        }
        // (Back from a page, the lockup and the menu come in as the page goes, with the shade
        // behind them.)
        let fade = self.title_fade;
        // Title lockup, top left over the live battle.
        let left = screen.min.x + TITLE_LEFT * s;
        let (big, small) = if compact { (60.0, 20.0) } else { (84.0 * s, 27.0 * s) };
        // The name in polished steel, as TA's logo is; "REBORN" in brass under it.
        let mut faded = ui.painter().clone();
        faded.multiply_opacity(shown * fade);
        let painter = &faded;
        let name = painter.layout_no_wrap(ta_ui::i18n::t("TOTAL ANNIHILATION").to_owned(), display(big), Color32::WHITE);
        // (Placed by their capitals: the name's a margin under the screen's top, REBORN's a
        // gap under the name's.)
        let (name_top, name_foot) = ta_ui::metal::cap_band(&name);
        let at = pos2(left, (screen.min.y + if compact { 39.0 } else { 57.0 * s } - name_top).round());
        for (dx, dy, a) in [(0.0, 5.0, 90), (3.0, 9.0, 45)] {
            painter.galley_with_override_text_color(at + vec2(dx, dy) * s, name.clone(), Color32::from_black_alpha(a));
        }
        ta_ui::metal::chrome(painter, at, name.clone());
        // Now and then a soft band of light sweeps across the steel, as light catches polished
        // metal turned a little (none with reduced motion).
        if !ta_ui::motion::still(ui.ctx()) {
            let since = (ui.input(|i| i.time) - self.title_since) as f32 - SHINE_FIRST;
            if since >= 0.0 && since.rem_euclid(SHINE_EVERY) < SHINE_LENGTH {
                shine(painter, at, &name, egui::emath::easing::cubic_in_out(since.rem_euclid(SHINE_EVERY) / SHINE_LENGTH));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        let reborn = w::galley(ui, RichText::new("REBORN").font(display(small)).extra_letter_spacing(if compact { 7.0 } else { 9.0 * s }));
        let reborn_height = reborn.size().y;
        let y = (at.y + name_foot + if compact { 28.0 } else { 38.0 * s } - ta_ui::metal::cap_band(&reborn).0).round();
        let k = if compact { 1.0 } else { s };
        let bar = Rect::from_min_size(pos2(left + 3.0 * k, (y + reborn.size().y / 2.0 - 2.0 * k).round()), vec2(44.0 * k, 4.0 * k).round());
        // (REBORN and its rule on a soft dark halo of their own shapes: brass reads over a lava
        // world's bright melt as over the night, with no pool's edge to show on bright fog.)
        let reborn_at = pos2((left + 60.0 * k).round(), y);
        let radius = if compact { 6.0 } else { 8.0 * s };
        ta_ui::metal::halo(painter, radius, 2.2, |d, c| egui::Shape::galley_with_override_text_color(reborn_at + d, reborn.clone(), c));
        ta_ui::metal::halo(painter, radius, 2.2, |d, c| egui::Shape::rect_filled(bar.expand(1.0).translate(d), CornerRadius::same(1), c));
        painter.rect_filled(bar.expand(1.0), CornerRadius::same(1), ta_ui::metal::STEEL_EDGE);
        painter.add(ta_ui::metal::vgrad(bar, AMBER_BRIGHT, AMBER_DIM));
        ta_ui::metal::embossed(painter, reborn_at, reborn, AMBER);

        // The entries build up one after another whenever the menu comes into view, or shows
        // another of its lists.
        let pass = ui.ctx().cumulative_pass_nr();
        if self.title_pass + 1 < pass {
            self.title_since = ui.input(|i| i.time);
        }
        self.title_pass = pass;
        // The menu stands a little below the title, its first entry in the same place whatever
        // it lists; when every entry's line under its name would reach too low, only the
        // essential ones keep theirs.
        let foot = screen.max.y - if compact { 44.0 } else { 64.0 * s };
        let top = y + reborn_height + if compact { 24.0 } else { 44.0 * s };
        let dense = self.title_full_height > foot - top;
        let mut items = MenuItems { since: self.title_since, from: vec2(self.title_from, 0.0), compact, scale: s, dense, count: 0, first: None };
        let gap = s * if dense { 4.0 } else { 12.0 };
        // Escape (or a click on the group's name) goes back from a group to the main entries.
        if self.title_menu != TitleMenu::Main && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            sound::play(ui.ctx(), UiSound::Back);
            self.show_title_menu(TitleMenu::Main);
        }
        // (The newest saved game: Continue picks it up, and a player with one is no newcomer.)
        self.refresh_latest_save(assets);
        let list = self.title_menu;
        let menu = egui::Area::new(egui::Id::new("title_menu")).anchor(Align2::LEFT_TOP, [TITLE_LEFT * s, top - screen.min.y]).show(ui.ctx(), |ui| {
            ui.multiply_opacity(shown * fade);
            ui.set_width(title_width(compact) * s);
            ui.spacing_mut().item_spacing.y = if compact { 5.0 } else { 6.0 * s };
            match list {
                TitleMenu::Main => self.main_entries(ui, &mut items, gap, progress, &mut action),
                TitleMenu::SinglePlayer => self.single_player_entries(ui, &mut items, gap, assets, progress),
                TitleMenu::Extras => self.extras_entries(ui, &mut items, gap, game, &mut action),
            }
            // The arrow keys start on the first entry when nothing has the keyboard (then
            // move between entries; Enter picks one).
            let arrows = ui.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowUp));
            if arrows && ui.ctx().memory(|m| m.focused().is_none()) && let Some(first) = &items.first {
                first.request_focus();
            }
        });
        let height = menu.response.rect.height();
        if (height - self.title_menu_height).abs() > 0.5 {
            self.title_menu_height = height;
            if !dense {
                self.title_full_height = height;
            }
            ui.ctx().request_repaint();
        }
        // The footer: the film's WATCH, the jukebox and the version, on a soft dark pool, the
        // words rimmed in black, so they read over whatever passes behind.
        let pad = self.pad;
        egui::Area::new(egui::Id::new("title_footer")).anchor(Align2::RIGHT_BOTTOM, [-24.0, -18.0]).show(ui.ctx(), |ui| {
            ui.multiply_opacity(shown * fade);
            let pool = ui.painter().add(egui::Shape::Noop);
            let row = ui.horizontal(|ui| {
                if pad {
                    // (A gamepad in use: how it works the menus.)
                    ui.label(RichText::new(t("D-pad: move · A: choose · B: back")).font(strong(12.0)).color(STEEL));
                    ui.add_space(10.0);
                }
                if film.is_some() && w::footer_link_with(ui, ta_ui::i18n::n("Watch"), w::FooterGlyph::Play).on_hover_text(ta_ui::i18n::t("The menu steps aside and the film plays on its own; any key, click or move brings the menu back")).clicked() {
                    action = MenuAction::Watch;
                }
                ui.add_space(6.0);
                if w::footer_link(ui, ta_ui::i18n::n("Jukebox")).on_hover_text(ta_ui::i18n::t("Jukebox: TA's music and your own")).clicked() {
                    action = MenuAction::Jukebox;
                }
                ui.add_space(6.0);
                if !crate::mod_names().is_empty() {
                    w::tag(ui, &tf("MOD · {names}", &[("names", &crate::mod_names().join(", ").to_uppercase())]), AMBER);
                }
                if let Some(problem) = crate::mod_problem() {
                    w::tag(ui, t("MODS NOT LOADED"), RED).on_hover_text(tf("The mods that are on could not be loaded together, so this is the original game: {problem}. Put it right in the launcher's load order.", &[("problem", &problem)]));
                }
                version_line(ui);
            });
            ui.painter().set(pool, panel::scrim(row.response.rect.expand2(vec2(16.0, 6.0))));
        });
        action
    }

    /// A player new to the game: nothing played, nothing saved.
    fn newcomer(&self, progress: &ta_config::CampaignProgress) -> bool {
        progress.tutorials.is_empty() && progress.completed.values().all(|&n| n == 0) && progress.scenarios.is_empty() && self.latest_save.is_none()
    }

    /// The main entries: the last saved game to pick up, playing alone or with others, the
    /// extras, the options, and leaving.
    fn main_entries(&mut self, ui: &mut Ui, items: &mut MenuItems, gap: f32, progress: &ta_config::CampaignProgress, action: &mut MenuAction) {
        // Pick up the newest saved game where it was left.
        if let Some((path, time, title)) = self.latest_save.clone() {
            let age = time.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if items.menu_item_beacon(ui, "CONTINUE", Some(&tf("{title} · saved {age}", &[("title", &title), ("age", &format_age(age))])), true, false).clicked() {
                self.load_record(path);
            }
        }
        // A player new to the game: playing alone lit as the way in (the tutorial is lit in it).
        let newcomer = self.newcomer(progress);
        let detail = if newcomer { t("New to Total Annihilation? Start with the tutorial") } else { t("Skirmish · campaign · Conquest · scenarios") };
        if items.menu_item_beacon(ui, "SINGLE PLAYER", Some(detail), true, newcomer).clicked() {
            self.show_title_menu(TitleMenu::SinglePlayer);
        }
        if items.menu_item(ui, "MULTIPLAYER", None, true).clicked() {
            sound::play(ui.ctx(), UiSound::Multiplayer);
            self.page = Page::Multiplayer;
        }
        if items.menu_item(ui, "EXTRAS", Some(t("Units · the galactic war · replays · map editor")), true).clicked() {
            self.show_title_menu(TitleMenu::Extras);
        }
        if items.menu_item(ui, "OPTIONS", None, true).clicked() {
            sound::play(ui.ctx(), UiSound::Options);
            *action = MenuAction::Options;
        }
        ui.add_space(gap);
        if items.menu_item(ui, "QUIT", None, true).clicked() {
            sound::play(ui.ctx(), UiSound::Exit);
            *action = MenuAction::Quit;
        }
    }

    /// Playing alone: a skirmish, the campaigns, Conquest, the scenarios, the tutorial, and the
    /// saved games.
    fn single_player_entries(&mut self, ui: &mut Ui, items: &mut MenuItems, gap: f32, assets: &Assets, progress: &ta_config::CampaignProgress) {
        if items.group_heading(ui, "SINGLE PLAYER").clicked() {
            self.show_title_menu(TitleMenu::Main);
            return;
        }
        if items.menu_item(ui, "SKIRMISH", None, true).clicked() {
            sound::play(ui.ctx(), UiSound::Skirmish);
            self.page = Page::Skirmish;
        }
        // Where the campaign would pick up.
        let campaign = assets.campaigns.get(self.campaign.campaign).map(|c| {
            let next = progress.unlocked(&c.id).min(c.missions.len().saturating_sub(1));
            tf("{campaign} · mission {n} of {total}", &[("campaign", &crate::campaign::dashes(&c.title)), ("n", &(next + 1)), ("total", &c.missions.len())])
        });
        if items.menu_item(ui, "CAMPAIGN", campaign.as_deref(), !assets.campaigns.is_empty()).clicked() {
            // Resume at the furthest unlocked mission of the current campaign.
            if let Some(c) = assets.campaigns.get(self.campaign.campaign) {
                let next = progress.unlocked(&c.id).min(c.missions.len().saturating_sub(1));
                let id = c.id.clone();
                self.campaign.select(assets, &id, next);
            }
            self.campaign.set_difficulty(&progress.difficulty);
            self.page = Page::Campaign;
        }
        let conquest = crate::conquest_ui::menu_detail(self.conquest.state.as_ref());
        if items.menu_item(ui, "CONQUEST", Some(&conquest), true).clicked() {
            self.open_conquest();
        }
        let played = self.scenarios.iter().filter(|e| progress.scenarios.get(&e.scenario.id).is_some_and(|r| !r.won.is_empty() || !r.best.is_empty())).count();
        let detail = tf("{n} battles with a goal of their own · {done} done", &[("n", &self.scenarios.len()), ("done", &played)]);
        if items.menu_item(ui, "SCENARIOS", Some(&detail), !self.scenarios.is_empty()).clicked() {
            sound::play(ui.ctx(), UiSound::Skirmish);
            self.open_scenarios(assets);
        }
        let lessons = crate::tutorial::LESSONS.len();
        let done = progress.tutorials.iter().filter(|&&d| (d as usize) < lessons).count();
        let newcomer = self.newcomer(progress);
        let detail = if newcomer {
            tf("New to Total Annihilation? Start here: {lessons} short lessons", &[("lessons", &lessons)])
        } else if done == lessons {
            t("All lessons complete").to_string()
        } else {
            tf("{lessons} short lessons · {done} done", &[("lessons", &lessons), ("done", &done)])
        };
        if items.menu_item_beacon(ui, "TUTORIAL", Some(&detail), true, newcomer).clicked() {
            self.page = Page::Tutorial;
        }
        ui.add_space(gap);
        if items.menu_item(ui, "LOAD GAME", None, true).clicked() {
            self.open_records(crate::replay::RecordKind::Save);
        }
        ui.add_space(gap);
        if items.back(ui).clicked() {
            self.show_title_menu(TitleMenu::Main);
        }
    }

    /// The rest: the units in their worlds, the galactic war, replays, the map editor, the
    /// credits and the intro.
    fn extras_entries(&mut self, ui: &mut Ui, items: &mut MenuItems, gap: f32, game: &crate::game::Game, action: &mut MenuAction) {
        if items.group_heading(ui, "EXTRAS").clicked() {
            self.show_title_menu(TitleMenu::Main);
            return;
        }
        let units = game.sim.types.types.iter().filter(|t| !t.def.side.trim().is_empty() && !t.def.display_name.trim().is_empty()).count();
        if items.menu_item(ui, "UNITS", Some(&tf("{n} units, each in a world of your choosing", &[("n", &units)])), true).clicked() {
            self.open_units();
        }
        if items.menu_item(ui, "THE GALACTIC WAR", Some(t("The war's chronicle, its archive and its campaigns")), true).clicked() {
            self.page = Page::Timeline;
        }
        if items.menu_item(ui, "REPLAYS", None, true).clicked() {
            self.open_records(crate::replay::RecordKind::Replay);
        }
        if items.menu_item(ui, "MAP EDITOR", None, true).clicked() {
            *action = MenuAction::Editor;
        }
        if items.menu_item(ui, "CREDITS", None, true).clicked() {
            self.page = Page::Credits;
        }
        if items.menu_item(ui, "WATCH THE INTRO", None, true).clicked() {
            *action = MenuAction::PlayIntro;
        }
        ui.add_space(gap);
        if items.back(ui).clicked() {
            self.show_title_menu(TitleMenu::Main);
        }
    }

    fn skirmish(&mut self, ui: &mut egui::Ui, assets: &Assets) -> MenuAction {
        let mut action = MenuAction::None;
        self.selected_map = self.selected_map.min(assets.selectable_maps.len().saturating_sub(1));
        let ctx = ui.ctx().clone();
        let max_players = assets.selectable_maps.get(self.selected_map).map_or(2, |m| m.start_positions.min(10));
        // The seats in view (as many as the map has starts, two at least) each a colour of
        // their own: a seat back in view on a map with more starts, or a skirmish kept from
        // before, may share one.
        let seats_shown = self.slots.len().min(max_players.max(2));
        settle_colours(&mut self.slots[..seats_shown], None);
        let page = Panel::new("SKIRMISH").subtitle("CHOOSE A BATTLEFIELD AND YOUR OPPONENTS").width(1100.0).exit(Exit::Back);
        // The players' column's foot (START BATTLE's) as it was laid out last, below the columns'
        // top: the map list reaches down level with it. (Measured from the columns' top, not the
        // screen's: the page stands in the middle of the screen by its height, which the list's
        // makes, so a foot on the screen moved with every height the page took, and the page
        // took four layouts to settle.)
        let foot_id = egui::Id::new("skirmish_column_foot");
        let foot = ctx.data(|d| d.get_temp::<f32>(foot_id));
        let reveal = std::mem::take(&mut self.reveal);
        let response = panel::page(&ctx, "skirmish", page, |ui| {
            let columns_top = ui.cursor().top();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                // Map list.
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let maps = &assets.selectable_maps;
                    let shown = self.map_choice.shown(maps);
                    let title = if self.map_choice.narrowed() {
                        tf("MAPS · {shown} OF {n}", &[("shown", &shown.len()), ("n", &maps.len())])
                    } else {
                        tf("MAPS · {n}", &[("n", &maps.len())])
                    };
                    panel::section(ui, &title);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let search = w::search_field(ui, &mut self.map_choice.words, t("Search maps and planets…"), 300.0 - 6.0 - 32.0, None);
                        // (A map at random, of those with room for the players.)
                        let size = vec2(32.0, search.rect.height());
                        if w::glyph_button(ui, w::Glyph::Shuffle, size, w::Tone::Secondary, maps.len() > 1).on_hover_text(t("A map at random, with room for these players")).clicked() {
                            self.random_map(assets);
                        }
                    });
                    ui.add_space(4.0);
                    // Where the maps come from, and their order (the map chosen brought into
                    // view after a change, where the list shows it).
                    if self.map_choice.controls(ui, "skirmish_maps", maps, 300.0) {
                        self.reveal = true;
                    }
                    ui.add_space(4.0);
                    panel::well(ui, |ui| {
                        // Down to the foot (the well's margin under it), in whole rows; the
                        // well keeps on to the foot under the last of them.
                        let room = foot.map_or_else(|| panel::fit_height(&ctx, 330.0, 260.0, 468.0), |foot| (columns_top + foot - 6.0 - ui.cursor().top()).max(260.0));
                        ui.set_min_height(room);
                        let height = scroll::whole_rows(room, panel::LIST_ROW + 1.0, 1.0);
                        let area = egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
                        scroll::show(ui, area, Behind::Well, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            // (As chosen this frame.)
                            let shown = self.map_choice.shown(maps);
                            if shown.is_empty() && self.map_choice.narrowed() {
                                ui.add_space(8.0);
                                ui.add(egui::Label::new(RichText::new(t("No map matches. Try another word, or clear the search.")).color(STEEL)).wrap());
                                if ui.add(ActionButton::secondary("CLEAR")).clicked() {
                                    self.map_choice.clear();
                                }
                            }
                            let by_origin = self.map_choice.order == crate::map_list::Order::Origin;
                            let mut origin = None;
                            for &i in &shown {
                                let m = &maps[i];
                                // (Put in order by origin: each origin's name over its maps.)
                                if by_origin && origin != Some(m.origin) {
                                    origin = Some(m.origin);
                                    crate::map_list::heading(ui, m.origin, shown.iter().filter(|&&j| maps[j].origin == m.origin).count());
                                }
                                let players = RichText::new(format!("{}P", m.start_positions)).font(strong(11.5));
                                let row = crate::map_list::row(ui, m, i == self.selected_map, players);
                                if reveal && i == self.selected_map {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if row.clicked() {
                                    self.selected_map = i;
                                }
                            }
                        });
                    });
                });
                // Map details.
                ui.vertical(|ui| {
                    ui.set_width(340.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    panel::section(ui, "BATTLEFIELD");
                    let Some(m) = assets.selectable_maps.get(self.selected_map).cloned() else { return };
                    let (frame, _) = ui.allocate_exact_size(vec2(340.0, 340.0), Sense::hover());
                    match crate::map_preview::picture(&ctx, assets, &m.name, 330.0) {
                        Some(picture) => {
                            let painter = ui.painter();
                            let placed = crate::map_preview::show(painter, &picture, frame);
                            // Start positions, in the colour of the player starting there
                            // (unknown until the battle with random starts).
                            let random = self.rules.random_starts;
                            let mut seats: Vec<crate::setup::SlotSetup> = self.slots.iter().take(max_players).filter(|s| s.kind != SlotKind::Closed && !random).enumerate()
                                .map(|(i, s)| crate::setup::SlotSetup { side: String::new(), color: s.color, controller: Controller::Human, start: i, team: s.team, name: String::new() }).collect();
                            if !seats.is_empty() {
                                crate::setup::assign_starts(&mut seats, max_players, 0, false, &crate::setup::start_spots(assets, &m.name));
                            }
                            for (i, &(_, x, z)) in m.info.schemas.first().map(|sc| sc.start_positions.as_slice()).unwrap_or_default().iter().enumerate().take(m.start_positions) {
                                let fill = seats.iter().find(|s| s.start == i).map_or(Color32::from_gray(58), |s| team_color(s.color));
                                crate::map_preview::start_marker(painter, placed.marker_at(x, z, 10.0), fill, &format!("{}", i + 1));
                            }
                        }
                        None => {
                            ui.painter().add(ta_ui::metal::well(frame.shrink(5.0), INSET));
                        }
                    }
                    ui.add_space(10.0);
                    w::fitted_title(ui, &m.title, 26.0, TEXT_BRIGHT);
                    ui.label(RichText::new(tf("{planet} · {size} · UP TO {n} PLAYERS", &[("planet", &m.planet.to_uppercase()), ("size", &m.size), ("n", &m.start_positions)])).font(strong(11.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                    if !m.description.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(&m.description).color(TEXT));
                    }
                    ui.add_space(8.0);
                    let i = &m.info;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        let whole = |v: f64| format!("{v:.0}");
                        w::tag(ui, &tf("WIND {low}–{high}", &[("low", &whole(i.min_wind / 100.0)), ("high", &whole(i.max_wind / 100.0))]), STEEL);
                        w::tag(ui, &tf("TIDAL {n}", &[("n", &whole(i.tidal_strength))]), STEEL);
                        w::tag(ui, &tf("GRAVITY {n}", &[("n", &whole(i.gravity))]), STEEL);
                        // (Acid seas eat what goes in them: TA's `waterdoesdamage`.)
                        if i.water_does_damage && i.water_damage > 0.0 {
                            w::tag(ui, &tf("ACID SEA · {n} DAMAGE A SECOND", &[("n", &whole(i.water_damage))]), RED);
                        }
                    });
                    // What the mutators make of the battle (set under the players).
                    let mutators = mutator_tags(&self.rules.mutators);
                    if !mutators.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            for tag in &mutators {
                                w::tag(ui, tag, AMBER);
                            }
                        });
                    }
                });
                // Players and rules.
                let players = ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.x = 8.0;
                    panel::section(ui, "PLAYERS");
                    let sides = crate::setup::side_choices(&assets.data);
                    // Each column of drop-downs one width, the widest of what it can show, so the
                    // rows line up.
                    let kinds = [SlotKind::Human, SlotKind::AiEasy, SlotKind::AiNormal, SlotKind::AiHard, SlotKind::Closed];
                    let kind_width = w::combo_width(ui, kinds.map(SlotKind::label), 132.0);
                    let side_width = w::combo_width(ui, sides.iter().map(|s| s.label.clone()), 84.0);
                    let team_width = w::combo_width(ui, (1..=5u8).map(|n| tf("Team {n}", &[("n", &n)])), 86.0);
                    // (The colours the seats have, for a click to pass over those taken.)
                    let colours: Vec<(u8, bool)> = self.slots.iter().take(max_players.max(2)).map(|s| (s.color, s.kind != SlotKind::Closed)).collect();
                    // (A seat opened from its drop-down this frame: its colour is settled after.)
                    let mut opened = None;
                    for (n, slot) in self.slots.iter_mut().enumerate().take(max_players.max(2)) {
                        ui.horizontal(|ui| {
                            let open = slot.kind != SlotKind::Closed;
                            let (badge, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                            let painter = ui.painter();
                            painter.rect(badge, CornerRadius::same(2), if open { PANEL_HI } else { INSET }, Stroke::new(1.0, if open { team_color(slot.color) } else { LINE }), StrokeKind::Inside);
                            painter.text(badge.center(), Align2::CENTER_CENTER, format!("{}", n + 1), strong(13.0), if open { TEXT_BRIGHT } else { STEEL_DIM });
                            egui::ComboBox::from_id_salt(("kind", n)).selected_text(slot.kind.label()).width(kind_width).show_ui(ui, |ui| {
                                for k in kinds {
                                    if n == 0 && k == SlotKind::Closed {
                                        continue;
                                    }
                                    ui.selectable_value(&mut slot.kind, k, k.label());
                                }
                            });
                            if !open && slot.kind != SlotKind::Closed {
                                opened = Some(n);
                            }
                            if open {
                                slot.side = slot.side.min(sides.len() - 1);
                                egui::ComboBox::from_id_salt(("side", n)).selected_text(&sides[slot.side].label).width(side_width).show_ui(ui, |ui| {
                                    for (i, s) in sides.iter().enumerate() {
                                        ui.selectable_value(&mut slot.side, i, &s.label);
                                    }
                                });
                                let (r, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
                                let border = if resp.hovered() { AMBER_BRIGHT } else { Color32::from_black_alpha(180) };
                                ui.painter().add(Shape::convex_polygon(w::chamfer(r, 6.0), team_color(slot.color), Stroke::new(1.0, border)));
                                if resp.on_hover_text(ta_ui::i18n::t("Click to change colour")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    slot.color = crate::palette::next_free_colour(slot.color, |c| colours.iter().enumerate().any(|(i, &(other, open))| i != n && open && other == c));
                                }
                                egui::ComboBox::from_id_salt(("team", n)).selected_text(tf("Team {n}", &[("n", &(slot.team + 1))])).width(team_width).show_ui(ui, |ui| {
                                    // (As many teams as there can be players: everyone on their own.)
                                    for team in 0..10u8 {
                                        ui.selectable_value(&mut slot.team, team, tf("Team {n}", &[("n", &(team + 1))]));
                                    }
                                });
                            }
                        });
                    }
                    if opened.is_some() {
                        let shown = self.slots.len().min(max_players.max(2));
                        settle_colours(&mut self.slots[..shown], opened);
                    }
                    ui.add_space(2.0);
                    // Another seat, at the foot of the seats it adds to. (A closed seat is opened
                    // first: it added a seat after the closed ones.)
                    let closed = self.slots.iter().take(max_players).position(|s| s.kind == SlotKind::Closed);
                    if (closed.is_some() || self.slots.len() < max_players) && ui.add(ActionButton::secondary("+ ADD PLAYER")).clicked() {
                        let playing = |s: &&Slot| s.kind != SlotKind::Closed;
                        let color = (0..10u8).find(|c| !self.slots.iter().filter(playing).any(|s| s.color == *c)).unwrap_or(0);
                        let team = (0..10u8).find(|t| !self.slots.iter().filter(playing).any(|s| s.team == *t)).unwrap_or(0);
                        match closed {
                            Some(seat) => self.slots[seat] = Slot { kind: SlotKind::AiNormal, side: seat % 2, color, team },
                            None => self.slots.push(Slot { kind: SlotKind::AiNormal, side: self.slots.len() % 2, color, team }),
                        }
                    }
                    ui.add_space(4.0);
                    // Teams at a click, three equal buttons across (wrapping where a translation
                    // will not fit them).
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        let preset = ((ui.available_width() - 12.0) / 3.0).floor();
                        // Everyone for themselves, or the player against the rest.
                        let open: Vec<usize> = (0..self.slots.len().min(max_players)).filter(|&i| self.slots[i].kind != SlotKind::Closed).collect();
                        if ui.add(ActionButton::secondary("FREE FOR ALL").min_width(preset)).on_hover_text(t("Everyone on a team of their own")).clicked() {
                            for (team, &i) in open.iter().enumerate() {
                                self.slots[i].team = team as u8;
                            }
                        }
                        let human = open.iter().any(|&i| self.slots[i].kind == SlotKind::Human);
                        if ui.add_enabled(human, ActionButton::secondary("YOU VS ALL").min_width(preset)).on_hover_text(t("Every computer player on one team against you"))
                            .on_disabled_hover_text(t("One of the seats must be yours (You)")).clicked() {
                            for &i in &open {
                                self.slots[i].team = u8::from(self.slots[i].kind != SlotKind::Human);
                            }
                        }
                        // Allies: seats taken in turn by two teams (with you, a computer ally).
                        if ui.add_enabled(open.len() >= 3, ActionButton::secondary("TWO TEAMS").min_width(preset)).on_hover_text(t("Seats taken in turn by Team 1 and Team 2; allies start side by side"))
                            .on_disabled_hover_text(t("Needs three players or more")).clicked() {
                            for (k, &i) in open.iter().enumerate() {
                                self.slots[i].team = (k % 2) as u8;
                            }
                        }
                    });
                    ui.add_space(14.0);
                    battle_options(ui, "skirmish", &mut self.rules, true);
                    ui.add_space(14.0);
                    let humans = self.slots.iter().take(max_players).filter(|s| s.kind == SlotKind::Human).count();
                    let active = self.slots.iter().take(max_players).filter(|s| s.kind != SlotKind::Closed).count();
                    let teams: std::collections::BTreeSet<u8> = self.slots.iter().take(max_players).filter(|s| s.kind != SlotKind::Closed).map(|s| s.team).collect();
                    let ok = !assets.selectable_maps.is_empty() && humans <= 1 && active >= 2 && teams.len() >= 2;
                    if assets.selectable_maps.is_empty() {
                        ui.label(RichText::new(t("Activate a map in the launcher before starting a new battle.")).color(RED));
                    } else if !ok {
                        let why = t(if teams.len() < 2 && active >= 2 { n("Everyone is on one team: nobody to fight.") } else { n("Need at least two players and at most one human.") });
                        ui.label(RichText::new(why).color(RED));
                        ui.add_space(4.0);
                    }
                    if panel::hero_button(ui, "START BATTLE", vec2(ui.available_width(), 58.0), ok).clicked() && let Some(m) = assets.selectable_maps.get(self.selected_map) {
                        self.keep_skirmish(assets, &m.name);
                        action = MenuAction::Start(self.build_setup(assets, &m.name, max_players));
                    }
                });
                let bottom = players.response.rect.bottom() - columns_top;
                if foot.is_none_or(|f| (f - bottom).abs() > 0.5) {
                    ctx.data_mut(|d| d.insert_temp(foot_id, bottom));
                    ctx.request_discard("the skirmish list follows its page's foot");
                }
            });
        });
        if response.exit {
            self.page = Page::Title;
        }
        action
    }

    /// A map at random (not the one chosen) with a start for each player set up, or for two.
    fn random_map(&mut self, assets: &Assets) {
        let players = self.slots.iter().filter(|s| s.kind != SlotKind::Closed).count().max(2);
        let maps = &assets.selectable_maps;
        // (Of the maps the list shows, those with room for the players; else of them all.)
        let fits = |pool: Vec<usize>| -> Vec<usize> { pool.into_iter().filter(|&i| i != self.selected_map && maps[i].start_positions >= players).collect() };
        let mut pool = fits(self.map_choice.shown(maps));
        if pool.is_empty() {
            pool = fits((0..maps.len()).collect());
        }
        if pool.is_empty() {
            pool = (0..maps.len()).filter(|&i| i != self.selected_map).collect();
        }
        if pool.is_empty() {
            return;
        }
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.selected_map = pool[((self.seed >> 33) % pool.len() as u64) as usize];
        // (One the list does not show: all of them shown again.)
        if !self.map_choice.accepts(&maps[self.selected_map]) {
            self.map_choice.clear();
        }
        self.reveal = true;
    }

    /// Keeps the skirmish as set up for the next (`skirmish.toml`, the profile's): its map,
    /// players and rules.
    fn keep_skirmish(&self, assets: &Assets, map: &str) {
        let sides = crate::setup::side_choices(&assets.data);
        let kept = KeptSkirmish {
            map: map.to_string(),
            slots: self
                .slots
                .iter()
                .map(|s| KeptSlot { kind: s.kind.key().to_string(), side: sides.get(s.side).map_or_else(String::new, |c| c.key.clone()), color: s.color, team: s.team })
                .collect(),
            rules: Some(self.rules.clone()),
        };
        match toml::to_string(&kept) {
            Ok(text) => {
                if let Err(e) = ta_config::write_atomic(&ta_config::player_data_dir().join(KEPT_SKIRMISH), &text) {
                    log::warn!("could not keep the skirmish set up: {e}");
                }
            }
            Err(e) => log::warn!("could not keep the skirmish set up: {e}"),
        }
    }

    /// The skirmish last set up (see [`Menu::keep_skirmish`]), if any: its map, players and
    /// rules, but the unit limit the settings give.
    pub fn restore_skirmish(&mut self, assets: &Assets) {
        let Ok(text) = std::fs::read_to_string(ta_config::player_data_dir().join(KEPT_SKIRMISH)) else { return };
        let kept: KeptSkirmish = match toml::from_str(&text) {
            Ok(k) => k,
            Err(e) => {
                log::warn!("the skirmish kept from last time is unreadable: {e}");
                return;
            }
        };
        if let Some(i) = assets.selectable_maps.iter().position(|m| m.name.eq_ignore_ascii_case(&kept.map)) {
            self.selected_map = i;
        }
        let sides = crate::setup::side_choices(&assets.data);
        let slots: Vec<Slot> = kept
            .slots
            .iter()
            .take(10)
            .filter_map(|k| {
                let kind = SlotKind::from_key(&k.kind)?;
                let side = sides.iter().position(|s| s.key.eq_ignore_ascii_case(&k.side)).unwrap_or(0);
                Some(Slot { kind, side, color: k.color.min(9), team: k.team.min(9) })
            })
            .collect();
        // (One of them the player's, at most, and the first.)
        if slots.len() >= 2 && slots.iter().filter(|s| s.kind == SlotKind::Human).count() <= 1 && slots[0].kind != SlotKind::Closed {
            self.slots = slots;
        }
        if let Some(rules) = kept.rules {
            let limit = self.rules.unit_limit;
            self.rules = rules;
            self.rules.unit_limit = limit;
        }
    }

    fn build_setup(&mut self, assets: &Assets, map: &str, max_players: usize) -> GameSetup {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let sides = crate::setup::side_choices(&assets.data);
        let mut players = Vec::new();
        let mut local = None;
        for slot in self.slots.iter().take(max_players).filter(|s| s.kind != SlotKind::Closed) {
            let controller = match slot.kind {
                SlotKind::Human => {
                    local = Some(players.len() as u8);
                    Controller::Human
                }
                SlotKind::AiEasy => Controller::Ai(ta_ai::Difficulty::Easy),
                SlotKind::AiHard => Controller::Ai(ta_ai::Difficulty::Hard),
                _ => Controller::Ai(ta_ai::Difficulty::Normal),
            };
            let side = sides[slot.side.min(sides.len() - 1)].key.clone();
            players.push(SlotSetup { side, color: slot.color, controller, start: players.len(), team: slot.team, name: String::new() });
        }
        let seed = self.seed ^ 0x5EED;
        crate::setup::assign_starts(&mut players, max_players, seed, self.rules.random_starts, &crate::setup::start_spots(assets, map));
        GameSetup { map: map.to_string(), players, local_player: local, seed, mission: None, rules: self.rules.clone(), conquest: None, scenario: None }
    }
}

/// Skirmish options: starting resources, fog, mapping, the commander rule and the unit
/// limit. Returns whether anything changed; read-only unless `editable`.
pub fn rules_editor(ui: &mut egui::Ui, rules: &mut ta_sim::GameRules, editable: bool) -> bool {
    let before = rules.clone();
    // Every rule's drop-down one width, the widest choice's, so the column ends at one edge.
    let resources = |m: u64, e: u64| tf("{metal} metal / {energy} energy", &[("metal", &m), ("energy", &e)]);
    let choices: Vec<String> = [(1000, 1000), (2500, 2500), (5000, 5000), (10000, 10000), (1000, 5000)]
        .iter()
        .map(|&(m, e)| resources(m, e))
        .chain(["Permanent (no fog)", "True: hills hide", "Circular", "Mapped", "Unmapped", "Game ends", "Game continues", "In slot order", "Random", "Formations, units give way", "As in TA (they queue)"].map(|c| t(c).to_owned()))
        .chain([20_000u32].map(|n| tf("{n} per player", &[("n", &n)])))
        .collect();
    let width = w::combo_width(ui, choices, 200.0);
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new("rules").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label(w::caption("STARTING RESOURCES"));
            egui::ComboBox::from_id_salt("rules_res").selected_text(resources(rules.start_metal, rules.start_energy)).width(width).show_ui(ui, |ui| {
                for (m, e) in [(1000, 1000), (2500, 2500), (5000, 5000), (10000, 10000), (1000, 5000)] {
                    if ui.selectable_label(rules.start_metal == m && rules.start_energy == e, resources(m, e)).clicked() {
                        rules.start_metal = m;
                        rules.start_energy = e;
                    }
                }
            });
            ui.end_row();
            // TA's own choices: circular, true (hills hide what is behind them), permanent.
            ui.label(w::caption("LINE OF SIGHT"));
            let sight = |fog: bool, true_sight: bool| if !fog { t("Permanent (no fog)") } else if true_sight { t("True: hills hide") } else { t("Circular") };
            egui::ComboBox::from_id_salt("rules_los")
                .selected_text(sight(rules.fog_of_war, rules.true_sight))
                .width(width)
                .show_ui(ui, |ui| {
                    for (fog, true_sight) in [(true, false), (true, true), (false, false)] {
                        let on = rules.fog_of_war == fog && (!fog || rules.true_sight == true_sight);
                        if ui.selectable_label(on, sight(fog, true_sight)).clicked() {
                            rules.fog_of_war = fog;
                            rules.true_sight = true_sight;
                        }
                    }
                })
                .response
                .on_hover_text(t("Circular: each unit sees round itself. True: hills hide what is behind them, and high ground sees further. Permanent: the whole battle in view"));
            ui.end_row();
            ui.label(w::caption("MAPPING"));
            ui.add_enabled_ui(rules.fog_of_war, |ui| {
                egui::ComboBox::from_id_salt("rules_mapping").selected_text(if rules.mapped { t("Mapped") } else { t("Unmapped") }).width(width).show_ui(ui, |ui| {
                    ui.selectable_value(&mut rules.mapped, true, t("Mapped"));
                    ui.selectable_value(&mut rules.mapped, false, t("Unmapped"));
                })
                .response
                .on_hover_text(t("Mapped: the ground is known from the start. Unmapped: dark until explored"))
                .on_disabled_hover_text(t("Only with the fog of war on"));
            });
            ui.end_row();
            ui.label(w::caption("COMMANDER DIES"));
            egui::ComboBox::from_id_salt("rules_com").selected_text(if rules.commander_ends_game { t("Game ends") } else { t("Game continues") }).width(width).show_ui(ui, |ui| {
                ui.selectable_value(&mut rules.commander_ends_game, true, t("Game ends"));
                ui.selectable_value(&mut rules.commander_ends_game, false, t("Game continues"));
            });
            ui.end_row();
            ui.label(w::caption("START POSITIONS"));
            egui::ComboBox::from_id_salt("rules_starts").selected_text(if rules.random_starts { t("Random") } else { t("In slot order") }).width(width).show_ui(ui, |ui| {
                ui.selectable_value(&mut rules.random_starts, false, t("In slot order"));
                ui.selectable_value(&mut rules.random_starts, true, t("Random"));
            });
            ui.end_row();
            ui.label(w::caption("MOVEMENT"));
            egui::ComboBox::from_id_salt("rules_traffic").selected_text(if rules.give_way { t("Formations, units give way") } else { t("As in TA (they queue)") }).width(width).show_ui(ui, |ui| {
                ui.selectable_value(&mut rules.give_way, true, t("Formations, units give way"));
                ui.selectable_value(&mut rules.give_way, false, t("As in TA (they queue)"));
            });
            ui.end_row();
            ui.label(w::caption("UNIT LIMIT"));
            egui::ComboBox::from_id_salt("rules_cap").selected_text(tf("{n} per player", &[("n", &rules.unit_limit)])).width(width).show_ui(ui, |ui| {
                for n in [250, 500, 1000, 1500, 2500, 5000, 10_000, 20_000] {
                    ui.selectable_value(&mut rules.unit_limit, n, tf("{n} per player", &[("n", &n)]));
                }
            });
            ui.end_row();
        });
    });
    *rules != before
}

/// What a skirmish's mutators can be set to: resources (quarters of TA's), build speed (times
/// TA's), how tough units are (quarters) and the ceasefire (minutes; 0: none).
const RESOURCES: [u8; 6] = [2, 4, 6, 8, 12, 16];
const BUILD_SPEEDS: [u8; 4] = [1, 2, 3, 4];
const TOUGHNESS: [u8; 5] = [2, 4, 6, 8, 12];
const NO_RUSH: [u8; 7] = [0, 3, 5, 10, 15, 20, 30];

/// Quarters of TA's as a multiple: "Half", "As in TA", "×1.5", "×2".
fn quarters(q: u8) -> String {
    match q {
        2 => t("Half").to_owned(),
        4 => t("As in TA").to_owned(),
        q if q.is_multiple_of(4) => format!("×{}", q / 4),
        q => format!("×{}", f32::from(q) / 4.0),
    }
}

/// The mutators' presets: a name, what it makes of a battle, and the mutators.
fn mutator_presets() -> [(&'static str, &'static str, ta_sim::Mutators); 8] {
    let ta = ta_sim::Mutators::default();
    [
        (n("As in TA"), n("No mutators: Total Annihilation's own rules."), ta),
        (n("Blitz"), n("Double the resources and double the build speed: a fast and furious battle."), ta_sim::Mutators { resources: 8, build_speed: 2, ..ta }),
        (n("Peacetime"), n("Ten minutes of ceasefire to build up before the war."), ta_sim::Mutators { no_rush_minutes: 10, ..ta }),
        (n("Arms race"), n("Five minutes of ceasefire, then triple the resources and build speed: a race to the biggest guns."), ta_sim::Mutators { resources: 12, build_speed: 3, no_rush_minutes: 5, ..ta }),
        (n("Ground war"), n("No aircraft and no ships: the war is fought on land."), ta_sim::Mutators { no_air: true, no_naval: true, ..ta }),
        (n("No superweapons"), n("No missile silos, long-range cannons or experimentals."), ta_sim::Mutators { no_nukes: true, no_artillery: true, no_experimentals: true, ..ta }),
        (n("Ironclad"), n("Units twice as tough: long, grinding battles."), ta_sim::Mutators { toughness: 8, ..ta }),
        (n("Glass cannons"), n("Units half as tough: every shot counts."), ta_sim::Mutators { toughness: 2, ..ta }),
    ]
}

/// The kinds of unit a battle can leave out: the switch's name, what it stands for, and
/// whether they are out.
fn unit_kinds(m: &mut ta_sim::Mutators) -> [(&'static str, &'static str, &mut bool); 5] {
    [
        (n("AIRCRAFT"), n("Aircraft, and the plants that build them"), &mut m.no_air),
        (n("SHIPS"), n("Ships and submarines, and the shipyards that build them"), &mut m.no_naval),
        (n("MISSILES"), n("Nuclear and EMP missile silos, and the defences against them"), &mut m.no_nukes),
        (n("LONG-RANGE GUNS"), n("Long-range plasma cannons: the Big Bertha, the Intimidator, the Vulcan and the Buzzsaw"), &mut m.no_artillery),
        (n("EXPERIMENTALS"), n("The experimental units: the Krogoth, and its gantry"), &mut m.no_experimentals),
    ]
}

/// How many of the mutators are set (counted on their tab).
fn mutators_set(m: &ta_sim::Mutators) -> usize {
    let ta = ta_sim::Mutators::default();
    [m.resources != ta.resources, m.build_speed != ta.build_speed, m.toughness != ta.toughness, m.no_rush_minutes > 0, m.no_air, m.no_naval, m.no_nukes, m.no_artillery, m.no_experimentals]
        .into_iter()
        .filter(|&on| on)
        .count()
}

/// The mutators set, as tags ("RESOURCES ×2", "NO AIRCRAFT").
pub fn mutator_tags(m: &ta_sim::Mutators) -> Vec<String> {
    let ta = ta_sim::Mutators::default();
    let upper = |s: String| ta_ui::i18n::upper(&s);
    let mut tags = Vec::new();
    if m.resources != ta.resources {
        tags.push(tf("RESOURCES {x}", &[("x", &upper(quarters(m.resources)))]));
    }
    if m.build_speed != ta.build_speed {
        tags.push(tf("BUILD SPEED ×{n}", &[("n", &m.build_speed)]));
    }
    if m.toughness != ta.toughness {
        tags.push(tf("TOUGHNESS {x}", &[("x", &upper(quarters(m.toughness)))]));
    }
    if m.no_rush_minutes > 0 {
        tags.push(tf("CEASEFIRE {n} MIN", &[("n", &m.no_rush_minutes)]));
    }
    let mut m = *m;
    for (name, _, out) in unit_kinds(&mut m) {
        if *out {
            tags.push(tf("NO {kind}", &[("kind", &t(name))]));
        }
    }
    tags
}

/// A skirmish's mutators (`ta_sim::Mutators`): a preset, the resources, the build speed, how
/// tough units are, a ceasefire to start with, and the kinds of unit in the battle. Returns
/// whether anything changed; read-only unless `editable`.
pub fn mutators_editor(ui: &mut egui::Ui, m: &mut ta_sim::Mutators, editable: bool) -> bool {
    let before = *m;
    let presets = mutator_presets();
    let minutes = |n: u8| if n == 0 { t("Off").to_owned() } else { tf("{n} minutes", &[("n", &n)]) };
    let times = |n: u8| if n == 1 { t("As in TA").to_owned() } else { format!("×{n}") };
    let choices: Vec<String> = presets.iter().map(|p| t(p.0).to_owned()).chain([t("Custom").to_owned()]).chain(RESOURCES.map(quarters)).chain(NO_RUSH.map(minutes)).collect();
    let width = w::combo_width(ui, choices, 200.0);
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new("mutators").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label(w::caption("PRESET"));
            let chosen = presets.iter().find(|p| p.2 == *m).map_or(t("Custom"), |p| t(p.0));
            egui::ComboBox::from_id_salt("mutators_preset").selected_text(chosen).width(width).show_ui(ui, |ui| {
                for (name, about, preset) in &presets {
                    if ui.selectable_label(*m == *preset, t(name)).on_hover_text(t(about)).clicked() {
                        *m = *preset;
                    }
                }
            });
            ui.end_row();
            ui.label(w::caption("RESOURCES"));
            egui::ComboBox::from_id_salt("mutators_resources")
                .selected_text(quarters(m.resources))
                .width(width)
                .show_ui(ui, |ui| {
                    for q in RESOURCES {
                        ui.selectable_value(&mut m.resources, q, quarters(q));
                    }
                })
                .response
                .on_hover_text(t("The metal and energy every extractor, generator and maker brings in, against TA's"));
            ui.end_row();
            ui.label(w::caption("BUILD SPEED"));
            egui::ComboBox::from_id_salt("mutators_build").selected_text(times(m.build_speed)).width(width).show_ui(ui, |ui| {
                for n in BUILD_SPEEDS {
                    ui.selectable_value(&mut m.build_speed, n, times(n));
                }
            });
            ui.end_row();
            ui.label(w::caption("UNIT TOUGHNESS"));
            egui::ComboBox::from_id_salt("mutators_tough")
                .selected_text(quarters(m.toughness))
                .width(width)
                .show_ui(ui, |ui| {
                    for q in TOUGHNESS {
                        ui.selectable_value(&mut m.toughness, q, quarters(q));
                    }
                })
                .response
                .on_hover_text(t("How much punishment every unit takes: twice as tough, battles last twice as long"));
            ui.end_row();
            ui.label(w::caption("NO RUSH"));
            egui::ComboBox::from_id_salt("mutators_rush")
                .selected_text(minutes(m.no_rush_minutes))
                .width(width)
                .show_ui(ui, |ui| {
                    for n in NO_RUSH {
                        ui.selectable_value(&mut m.no_rush_minutes, n, minutes(n));
                    }
                })
                .response
                .on_hover_text(t("A ceasefire to start with: until it ends, weapons do enemies no harm and nobody builds near an enemy base"));
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.label(w::caption("UNITS IN THE BATTLE"));
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (name, about, out) in unit_kinds(m) {
                if w::chip(ui, name, !*out).on_hover_text(t(about)).clicked() {
                    *out = !*out;
                }
            }
        });
    });
    *m != before
}

/// Which of a battle's options show: its rules, or its mutators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OptionsTab {
    #[default]
    Rules,
    Mutators,
}

/// A battle's rules and mutators, a tab each (`id` keeps which shows, and the room the taller
/// takes, so that what follows stays where it is). Returns whether anything changed; read-only
/// unless `editable` (the tabs still turn).
pub fn battle_options(ui: &mut egui::Ui, id: &str, rules: &mut ta_sim::GameRules, editable: bool) -> bool {
    let key = egui::Id::new(("battle_options", id));
    let mut tab = ui.data(|d| d.get_temp::<OptionsTab>(key)).unwrap_or_default();
    let set = mutators_set(&rules.mutators);
    let mutators = if set == 0 { t("MUTATORS").to_owned() } else { tf("MUTATORS · {n}", &[("n", &set)]) };
    if w::segmented_raw_fill(ui, &mut tab, &[(OptionsTab::Rules, t("RULES")), (OptionsTab::Mutators, &mutators)]).changed() {
        ui.data_mut(|d| d.insert_temp(key, tab));
    }
    ui.add_space(10.0);
    let room = ui.data(|d| d.get_temp::<f32>(key.with("room"))).unwrap_or(0.0);
    let top = ui.cursor().top();
    let changed = match tab {
        OptionsTab::Rules => rules_editor(ui, rules, editable),
        OptionsTab::Mutators => mutators_editor(ui, &mut rules.mutators, editable),
    };
    let used = ui.cursor().top() - top;
    if used > room + 0.5 {
        ui.data_mut(|d| d.insert_temp(key.with("room"), used));
    } else if room > used {
        ui.add_space(room - used);
    }
    changed
}

/// "5 min ago", "3 h ago", "2 days ago".
/// The emblem of an empty list of records, in `rect`: a disk for saved games, a film reel for
/// replays, stamped as the interface's icons are (a dark drop under them).
/// The order of the saved games and replays page's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordSort {
    Newest,
    Oldest,
    Longest,
    Name,
}

impl RecordSort {
    const ALL: [Self; 4] = [Self::Newest, Self::Oldest, Self::Longest, Self::Name];

    fn label(self) -> &'static str {
        match self {
            Self::Newest => n("Newest first"),
            Self::Oldest => n("Oldest first"),
            Self::Longest => n("Longest first"),
            Self::Name => n("By name"),
        }
    }
}

fn records_emblem(painter: &egui::Painter, rect: Rect, save: bool, color: Color32) {
    let s = rect.width() / 64.0;
    let p = |x: f32, y: f32| rect.min + vec2(x * s, y * s);
    let hollow = INSET;
    for (offset, color) in [(vec2(0.0, 1.5), Color32::from_black_alpha(170)), (egui::Vec2::ZERO, color)] {
        let q = |x: f32, y: f32| p(x, y) + offset;
        if save {
            // A disk: its corner cut, the shutter at its top, the label window below.
            painter.add(egui::Shape::convex_polygon(vec![q(8.0, 8.0), q(46.0, 8.0), q(56.0, 18.0), q(56.0, 56.0), q(8.0, 56.0)], color, Stroke::NONE));
            if offset == egui::Vec2::ZERO {
                painter.rect_filled(Rect::from_min_max(q(18.0, 8.0), q(42.0, 22.0)), CornerRadius::same(1), hollow);
                painter.rect_filled(Rect::from_min_max(q(34.0, 10.0), q(39.0, 19.0)), CornerRadius::ZERO, color);
                painter.rect_filled(Rect::from_min_max(q(16.0, 32.0), q(48.0, 50.0)), CornerRadius::same(2), hollow);
            }
        } else {
            // A film reel: windows round its hub, the film running off it.
            painter.rect_filled(Rect::from_min_max(q(29.0, 50.0), q(60.0, 60.0)), CornerRadius::same(1), color);
            painter.circle_filled(q(29.0, 29.0), 24.0 * s, color);
            if offset == egui::Vec2::ZERO {
                painter.circle_filled(q(29.0, 29.0), 4.0 * s, hollow);
                for k in 0..6 {
                    let a = (-90.0 + 60.0 * k as f32).to_radians();
                    painter.circle_filled(q(29.0 + 14.0 * a.cos(), 29.0 + 14.0 * a.sin()), 6.2 * s, hollow);
                }
                for x in [36.0, 44.0, 52.0] {
                    painter.rect_filled(Rect::from_min_max(q(x, 53.0), q(x + 4.0, 57.0)), CornerRadius::ZERO, hollow);
                }
            }
        }
    }
}

fn format_age(unix: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let s = now.saturating_sub(unix);
    match s {
        0..60 => t("just now").to_string(),
        60..3600 => tf("{n} min ago", &[("n", &(s / 60))]),
        3600..86400 => tf("{n} h ago", &[("n", &(s / 3600))]),
        86400..172800 => t("yesterday").to_string(),
        _ => tf("{n} days ago", &[("n", &(s / 86400))]),
    }
}
