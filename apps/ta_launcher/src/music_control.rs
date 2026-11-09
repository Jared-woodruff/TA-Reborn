//! The header's music control, a recessed readout: the note and its level meter turn the
//! launcher's music on and off at a click (lit, the meter's bars dancing while a track plays;
//! dim, struck through and flat when off), and the arrow past its seam opens a small panel: the
//! track playing, the music's volume, the next track.

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::metal::{self, Slab};
use ta_ui::widgets::{self, ActionButton};

use crate::theme::*;

/// What the control was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicAction {
    Toggle,
    Skip,
}

/// The control's panel, open or not.
#[derive(Default)]
pub struct MusicControl {
    open: bool,
}

/// The arrow's width beside the note.
const ARROW: f32 = 18.0;

impl MusicControl {
    /// Opens its panel (captures).
    pub fn open(&mut self) {
        self.open = true;
    }

    /// The control at `rect` (the note, and the arrow at its right): `on` the music's setting,
    /// `playing` the track playing (a VFS path or `file:` and a path), `volume` its share of
    /// the music volume, `available` whether the install has music to play.
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, on: bool, playing: Option<&str>, volume: &mut f32, available: bool) -> Option<MusicAction> {
        let mut action = None;
        // One recessed readout, as an instrument's: the note and its level meter (a click turns
        // the music on or off), and past a seam the arrow (the panel).
        let note = Rect::from_min_max(rect.min, pos2(rect.max.x - ARROW, rect.max.y));
        let arrow = Rect::from_min_max(pos2(note.max.x, rect.min.y), rect.max);
        let id = ui.id().with("launcher_music");
        let response = ui.interact(note, id.with("note"), if available { Sense::click() } else { Sense::hover() });
        let opener = ui.interact(arrow, id.with("arrow"), Sense::click());
        let ctx = ui.ctx().clone();
        let hot = ctx.animate_bool_with_time(id.with("hot"), (available && response.hovered()) || opener.hovered() || self.open, 0.12);
        let down = (available && response.is_pointer_button_down_on()) || opener.is_pointer_button_down_on();
        let painter = ui.painter();
        painter.add(metal::slab(&ctx, rect, Slab::Slot, hot, down));
        let seam = rect.y_range().shrink(rect.height() * 0.22);
        painter.vline(arrow.min.x, seam, Stroke::new(1.0, Color32::from_black_alpha(160)));
        painter.vline(arrow.min.x + 1.0, seam, Stroke::new(1.0, Color32::from_white_alpha(14)));
        let lit = on && available;
        let colour = if lit { AMBER_BRIGHT } else { STEEL_DIM };
        let size = (note.height() * 0.44).clamp(12.0, 18.0);
        let glyph = Rect::from_center_size(pos2(note.min.x + 7.0 + size / 2.0, note.center().y + if down { 1.0 } else { 0.0 }), vec2(size, size));
        paint_note(painter, glyph, colour);
        if !lit {
            painter.line_segment([glyph.left_bottom() + vec2(-1.0, 1.0), glyph.right_top() + vec2(1.0, -1.0)], Stroke::new(2.0, STEEL_DIM));
        }
        // The level meter: four bars beside the note, dancing while a track plays (held at a
        // level with reduced motion, or with the window in the background), flat when it is off.
        let sounding = lit && playing.is_some();
        let time = if sounding && !ta_ui::motion::still(&ctx) { ui.input(|i| i.time) } else { 0.0 };
        let (left, right) = (glyph.max.x + 6.0, note.max.x - 7.0);
        let (gap, bars) = (2.0, 4);
        let bar = ((right - left - gap * (bars - 1) as f32) / bars as f32).clamp(2.0, 4.0);
        let foot = glyph.max.y;
        for k in 0..bars {
            let level = if !sounding {
                0.12
            } else if time > 0.0 {
                0.25 + 0.75 * (0.5 + 0.5 * (time * (4.3 + k as f64 * 1.9) + k as f64 * 1.7).sin() as f32)
            } else {
                [0.55, 0.85, 0.45, 0.7][k]
            };
            let x = left + k as f32 * (bar + gap);
            let top = foot - (size * level).max(2.0);
            painter.rect_filled(Rect::from_min_max(pos2(x, top), pos2(x + bar, foot)), CornerRadius::same(1), if sounding { AMBER } else { STEEL_DIM.gamma_multiply(0.6) });
        }
        if time > 0.0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(66));
        }
        let tip = if !available {
            t("No music in your install").to_string()
        } else if on {
            match playing {
                Some(track) => tf("Music: {track} · click to turn it off", &[("track", &track_name(track))]),
                None => t("The music is on · click to turn it off").to_string(),
            }
        } else {
            t("The music is off · click to turn it on").to_string()
        };
        let response = response.on_hover_text(tip);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, available, on, t("LAUNCHER MUSIC")));
        if response.clicked() {
            ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
            action = Some(MusicAction::Toggle);
        }
        // The arrow: the panel.
        let painter = ui.painter();
        let c = arrow.center() + vec2(0.0, if down && opener.is_pointer_button_down_on() { 2.0 } else { 1.0 });
        let points = if self.open { vec![c + vec2(-4.0, 2.0), c + vec2(4.0, 2.0), c + vec2(0.0, -3.0)] } else { vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)] };
        painter.add(Shape::convex_polygon(points, if self.open || opener.hovered() { AMBER_BRIGHT } else { STEEL }, Stroke::NONE));
        let opener = opener.on_hover_text(t("The music: what plays, its volume, the next track"));
        if opener.clicked() {
            self.open = !self.open;
            ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
        }
        if self.open {
            action = self.panel(ui, rect, opener.rect, on, playing, volume, available).or(action);
        }
        action
    }

    /// The panel under the control, its right edge with the control's.
    #[allow(clippy::too_many_arguments)]
    fn panel(&mut self, ui: &mut Ui, control: Rect, arrow: Rect, on: bool, playing: Option<&str>, volume: &mut f32, available: bool) -> Option<MusicAction> {
        let mut action = None;
        let width = 280.0;
        let at = Pos2::new(control.max.x - width, control.max.y + 8.0);
        let area = egui::Area::new(Id::new("launcher_music_panel")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
            let frame = egui::Frame::new().inner_margin(egui::Margin::same(14));
            let background = ui.painter().add(Shape::Noop);
            let inner = frame.show(ui, |ui| {
                ui.set_width(width - 28.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.label(widgets::caption(n("LAUNCHER MUSIC")).color(AMBER));
                let what = if !available {
                    t("No music in your install").to_string()
                } else if !on {
                    t("Off").to_string()
                } else {
                    playing.map_or_else(|| t("Starting…").to_string(), track_name)
                };
                ui.label(egui::RichText::new(what).font(strong(14.0)).color(if on && available { TEXT_BRIGHT } else { STEEL }));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t("Volume")).font(body(13.0)).color(TEXT));
                    let mut percent = (*volume * 100.0).round();
                    let slider = egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%").fixed_decimals(0);
                    if widgets::value_slider(ui, slider, 48.0).changed() {
                        *volume = percent / 100.0;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(available && on && playing.is_some(), ActionButton::secondary(n("NEXT TRACK")).height(28.0)).clicked() {
                        action = Some(MusicAction::Skip);
                    }
                    let label = if on { n("TURN OFF") } else { n("TURN ON") };
                    if ui.add_enabled(available, ActionButton::primary(label).height(28.0)).clicked() {
                        action = Some(MusicAction::Toggle);
                    }
                });
            });
            let rect = inner.response.rect;
            ui.painter().set(
                background,
                Shape::Vec(vec![
                    Shape::Rect(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(rect, CornerRadius::same(3))),
                    metal::riveted_plate(ui.ctx(), rect, PANEL_HI, 0.0),
                    Shape::rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, LINE), StrokeKind::Inside),
                ]),
            );
        });
        // A click elsewhere, or Esc, closes it.
        let panel = area.response.rect;
        if ui.input(|i| i.pointer.any_pressed()) && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !panel.contains(p) && !arrow.contains(p)) {
            self.open = false;
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.open = false;
        }
        action
    }
}

/// "TA's soundtrack · track 14", or a file of the player's own music by its name.
pub fn track_name(path: &str) -> String {
    let file = path.strip_prefix("file:").unwrap_or(path);
    // (Either separator: a path saved on Windows names its track the same everywhere.)
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    };
    match stem.parse::<u32>() {
        Ok(n) if !path.starts_with("file:") => tf("TA's soundtrack · track {n}", &[("n", &n)]),
        _ => stem,
    }
}

/// Two quavers beamed together, in `colour`, inside `rect`.
fn paint_note(painter: &egui::Painter, rect: Rect, colour: Color32) {
    let (w, h) = (rect.width(), rect.height());
    let head = |x: f32, y: f32| {
        let centre = pos2(rect.min.x + x * w, rect.min.y + y * h);
        let points: Vec<Pos2> = (0..12)
            .map(|k| {
                let a = k as f32 / 12.0 * std::f32::consts::TAU;
                // (A head leaning, as engraved.)
                let (s, c) = (a.sin(), a.cos());
                let (rx, ry) = (0.17 * w, 0.12 * h);
                centre + vec2(c * rx * 0.94 - s * ry * 0.34, c * rx * 0.34 + s * ry * 0.94)
            })
            .collect();
        painter.add(Shape::convex_polygon(points, colour, Stroke::NONE));
    };
    head(0.24, 0.82);
    head(0.76, 0.70);
    let stem = Stroke::new((w * 0.09).max(1.5), colour);
    painter.line_segment([pos2(rect.min.x + 0.38 * w, rect.min.y + 0.80 * h), pos2(rect.min.x + 0.38 * w, rect.min.y + 0.16 * h)], stem);
    painter.line_segment([pos2(rect.min.x + 0.90 * w, rect.min.y + 0.68 * h), pos2(rect.min.x + 0.90 * w, rect.min.y + 0.06 * h)], stem);
    let beam = vec![
        pos2(rect.min.x + 0.38 * w - 1.0, rect.min.y + 0.16 * h),
        pos2(rect.min.x + 0.90 * w + 1.0, rect.min.y + 0.04 * h),
        pos2(rect.min.x + 0.90 * w + 1.0, rect.min.y + 0.20 * h),
        pos2(rect.min.x + 0.38 * w - 1.0, rect.min.y + 0.32 * h),
    ];
    painter.add(Shape::convex_polygon(beam, colour, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_are_named_for_the_player() {
        assert_eq!(track_name("music/14.mp3"), "TA's soundtrack · track 14");
        assert_eq!(track_name("file:C:\\Music\\Hadal Static.mp3"), "Hadal Static");
        assert_eq!(track_name("file:/home/me/13.ogg"), "13");
    }
}
