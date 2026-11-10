//! Custom-painted widgets that give TA Reborn its look, in TA's materials
//! ([`crate::metal`]): cards of grimy plate in steel pipe frames, gunmetal and brass
//! buttons with stencil lettering, switches with indicator lamps, segmented selectors, the
//! launcher's Play button and the emblem.

use crate::tooltip::TooltipExt as _;
use std::sync::Arc;

use egui::{
    Align, Color32, CornerRadius, CursorIcon, Frame, Galley, InnerResponse, Label, Layout, Margin, Mesh, Painter, Pos2, Rect,
    Response, RichText, Sense, Shape, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, Widget, WidgetInfo,
    WidgetText, WidgetType, lerp, pos2, vec2,
};

use crate::i18n::{n, t, tf};
use crate::metal::{self, Slab};
use crate::theme::*;

/// Modifiers attached to the release that activated a pointer control. The frame's
/// final modifiers can already reflect a later Shift/Ctrl release. Keyboard activation
/// and synthetic clicks without a matching pointer event keep the current modifiers.
pub fn click_modifiers(response: &Response, button: egui::PointerButton) -> egui::Modifiers {
    // Egui owns the hit test: a valid nearby click or completed drag may end outside
    // the visual rectangle. Conversely, a release inside it need not activate it.
    let pointer = response.clicked_by(button) || response.drag_stopped_by(button);
    response.ctx.input(|input| {
        if !pointer {
            return input.modifiers;
        }
        input.raw.events.iter().rev().find_map(|event| match event {
            egui::Event::PointerButton { button: released, pressed: false, modifiers, .. }
                if *released == button => Some(*modifiers),
            _ => None,
        }).unwrap_or(input.modifiers)
    })
}

// ---- Geometry helpers ------------------------------------------------------------------

/// Outline of `rect` with the top-left and bottom-right corners cut off (clockwise): the
/// team colour swatches.
pub fn chamfer(rect: Rect, cut: f32) -> Vec<Pos2> {
    let c = cut.min(rect.height() / 2.0).min(rect.width() / 2.0);
    vec![
        pos2(rect.min.x + c, rect.min.y),
        pos2(rect.max.x, rect.min.y),
        pos2(rect.max.x, rect.max.y - c),
        pos2(rect.max.x - c, rect.max.y),
        pos2(rect.min.x, rect.max.y),
        pos2(rect.min.x, rect.min.y + c),
    ]
}

/// Lays out rich text on a single line, for widgets that paint their own labels.
pub fn galley(ui: &Ui, text: RichText) -> Arc<Galley> {
    WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Body)
}

/// `text` wrapped across `width`.
pub fn wrapped_galley(ui: &Ui, text: RichText, width: f32) -> Arc<Galley> {
    WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Wrap), width, TextStyle::Body)
}

/// How wide `text` is best wrapped within `width`: as narrow as it goes on as few lines as
/// there, so they come out even and no word is left alone on the last.
pub fn balanced_width(ui: &Ui, text: &RichText, width: f32) -> f32 {
    let lines = |width: f32| wrapped_galley(ui, text.clone(), width).rows.len();
    let n = lines(width);
    if n < 2 {
        return width;
    }
    let (mut narrow, mut wide) = (width / n as f32, width);
    while wide - narrow > 1.0 {
        let mid = (narrow + wide) / 2.0;
        if lines(mid) <= n {
            wide = mid;
        } else {
            narrow = mid;
        }
    }
    wide.ceil().min(width)
}

/// How wide `text` is best wrapped within `width` with no word alone on its last line: as
/// wide as it goes on as few lines as there with two words or more on the last (one where the
/// language is written without spaces).
pub fn pretty_width(ui: &Ui, text: &RichText, width: f32) -> f32 {
    let wrap = |width: f32| wrapped_galley(ui, text.clone(), width);
    let lone = |galley: &Galley| galley.rows.last().is_some_and(|row| row.row.glyphs.iter().map(|g| g.chr).collect::<String>().split_whitespace().count() < 2);
    let full = wrap(width);
    let n = full.rows.len();
    if n < 2 || !lone(&full) {
        return width;
    }
    // Narrower, until a second word comes down to the last line.
    let (mut good, mut lone_word) = (balanced_width(ui, text, width), width);
    if lone(&wrap(good)) {
        return width;
    }
    while lone_word - good > 1.0 {
        let mid = (good + lone_word) / 2.0;
        let galley = wrap(mid);
        if galley.rows.len() <= n && !lone(&galley) {
            good = mid;
        } else {
            lone_word = mid;
        }
    }
    good.floor()
}

/// A wrapped label with even lines ([`balanced_width`]), placed as the layout places it.
pub fn balanced_label(ui: &mut Ui, text: RichText) -> Response {
    let width = balanced_width(ui, &text, ui.available_width());
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.add(Label::new(text).wrap())
    })
    .inner
}

/// A wrapped label with no word alone on its last line ([`pretty_width`]).
pub fn pretty_label(ui: &mut Ui, text: RichText) -> Response {
    let width = pretty_width(ui, &text, ui.available_width());
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.add(Label::new(text).wrap())
    })
    .inner
}

/// `text(size)` on one line no wider than `width`: smaller when it would be wider (down to
/// `min_scale` of `size`), then cut short with "…" (translations run longer than English).
pub fn fitted_galley(ui: &Ui, text: impl Fn(f32) -> RichText, size: f32, width: f32, min_scale: f32) -> Arc<Galley> {
    let full = galley(ui, text(size));
    if full.size().x <= width {
        return full;
    }
    let scaled = (size * width / full.size().x - 0.25).max(size * min_scale);
    let smaller = galley(ui, text(scaled));
    if smaller.size().x <= width {
        return smaller;
    }
    WidgetText::from(text(scaled)).into_galley(ui, Some(TextWrapMode::Truncate), width, TextStyle::Body)
}

/// A display-font title, lettered as TA's are, that shrinks (to at most 60% of `size`) to
/// stay on one line. If even that minimum cannot fit, ellipsize within the available width.
pub fn fitted_title(ui: &mut Ui, text: &str, size: f32, color: Color32) -> Response {
    let rich = |size: f32| RichText::new(text).font(display(size)).extra_letter_spacing(crate::i18n::tracking(1.2));
    let available = ui.available_width().max(0.0);
    // The outline starts one point to the left, and its shadow reaches two to
    // the right. Letter spacing is fixed, so scaling the font by a width ratio
    // does not scale the whole label (notably with the wider bundled faces).
    let room = (available - 3.0).max(0.0);
    let mut heading = galley(ui, rich(size));
    let mut truncated = false;
    if heading.size().x > room {
        let (mut low, mut high) = (size * 0.6, size);
        heading = galley(ui, rich(low));
        if heading.size().x <= room {
            // Keep all words at the largest size that actually fits. These
            // measured galleys are cached by egui between unchanged frames.
            for _ in 0..8 {
                let middle = (low + high) * 0.5;
                let candidate = galley(ui, rich(middle));
                if candidate.size().x <= room {
                    low = middle;
                    heading = candidate;
                } else {
                    high = middle;
                }
            }
        } else {
            heading = WidgetText::from(rich(low)).into_galley(ui, Some(TextWrapMode::Truncate), room, TextStyle::Body);
            truncated = true;
        }
    }
    let (rect, response) = ui.allocate_exact_size(vec2((heading.size().x + 3.0).min(available), heading.size().y + 2.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(Rect::from_x_y_ranges(rect.x_range(), ui.clip_rect().y_range())));
    metal::embossed(&painter, rect.min + vec2(1.0, 0.0), heading, color);
    if truncated { response.hover_tip(text) } else { response }
}

/// Small letter-spaced caption, used for field names ("LOCATION", "SOURCE").
pub fn caption(text: &str) -> RichText {
    RichText::new(t(text)).font(strong(11.5)).color(STEEL).extra_letter_spacing(crate::i18n::tracking(1.2))
}

/// Fills `rect` with a horizontal gradient.
pub fn gradient_h(painter: &Painter, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
}

/// A soft round glow: `color` (its alpha the peak) out to `inner`, fading smoothly to nothing
/// at `outer` ((1 − s²)², no edge to band or alias), as one mesh whose vertex colours egui
/// blends (and dithers), where stacked translucent discs show their steps.
pub fn soft_glow(painter: &Painter, center: Pos2, inner: f32, outer: f32, color: Color32) {
    let rings = ((outer - inner) / 16.0).ceil().clamp(6.0, 32.0) as u32;
    let segments = (outer * 0.6).clamp(24.0, 96.0) as u32;
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let shade = |f: f32| {
        let alpha = a as f32 * f;
        Color32::from_rgba_premultiplied((r as f32 * alpha / 255.0).round() as u8, (g as f32 * alpha / 255.0).round() as u8, (b as f32 * alpha / 255.0).round() as u8, alpha.round() as u8)
    };
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, shade(1.0));
    for i in 0..=rings {
        let s = i as f32 / rings as f32;
        let radius = inner + (outer - inner) * s;
        let f = (1.0 - s * s).powi(2);
        for k in 0..segments {
            let angle = k as f32 / segments as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(center + vec2(angle.cos(), angle.sin()) * radius, shade(f));
        }
    }
    let at = |ring: u32, k: u32| 1 + ring * segments + k % segments;
    for k in 0..segments {
        mesh.add_triangle(0, at(0, k), at(0, k + 1));
    }
    for ring in 0..rings {
        for k in 0..segments {
            mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
            mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Fills `rect` with a vertical gradient.
pub fn gradient_v(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
}

/// Keeps a newly focused control in sight, including when it was below a scroll area's
/// clipping edge. Called before visibility culling; ordinary pointer scrolling is unchanged.
pub fn reveal_focus(ui: &Ui, response: &Response) {
    if response.gained_focus() {
        ui.scroll_to_rect_animation(response.rect.expand(7.0), None, egui::style::ScrollAnimation::none());
    }
}

/// How lit a button still is from its last click (1 as it is let go, fading to nothing in a
/// fifth of a second): the metal springs back up lit and dims, so a click that leaves the button
/// where it is (adding a player, hearing a briefing again) is seen to have been taken. Nothing
/// with reduced flashes.
pub fn press_light(ui: &Ui, response: &Response) -> f32 {
    const FADE: f64 = 0.22;
    let key = response.id.with("ta_ui::widgets::pressed_at");
    let now = ui.input(|i| i.time);
    if response.clicked() {
        ui.data_mut(|d| d.insert_temp(key, now));
    }
    if crate::motion::reduced_flashes(ui.ctx()) {
        return 0.0;
    }
    match ui.data(|d| d.get_temp::<f64>(key)) {
        Some(at) if now - at < FADE => {
            ui.ctx().request_repaint();
            let k = (1.0 - (now - at) / FADE).clamp(0.0, 1.0) as f32;
            k * k
        }
        _ => 0.0,
    }
}

/// Keyboard-focus ring around a custom widget: brass over a dark edge so it remains clear
/// on both dark steel and bright brass.
pub fn focus_ring(ui: &Ui, response: &Response, rect: Rect) {
    if response.has_focus() {
        let rim = rect.expand(3.0);
        ui.painter().rect_stroke(rim, CornerRadius::same(3), Stroke::new(4.0, Color32::BLACK), StrokeKind::Outside);
        ui.painter().rect_stroke(rim, CornerRadius::same(3), Stroke::new(2.0, AMBER_BRIGHT), StrokeKind::Outside);
    }
}

// ---- Layout --------------------------------------------------------------------------------

/// Gap between side-by-side cards.
pub const COLUMN_GAP: f32 = space::L;

/// Equal-width columns separated by [`COLUMN_GAP`] (egui's `columns` uses the item spacing,
/// which is too tight between cards).
pub fn columns<R>(ui: &mut Ui, count: usize, add_contents: impl FnOnce(&mut [Ui]) -> R) -> R {
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = COLUMN_GAP;
    let result = ui.columns(count, |columns| {
        for column in columns.iter_mut() {
            column.spacing_mut().item_spacing.x = spacing;
        }
        add_contents(columns)
    });
    ui.spacing_mut().item_spacing.x = spacing;
    result
}

// ---- Cards ---------------------------------------------------------------------------------

/// Thickness of a card's pipe frame.
const CARD_PIPE: f32 = 3.0;

/// The margin inside a card: the pipe frame and a gap.
pub fn card_margin() -> f32 {
    (metal::pipe_inset(CARD_PIPE) + space::M).round()
}

/// A plate of grimy metal in a steel pipe frame that fills the available width.
pub fn card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    card_accent(ui, None, add_contents)
}

/// Like [`card`], flagged (a warning, an error, news) by a lit lamp in its corner and its
/// top joints plated in the colour.
pub fn card_accent<R>(ui: &mut Ui, accent: Option<Color32>, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let background = ui.painter().add(Shape::Noop);
    let margin = card_margin() as i8;
    let inner = Frame::new().inner_margin(Margin::same(margin)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        // A plain top-down layout: inside a justified parent (e.g. `columns`) egui would
        // otherwise justify wrapped text, stretching the spaces between words.
        ui.with_layout(Layout::top_down(Align::Min), add_contents).inner
    });
    let rect = inner.response.rect;
    let ctx = ui.ctx().clone();
    let mut shapes = vec![metal::plate(&ctx, rect, PANEL), metal::pipe_frame(rect, CARD_PIPE)];
    if let Some(color) = accent {
        let path = metal::pipe_frame_path(rect, CARD_PIPE);
        for c in [path.left_top(), path.right_top()] {
            shapes.push(metal::accent_joint(c, metal::pipe_frame_joint(CARD_PIPE), color));
        }
        shapes.push(metal::lamp(pos2(rect.max.x - 20.0, rect.min.y + 20.0), 4.0, color, 1.0));
    }
    ui.painter().set(background, Shape::Vec(shapes));
    inner
}

/// What a [`footer_link_with`] shows before its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FooterGlyph {
    /// An eighth note (the Jukebox).
    Note,
    /// A play triangle (the film behind the title menu: WATCH).
    Play,
}

/// A link in a line of small text straight on the battle (the title screen's Jukebox): a note
/// and its name in brass, outlined as [`metal::outlined_label`] is, lit and underlined under the
/// pointer.
pub fn footer_link(ui: &mut Ui, label: &str) -> Response {
    footer_link_with(ui, label, FooterGlyph::Note)
}

/// [`footer_link`] with another glyph before its name.
pub fn footer_link_with(ui: &mut Ui, label: &str, glyph: FooterGlyph) -> Response {
    let label = t(label);
    let text = galley(ui, RichText::new(label).font(strong(11.5)));
    const NOTE: f32 = 9.0;
    let (rect, response) = ui.allocate_exact_size(vec2(NOTE + 5.0 + text.size().x + 2.0, text.size().y + 2.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, true, label));
    reveal_focus(ui, &response);
    if response.clicked() {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Tab);
    }
    if ui.is_rect_visible(rect) {
        let hot = response.hovered() || response.has_focus();
        let color = if hot { AMBER_BRIGHT } else { AMBER };
        let painter = ui.painter();
        match glyph {
            FooterGlyph::Note => {
                // An eighth note: its head, its stem and its flag, rimmed in black.
                let c = pos2(rect.min.x + 3.0, rect.center().y + 2.5);
                let stem = [pos2(c.x + 2.3, c.y), pos2(c.x + 2.3, c.y - 7.5)];
                let flag = [stem[1], pos2(c.x + 5.8, c.y - 4.2)];
                for (color, grow) in [(Color32::from_black_alpha(150), 1.0), (color, 0.0)] {
                    painter.circle_filled(c, 2.6 + grow, color);
                    painter.line_segment(stem, Stroke::new(1.4 + 2.0 * grow, color));
                    painter.line_segment(flag, Stroke::new(1.4 + 2.0 * grow, color));
                }
            }
            FooterGlyph::Play => {
                // A play triangle, rimmed in black.
                let c = pos2(rect.min.x + 4.0, rect.center().y);
                let points = vec![pos2(c.x - 3.0, c.y - 4.5), pos2(c.x + 4.5, c.y), pos2(c.x - 3.0, c.y + 4.5)];
                painter.add(Shape::convex_polygon(points.clone(), Color32::TRANSPARENT, Stroke::new(2.0, Color32::from_black_alpha(150))));
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            }
        }
        let at = pos2(rect.min.x + NOTE + 5.0 + 1.0, rect.min.y + 1.0);
        let foot = at.y + text.size().y;
        metal::outlined(painter, at, text.clone(), color);
        if hot {
            painter.hline(at.x..=at.x + text.size().x, foot - 1.5, Stroke::new(1.0, color));
        }
        focus_ring(ui, &response, rect.expand(2.0));
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A line of facts set apart by middle dots ("7 steps · about 7 min") with each fact's words
/// kept on one line where the line wraps (no "about 7" at the end of one line and "min" alone on
/// the next); `free` (a sentence among them, a summary) is left to wrap as it will.
pub fn keep_facts_whole(line: &str, free: &str) -> String {
    const NO_BREAK: char = '\u{a0}';
    line.split(" · ").map(|fact| if fact.contains(free) && !free.is_empty() { fact.to_owned() } else { fact.replace(' ', &NO_BREAK.to_string()) }).collect::<Vec<_>>().join(" · ")
}

/// [`card_title`] painted where it is wanted (`at` its top left) rather than laid out: for a
/// heading over something placed by hand. Returns the rect it covers.
pub fn card_title_at(ui: &Ui, painter: &Painter, at: Pos2, title: &str) -> Rect {
    let text = galley(ui, RichText::new(t(title)).font(strong(13.5)).extra_letter_spacing(crate::i18n::tracking(1.6)));
    let size = text.size();
    let text_at = at + vec2(15.0 + 1.0, 0.0);
    metal::embossed(painter, text_at, text, CREAM);
    painter.add(metal::rivet(pos2(at.x + 4.0, at.y + size.y / 2.0 - 0.6), 3.2));
    Rect::from_min_size(at, vec2(15.0 + 2.0 + size.x, size.y + 2.0))
}

/// A card's heading: letter-spaced cream capitals, outlined as TA's labels are, after a
/// rivet; close over what it heads.
pub fn card_title(ui: &mut Ui, title: &str) {
    let heading = ui.scope(|ui| {
        // (A row as tall as its lettering, not as a control.)
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let (mark, _) = ui.allocate_exact_size(vec2(8.0, 15.0), Sense::hover());
            let label = metal::embossed_label(ui, RichText::new(t(title)).font(strong(13.5)).extra_letter_spacing(crate::i18n::tracking(1.6)), CREAM);
            // The rivet level with the capitals (the label's foot holds their shadow; the
            // rivet's own falls below it).
            ui.painter().add(metal::rivet(pos2(mark.center().x, label.rect.min.y + (label.rect.height() - 2.0) / 2.0 - 0.4), 3.2));
        });
    });
    ui.add_space(space::XS);
    // (In a scroll area, never the last thing in sight: kept with the top of what follows.)
    crate::scroll::keep_together(ui, Rect::from_x_y_ranges(heading.response.rect.x_range(), heading.response.rect.min.y..=ui.cursor().min.y + 12.0));
}

// ---- Buttons -------------------------------------------------------------------------------

/// Colour scheme of an [`ActionButton`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Brass; reserved for the main action of a card.
    Primary,
    /// Gunmetal, TA's own buttons.
    Secondary,
    /// Rusted red.
    Danger,
}

impl Tone {
    fn slab(self) -> Slab {
        match self {
            Tone::Primary => Slab::Brass,
            Tone::Secondary => Slab::Steel,
            Tone::Danger => Slab::Rust,
        }
    }
}

/// A bevelled metal button with stencil lettering. Honors `ui.add_enabled(false, …)`.
pub struct ActionButton<'a> {
    tone: Tone,
    label: &'a str,
    /// Fixed width (else sized to the label, at least `min_width`), and height.
    width: Option<f32>,
    min_width: f32,
    height: f32,
}

impl<'a> ActionButton<'a> {
    pub fn primary(label: &'a str) -> Self {
        ActionButton { tone: Tone::Primary, label, width: None, min_width: 72.0, height: 32.0 }
    }

    pub fn secondary(label: &'a str) -> Self {
        ActionButton { tone: Tone::Secondary, label, width: None, min_width: 72.0, height: 32.0 }
    }

    pub fn danger(label: &'a str) -> Self {
        ActionButton { tone: Tone::Danger, label, width: None, min_width: 72.0, height: 32.0 }
    }

    /// A fixed size, e.g. the full width of a menu column.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = Some(width);
        self.height = height;
        self
    }

    /// A height of its own, the width still sized to the label (a translation longer than the
    /// English widens the button rather than spilling out of it).
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// At least this wide (buttons side by side kept alike), wider when the label needs it.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
}

impl Widget for ActionButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let label = t(self.label);
        let text = galley(ui, RichText::new(label).font(strong(if self.height >= 36.0 { 15.0 } else { 13.5 })).extra_letter_spacing(crate::i18n::tracking(0.8)));
        let size = vec2(self.width.unwrap_or((text.size().x + 30.0).max(self.min_width)), self.height);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
        reveal_focus(ui, &response);
        if response.clicked() {
            crate::sound::play(ui.ctx(), crate::sound::UiSound::Button);
        }
        if ui.is_rect_visible(rect) {
            let hot = ui.ctx().animate_bool_with_time(response.id.with("hot"), enabled && response.hovered(), 0.1);
            // (Just clicked: lit as it is when pressed, dimming back.)
            let lit = press_light(ui, &response);
            let down = enabled && response.is_pointer_button_down_on();
            // Unavailable, every tone is dull gunmetal, as hero_button's and play_button's are.
            let kind = if enabled { self.tone.slab() } else { Slab::Steel };
            let mut painter = ui.painter().clone();
            if !enabled {
                // add_enabled(false) halves all a widget paints (egui's disabled_alpha), and the
                // button dulls itself: it is painted solid (in any fade of the panel round it).
                painter.set_opacity(painter.opacity() / ui.visuals().disabled_alpha().max(0.05));
            }
            painter.add(metal::slab(ui.ctx(), rect, kind, hot.max(lit * 1.6), down));
            let hot = hot.max(lit);
            if enabled {
                metal::slab_text(&painter, rect.center(), text, kind.light_text(), kind.text(hot), down);
            } else {
                // Unavailable: the metal dulled, the lettering faded into it with a soft edge
                // under it (a stencil outline round dim letters smudged them).
                painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(110));
                let pos = (rect.center() - text.size() / 2.0).round();
                painter.galley_with_override_text_color(pos + vec2(0.0, 1.0), text.clone(), Color32::from_black_alpha(150));
                painter.galley_with_override_text_color(pos, text, STEEL_DIM);
            }
            focus_ring(ui, &response, rect);
        }
        if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
    }
}

/// Glyphs for [`icon_button`], painted as vectors so they stay crisp at any scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Up,
    Down,
    Left,
    Right,
    Close,
    /// A player's transport: the track before, play, stop, the next track.
    Previous,
    Play,
    Stop,
    Next,
    /// A magnifying glass (a search box's).
    Search,
    /// A waste bin (removing something).
    Trash,
    /// A folder (opening one).
    Folder,
    /// A triangle up to a bar, and down to one: to the top of a list, and to its bottom.
    Top,
    Bottom,
    /// A player's order of play: crossed arrows (at random), a loop (again).
    Shuffle,
    Repeat,
    /// A loudspeaker (a volume).
    Speaker,
}

/// A small square gunmetal button with a painted glyph.
pub fn icon_button(ui: &mut Ui, glyph: Glyph, enabled: bool) -> Response {
    glyph_button(ui, glyph, vec2(26.0, 26.0), Tone::Secondary, enabled)
}

/// A button of `size` with a painted glyph (its size follows the button's height), in metal of
/// `tone`.
pub fn glyph_button(ui: &mut Ui, glyph: Glyph, size: egui::Vec2, tone: Tone, enabled: bool) -> Response {
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let label = match glyph {
        Glyph::Up => t("Up"), Glyph::Down => t("Down"), Glyph::Left => t("Left"), Glyph::Right => t("Right"),
        Glyph::Close => t("Close"), Glyph::Previous => t("Previous"), Glyph::Next => t("Next"),
        Glyph::Play => t("Play"), Glyph::Stop => t("Stop"), Glyph::Search => t("Search"),
        Glyph::Trash => t("Remove"), Glyph::Folder => t("Open folder"), Glyph::Top => t("Move to the top"), Glyph::Bottom => t("Move to the bottom"),
        Glyph::Shuffle => t("Shuffle"), Glyph::Repeat => t("Repeat"), Glyph::Speaker => t("Volume"),
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    reveal_focus(ui, &response);
    if response.clicked() {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Button);
    }
    if ui.is_rect_visible(rect) {
        let hot = if enabled && response.hovered() { 1.0f32 } else { 0.0 };
        let lit = press_light(ui, &response);
        let down = enabled && response.is_pointer_button_down_on();
        let kind = tone.slab();
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, kind, hot.max(lit * 1.6), down));
        let color = if enabled {
            kind.text(hot)
        } else {
            painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(110));
            STEEL_DIM
        };
        // The glyph, with a dark edge below and to the right as the lettering has (a light one
        // under it on brass, as its lettering is engraved).
        let scale = (rect.height() / 26.0).max(0.5);
        let c = rect.center() + vec2(0.0, if down { 1.0 } else { 0.0 });
        let edge = if kind.light_text() || !enabled { Color32::from_black_alpha(190) } else { Color32::from_white_alpha(80) };
        paint_glyph(painter, glyph, c + if kind.light_text() || !enabled { vec2(1.0, 1.0) } else { vec2(0.0, 1.0) }, scale, edge);
        paint_glyph(painter, glyph, c, scale, color);
        focus_ring(ui, &response, rect);
    }
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// A button's glyph on its own, centred on `c` (`scale` 1 for a 26-point button's).
pub fn paint_glyph(painter: &Painter, glyph: Glyph, c: Pos2, scale: f32, color: Color32) {
    let p = |x: f32, y: f32| pos2(c.x + x * scale, c.y + y * scale);
    let triangle = |points: [Pos2; 3]| Shape::convex_polygon(points.to_vec(), color, Stroke::NONE);
    match glyph {
        Glyph::Up => {
            painter.add(triangle([p(0.0, -4.0), p(5.0, 3.0), p(-5.0, 3.0)]));
        }
        Glyph::Down => {
            painter.add(triangle([p(-5.0, -3.0), p(5.0, -3.0), p(0.0, 4.0)]));
        }
        Glyph::Left => {
            painter.add(triangle([p(-4.0, 0.0), p(3.0, -5.0), p(3.0, 5.0)]));
        }
        Glyph::Right => {
            painter.add(triangle([p(4.0, 0.0), p(-3.0, 5.0), p(-3.0, -5.0)]));
        }
        Glyph::Close => {
            let s = Stroke::new(1.8 * scale, color);
            painter.line_segment([p(-4.0, -4.0), p(4.0, 4.0)], s);
            painter.line_segment([p(4.0, -4.0), p(-4.0, 4.0)], s);
        }
        Glyph::Play => {
            painter.add(triangle([p(-3.5, -5.5), p(5.0, 0.0), p(-3.5, 5.5)]));
        }
        Glyph::Stop => {
            painter.add(Shape::rect_filled(Rect::from_min_max(p(-4.5, -4.5), p(4.5, 4.5)), CornerRadius::same(1), color));
        }
        Glyph::Search => {
            painter.circle_stroke(p(-1.5, -1.5), 4.2 * scale, Stroke::new(1.6 * scale, color));
            painter.line_segment([p(1.6, 1.6), p(5.0, 5.0)], Stroke::new(2.0 * scale, color));
        }
        Glyph::Trash => {
            let s = Stroke::new(1.5 * scale, color);
            // The lid and its handle, the bin narrowing to its foot, two ribs.
            painter.line_segment([p(-5.5, -3.5), p(5.5, -3.5)], s);
            painter.add(Shape::line(vec![p(-2.0, -3.5), p(-2.0, -5.5), p(2.0, -5.5), p(2.0, -3.5)], s));
            painter.add(Shape::closed_line(vec![p(-4.2, -1.8), p(4.2, -1.8), p(3.4, 5.5), p(-3.4, 5.5)], s));
            painter.line_segment([p(-1.2, 0.0), p(-1.0, 3.8)], Stroke::new(1.2 * scale, color));
            painter.line_segment([p(1.2, 0.0), p(1.0, 3.8)], Stroke::new(1.2 * scale, color));
        }
        Glyph::Folder => {
            let s = Stroke::new(1.5 * scale, color);
            painter.add(Shape::closed_line(vec![p(-6.0, -4.5), p(-1.5, -4.5), p(0.0, -3.0), p(6.0, -3.0), p(6.0, 4.5), p(-6.0, 4.5)], s));
            painter.line_segment([p(-6.0, -1.0), p(6.0, -1.0)], s);
        }
        Glyph::Top | Glyph::Bottom => {
            let d = if glyph == Glyph::Top { 1.0 } else { -1.0 };
            painter.add(triangle([p(0.0, -2.0 * d), p(5.0, 4.5 * d), p(-5.0, 4.5 * d)]));
            painter.add(Shape::rect_filled(Rect::from_two_pos(p(-5.0, -5.5 * d), p(5.0, -3.6 * d)), CornerRadius::ZERO, color));
        }
        Glyph::Shuffle => {
            // Two paths crossing, each ending in an arrowhead to the right.
            let s = Stroke::new(1.5 * scale, color);
            painter.add(Shape::line(vec![p(-6.0, -3.5), p(-2.5, -3.5), p(2.0, 3.5), p(4.5, 3.5)], s));
            painter.add(Shape::line(vec![p(-6.0, 3.5), p(-2.5, 3.5), p(2.0, -3.5), p(4.5, -3.5)], s));
            painter.add(triangle([p(4.0, 1.0), p(7.0, 3.5), p(4.0, 6.0)]));
            painter.add(triangle([p(4.0, -6.0), p(7.0, -3.5), p(4.0, -1.0)]));
        }
        Glyph::Repeat => {
            // A loop: along the top to the right, back along the bottom to the left.
            let s = Stroke::new(1.5 * scale, color);
            painter.add(Shape::line(vec![p(-5.5, 1.5), p(-5.5, -3.5), p(3.0, -3.5)], s));
            painter.add(triangle([p(2.5, -6.0), p(6.0, -3.5), p(2.5, -1.0)]));
            painter.add(Shape::line(vec![p(5.5, -1.5), p(5.5, 3.5), p(-3.0, 3.5)], s));
            painter.add(triangle([p(-2.5, 1.0), p(-6.0, 3.5), p(-2.5, 6.0)]));
        }
        Glyph::Speaker => {
            // Its box and cone, and two waves of sound.
            painter.add(Shape::convex_polygon(vec![p(-6.0, -2.2), p(-3.0, -2.2), p(0.5, -5.5), p(0.5, 5.5), p(-3.0, 2.2), p(-6.0, 2.2)], color, Stroke::NONE));
            let s = Stroke::new(1.3 * scale, color);
            for (r, reach) in [(3.2, 0.75), (6.0, 0.8)] {
                let arc: Vec<Pos2> = (0..=8).map(|i| {
                    let a = (i as f32 / 8.0 - 0.5) * 2.0 * reach;
                    p(1.5 + r * a.cos(), r * a.sin())
                }).collect();
                painter.add(Shape::line(arc, s));
            }
        }
        Glyph::Previous | Glyph::Next => {
            // A triangle pointing at a bar: right for the next track, left for the one before.
            let d = if glyph == Glyph::Next { 1.0 } else { -1.0 };
            painter.add(triangle([p(-4.0 * d, -5.0), p(3.0 * d, 0.0), p(-4.0 * d, 5.0)]));
            painter.add(Shape::rect_filled(Rect::from_two_pos(p(3.0 * d, -5.0), p(5.2 * d, 5.0)), CornerRadius::ZERO, color));
        }
    }
}

/// State of the big Play button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Ready,
    Running,
    /// No valid install: shown, but inert.
    Unavailable,
}

/// The large brass Play button, bolted on; `compact` for narrow windows.
pub fn play_button(ui: &mut Ui, state: PlayState, compact: bool) -> Response {
    play_button_width(ui, state, if compact { 214.0 } else { 270.0 })
}

/// One launch target with its readiness indicator built into the same metal housing.
/// The status has a dark, high-contrast well beneath the brass action rather than
/// competing with its label. Width is bounded for compact navigation and translations.
pub fn play_button_width(ui: &mut Ui, state: PlayState, width: f32) -> Response {
    let ready = state == PlayState::Ready;
    let width = width.min(ui.available_width()).max(1.0);
    let compact = width < 240.0;
    let status = match state {
        PlayState::Ready => n("GAME DATA VERIFIED"),
        PlayState::Running => n("GAME RUNNING"),
        PlayState::Unavailable => n("ORIGINAL GAME NOT FOUND"),
    };
    let status_text = wrapped_galley(ui, RichText::new(t(status)).font(strong(10.0)), (width - 34.0).max(1.0));
    let status_height = (status_text.size().y + 12.0).max(26.0);
    let size = vec2(width, if compact { 54.0 } else { 66.0 } + status_height);
    let (rect, response) = ui.allocate_exact_size(size, if ready { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ready && ui.is_enabled(),
        tf("{action} · {status}", &[("action", &t(if state == PlayState::Running { "Running" } else { "Play" })), ("status", &t(status))])));
    reveal_focus(ui, &response);
    if response.clicked() {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::BigButton);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), ready && response.hovered(), 0.15);
    let down = ready && response.is_pointer_button_down_on();
    let painter = ui.painter();
    let size = if width < 190.0 { 27.0 } else if compact { 32.0 } else { 40.0 };
    let action_rect = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - status_height));
    match state {
        PlayState::Ready => {
            painter.add(metal::slab(ui.ctx(), rect, Slab::Brass, hover, down));
            // Now and then a gleam runs across the brass (not while the window is in the
            // background).
            let (period, crossing) = (5.0, 0.9);
            let time = ui.input(|i| i.time) % period;
            if time < crossing && !crate::motion::still(ui.ctx()) {
                let x = rect.min.x - 40.0 + (rect.width() + 80.0) * (time / crossing) as f32;
                let band = vec![pos2(x, rect.min.y), pos2(x + 30.0, rect.min.y), pos2(x + 8.0, rect.max.y), pos2(x - 22.0, rect.max.y)];
                painter.with_clip_rect(rect.shrink(2.0)).add(Shape::convex_polygon(band, Color32::from_white_alpha(42), Stroke::NONE));
                crate::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(16));
            } else {
                crate::motion::next_frame(ui.ctx(), std::time::Duration::from_secs_f64((period - time).max(0.0)));
            }
            bolted(painter, rect);
            play_label(painter, action_rect, "PLAY", size, Slab::Brass, ON_AMBER, down);
        }
        PlayState::Running => {
            painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, 0.0, false));
            // The lamp pulses (steady while the window is in the background: the game runs
            // beside it, and every frame drawn here is one taken from it).
            let pulse = if crate::motion::still(ui.ctx()) { 1.0 } else { (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.3 + 0.7 };
            crate::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(33));
            painter.add(metal::lamp(pos2(rect.min.x + 22.0, rect.center().y), 5.0, GREEN, pulse));
            play_label(painter, action_rect, "RUNNING", size * 0.8, Slab::Slot, AMBER, false);
        }
        PlayState::Unavailable => {
            painter.add(metal::slab(ui.ctx(), rect, Slab::Steel, 0.0, false));
            painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(110));
            bolted(painter, rect);
            play_label(painter, action_rect, "PLAY", size, Slab::Steel, STEEL_DIM, false);
        }
    }
    let status_rect = Rect::from_min_max(pos2(rect.min.x + 3.0, action_rect.max.y), rect.max - vec2(3.0, 3.0));
    painter.rect_filled(status_rect, CornerRadius::same(1), BG_DEEP);
    painter.hline(status_rect.x_range(), status_rect.min.y, Stroke::new(1.0, Color32::from_black_alpha(170)));
    let status_color = match state { PlayState::Ready => GREEN, PlayState::Running => AMBER, PlayState::Unavailable => STEEL };
    let row_width = status_text.size().x + 13.0;
    let start = status_rect.center().x - row_width / 2.0;
    painter.add(metal::lamp(pos2(start + 3.0, status_rect.center().y), 2.6, status_color, 1.0));
    painter.galley(pos2(start + 13.0, status_rect.center().y - status_text.size().y / 2.0), status_text, status_color);
    focus_ring(ui, &response, rect);
    if ready { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// A bolt in each corner of a big slab.
fn bolted(painter: &Painter, rect: Rect) {
    let inset = 9.0;
    for c in [rect.left_top() + vec2(inset, inset), rect.right_top() + vec2(-inset, inset), rect.left_bottom() + vec2(inset, -inset), rect.right_bottom() + vec2(-inset, -inset)] {
        painter.add(metal::bolt(c, 3.6));
    }
}

/// A triangle and a word, centred on a big slab, lettered as the slab's are.
fn play_label(painter: &Painter, rect: Rect, text: &str, size: f32, kind: Slab, color: Color32, down: bool) {
    let natural = painter.layout_no_wrap(t(text).to_owned(), display(size), color);
    let size = size * ((rect.width() - 24.0).max(1.0) / (natural.size().x + size * 0.83)).min(1.0);
    let galley = painter.layout_no_wrap(t(text).to_owned(), display(size), color);
    let (tri_w, tri_h, gap) = (size * 0.48, size * 0.6, size * 0.35);
    let left = rect.center().x - (tri_w + gap + galley.size().x) / 2.0;
    let cy = rect.center().y + if down { 1.0 } else { 0.0 };
    let triangle = |dx: f32, dy: f32| vec![pos2(left + dx, cy - tri_h / 2.0 + dy), pos2(left + tri_w + dx, cy + dy), pos2(left + dx, cy + tri_h / 2.0 + dy)];
    let edge = if kind.light_text() { Color32::from_black_alpha(190) } else { Color32::from_white_alpha(80) };
    painter.add(Shape::convex_polygon(triangle(if kind.light_text() { 1.0 } else { 0.0 }, 1.0), edge, Stroke::NONE));
    painter.add(Shape::convex_polygon(triangle(0.0, 0.0), color, Stroke::NONE));
    let text_center = pos2(left + tri_w + gap + galley.size().x / 2.0, rect.center().y);
    metal::slab_text(painter, text_center, galley, kind.light_text(), color, down);
}

// ---- Inputs --------------------------------------------------------------------------------

/// A square recessed checkbox with a brass check mark.
pub fn check_box(ui: &mut Ui, checked: &mut bool) -> Response {
    check_box_named(ui, checked, "")
}

/// [`check_box`] with a translated name for keyboard and assistive-technology metadata.
pub fn check_box_named(ui: &mut Ui, checked: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle);
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *checked, t(label)));
    reveal_focus(ui, &response);
    if ui.is_rect_visible(rect) {
        let r = rect.shrink(1.0);
        let painter = ui.painter();
        painter.add(metal::well(r, if response.hovered() { PANEL_HI } else { INSET }));
        if *checked {
            paint_check(painter, r.translate(vec2(0.5, 1.0)), Stroke::new(2.4, Color32::from_black_alpha(200)));
            paint_check(painter, r, Stroke::new(2.2, AMBER_BRIGHT));
        }
        focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn paint_check(painter: &Painter, r: Rect, stroke: Stroke) {
    painter.add(Shape::line(
        vec![
            pos2(r.min.x + r.width() * 0.24, r.min.y + r.height() * 0.52),
            pos2(r.min.x + r.width() * 0.43, r.min.y + r.height() * 0.72),
            pos2(r.min.x + r.width() * 0.78, r.min.y + r.height() * 0.30),
        ],
        stroke,
    ));
}

/// An on/off switch: a gunmetal knob sliding in a dark slot, uncovering a green lamp when
/// on (TA's switches light a lamp).
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    toggle_named(ui, on, "")
}

/// [`toggle`] with a translated name for keyboard and assistive-technology metadata.
pub fn toggle_named(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(48.0, 22.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle);
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, t(label)));
    reveal_focus(ui, &response);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, hot, false));
        painter.add(metal::lamp(pos2(rect.min.x + 11.0, rect.center().y), 4.0, GREEN, t));
        let knob_w = 22.0;
        let x = lerp((rect.min.x + 2.0)..=(rect.max.x - 2.0 - knob_w), t);
        let knob = Rect::from_min_size(pos2(x, rect.min.y + 2.0), vec2(knob_w, rect.height() - 4.0));
        painter.add(metal::slab(ui.ctx(), knob, Slab::Steel, hot, false));
        // Grip ridges.
        for dx in [-4.0, 0.0, 4.0] {
            let x = knob.center().x + dx;
            painter.vline(x, (knob.min.y + 5.0)..=(knob.max.y - 5.0), Stroke::new(1.0, Color32::from_black_alpha(130)));
            painter.vline(x + 1.0, (knob.min.y + 5.0)..=(knob.max.y - 5.0), Stroke::new(1.0, Color32::from_white_alpha(40)));
        }
        focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The widest figure ('0' to '9') of `font`. Bahnschrift's figures are proportional (a 1 is
/// two thirds as wide as a 4) and egui shapes text without OpenType features (no tabular
/// figures), so a column of numbers is set in cells this wide.
pub fn figure_width(painter: &Painter, font: &egui::FontId) -> f32 {
    ('0'..='9').map(|d| painter.layout_no_wrap(d.to_string(), font.clone(), TEXT).size().x).fold(0.0, f32::max)
}

/// `text` (figures) ending at `right`, each figure centred in a cell of the widest figure's
/// width, so numbers line up row after row; the rect it covers.
pub fn paint_figures(painter: &Painter, right: f32, centre_y: f32, text: &str, font: &egui::FontId, color: Color32) -> Rect {
    let cell = figure_width(painter, font);
    let count = text.chars().count() as f32;
    let left = right - cell * count;
    let mut height = 0.0f32;
    for (i, c) in text.chars().enumerate() {
        let figure = painter.layout_no_wrap(c.to_string(), font.clone(), color);
        height = height.max(figure.size().y);
        painter.galley(pos2(left + cell * i as f32 + (cell - figure.size().x) / 2.0, centre_y - figure.size().y / 2.0), figure, color);
    }
    Rect::from_min_max(pos2(left, centre_y - height / 2.0), pos2(right, centre_y + height / 2.0))
}

/// `n` in a column `digits` figures wide (leading zeros when `zeros`), so numbers line up row
/// after row and what follows them starts at one place.
pub fn figures(ui: &mut Ui, n: usize, digits: usize, zeros: bool, font: egui::FontId, color: Color32) -> Response {
    let text = if zeros { format!("{n:0digits$}") } else { n.to_string() };
    let cell = figure_width(ui.painter(), &font);
    let height = ui.painter().layout_no_wrap("0".to_owned(), font.clone(), color).size().y;
    let (rect, response) = ui.allocate_exact_size(vec2(digits as f32 * cell, height), Sense::hover());
    paint_figures(ui.painter(), rect.max.x, rect.center().y, &text, &font, color);
    response
}

/// A search box that matches the drop-downs it stands with: their height and side padding,
/// a magnifier in their arrows' column (where the hint leaves room for it), its hint (`hint`,
/// translated) quieter than typed text. `id` keeps its focus where it is looked up by (Ctrl+F).
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32, id: Option<egui::Id>) -> Response {
    let s = ui.spacing().clone();
    // (A drop-down's height.)
    let height = (ui.text_style_height(&TextStyle::Button) + 2.0 * s.button_padding.y).max(s.interact_size.y);
    let left = s.button_padding.x.round() as i8;
    let glyph_room = (s.button_padding.x + s.icon_width + s.icon_spacing).round() as i8;
    let hint_width = WidgetText::from(RichText::new(hint)).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Body).size().x;
    let magnifier = hint_width + (left + glyph_room) as f32 + 2.0 <= width;
    let right = if magnifier { glyph_room } else { left };
    let mut edit = egui::TextEdit::singleline(text)
        .hint_text(RichText::new(hint).color(STEEL_DIM))
        .desired_width(width)
        .min_size(vec2(0.0, height))
        .vertical_align(Align::Center)
        .margin(Margin { left, right, top: 2, bottom: 2 });
    if let Some(id) = id {
        edit = edit.id(id);
    }
    let response = ui.add(edit);
    if magnifier {
        let glyph = pos2(response.rect.right() - s.button_padding.x - s.icon_width / 2.0, response.rect.center().y);
        let color = if response.has_focus() {
            AMBER
        } else if response.hovered() {
            TEXT
        } else {
            STEEL
        };
        paint_glyph(ui.painter(), Glyph::Search, glyph, 1.0, color);
    }
    response
}

/// The height of a form's controls: a value box, a row of choices, a key binding.
pub const CONTROL_HEIGHT: f32 = 28.0;

/// How wide a slider's value box must be for the widest of `samples` (as the box letters
/// them), so a form's boxes can all be one width.
pub fn value_box_width(ui: &Ui, samples: &[&str]) -> f32 {
    let style = ui.style();
    let font = style.drag_value_text_style.resolve(style);
    let widest = samples.iter().map(|s| ui.painter().layout_no_wrap((*s).to_owned(), font.clone(), TEXT).size().x).fold(0.0, f32::max);
    (widest + 2.0 * style.spacing.button_padding.x + 2.0).max(style.spacing.interact_size.x).ceil()
}

/// A slider filling the room it has, its value in a box `value_box` wide at the right: the
/// rail running in a slot to a gap before the box, the box as tall as a form's other controls
/// and level with the rail, so a form's sliders and boxes line up whatever their values.
pub fn value_slider(ui: &mut Ui, slider: egui::Slider<'_>, value_box: f32) -> Response {
    let room = ui.available_width();
    ui.scope(|ui| {
        let spacing = ui.spacing_mut();
        spacing.item_spacing.x = space::M;
        spacing.interact_size = vec2(value_box, CONTROL_HEIGHT);
        spacing.slider_width = (room - value_box - space::M).max(60.0);
        // (egui paints the rail and the knob at rest in the same fill: a little lighter than
        // the plate, so the rail shows and the knob reads as solid; lighter still under the
        // pointer.)
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.bg_fill = LINE;
        widgets.hovered.bg_fill = Color32::from_rgb(78, 75, 65);
        let slot = ui.painter().add(Shape::Noop);
        let response = ui.add(slider);
        if response.changed() {
            let key = response.id.with("ta_ui::widgets::slider_tick");
            let now = ui.input(|i| i.time);
            if ui.data(|d| d.get_temp::<f64>(key)).is_none_or(|then| now - then >= 0.08) {
                ui.data_mut(|d| d.insert_temp(key, now));
                crate::sound::play(ui.ctx(), crate::sound::UiSound::Slider);
            }
        }
        let (width, height) = (ui.spacing().slider_width, ui.spacing().slider_rail_height);
        let rail = Rect::from_center_size(pos2(response.rect.min.x + width / 2.0, response.rect.center().y), vec2(width, height));
        ui.painter().set(
            slot,
            Shape::Vec(vec![
                Shape::rect_filled(rail.expand(1.0), CornerRadius::same(3), Color32::from_black_alpha(140)),
                Shape::hline(rail.x_range(), rail.max.y + 1.5, Stroke::new(1.0, Color32::from_white_alpha(16))),
            ]),
        );
        response
    })
    .inner
}

/// A key binding ("Ctrl+F") as the keys it takes, [CTRL] + [F], in a dark slot `width` wide
/// (wider for a long one), lit under the pointer; "PRESS A KEY…" in brass while it waits for
/// one.
pub fn key_binding(ui: &mut Ui, binding: &str, capturing: bool, width: f32) -> Response {
    const CAP: f32 = 22.0;
    const PLUS: f32 = 6.0;
    let keys: Vec<Arc<Galley>> = binding.split('+').map(str::trim).filter(|k| !k.is_empty()).map(|k| galley(ui, RichText::new(k.to_uppercase()).font(strong(12.0)))).collect();
    let plus = galley(ui, RichText::new("+").font(strong(11.0)));
    let caps: Vec<f32> = keys.iter().map(|k| (k.size().x + 12.0).max(CAP)).collect();
    let row = caps.iter().sum::<f32>() + (caps.len().saturating_sub(1)) as f32 * (plus.size().x + 2.0 * PLUS);
    let (rect, response) = ui.allocate_exact_size(vec2(width.max(row + 12.0), CONTROL_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), binding));
    reveal_focus(ui, &response);
    if response.clicked() {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Button);
    }
    if ui.is_rect_visible(rect) {
        let hot = if capturing || response.hovered() { 1.0 } else { 0.0 };
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, hot, false));
        if capturing {
            let pulse = 0.65 + 0.35 * (ui.input(|i| i.time) as f32 * 4.0).sin().abs();
            let text = galley(ui, RichText::new(t("PRESS A KEY…")).font(strong(12.0)));
            painter.galley(rect.center() - text.size() / 2.0, text, AMBER_BRIGHT.gamma_multiply(pulse));
            ui.ctx().request_repaint();
        } else {
            let mut x = rect.center().x - row / 2.0;
            for (i, (key, w)) in keys.into_iter().zip(caps).enumerate() {
                if i > 0 {
                    painter.galley(pos2(x + PLUS, rect.center().y - plus.size().y / 2.0), plus.clone(), STEEL_DIM);
                    x += plus.size().x + 2.0 * PLUS;
                }
                let cap = Rect::from_min_size(pos2(x, rect.center().y - CAP / 2.0), vec2(w, CAP));
                painter.add(metal::slab(ui.ctx(), cap, Slab::Steel, hot, false));
                metal::slab_text(painter, cap.center(), key, true, Slab::Steel.text(hot), false);
                x += w;
            }
        }
        focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A row of mutually exclusive options; `changed()` is set when the selection changes. The
/// segments are all as wide as the widest needs where there is room; where there is not, as
/// wide as their own lettering, then tighter, then in smaller lettering, and failing that they
/// become a drop-down list. The one chosen is a pressed gunmetal chip with a brass bar under
/// its lettering.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    segments(ui, value, options, true, false)
}

/// [`segmented`] across all the room it has (a form's column), its segments sharing it.
pub fn segmented_fill<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    segments(ui, value, options, true, true)
}

/// [`segmented`] with labels shown as they are, not translated (names from the game data).
pub fn segmented_raw<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    segments(ui, value, options, false, false)
}

/// [`segmented_raw`] across all the room it has (a tab for each, across a column).
pub fn segmented_raw_fill<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    segments(ui, value, options, false, true)
}

fn segments<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], translate: bool, fill: bool) -> Response {
    const PAD: f32 = 14.0;
    const TIGHT_PAD: f32 = 7.0;
    const LEAST: f32 = 58.0;
    const HEIGHT: f32 = 28.0;
    let shown = |label: &str| if translate { t(label).to_owned() } else { label.to_owned() };
    let lettering = |size: f32, tracking: f32| -> Vec<Arc<Galley>> {
        options
            .iter()
            .map(|(_, label)| galley(ui, RichText::new(shown(label)).font(strong(size)).extra_letter_spacing(crate::i18n::tracking(tracking))))
            .collect()
    };
    let fit = |galleys: &[Arc<Galley>], pad: f32, least: f32| -> Vec<f32> { galleys.iter().map(|g| (g.size().x + 2.0 * pad).max(least)).collect() };
    let mut galleys = lettering(12.5, 0.9);
    let room = ui.available_width();
    let widest = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let equal = (widest + 2.0 * PAD).max(LEAST);
    let mut widths = vec![equal; galleys.len()];
    if equal * galleys.len() as f32 > room {
        widths = fit(&galleys, PAD, LEAST);
        if widths.iter().sum::<f32>() > room {
            widths = fit(&galleys, TIGHT_PAD, 0.0);
            if widths.iter().sum::<f32>() > room {
                galleys = lettering(11.0, 0.5);
                widths = fit(&galleys, TIGHT_PAD, 0.0);
                if widths.iter().sum::<f32>() > room {
                    return segmented_list(ui, value, options, translate);
                }
            }
        }
    }
    if fill {
        let spare = (room - widths.iter().sum::<f32>()) / widths.len() as f32;
        widths.iter_mut().for_each(|w| *w += spare.max(0.0));
    }
    let (rect, mut response) = ui.allocate_exact_size(vec2(widths.iter().sum(), HEIGHT), Sense::hover());
    let segments: Vec<Rect> = widths
        .iter()
        .scan(rect.min.x, |x, w| {
            let segment = Rect::from_min_size(pos2(*x, rect.min.y), vec2(*w, HEIGHT));
            *x += w;
            Some(segment)
        })
        .collect();
    // Clicks first, so that the chip sets off for a new choice in the frame it is made.
    let mut hits = Vec::with_capacity(options.len());
    for (i, ((option, label), segment)) in options.iter().zip(&segments).enumerate() {
        let seg = ui.interact(*segment, response.id.with(i), Sense::click());
        let selected = *value == *option;
        seg.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), selected, shown(label)));
        reveal_focus(ui, &seg);
        if seg.clicked() && !selected {
            *value = *option;
            response.mark_changed();
            crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle);
        }
        hits.push(seg);
    }
    let chosen = options.iter().position(|(option, _)| *option == *value);
    let painter = ui.painter();
    painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, 0.0, false));
    // The chosen one stands out as a pressed gunmetal chip with a brass bar under its lettering
    // (brass marks what is chosen, as on a list's row), gliding from one choice to the next; the
    // others stay in the slot, lighter under the pointer, a seam between each two.
    for (i, (segment, seg)) in segments.iter().zip(&hits).enumerate() {
        if Some(i) != chosen && seg.hovered() {
            painter.rect_filled(segment.shrink(2.5), CornerRadius::same(1), Color32::from_white_alpha(10));
        }
        if i > 0 && chosen != Some(i) && chosen != Some(i - 1) {
            painter.vline(segment.min.x, (segment.min.y + 7.0)..=(segment.max.y - 7.0), Stroke::new(1.0, Color32::from_black_alpha(160)));
            painter.vline(segment.min.x + 1.0, (segment.min.y + 7.0)..=(segment.max.y - 7.0), Stroke::new(1.0, Color32::from_white_alpha(14)));
        }
    }
    let chip = chosen.map(|i| {
        let target = segments[i].shrink(2.5);
        let glide = crate::motion::Pace::of(ui.ctx()).glide;
        // (Its place along the row glides, as a share of the row's width, not its place on the
        // screen: a row that moves takes its chip along. A panel is laid out elsewhere, and as
        // wide as it likes, for its first frame; the chip came gliding in from there, out of the
        // screen's edge, each time a panel with a row opened: photo mode's, for one.)
        // (Its segment's share, the chip's margin taken off after: equal segments keep their
        // shares at any width.)
        let along = |x: f32| (x - rect.min.x) / rect.width().max(1.0);
        let left = rect.min.x + rect.width() * crate::motion::glide(ui.ctx(), response.id.with("chip_left"), along(segments[i].min.x), glide) + 2.5;
        let right = rect.min.x + rect.width() * crate::motion::glide(ui.ctx(), response.id.with("chip_right"), along(segments[i].max.x), glide) - 2.5;
        Rect::from_x_y_ranges(left..=right.max(left + 1.0), target.y_range())
    });
    if let Some(chip) = chip {
        painter.add(metal::slab(ui.ctx(), chip, Slab::Steel, 0.0, true));
        let bar = Rect::from_min_max(pos2(chip.min.x + 6.0, chip.max.y - 4.0), pos2(chip.max.x - 6.0, chip.max.y - 2.0));
        painter.add(metal::vgrad(bar, AMBER_BRIGHT, AMBER_DIM));
    }
    for (i, ((segment, seg), text)) in segments.iter().zip(&hits).zip(galleys).enumerate() {
        let inner = segment.shrink(2.5);
        // How much of the chip is over this one (all of it once it has arrived): its lettering
        // lights as the chip comes.
        let lit = chip.map_or(0.0, |c| (c.intersect(inner).width().max(0.0) / inner.width().max(1.0)).clamp(0.0, 1.0));
        // (The chosen one's lettering a point up, clear of its bar, with a shadow: small
        // capitals outlined all round, as a button's are, look heavy.)
        let resting = if seg.hovered() { TEXT_BRIGHT } else { STEEL };
        let at = (segment.center() - text.size() / 2.0 - vec2(0.0, lit)).round();
        if lit > 0.0 {
            painter.galley_with_override_text_color(at + vec2(1.0, 1.0), text.clone(), Color32::from_black_alpha((170.0 * lit) as u8));
        }
        painter.galley_with_override_text_color(at, text, resting.lerp_to_gamma(Color32::from_rgb(255, 250, 214), lit));
        focus_ring(ui, seg, inner.shrink(5.0));
        if seg.hovered() && chosen != Some(i) {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }
    response
}

/// The width to give an egui `ComboBox` so it keeps one width whichever of `labels` it
/// shows (its `width` is only a least: a box grows to the text chosen), at least `min`.
pub fn combo_width<S: Into<String>>(ui: &Ui, labels: impl IntoIterator<Item = S>, min: f32) -> f32 {
    let font = TextStyle::Button.resolve(ui.style());
    let s = ui.spacing();
    let widest = labels.into_iter().map(|l| ui.painter().layout_no_wrap(l.into(), font.clone(), TEXT).size().x).fold(0.0, f32::max);
    (widest + s.icon_spacing + s.icon_width + 2.0 * s.button_padding.x).ceil().max(min)
}

/// [`segmented`] as a drop-down list, where its segments would not fit.
fn segmented_list<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], translate: bool) -> Response {
    let shown_label = |label: &str| if translate { t(label).to_owned() } else { label.to_owned() };
    let shown = options.iter().find(|(o, _)| *o == *value).map_or(String::new(), |(_, label)| shown_label(label));
    let id = ui.id().with(("ta_ui::segmented", options.iter().map(|(_, label)| *label).collect::<Vec<_>>()));
    let mut changed = false;
    let mut response = egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(shown).font(strong(12.5)).extra_letter_spacing(crate::i18n::tracking(0.9)))
        .width(ui.available_width().min(240.0))
        .show_ui(ui, |ui| {
            for (option, label) in options {
                if ui.selectable_label(*value == *option, RichText::new(shown_label(label)).font(strong(12.5)).extra_letter_spacing(crate::i18n::tracking(0.9))).clicked() && *value != *option {
                    *value = *option;
                    changed = true;
                }
            }
        })
        .response;
    if changed {
        response.mark_changed();
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle);
    }
    response
}

// ---- Status --------------------------------------------------------------------------------

/// Kind of [`status_mark`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Present / fine (green check).
    Ok,
    /// Optional and absent (dim cross).
    Absent,
    /// Required and missing (red cross).
    Error,
}

/// A small square status indicator.
pub fn status_mark(ui: &mut Ui, mark: Mark) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let r = rect.shrink(1.5);
    let painter = ui.painter();
    match mark {
        Mark::Ok => {
            painter.rect(r, CornerRadius::same(2), GREEN_DEEP, Stroke::new(1.0, GREEN.gamma_multiply(0.8)), StrokeKind::Inside);
            paint_check(painter, r, Stroke::new(2.0, GREEN));
        }
        Mark::Absent | Mark::Error => {
            let (fill, border, color) = if mark == Mark::Error {
                (RED_DEEP, RED.gamma_multiply(0.8), RED)
            } else {
                (INSET, LINE_HI, STEEL_DIM)
            };
            painter.rect(r, CornerRadius::same(2), fill, Stroke::new(1.0, border), StrokeKind::Inside);
            let s = r.shrink(4.2);
            let stroke = Stroke::new(1.6, color);
            painter.line_segment([s.left_top(), s.right_bottom()], stroke);
            painter.line_segment([s.right_top(), s.left_bottom()], stroke);
        }
    }
    response
}

/// A compact status plate with a lit lamp, e.g. "● GAME DATA VERIFIED".
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let text = galley(ui, RichText::new(t(text)).font(strong(11.5)).color(color).extra_letter_spacing(crate::i18n::tracking(1.3)));
    let size = vec2(text.size().x + 36.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, 0.0, false));
    painter.add(metal::lamp(pos2(rect.min.x + 13.0, rect.center().y), 3.5, color, 1.0));
    painter.galley(pos2(rect.min.x + 25.0, rect.center().y - text.size().y / 2.0), text, color);
    response
}

/// A switch plate: a lamp, lit when `on`, and its lettering, brighter then; a click asks for it
/// to turn (the caller turns what it stands for). For a set of options any of which may be on.
pub fn chip(ui: &mut Ui, text: &str, on: bool) -> Response {
    let enabled = ui.is_enabled();
    let label = t(text);
    let color = if !enabled { STEEL_DIM } else if on { TEXT_BRIGHT } else { STEEL };
    let text = galley(ui, RichText::new(label).font(strong(11.0)).color(color).extra_letter_spacing(crate::i18n::tracking(1.0)));
    let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 32.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, label));
    reveal_focus(ui, &response);
    let painter = ui.painter();
    painter.add(metal::slab(ui.ctx(), rect, Slab::Slot, 0.0, false));
    if enabled && response.hovered() {
        painter.rect_filled(rect.shrink(2.5), CornerRadius::same(1), Color32::from_white_alpha(10));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let lit = if !on { 0.0 } else if enabled { 1.0 } else { 0.5 };
    painter.add(metal::lamp(pos2(rect.min.x + 13.0, rect.center().y), 3.5, AMBER_BRIGHT, lit));
    painter.galley(pos2(rect.min.x + 23.0, rect.center().y - text.size().y / 2.0), text, color);
    focus_ring(ui, &response, rect.shrink(2.0));
    if response.clicked() {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Toggle);
    }
    response
}

/// A tiny tag such as "FOLDER" or "v1.2": a small dark slot with coloured lettering.
pub fn tag(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let text = galley(ui, RichText::new(t(text)).font(strong(10.5)).color(color).extra_letter_spacing(crate::i18n::tracking(0.8)));
    let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 12.0, 18.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(2), Color32::from_black_alpha(90), Stroke::new(1.0, color.gamma_multiply(0.4)), StrokeKind::Inside);
    ui.painter().galley(rect.center() - text.size() / 2.0, text, color);
    response
}

// ---- Navigation ----------------------------------------------------------------------------

/// Icons of the navigation rail (the launcher's pages, the settings' categories).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavIcon {
    Play,
    Mods,
    /// A folded map.
    Maps,
    Settings,
    About,
    /// A crosshair.
    Gameplay,
    /// A mouse.
    Controls,
    /// A window with a side panel.
    Interface,
    /// A figure with open arms in a ring (the access symbol).
    Accessibility,
    /// Sun over a mountain.
    Graphics,
    /// A monitor.
    Display,
    /// A loudspeaker.
    Sound,
    /// A dial and its needle (the benchmark).
    Benchmark,
    /// A person from the shoulders up (the profile).
    Profile,
}

/// One entry of the navigation rail: a lamp, lit green on the page shown, an icon and the
/// name. `badge` shows a small count on the right.
pub fn nav_item(ui: &mut Ui, icon: NavIcon, label: &str, badge: Option<(String, Color32)>, selected: bool) -> Response {
    nav_item_sized(ui, icon, label, badge, selected, NAV_ITEM_HEIGHT)
}

/// A [`nav_item`]'s height.
pub const NAV_ITEM_HEIGHT: f32 = 46.0;

/// A [`nav_item`] `height` tall (a rail that has to fit a short window).
pub fn nav_item_sized(ui: &mut Ui, icon: NavIcon, label: &str, badge: Option<(String, Color32)>, selected: bool, height: f32) -> Response {
    nav_item_hinted(ui, icon, label, badge, selected, height, None)
}

/// [`nav_item_sized`] with one hint beside it (`hint`: a shortcut), never under the pointer
/// where it would cover the next item; the label in it too when the label had to be shrunk to
/// fit, and no hint at all for a label shown whole with nothing to add.
pub fn nav_item_hinted(ui: &mut Ui, icon: NavIcon, label: &str, badge: Option<(String, Color32)>, selected: bool, height: f32, hint: Option<&str>) -> Response {
    let label = t(label);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label));
    reveal_focus(ui, &response);
    if response.clicked() && !selected {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Tab);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = response.hovered();
    let painter = ui.painter();
    let on = if crate::motion::reduced(ui.ctx()) { if selected { 1.0 } else { 0.0 } } else { ui.ctx().animate_bool_with_time(response.id.with("on"), selected, 0.15) };
    if selected {
        // A raised plate under the page shown.
        painter.add(metal::vgrad(rect, Color32::from_rgb(50, 48, 42), Color32::from_rgb(34, 33, 28)));
        painter.hline(rect.x_range(), rect.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(22)));
        painter.hline(rect.x_range(), rect.max.y - 0.5, Stroke::new(1.0, Color32::from_black_alpha(150)));
    } else if hovered {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_white_alpha(8));
    }
    let color = if selected {
        CREAM
    } else if hovered {
        TEXT_BRIGHT
    } else {
        STEEL
    };
    painter.add(metal::lamp(pos2(rect.min.x + 14.0, rect.center().y), 3.5, GREEN, on));
    paint_nav_icon(painter, icon, Rect::from_center_size(pos2(rect.min.x + 38.0, rect.center().y), vec2(18.0, 18.0)), color);
    let badge_text = badge.map(|(text, color)| (galley(ui, RichText::new(text).font(strong(11.0)).color(color)), color));
    let room = (rect.width() - 60.0 - badge_text.as_ref().map_or(12.0, |(text, _)| 36.0 + text.size().x)).max(1.0);
    let natural = galley(ui, RichText::new(label).font(strong(15.0)).extra_letter_spacing(crate::i18n::tracking(1.8))).size().x;
    let shrunk = natural > room + 0.5;
    let text = fitted_galley(ui, |size| RichText::new(label).font(strong(size)).color(color).extra_letter_spacing(crate::i18n::tracking(1.8 * size / 15.0)), 15.0, room, 0.8);
    let at = pos2(rect.min.x + 60.0, rect.center().y - text.size().y / 2.0);
    if selected {
        metal::embossed(painter, at, text, color);
    } else {
        painter.galley(at, text, color);
    }
    focus_ring(ui, &response, rect.shrink(7.0));
    if let Some((text, badge_color)) = badge_text {
        let r = Rect::from_center_size(
            pos2(rect.max.x - 22.0 - text.size().x / 2.0, rect.center().y),
            vec2(text.size().x + 12.0, 18.0),
        );
        painter.rect(r, CornerRadius::same(2), badge_color.gamma_multiply(0.12), Stroke::new(1.0, badge_color.gamma_multiply(0.5)), StrokeKind::Inside);
        painter.galley(r.center() - text.size() / 2.0, text, badge_color);
    }
    let tip = match (shrunk, hint) {
        (true, Some(hint)) => Some(format!("{label} · {hint}")),
        (true, None) => Some(label.to_string()),
        (false, Some(hint)) => Some(hint.to_string()),
        (false, None) => None,
    };
    let response = match tip {
        Some(tip) => response.side_tip(tip),
        None => response,
    };
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A [`nav_item`] of a narrow rail: the icon alone (the name and `hint` in its tooltip), the
/// lamp beside it and `badge` over its corner; `size` square.
pub fn nav_icon_item(ui: &mut Ui, icon: NavIcon, label: &str, badge: Option<(String, Color32)>, selected: bool, size: f32, hint: Option<&str>) -> Response {
    let label = t(label);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label));
    reveal_focus(ui, &response);
    if response.clicked() && !selected {
        crate::sound::play(ui.ctx(), crate::sound::UiSound::Tab);
    }
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();
        let on = if crate::motion::reduced(ui.ctx()) { if selected { 1.0 } else { 0.0 } } else { ui.ctx().animate_bool_with_time(response.id.with("on"), selected, 0.15) };
        if selected {
            painter.add(metal::vgrad(rect, Color32::from_rgb(50, 48, 42), Color32::from_rgb(34, 33, 28)));
            painter.hline(rect.x_range(), rect.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(22)));
            painter.hline(rect.x_range(), rect.max.y - 0.5, Stroke::new(1.0, Color32::from_black_alpha(150)));
        } else if hovered {
            painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_white_alpha(8));
        }
        let color = if selected { CREAM } else if hovered { TEXT_BRIGHT } else { STEEL };
        painter.add(metal::lamp(pos2(rect.min.x + 10.0, rect.center().y), 3.0, GREEN, on));
        let icon_at = pos2(rect.center().x + 4.0, rect.center().y);
        paint_nav_icon(painter, icon, Rect::from_center_size(icon_at, vec2(20.0, 20.0)), color);
        focus_ring(ui, &response, rect.shrink(5.0));
        if let Some((text, badge_color)) = badge {
            let text = galley(ui, RichText::new(text).font(strong(10.0)).color(badge_color));
            let r = Rect::from_center_size(icon_at + vec2(12.0, -11.0), vec2((text.size().x + 7.0).max(15.0), 15.0));
            painter.rect(r, CornerRadius::same(2), Color32::from_rgb(24, 23, 20), Stroke::new(1.0, badge_color.gamma_multiply(0.7)), StrokeKind::Inside);
            painter.galley(r.center() - text.size() / 2.0, text, badge_color);
        }
    }
    let tip = match hint {
        Some(hint) => format!("{label} · {hint}"),
        None => label.to_string(),
    };
    response.side_tip(tip).on_hover_cursor(CursorIcon::PointingHand)
}

/// The brass marker beside the [`nav_item`] chosen (`at`, its rect), gliding to the next one
/// chosen; `id` names the rail.
pub fn nav_marker(ui: &Ui, id: &str, at: Rect) {
    let y = if crate::motion::reduced(ui.ctx()) { at.center().y } else { ui.ctx().animate_value_with_time(ui.id().with(("ta_ui::nav_marker", id)), at.center().y, 0.18) };
    let marker = Rect::from_center_size(pos2(at.min.x + 2.0, y), vec2(4.0, at.height() - 12.0));
    ui.painter().rect_filled(marker.expand(1.0), CornerRadius::same(1), Color32::from_black_alpha(160));
    ui.painter().add(metal::vgrad(marker, AMBER_BRIGHT, AMBER_DIM));
}

fn paint_nav_icon(painter: &Painter, icon: NavIcon, r: Rect, color: Color32) {
    let c = r.center();
    let stroke = Stroke::new(1.6, color);
    match icon {
        NavIcon::Play => {
            painter.add(Shape::convex_polygon(
                vec![pos2(c.x - 5.0, c.y - 7.0), pos2(c.x + 7.0, c.y), pos2(c.x - 5.0, c.y + 7.0)],
                color,
                Stroke::NONE,
            ));
        }
        NavIcon::Mods => {
            // Stacked layers: the load order.
            for (k, dy) in [(0, 5.0), (1, 0.0), (2, -5.0)] {
                let y = c.y + dy;
                let diamond = vec![pos2(c.x, y - 4.0), pos2(c.x + 8.0, y), pos2(c.x, y + 4.0), pos2(c.x - 8.0, y)];
                if k == 2 {
                    painter.add(Shape::convex_polygon(diamond, color, Stroke::NONE));
                } else {
                    painter.add(Shape::closed_line(diamond, stroke));
                }
            }
        }
        NavIcon::Maps => {
            painter.add(Shape::closed_line(vec![pos2(c.x - 8.0,c.y - 6.0),pos2(c.x - 2.5,c.y - 8.0),pos2(c.x + 2.5,c.y - 5.0),pos2(c.x + 8.0,c.y - 7.0),pos2(c.x + 8.0,c.y + 6.0),pos2(c.x + 2.5,c.y + 8.0),pos2(c.x - 2.5,c.y + 5.0),pos2(c.x - 8.0,c.y + 7.0)], stroke));
            painter.line_segment([pos2(c.x - 2.5,c.y - 8.0),pos2(c.x - 2.5,c.y + 5.0)], stroke);
            painter.line_segment([pos2(c.x + 2.5,c.y - 5.0),pos2(c.x + 2.5,c.y + 8.0)], stroke);
        }
        NavIcon::Settings => {
            // Three faders.
            for (dy, knob) in [(-5.5, -3.0), (0.0, 4.0), (5.5, -1.0)] {
                painter.hline((c.x - 8.0)..=(c.x + 8.0), c.y + dy, stroke);
                painter.rect_filled(Rect::from_center_size(pos2(c.x + knob, c.y + dy), vec2(4.0, 5.0)), CornerRadius::ZERO, color);
            }
        }
        NavIcon::About => {
            painter.circle_stroke(c, 8.0, stroke);
            painter.circle_filled(pos2(c.x, c.y - 3.8), 1.3, color);
            painter.vline(c.x, (c.y - 1.0)..=(c.y + 4.5), Stroke::new(1.8, color));
        }
        NavIcon::Gameplay => {
            painter.circle_stroke(c, 6.0, stroke);
            painter.circle_filled(c, 1.6, color);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                painter.line_segment([c + vec2(dx, dy) * 4.0, c + vec2(dx, dy) * 9.0], stroke);
            }
        }
        NavIcon::Controls => {
            let body = Rect::from_center_size(c, vec2(11.0, 16.0));
            painter.rect_stroke(body, CornerRadius::same(5), stroke, StrokeKind::Middle);
            painter.vline(c.x, (body.min.y)..=(c.y - 2.0), stroke);
            painter.hline((body.min.x)..=(body.max.x), c.y - 2.0, stroke);
            painter.rect_filled(Rect::from_min_max(pos2(body.min.x + 1.0, body.min.y + 1.0), pos2(c.x, c.y - 2.0)), CornerRadius { nw: 4, ..CornerRadius::ZERO }, color);
        }
        NavIcon::Interface => {
            let frame = Rect::from_center_size(c, vec2(17.0, 13.0));
            painter.rect_stroke(frame, CornerRadius::same(1), stroke, StrokeKind::Middle);
            painter.rect_filled(Rect::from_min_max(frame.min, pos2(frame.min.x + 5.0, frame.max.y)), CornerRadius::ZERO, color);
            painter.hline((frame.min.x + 7.5)..=(frame.max.x - 2.0), frame.min.y + 4.0, stroke);
        }
        NavIcon::Accessibility => {
            painter.circle_stroke(c, 8.5, Stroke::new(1.4, color));
            painter.circle_filled(pos2(c.x, c.y - 4.6), 1.7, color);
            painter.hline((c.x - 5.0)..=(c.x + 5.0), c.y - 1.6, stroke);
            painter.vline(c.x, (c.y - 1.6)..=(c.y + 1.8), Stroke::new(2.0, color));
            painter.line_segment([pos2(c.x, c.y + 1.4), pos2(c.x - 3.0, c.y + 5.6)], stroke);
            painter.line_segment([pos2(c.x, c.y + 1.4), pos2(c.x + 3.0, c.y + 5.6)], stroke);
        }
        NavIcon::Graphics => {
            painter.add(Shape::closed_line(vec![pos2(c.x - 8.5, c.y + 6.0), pos2(c.x - 2.5, c.y - 3.0), pos2(c.x + 1.0, c.y + 1.5), pos2(c.x + 3.5, c.y - 1.0), pos2(c.x + 8.5, c.y + 6.0)], stroke));
            painter.circle_filled(pos2(c.x + 4.5, c.y - 5.5), 2.4, color);
        }
        NavIcon::Display => {
            let screen = Rect::from_min_max(pos2(c.x - 8.5, c.y - 7.0), pos2(c.x + 8.5, c.y + 4.0));
            painter.rect_stroke(screen, CornerRadius::same(1), stroke, StrokeKind::Middle);
            painter.vline(c.x, (screen.max.y)..=(c.y + 7.5), stroke);
            painter.hline((c.x - 4.5)..=(c.x + 4.5), c.y + 7.5, stroke);
        }
        NavIcon::Benchmark => {
            let centre = pos2(c.x, c.y + 4.0);
            let arc: Vec<Pos2> = (0..=12)
                .map(|k| {
                    let a = std::f32::consts::PI * (1.0 + k as f32 / 12.0);
                    pos2(centre.x + 8.5 * a.cos(), centre.y + 8.5 * a.sin())
                })
                .collect();
            painter.add(Shape::line(arc, stroke));
            let a = std::f32::consts::PI * 1.72;
            painter.line_segment([centre, pos2(centre.x + 7.5 * a.cos(), centre.y + 7.5 * a.sin())], Stroke::new(1.8, color));
            painter.circle_filled(centre, 2.0, color);
        }
        NavIcon::Profile => {
            painter.circle_stroke(pos2(c.x, c.y - 3.5), 3.8, stroke);
            let shoulders: Vec<Pos2> = (0..=12).map(|k| {
                let a = std::f32::consts::PI * (1.0 + k as f32 / 12.0);
                pos2(c.x + 7.5 * a.cos(), c.y + 8.5 + 6.5 * a.sin())
            }).collect();
            painter.add(Shape::line(shoulders, stroke));
        }
        NavIcon::Sound => {
            painter.add(Shape::convex_polygon(
                vec![pos2(c.x - 8.0, c.y - 3.0), pos2(c.x - 4.5, c.y - 3.0), pos2(c.x, c.y - 7.0), pos2(c.x, c.y + 7.0), pos2(c.x - 4.5, c.y + 3.0), pos2(c.x - 8.0, c.y + 3.0)],
                color,
                Stroke::NONE,
            ));
            for r in [4.0, 7.5] {
                let arc: Vec<Pos2> = (-3..=3).map(|k| {
                    let a = k as f32 * 0.22;
                    pos2(c.x + 1.0 + r * a.cos(), c.y + r * a.sin())
                }).collect();
                painter.add(Shape::line(arc, stroke));
            }
        }
    }
}

// ---- Emblem --------------------------------------------------------------------------------

/// Paints the emblem into `rect` (square): its picture ([`crate::icon`]) at the size it is drawn
/// on the screen, made once for each size.
pub fn paint_emblem(painter: &Painter, rect: Rect) {
    let px = (rect.width().max(rect.height()) * painter.ctx().pixels_per_point()).round().clamp(16.0, 1024.0) as u32;
    let id = egui::Id::new(("ta_ui::emblem", px));
    let texture = painter.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied([px as usize; 2], &crate::icon::emblem_rgba(px));
        let texture = painter.ctx().load_texture(format!("emblem-{px}"), image, egui::TextureOptions::LINEAR);
        painter.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each fact of a line keeps its words together; the sentence among them wraps as it will;
    /// a line with no facts is left as it is.
    #[test]
    fn a_line_of_facts_keeps_each_fact_whole() {
        let nb = '\u{a0}';
        let line = keep_facts_whole("Your Commander and a first factory. · 7 steps · about 7 min", "Your Commander and a first factory.");
        assert_eq!(line, format!("Your Commander and a first factory. · 7{nb}steps · about{nb}7{nb}min"));
        assert_eq!(keep_facts_whole("one sentence alone", "one sentence alone"), "one sentence alone");
        assert_eq!(keep_facts_whole("a b · c d", ""), format!("a{nb}b · c{nb}d"));
    }

    /// A row of choices that moves on the screen (a panel sliding in, or laid out in the screen's
    /// corner for its first frame) takes its chip along at once: the chip glides along its row
    /// only.
    #[test]
    fn the_chosen_chip_stays_in_a_row_that_moves() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let options = [(0, "ONE"), (1, "TWO"), (2, "THREE")];
        // The chip's left edge and the row's, the row `indent` points in from the left.
        let chip = |time: f64, indent: f32, fill: bool| {
            let mut value = 1;
            let mut row = 0.0;
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 200.0))), ..Default::default() }, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    let r = if fill { segmented_fill(ui, &mut value, &options) } else { segmented(ui, &mut value, &options) };
                    row = r.rect.min.x;
                });
            });
            out.textures_delta.clear();
            let chip = out.shapes.iter().filter_map(|c| match &c.shape { Shape::Vec(v) if !v.is_empty() => Some(c.shape.visual_bounding_rect()), _ => None }).nth(1).map(|r| r.min.x);
            (chip.unwrap(), row)
        };
        for fill in [false, true] {
            let (at, row) = chip(0.0, 0.0, fill);
            let within = at - row;
            assert!(within > 20.0, "the second of three: {within}");
            // The next frame the row stands 900 points to the right (and, filling its room, is
            // narrower): the chip is on its second choice there, not on its way from the left.
            let (at, row) = chip(1.0 / 60.0, 900.0, fill);
            if fill {
                assert!(at > row + 20.0 && at < row + 400.0, "on the second choice of the narrower row: {at} {row}");
            } else {
                assert!((at - row - within).abs() < 0.5, "in its place along the row: {at} {row} {within}");
            }
            let (later, _) = chip(1.0, 900.0, fill);
            assert!((later - at).abs() < 0.5, "and it was not gliding: {at} {later}");
        }
    }

    /// A row of choices' chosen chip glides to a new choice, and is there at once with reduced
    /// motion.
    #[test]
    fn the_chosen_chip_glides_to_a_new_choice() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let options = [(0, "ONE"), (1, "TWO"), (2, "THREE")];
        // The chip's left edge at `time`, the choice being `value`.
        let chip = |time: f64, value: i32| {
            let mut value = value;
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 200.0))), ..Default::default() }, |ui| {
                segmented(ui, &mut value, &options);
            });
            out.textures_delta.clear();
            // (The chip: the second slab, a pressed gunmetal one, after the slot.)
            out.shapes.iter().filter_map(|c| match &c.shape { Shape::Vec(v) if !v.is_empty() => Some(c.shape.visual_bounding_rect()), _ => None }).nth(1).map(|r| r.min.x)
        };
        let start = chip(0.0, 0).unwrap();
        chip(0.05, 0);
        let end = chip(1.0, 2).unwrap();
        assert!((end - start).abs() < 1.0, "the moment it is chosen, still where it was: {start} {end}");
        let mid = chip(1.09, 2).unwrap();
        let arrived = chip(1.5, 2).unwrap();
        assert!(mid > start + 1.0 && mid < arrived - 1.0, "on its way: {start} {mid} {arrived}");
        crate::motion::set_reduced(&ctx, true);
        let back = chip(2.0, 0).unwrap();
        assert!((back - start).abs() < 1.0, "reduced motion: there at once ({back})");
    }

    #[test]
    fn fitted_titles_keep_bundled_font_letter_spacing_inside_the_available_width() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        fonts.families.insert(egui::FontFamily::Name(DISPLAY_FAMILY.into()), vec!["Ubuntu-Light".into()]);
        ctx.set_fonts(fonts);
        for width in [40.0, 120.0, 300.0, 380.0, 420.0] {
            let output = frame(&ctx, input(None), |ui| {
                ui.set_max_width(width);
                let start = ui.cursor().min.x;
                let response = fitted_title(ui, "TOTAL ANNIHILATION REQUIRED", 38.0, TEXT_BRIGHT);
                assert!(response.rect.right() <= start + width, "{width}: {:?}", response.rect);
                assert!(ui.min_rect().right() <= start + width);
            });
            for clipped in output.shapes {
                if let Shape::Text(text) = clipped.shape {
                    let bounds = text.galley.rect.translate(text.pos.to_vec2());
                    assert!(bounds.right().min(clipped.clip_rect.right()) <= width, "{width}: {bounds:?}");
                }
            }
        }
    }

    // These paint-only tests intentionally discard GPU uploads after inspecting the UI.
    fn frame(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&mut Ui)) -> egui::FullOutput {
        let mut output = ctx.run_ui(input, draw);
        output.textures_delta.clear();
        output
    }

    fn input(key: Option<egui::Key>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
            events: key.into_iter().map(|key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn click_modifiers_keep_release_state_when_keys_change_before_redraw() {
        for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
            for (at_click, at_redraw) in [(egui::Modifiers::SHIFT, egui::Modifiers::NONE),
                (egui::Modifiers::NONE, egui::Modifiers::SHIFT)] {
                let ctx = egui::Context::default();
                let mut rect = Rect::NOTHING;
                let _ = frame(&ctx, input(None), |ui| { rect = ui.button("Batch construction").rect; });
                let at = rect.center();
                let mut raw = input(None);
                raw.events = vec![egui::Event::ModifiersChanged(at_click), egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: at_click }];
                let _ = frame(&ctx, raw, |ui| { let _ = ui.button("Batch construction"); });
                let mut raw = input(None);
                raw.events = vec![egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: at_click }, egui::Event::ModifiersChanged(at_redraw)];
                let _ = frame(&ctx, raw, |ui| {
                    let response = ui.button("Batch construction");
                    assert!(response.clicked_by(button));
                    assert_eq!(click_modifiers(&response, button), at_click);
                });
            }
        }
    }

    #[test]
    fn click_modifiers_keep_valid_edge_clicks_outside_the_visual_rectangle() {
        for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
            let ctx = egui::Context::default();
            let mut rect = Rect::NOTHING;
            let _ = frame(&ctx, input(None), |ui| { rect = ui.button("Batch construction").rect; });
            // Egui accepts nearby clicks within its interaction radius, even outside the
            // painted rectangle. Their modifiers must follow the click too.
            let at = rect.right_center() + vec2(2.0, 0.0);
            assert!(!rect.contains(at));
            let mut raw = input(None);
            raw.events = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: egui::Modifiers::SHIFT }];
            let _ = frame(&ctx, raw, |ui| { let _ = ui.button("Batch construction"); });
            let mut raw = input(None);
            raw.events = vec![egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: egui::Modifiers::SHIFT },
                egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
            let _ = frame(&ctx, raw, |ui| {
                let response = ui.button("Batch construction");
                assert!(response.clicked_by(button), "the nearby pointer belongs to this control");
                assert_eq!(click_modifiers(&response, button), egui::Modifiers::SHIFT);
            });
        }
    }

    #[test]
    fn click_modifiers_keep_drag_release_state_outside_the_control() {
        for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
            let ctx = egui::Context::default();
            let mut rect = Rect::NOTHING;
            let _ = frame(&ctx, input(None), |ui| { rect = ui.allocate_response(vec2(100.0, 80.0), Sense::click_and_drag()).rect; });
            let at = rect.center();
            let outside = rect.right_center() + vec2(100.0, 0.0);
            let mut raw = input(None);
            raw.events = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: egui::Modifiers::SHIFT }];
            let _ = frame(&ctx, raw, |ui| { let _ = ui.allocate_response(vec2(100.0, 80.0), Sense::click_and_drag()); });
            let mut raw = input(None);
            raw.events = vec![egui::Event::PointerMoved(outside)];
            let _ = frame(&ctx, raw, |ui| {
                let response = ui.allocate_response(vec2(100.0, 80.0), Sense::click_and_drag());
                assert!(response.dragged_by(button));
            });
            let mut raw = input(None);
            raw.events = vec![egui::Event::PointerButton { pos: outside, button, pressed: false, modifiers: egui::Modifiers::SHIFT },
                egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
            let _ = frame(&ctx, raw, |ui| {
                let response = ui.allocate_response(vec2(100.0, 80.0), Sense::click_and_drag());
                assert!(response.drag_stopped_by(button));
                assert!(!response.rect.contains(response.interact_pointer_pos().unwrap()));
                assert_eq!(click_modifiers(&response, button), egui::Modifiers::SHIFT);
            });
        }
    }

    #[test]
    fn click_modifiers_use_current_state_for_keyboard_activation() {
        let ctx = egui::Context::default();
        let mut id = egui::Id::NULL;
        let mut rect = Rect::NOTHING;
        let _ = frame(&ctx, input(None), |ui| {
            let response = ui.button("Batch construction");
            id = response.id;
            rect = response.rect;
        });
        ctx.memory_mut(|memory| memory.request_focus(id));
        let mut raw = input(Some(egui::Key::Space));
        // A pointer release with no press did not activate this widget, even when it is
        // inside its rectangle. The keyboard activation uses the current modifiers.
        raw.events.insert(0, egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary,
            pressed: false, modifiers: egui::Modifiers::SHIFT });
        raw.events.push(egui::Event::ModifiersChanged(egui::Modifiers::CTRL));
        let _ = frame(&ctx, raw, |ui| {
            let response = ui.button("Batch construction");
            assert!(response.clicked());
            assert!(!response.clicked_by(egui::PointerButton::Primary));
            assert_eq!(click_modifiers(&response, egui::PointerButton::Primary), egui::Modifiers::CTRL);
        });
    }

    #[test]
    fn click_modifiers_ignore_another_controls_release() {
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let _ = frame(&ctx, input(None), |ui| {
            let _ = ui.button("First control");
            rect = ui.button("Second control").rect;
        });
        let at = rect.center();
        let mut raw = input(None);
        raw.events = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::SHIFT }];
        let _ = frame(&ctx, raw, |ui| { let _ = ui.button("First control"); let _ = ui.button("Second control"); });
        let mut raw = input(None);
        raw.events = vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::SHIFT },
            egui::Event::ModifiersChanged(egui::Modifiers::CTRL)];
        let _ = frame(&ctx, raw, |ui| {
            let first = ui.button("First control");
            let second = ui.button("Second control");
            assert!(!first.clicked());
            assert!(second.clicked());
            assert_eq!(click_modifiers(&first, egui::PointerButton::Primary), egui::Modifiers::CTRL);
            assert_eq!(click_modifiers(&second, egui::PointerButton::Primary), egui::Modifiers::SHIFT);
        });
    }

    #[test]
    fn named_controls_activate_from_keyboard_and_respect_disabled_state() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut on = false;
        let mut id = egui::Id::NULL;
        let mut changed = false;
        let _ = frame(&ctx, input(None), |ui| { id = toggle_named(ui, &mut on, "Music").id; });
        ctx.memory_mut(|m| m.request_focus(id));
        let output = frame(&ctx, input(Some(egui::Key::Space)), |ui| { changed = toggle_named(ui, &mut on, "Music").changed(); });
        assert!(on && changed);
        assert!(output.platform_output.events.iter().any(|event| {
            let info = event.widget_info();
            info.typ == WidgetType::Checkbox && info.label.as_deref() == Some("Music") && info.selected == Some(true)
        }));
        let _ = frame(&ctx, input(Some(egui::Key::Enter)), |ui| { changed = toggle_named(ui, &mut on, "Music").changed(); });
        assert!(!on && changed, "Enter activates the focused switch too");
        let _ = frame(&ctx, input(None), |ui| { ui.disable(); id = toggle_named(ui, &mut on, "Music").id; });
        ctx.memory_mut(|m| m.request_focus(id));
        let _ = frame(&ctx, input(Some(egui::Key::Space)), |ui| { ui.disable(); changed = toggle_named(ui, &mut on, "Music").changed(); });
        assert!(!on && !changed, "disabled controls remain inert when keyboard activation is attempted");
    }

    #[test]
    fn keyboard_focus_reveals_a_control_below_the_scroll_view() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut ids = Vec::new();
        let mut offset = 0.0;
        let mut focused_rect = Rect::NOTHING;
        let mut clip = Rect::NOTHING;
        let mut draw = |ui: &mut Ui| {
            ids.clear();
            let out = crate::scroll::show(ui, egui::ScrollArea::vertical().max_height(85.0).auto_shrink([false, false]), crate::scroll::Behind::Plate, |ui| {
                for n in 0..20 {
                    let response = ui.add(ActionButton::secondary("Button"));
                    ids.push(response.id);
                    if n == 12 { focused_rect = response.rect; clip = ui.clip_rect(); }
                }
            });
            offset = out.state.offset.y;
            if let Some(blank) = ctx.read_response(out.id.with(("ta_ui::scroll::blank", 0usize))) {
                assert!(!blank.sense.is_focusable(), "clipped-row blockers must not be invisible Tab stops");
            }
        };
        let _ = frame(&ctx, input(None), &mut draw);
        // Tab through the actual focus order, including controls initially below the fold.
        for _ in 0..13 {
            let _ = frame(&ctx, input(Some(egui::Key::Tab)), &mut draw);
        }
        for _ in 0..2 {
            let _ = frame(&ctx, input(None), &mut draw);
        }
        drop(draw);
        assert_eq!(ctx.memory(|m| m.focused()), Some(ids[12]));
        assert!(offset > 300.0, "focused button was revealed: {offset}");
        assert!(clip.contains_rect(focused_rect), "focused control must be fully visible: {focused_rect:?} in {clip:?}");
    }

    #[test]
    fn icon_buttons_report_a_name_and_parent_disabled_state() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut id = egui::Id::NULL;
        let _ = frame(&ctx, input(None), |ui| { id = icon_button(ui, Glyph::Close, true).id; });
        ctx.memory_mut(|m| m.request_focus(id));
        let output = frame(&ctx, input(Some(egui::Key::Enter)), |ui| { assert!(icon_button(ui, Glyph::Close, true).clicked()); });
        assert!(output.platform_output.events.iter().any(|event| event.widget_info().label.as_deref() == Some("Close")));
        let _ = frame(&ctx, input(None), |ui| {
            ui.disable();
            let response = icon_button(ui, Glyph::Close, true);
            assert!(!response.enabled());
            assert!(!response.sense.is_focusable());
        });
    }
}
