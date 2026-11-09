//! The libraries' context menus (Mods, Maps, Skins). A right-click on a card or a list's row
//! (or Shift+F10, or the Menu key, on the one with the keys) chooses it and opens a plate of what
//! can be done with it there: the page's own actions for it, a mark before each, what cannot be
//! done now dimmed with the reason under its name. The plate is the launcher's hints' metal (a
//! brass edge along its top), heavier: the item's name heads it.
//!
//! While it is open it is over everything (the page under it takes no clicks and keeps no keys):
//! Escape, a click outside it or the wheel close it; the arrows, Home, End and Tab go from entry
//! to entry, Enter or Space choose; the keys come back to the card it was opened from with them.
//! It moves not at all: egui's quick fade as it comes is all, under reduced motion as without.

use egui::{
    Align, Color32, Context, CursorIcon, EventFilter, FocusDirection, Frame, Id, Key, Layout, Modifiers, Pos2, Rect, Response,
    RichText, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use ta_ui::metal;
use ta_ui::sound::{self, UiSound};
use ta_ui::tooltip::TooltipExt as _;

use crate::theme::*;
use crate::widgets;

/// A mark before an entry's name, drawn as the launcher's other glyphs are (lines, no font).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    /// What it is: its details.
    Details,
    /// Played: a battle, the game with a mod.
    Play,
    /// A battle set up first (the game's skirmish page).
    Setup,
    /// Downloaded: GET, UPDATE.
    Download,
    /// A file of one's own (a mod installed from it).
    File,
    /// Stopped: an install cancelled.
    Cancel,
    /// A switch's lamp: lit for what turns on, dark for what turns off.
    Switch(bool),
    /// A lamp of a colour, lit when it is so now: the armies' skins (green yours, amber the
    /// enemy's), as their cards' lamps.
    Lamp(Color32, bool),
    Up,
    Down,
    Top,
    Bottom,
    /// A check: a mod's report.
    Report,
    Folder,
    /// Out of the launcher: a web page.
    Web,
    /// Elsewhere in the launcher (the Mods page, at a map's source).
    Goto,
    /// A favourite's star: filled for what becomes one, hollow for what stops being one.
    Star(bool),
    /// An eye: open for what comes back into view, struck through for what is hidden.
    Eye(bool),
    Edit,
    /// Two sheets: a copy made (a built-in skin's, to change).
    Copy,
    /// Out of a tray: shared.
    Export,
    Remove,
}

/// How an entry stands out: the page's main action in brass, removal in rust red.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Plain,
    Main,
    Danger,
}

/// An entry of a [`Menu`].
pub struct Entry<A> {
    label: String,
    icon: Icon,
    keys: Option<&'static str>,
    why_not: Option<String>,
    tone: Tone,
    action: A,
}

impl<A> Entry<A> {
    /// The key that does the same on the page ("Enter", "F").
    pub fn keys(&mut self, keys: &'static str) -> &mut Self {
        self.keys = Some(keys);
        self
    }

    /// Not possible now when there is a reason (translated): dimmed, the reason under its name.
    pub fn unless(&mut self, why: Option<&str>) -> &mut Self {
        self.why_not = why.map(str::to_string);
        self
    }

    /// The page's main action for the item, in brass.
    pub fn main(&mut self) -> &mut Self {
        self.tone = Tone::Main;
        self
    }

    /// Removal, in rust red.
    pub fn danger(&mut self) -> &mut Self {
        self.tone = Tone::Danger;
        self
    }

    /// Whether it can be chosen now.
    pub fn enabled(&self) -> bool {
        self.why_not.is_none()
    }
}

enum Line<A> {
    Entry(Entry<A>),
    Rule,
}

/// What can be done with one item of a library, built afresh each frame from what it is now.
pub struct Menu<A> {
    title: String,
    caption: String,
    lines: Vec<Line<A>>,
}

impl<A: Clone> Menu<A> {
    /// A menu headed by the item's name (`title`, as it is shown) over what it is (`caption`).
    pub fn new(title: impl Into<String>, caption: impl Into<String>) -> Self {
        Self { title: title.into(), caption: caption.into(), lines: Vec::new() }
    }

    /// An entry named `label` (English, translated here, shown in capitals as the launcher's
    /// buttons are).
    pub fn add(&mut self, label: &str, icon: Icon, action: A) -> &mut Entry<A> {
        self.add_text(ta_ui::i18n::t(label).to_string(), icon, action)
    }

    /// An entry whose name is already translated (one with a figure in it: "GET · 120 MB").
    pub fn add_text(&mut self, label: String, icon: Icon, action: A) -> &mut Entry<A> {
        self.lines.push(Line::Entry(Entry { label: ta_ui::i18n::upper(&label), icon, keys: None, why_not: None, tone: Tone::Plain, action }));
        match self.lines.last_mut() {
            Some(Line::Entry(entry)) => entry,
            _ => unreachable!("an entry was just added"),
        }
    }

    /// A rule before the next group (never first, never two together, never last).
    pub fn rule(&mut self) {
        if matches!(self.lines.last(), Some(Line::Entry(_))) {
            self.lines.push(Line::Rule);
        }
    }

    /// Its entries, in order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry<A>> {
        self.lines.iter().filter_map(|line| match line {
            Line::Entry(entry) => Some(entry),
            Line::Rule => None,
        })
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries().next().is_none()
    }

    /// How wide the plate is: its longest entry's or heading's, within reason (a longer name is
    /// cut short, a reason wraps).
    fn width(&self, ui: &Ui) -> f32 {
        let heading = [(self.title.as_str(), strong(13.5)), (self.caption.as_str(), body(11.5))].into_iter()
            .map(|(text, font)| widgets::galley(ui, RichText::new(text).font(font)).size().x + 2.0 * PAD).fold(0.0, f32::max);
        let widest = self.entries().map(|entry| {
            let label = widgets::galley(ui, label_text(&entry.label)).size().x;
            let keys = entry.keys.map_or(0.0, |keys| widgets::galley(ui, keys_text(keys)).size().x + 22.0);
            TEXT_X + label + keys + PAD + 6.0
        }).fold(heading, f32::max);
        let room = (ui.ctx().content_rect().width() - 24.0).max(160.0);
        widest.clamp(MIN_WIDTH.min(room), MAX_WIDTH.min(room)).ceil()
    }
}

/// The plate's room round its contents, an entry's height, and where an entry's mark and name
/// stand from its left.
const PAD: f32 = 12.0;
const ROW: f32 = 30.0;
const ICON_X: f32 = 22.0;
const TEXT_X: f32 = 40.0;
const MIN_WIDTH: f32 = 220.0;
const MAX_WIDTH: f32 = 340.0;

fn label_text(label: &str) -> RichText {
    RichText::new(label).font(strong(12.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.6))
}

fn keys_text(keys: &str) -> RichText {
    RichText::new(keys).font(strong(10.0))
}

/// A menu's state while it is open: what it is for, where, and where the keys come back to.
#[derive(Clone, Debug)]
struct Opened {
    target: String,
    at: Pos2,
    /// Each opening is a popup of its own (egui keeps a popup's place from its last frame).
    serial: u64,
    /// The frame it opened in (the click that opened it is not one outside it).
    frame: u64,
    /// The card or row it was opened from with the keys: its first entry takes them, and they
    /// go back to it when it closes.
    from: Option<Id>,
    focus_first: bool,
}

fn popup_id(menu: Id, serial: u64) -> Id {
    menu.with(("menu", serial))
}

/// Opens the menu `menu` (one a page) for `target` at `at`; `from`: the widget it was opened
/// from with the keys.
pub fn open(ctx: &Context, menu: Id, target: &str, at: Pos2, from: Option<Id>) {
    let serial = ctx.data_mut(|d| {
        let serial = d.get_temp_mut_or_default::<u64>(Id::new("library_menu_serial"));
        *serial += 1;
        *serial
    });
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(menu, Opened { target: target.to_string(), at, serial, frame, from, focus_first: from.is_some() }));
    egui::Popup::open_id(ctx, popup_id(menu, serial));
    sound::play(ctx, UiSound::Toggle);
}

/// What the menu `menu` is open for. None once it has closed (Escape, a click outside, the page
/// left), the keys then given back to where it was opened from with them.
pub fn target(ctx: &Context, menu: Id) -> Option<String> {
    let opened = ctx.data(|d| d.get_temp::<Opened>(menu))?;
    if egui::Popup::is_id_open(ctx, popup_id(menu, opened.serial)) {
        return Some(opened.target);
    }
    ctx.data_mut(|d| d.remove::<Opened>(menu));
    if let Some(from) = opened.from {
        ctx.memory_mut(|m| m.request_focus(from));
    }
    None
}

/// Closes the menu `menu` (its item has gone).
pub fn close(ctx: &Context, menu: Id) {
    if let Some(opened) = ctx.data(|d| d.get_temp::<Opened>(menu)) {
        egui::Popup::close_id(ctx, popup_id(menu, opened.serial));
    }
    ctx.data_mut(|d| d.remove::<Opened>(menu));
}

/// A right-click outside an open menu (which only closed it): where, and in which frame. The
/// card or row there opens its own menu in the next frame, as if the click had come to it.
fn hop_id() -> Id {
    Id::new("library_menu_hop")
}

/// Whether the menu was asked for on `response` (a card's, a row's): a right-click, at the
/// pointer (also one that closed another menu over it); or Shift+F10 with the keys on it (the
/// Menu key comes as that), at the item. Where to open it, and whether the keys asked.
pub fn asked(ui: &Ui, response: &Response) -> Option<(Pos2, bool)> {
    let ctx = ui.ctx();
    if response.secondary_clicked() {
        let at = response.interact_pointer_pos().or_else(|| ctx.pointer_latest_pos()).unwrap_or_else(|| response.rect.center());
        return Some((at, false));
    }
    if let Some((at, frame)) = ctx.data(|d| d.get_temp::<(Pos2, u64)>(hop_id()))
        && ctx.cumulative_frame_nr() <= frame + 2 && ui.is_rect_visible(response.rect) && response.rect.contains(at)
        && ui.clip_rect().contains(at)
    {
        ctx.data_mut(|d| d.remove::<(Pos2, u64)>(hop_id()));
        return Some((at, false));
    }
    if response.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F10)) {
        let rect = response.rect;
        return Some((pos2(rect.left() + (rect.width() * 0.5).min(160.0), rect.center().y), true));
    }
    None
}

/// Shows the menu `id` while it is open, as `menu` has it now (call it each frame from the page
/// that opened it, after its cards): what was chosen, if anything was.
pub fn show<A: Clone>(ui: &Ui, id: Id, menu: &Menu<A>) -> Option<A> {
    let ctx = ui.ctx().clone();
    let mut opened = ctx.data(|d| d.get_temp::<Opened>(id))?;
    let popup = popup_id(id, opened.serial);
    if !egui::Popup::is_id_open(&ctx, popup) {
        return None;
    }
    let width = menu.width(ui);
    let mut chosen = None;
    let mut focused_first = false;
    let shown = egui::Popup::new(popup, ctx.clone(), opened.at, ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .open_memory(None::<egui::SetOpenCommand>)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(Frame::NONE)
        .gap(2.0)
        .width(width)
        .layout(Layout::top_down(Align::Min))
        .show(|ui| {
            // (Not in the pass that only measures it: a widget there takes no keys, and would
            // give them up.)
            focused_first = opened.focus_first && !ui.is_sizing_pass();
            plate(ui, menu, width, focused_first, &mut chosen)
        })?;
    if focused_first {
        opened.focus_first = false;
        ctx.data_mut(|d| d.insert_temp(id, opened.clone()));
    }
    let inside = |at: Pos2| shown.response.rect.contains(at);
    // (Not in the frame it opened in: what opened it is no wheel or click outside it.)
    let settled = ctx.cumulative_frame_nr() != opened.frame;
    // The wheel outside it closes it: the page under it scrolls with the next turn.
    let wheel = ctx.input(|i| i.raw.events.iter().any(|event| matches!(event, egui::Event::MouseWheel { .. })));
    if settled && wheel && !ctx.pointer_hover_pos().is_some_and(inside) {
        egui::Popup::close_id(&ctx, popup);
    }
    // A right-click outside it (which closes it, as any click outside does) opens the menu of
    // what it was on, in the next frame.
    if settled && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
        && let Some(at) = ctx.input(|i| i.pointer.interact_pos()).filter(|at| !inside(*at))
    {
        egui::Popup::close_id(&ctx, popup);
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(hop_id(), (at, frame)));
    }
    // Over everything while it stays open: the page under it takes no clicks (a click outside
    // only closes it) and the keys stay in it.
    if egui::Popup::is_id_open(&ctx, popup) {
        ctx.memory_mut(|m| m.set_modal_layer(shown.response.layer_id));
    }
    let (action, by_keys) = chosen?;
    egui::Popup::close_id(&ctx, popup);
    ctx.data_mut(|d| d.remove::<Opened>(id));
    if by_keys && let Some(from) = opened.from {
        ctx.memory_mut(|m| m.request_focus(from));
    }
    sound::play(&ctx, UiSound::Button);
    Some(action)
}

/// The plate and its entries; `chosen`: the action chosen, and whether by the keys.
fn plate<A: Clone>(ui: &mut Ui, menu: &Menu<A>, width: f32, focus_first: bool, chosen: &mut Option<(A, bool)>) {
    let background = ui.painter().add(Shape::Noop);
    ui.set_width(width);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let top = ui.cursor().min;
    // The heading: the item's name over what it is.
    let room = width - 2.0 * PAD;
    let line = |text: &str, font: egui::FontId| egui::WidgetText::from(RichText::new(text).font(font)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
    let title = line(&menu.title, strong(13.5));
    let caption = (!menu.caption.is_empty()).then(|| line(&menu.caption, body(11.5)));
    let heading = 9.0 + title.size().y + caption.as_ref().map_or(0.0, |c| c.size().y + 1.0) + 8.0;
    let (head, _) = ui.allocate_exact_size(vec2(width, heading), Sense::hover());
    ui.painter().galley(pos2(head.left() + PAD, head.top() + 9.0), title.clone(), TEXT_BRIGHT);
    if let Some(caption) = caption {
        ui.painter().galley(pos2(head.left() + PAD, head.top() + 9.0 + title.size().y + 1.0), caption, STEEL);
    }
    ui.painter().add(metal::seam(head.x_range().shrink(6.0), head.bottom() - 0.5));
    ui.add_space(4.0);
    // The keys: from entry to entry (those that can be chosen), never out of the menu.
    let ids: Vec<Id> = (0..menu.lines.len()).map(|at| ui.id().with(("entry", at))).collect();
    let enabled: Vec<Id> = menu.lines.iter().zip(&ids).filter_map(|(line, id)| matches!(line, Line::Entry(entry) if entry.enabled()).then_some(*id)).collect();
    keys(ui, &enabled, focus_first);
    let last = menu.lines.len().saturating_sub(1);
    // (A reason said for the entry just before is not said again: "It loads first already" once
    // for the two moves it holds back.)
    let mut said: Option<&str> = None;
    for (at, (line, &id)) in menu.lines.iter().zip(&ids).enumerate() {
        match line {
            // (A rule before nothing is not drawn.)
            Line::Rule if at == last => {}
            Line::Rule => {
                let (rule, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                ui.painter().add(metal::seam(rule.x_range().shrink(10.0), rule.center().y));
                said = None;
            }
            Line::Entry(entry) => {
                let why = entry.why_not.as_deref();
                if let Some(by_keys) = entry_row(ui, id, width, entry, why.is_some() && why != said) {
                    *chosen = Some((entry.action.clone(), by_keys));
                }
                said = why;
            }
        }
    }
    ui.add_space(5.0);
    let rect = Rect::from_min_size(top, vec2(width, ui.cursor().min.y - top.y));
    let ctx = ui.ctx().clone();
    let brass = Rect::from_min_max(rect.min + vec2(1.0, 1.0), pos2(rect.max.x - 1.0, rect.min.y + 3.0));
    ui.painter().set(background, Shape::Vec(vec![
        egui::Shadow { offset: [0, 8], blur: 22, spread: 0, color: Color32::from_black_alpha(170) }.as_shape(rect, 3.0).into(),
        metal::plate(&ctx, rect, PANEL),
        // (The heading a shade lighter, as a plate's lit top.)
        metal::vgrad(Rect::from_min_max(rect.min, pos2(rect.max.x, head.bottom())), Color32::from_white_alpha(9), Color32::TRANSPARENT),
        Shape::rect_stroke(rect, 3.0, Stroke::new(1.0, metal::STEEL_EDGE), StrokeKind::Inside),
        Shape::rect_stroke(rect.shrink(1.0), 2.0, Stroke::new(1.0, Color32::from_white_alpha(8)), StrokeKind::Inside),
        metal::vgrad(brass, AMBER, AMBER_DIM),
    ]));
}

/// The keys while the menu is open: Down, Tab, Up, Shift+Tab, Home and End move from one entry
/// that can be chosen to the next (round from the last to the first); egui's own moves of the
/// keys (to the nearest widget that way, which may be under the menu) are held back.
fn keys(ui: &Ui, enabled: &[Id], focus_first: bool) {
    if enabled.is_empty() {
        return;
    }
    let focused = enabled.iter().position(|id| ui.memory(|m| m.has_focus(*id)));
    if let Some(at) = focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(enabled[at], EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
    }
    #[derive(Clone, Copy)]
    enum Step {
        Next,
        Back,
        First,
        Last,
    }
    let step = ui.input_mut(|i| {
        // (Shift+Tab before Tab: Tab's matches it too. Left and Right do nothing here.)
        let back = i.consume_key(Modifiers::SHIFT, Key::Tab);
        let next = !back && i.consume_key(Modifiers::NONE, Key::Tab);
        let _ = i.consume_key(Modifiers::NONE, Key::ArrowLeft) | i.consume_key(Modifiers::NONE, Key::ArrowRight);
        if back || i.consume_key(Modifiers::NONE, Key::ArrowUp) { Some(Step::Back) }
        else if next || i.consume_key(Modifiers::NONE, Key::ArrowDown) { Some(Step::Next) }
        else if i.consume_key(Modifiers::NONE, Key::Home) { Some(Step::First) }
        else if i.consume_key(Modifiers::NONE, Key::End) { Some(Step::Last) }
        else { None }
    });
    let count = enabled.len();
    let to = if focus_first { Some(0) } else {
        step.map(|step| match (step, focused) {
            (Step::First, _) | (Step::Next, None) => 0,
            (Step::Last, _) | (Step::Back, None) => count - 1,
            (Step::Next, Some(at)) => (at + 1) % count,
            (Step::Back, Some(at)) => (at + count - 1) % count,
        })
    };
    // (egui's own move of the keys, taken at the frame's start, is held back while the menu is
    // open: nothing under the menu may take them.)
    ui.memory_mut(|m| {
        if let Some(to) = to {
            m.request_focus(enabled[to]);
        }
        m.move_focus(FocusDirection::None);
    });
}

/// One entry: its mark, its name, its key, and (`say_why`) why not when it cannot be chosen; under
/// the pointer or with the keys, a brass lead and a warm band (red for removal). Chosen: whether
/// by the keys.
fn entry_row<A>(ui: &mut Ui, id: Id, width: f32, entry: &Entry<A>, say_why: bool) -> Option<bool> {
    let enabled = entry.enabled();
    let why = entry.why_not.as_ref().filter(|_| say_why).map(|why| widgets::wrapped_galley(ui, RichText::new(why).font(body(11.0)), (width - TEXT_X - PAD).max(1.0)));
    let height = ROW + why.as_ref().map_or(0.0, |why| why.size().y + 3.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &entry.label));
    if let Some(why) = &entry.why_not {
        ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(why.as_str()));
    }
    widgets::reveal_focus(ui, &response);
    // (A reason not said again under it, as the entry before has it, is its hint.)
    let response = match &entry.why_not {
        Some(why) if !say_why => response.hover_tip(why.as_str()),
        _ => response,
    };
    let hot = enabled && (response.hovered() || response.has_focus());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let band = rect.shrink2(vec2(4.0, 1.0));
        let danger = entry.tone == Tone::Danger;
        if hot {
            let (deep, lead) = if danger { (RED_DEEP, RED) } else { (AMBER_DEEP, AMBER) };
            widgets::gradient_h(painter, band, deep, deep.gamma_multiply(0.15));
            painter.rect_filled(Rect::from_min_size(band.min, vec2(3.0, band.height())), 1.0, lead);
            if response.is_pointer_button_down_on() {
                painter.rect_filled(band, 2.0, Color32::from_black_alpha(40));
            }
        }
        let tint = match (enabled, entry.tone, hot) {
            (false, ..) => STEEL_DIM,
            (true, Tone::Danger, false) => RED,
            (true, Tone::Danger, true) => Color32::from_rgb(246, 140, 120),
            (true, Tone::Main, false) => AMBER,
            (true, Tone::Main, true) | (true, Tone::Plain, true) => AMBER_BRIGHT,
            (true, Tone::Plain, false) => STEEL,
        };
        let middle = rect.top() + ROW / 2.0;
        paint_icon(painter, entry.icon, pos2(rect.left() + ICON_X, middle), if enabled { tint } else { tint.gamma_multiply(0.75) });
        let text = match (enabled, entry.tone, hot) {
            (false, ..) => STEEL_DIM,
            (true, Tone::Danger, _) => tint,
            (true, Tone::Main, false) => AMBER_BRIGHT,
            (true, _, true) => TEXT_BRIGHT,
            (true, Tone::Plain, false) => TEXT,
        };
        let keys = entry.keys.map(|keys| widgets::galley(ui, keys_text(keys)));
        let room = width - TEXT_X - PAD - keys.as_ref().map_or(0.0, |k| k.size().x + 16.0);
        let label = egui::WidgetText::from(label_text(&entry.label)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room.max(1.0), egui::TextStyle::Body);
        painter.galley_with_override_text_color(pos2(rect.left() + TEXT_X, middle - label.size().y / 2.0), label, text);
        if let Some(keys) = keys {
            // (A key the page answers too, as a small keycap at the right.)
            let cap = Rect::from_min_size(pos2(rect.right() - PAD - keys.size().x - 10.0, middle - 8.0), vec2(keys.size().x + 10.0, 16.0));
            painter.rect(cap, 2.0, INSET, Stroke::new(1.0, if hot { AMBER_DIM } else { LINE }), StrokeKind::Inside);
            painter.galley_with_override_text_color(cap.center() - keys.size() / 2.0, keys, if enabled { STEEL } else { STEEL_DIM });
        }
        if let Some(why) = why {
            painter.galley_with_override_text_color(pos2(rect.left() + TEXT_X, rect.top() + ROW - 5.0), why, STEEL);
        }
    }
    if enabled {
        let response = response.on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            return Some(!response.clicked_by(egui::PointerButton::Primary));
        }
    }
    None
}

/// An entry's mark, centred on `c`, about 13 points across.
pub fn paint_icon(painter: &egui::Painter, icon: Icon, c: Pos2, colour: Color32) {
    let p = |x: f32, y: f32| pos2(c.x + x, c.y + y);
    let stroke = Stroke::new(1.4, colour);
    let glyph = |glyph: widgets::Glyph, scale: f32| widgets::paint_glyph(painter, glyph, c, scale, colour);
    match icon {
        Icon::Details => {
            painter.circle_stroke(c, 6.0, stroke);
            painter.circle_filled(p(0.0, -2.8), 1.1, colour);
            painter.line_segment([p(0.0, -0.6), p(0.0, 3.4)], Stroke::new(1.6, colour));
        }
        Icon::Play => glyph(widgets::Glyph::Play, 0.8),
        Icon::Setup => {
            // Three sliders, their knobs set.
            for (y, knob) in [(-4.0, -2.5), (0.0, 2.5), (4.0, -0.5)] {
                painter.line_segment([p(-6.0, y), p(6.0, y)], Stroke::new(1.2, colour));
                painter.circle_filled(p(knob, y), 1.9, colour);
            }
        }
        Icon::Download => {
            painter.line_segment([p(0.0, -6.0), p(0.0, 2.0)], stroke);
            painter.add(Shape::line(vec![p(-3.6, -1.4), p(0.0, 2.4), p(3.6, -1.4)], stroke));
            painter.line_segment([p(-5.5, 5.5), p(5.5, 5.5)], stroke);
        }
        Icon::File => {
            painter.add(Shape::closed_line(vec![p(-4.5, -6.0), p(1.5, -6.0), p(4.5, -3.0), p(4.5, 6.0), p(-4.5, 6.0)], stroke));
            painter.add(Shape::line(vec![p(1.5, -6.0), p(1.5, -3.0), p(4.5, -3.0)], Stroke::new(1.0, colour)));
        }
        Icon::Cancel => glyph(widgets::Glyph::Close, 0.85),
        Icon::Switch(on) => {
            painter.add(metal::lamp(c, 3.8, GREEN, if on { 1.0 } else { 0.0 }));
        }
        Icon::Lamp(lamp, lit) => {
            painter.add(metal::lamp(c, 3.8, lamp, if lit { 1.0 } else { 0.0 }));
        }
        Icon::Up => glyph(widgets::Glyph::Up, 0.8),
        Icon::Down => glyph(widgets::Glyph::Down, 0.8),
        Icon::Top => glyph(widgets::Glyph::Top, 0.8),
        Icon::Bottom => glyph(widgets::Glyph::Bottom, 0.8),
        Icon::Report => {
            painter.rect_stroke(Rect::from_center_size(c, vec2(11.0, 11.0)), 2.0, stroke, StrokeKind::Middle);
            painter.add(Shape::line(vec![p(-2.8, 0.2), p(-0.6, 2.6), p(3.2, -2.4)], Stroke::new(1.6, colour)));
        }
        Icon::Folder => glyph(widgets::Glyph::Folder, 0.85),
        Icon::Web => {
            painter.add(Shape::line(vec![p(1.0, -5.0), p(-5.0, -5.0), p(-5.0, 5.0), p(5.0, 5.0), p(5.0, -1.0)], Stroke::new(1.2, colour)));
            painter.line_segment([p(-1.0, 1.0), p(5.5, -5.5)], stroke);
            painter.add(Shape::line(vec![p(1.5, -5.5), p(5.5, -5.5), p(5.5, -1.5)], stroke));
        }
        Icon::Goto => {
            painter.line_segment([p(-6.0, 0.0), p(3.5, 0.0)], stroke);
            painter.add(Shape::line(vec![p(0.0, -3.5), p(3.5, 0.0), p(0.0, 3.5)], stroke));
            painter.line_segment([p(6.0, -5.0), p(6.0, 5.0)], stroke);
        }
        Icon::Star(filled) => {
            if filled {
                super::star(painter, c, 6.5, colour);
            } else {
                let points: Vec<Pos2> = (0..10).map(|i| {
                    let angle = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
                    c + vec2(angle.cos(), angle.sin()) * if i % 2 == 0 { 6.5 } else { 2.9 }
                }).collect();
                painter.add(Shape::closed_line(points, Stroke::new(1.2, colour)));
            }
        }
        Icon::Eye(open) => {
            // An almond, its pupil; struck through when hidden.
            let lid = |sign: f32| -> Vec<Pos2> { (0..=12).map(|i| {
                let x = -6.5 + 13.0 * i as f32 / 12.0;
                p(x, sign * 4.2 * (1.0 - (x / 6.5).powi(2)))
            }).collect() };
            painter.add(Shape::line(lid(-1.0), Stroke::new(1.3, colour)));
            painter.add(Shape::line(lid(1.0), Stroke::new(1.3, colour)));
            painter.circle_filled(c, 2.0, colour);
            if !open {
                painter.line_segment([p(-5.5, 5.5), p(5.5, -5.5)], Stroke::new(3.2, PANEL));
                painter.line_segment([p(-5.5, 5.5), p(5.5, -5.5)], stroke);
            }
        }
        Icon::Edit => {
            // A pencil, its point down at the left.
            painter.line_segment([p(-2.6, 2.6), p(4.4, -4.4)], Stroke::new(3.2, colour));
            painter.add(Shape::convex_polygon(vec![p(-3.9, 1.3), p(-1.3, 3.9), p(-5.6, 5.6)], colour, Stroke::NONE));
            painter.line_segment([p(2.6, -6.2), p(6.2, -2.6)], Stroke::new(1.0, PANEL));
        }
        Icon::Copy => {
            painter.rect_stroke(Rect::from_min_max(p(-2.0, -5.5), p(5.5, 2.0)), 1.5, Stroke::new(1.2, colour), StrokeKind::Middle);
            painter.rect_filled(Rect::from_min_max(p(-5.5, -2.0), p(2.0, 5.5)), 1.5, PANEL);
            painter.rect_stroke(Rect::from_min_max(p(-5.5, -2.0), p(2.0, 5.5)), 1.5, stroke, StrokeKind::Middle);
        }
        Icon::Export => {
            painter.add(Shape::line(vec![p(-5.5, 0.5), p(-5.5, 5.5), p(5.5, 5.5), p(5.5, 0.5)], stroke));
            painter.line_segment([p(0.0, 2.5), p(0.0, -6.0)], stroke);
            painter.add(Shape::line(vec![p(-3.6, -2.6), p(0.0, -6.2), p(3.6, -2.6)], stroke));
        }
        Icon::Remove => glyph(widgets::Glyph::Trash, 0.85),
    }
}

/// The Menu key (VK_APPS) comes to egui as no key at all: read here as the frame begins (the key's
/// own event wakes the window), it is given to egui as Shift+F10. True as it goes down.
#[cfg(windows)]
pub fn menu_key(was_down: &mut bool, focused: bool) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_APPS};
    // (The high bit: down, as the messages taken in so far have it.)
    let down = focused && unsafe { GetKeyState(i32::from(VK_APPS.0)) } < 0;
    let pressed = down && !*was_down;
    *was_down = down;
    pressed
}

#[cfg(not(windows))]
pub fn menu_key(_was_down: &mut bool, _focused: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(output: &egui::FullOutput) -> Vec<(String, Rect)> {
        fn walk(shape: &Shape, into: &mut Vec<(String, Rect)>) {
            match shape {
                Shape::Text(text) => into.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()))),
                Shape::Vec(shapes) => for shape in shapes { walk(shape, into); },
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in &output.shapes { walk(&shape.shape, &mut texts); }
        texts
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Pick { Open, Star, Remove }

    fn sample() -> Menu<Pick> {
        let mut menu = Menu::new("Coast To Coast", "2 players · 16 × 16");
        menu.add("Open", Icon::Details, Pick::Open).keys("Enter");
        menu.rule();
        menu.add("Star", Icon::Star(true), Pick::Star).unless(Some("Not now"));
        menu.rule();
        menu.add("Remove", Icon::Remove, Pick::Remove).danger();
        menu.rule();
        menu
    }

    /// A right-click opens it at the pointer; a click on an entry chooses it and closes it; a
    /// dimmed entry cannot be chosen; Escape closes it; the keys go from entry to entry over the
    /// one that cannot be chosen, and come back to the card they opened it from.
    #[test]
    fn a_menu_opens_chooses_and_closes_as_a_menu_does() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        let menu_id = Id::new("test_menu");
        let card_id = Id::new("test_card");
        let mut picked = Vec::new();
        let frame = |events: Vec<egui::Event>, picked: &mut Vec<Pick>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(200.0, 150.0), Sense::hover());
                let card = ui.interact(rect, card_id, Sense::click());
                if let Some((at, keys)) = asked(ui, &card) {
                    open(ui.ctx(), menu_id, "coast", at, keys.then_some(card.id));
                }
                if target(ui.ctx(), menu_id).is_some() && let Some(pick) = show(ui, menu_id, &sample()) {
                    picked.push(pick);
                }
            });
            output.textures_delta.clear();
            output
        };
        let at = pos2(60.0, 60.0);
        // (A press and its release, a frame each, as the window gives them.)
        let click = |button, at, picked: &mut Vec<Pick>| {
            frame(vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: Modifiers::NONE }], picked);
            frame(vec![egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: Modifiers::NONE }], picked)
        };
        let open_now = |ctx: &Context| ctx.data(|d| d.get_temp::<Opened>(menu_id)).is_some_and(|opened| egui::Popup::is_id_open(ctx, popup_id(menu_id, opened.serial)));
        frame(Vec::new(), &mut picked);
        click(egui::PointerButton::Secondary, at, &mut picked);
        assert!(open_now(&ctx), "a right-click opens it");
        frame(Vec::new(), &mut picked);
        let texts = text_of(&frame(Vec::new(), &mut picked));
        let find = |texts: &[(String, Rect)], label: &str| texts.iter().find(|(text, _)| text == label).map(|(_, rect)| *rect);
        assert!(find(&texts, "Coast To Coast").is_some() && find(&texts, "Not now").is_some(), "the heading and the reason show: {texts:?}");
        let open_entry = find(&texts, "OPEN").expect("its entries in capitals");
        assert!(open_entry.left() > at.x && open_entry.top() > at.y, "it opens at the pointer, down and to the right: {open_entry:?}");
        // The dimmed entry does nothing; the others are chosen with a click, which closes it.
        click(egui::PointerButton::Primary, find(&texts, "STAR").unwrap().center(), &mut picked);
        assert!(picked.is_empty() && open_now(&ctx), "a dimmed entry cannot be chosen");
        click(egui::PointerButton::Primary, find(&texts, "REMOVE").unwrap().center(), &mut picked);
        assert_eq!(picked, [Pick::Remove]);
        frame(Vec::new(), &mut picked);
        assert!(!open_now(&ctx), "choosing closes it");
        // A click outside only closes it.
        click(egui::PointerButton::Secondary, at, &mut picked);
        frame(Vec::new(), &mut picked);
        assert!(open_now(&ctx));
        click(egui::PointerButton::Primary, pos2(700.0, 500.0), &mut picked);
        frame(Vec::new(), &mut picked);
        assert!(!open_now(&ctx) && picked.len() == 1, "a click outside closes it, choosing nothing");
        // A right-click outside closes it and opens it again where it was (on the card).
        click(egui::PointerButton::Secondary, at, &mut picked);
        frame(Vec::new(), &mut picked);
        let first_serial = ctx.data(|d| d.get_temp::<Opened>(menu_id)).unwrap().serial;
        click(egui::PointerButton::Secondary, pos2(120.0, 20.0), &mut picked);
        frame(Vec::new(), &mut picked);
        frame(Vec::new(), &mut picked);
        let hopped = ctx.data(|d| d.get_temp::<Opened>(menu_id)).unwrap();
        assert!(open_now(&ctx) && hopped.serial != first_serial && hopped.at == pos2(120.0, 20.0), "a right-click elsewhere on the card opens it there: {hopped:?}");
        click(egui::PointerButton::Primary, pos2(700.0, 500.0), &mut picked);
        frame(Vec::new(), &mut picked);
        assert!(!open_now(&ctx));
        // With the keys: Shift+F10 on the card opens it with its first entry focused; Down skips
        // the dimmed entry; Escape closes it, the keys back on the card.
        let key = |key, modifiers| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        ctx.memory_mut(|m| m.request_focus(card_id));
        frame(Vec::new(), &mut picked);
        frame(key(Key::F10, Modifiers::SHIFT), &mut picked);
        frame(Vec::new(), &mut picked);
        let first = ctx.memory(|m| m.focused());
        assert!(first.is_some() && first != Some(card_id), "the first entry takes the keys");
        frame(key(Key::ArrowDown, Modifiers::NONE), &mut picked);
        frame(Vec::new(), &mut picked);
        let second = ctx.memory(|m| m.focused());
        assert!(second.is_some() && second != first && second != Some(card_id), "Down goes to the next entry that can be chosen");
        frame(key(Key::ArrowDown, Modifiers::NONE), &mut picked);
        frame(Vec::new(), &mut picked);
        assert_eq!(ctx.memory(|m| m.focused()), first, "and round from the last to the first");
        frame(key(Key::Escape, Modifiers::NONE), &mut picked);
        frame(Vec::new(), &mut picked);
        assert!(!open_now(&ctx), "Escape closes it");
        frame(Vec::new(), &mut picked);
        assert_eq!(ctx.memory(|m| m.focused()), Some(card_id), "the keys come back to the card");
        assert_eq!(picked.len(), 1, "nothing chosen on the way");
        // Enter chooses with the keys.
        frame(key(Key::F10, Modifiers::SHIFT), &mut picked);
        frame(Vec::new(), &mut picked);
        frame(key(Key::Enter, Modifiers::NONE), &mut picked);
        assert_eq!(picked.last(), Some(&Pick::Open), "Enter chooses the entry with the keys");
    }

    #[test]
    fn rules_never_lead_double_or_trail() {
        let menu = sample();
        let kinds: Vec<bool> = menu.lines.iter().map(|line| matches!(line, Line::Rule)).collect();
        assert_eq!(kinds, [false, true, false, true, false, true], "(a trailing rule is drawn as nothing: see `plate`)");
        let mut empty: Menu<Pick> = Menu::new("", "");
        empty.rule();
        assert!(empty.lines.is_empty() && empty.is_empty());
    }
}
