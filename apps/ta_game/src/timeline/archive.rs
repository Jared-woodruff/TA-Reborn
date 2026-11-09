//! The archive: the war's documents (`lore::DOCUMENTS`), listed beside the one open, which is
//! shown as it was filed: its side's seal and colour, its file mark and date, its heading (its
//! title, from whom, to whom), its stamp across the corner, its text told a line at a time
//! with the blacked-out words barred; CORE's intercepted signals breaking up now and then.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, Key, Layout, Painter, Pos2, Rect, Sense, Shape, Stroke, UiBuilder, WidgetInfo, WidgetType, pos2, vec2};
use ta_config::profiles::Allegiance;
use ta_ui::i18n::{n, t, tracking};
use ta_ui::panel;
use ta_ui::widgets as w;

use super::galaxy::{ARM_BLUE, CORE_RED};
use crate::lore::{DOCUMENTS, Document, Side};
use crate::theme::*;

/// The paper the documents are on.
const PAPER: Color32 = Color32::from_rgb(27, 26, 22);
/// The stamp's tilt (radians, anticlockwise up).
const STAMP_TILT: f32 = -0.2;
/// The room kept at the heading's right for the stamp.
const STAMP_ROOM: f32 = 290.0;

/// A side's colour in the archive.
pub(super) fn colour(side: Side) -> Color32 {
    match side {
        Side::Arm => ARM_BLUE,
        Side::Core => CORE_RED,
        Side::Archive => AMBER,
    }
}

/// Whether a document is a signal caught on the way (it breaks up now and then).
fn signal(d: &Document) -> bool {
    d.side == Side::Core && (d.code.contains("/INT-") || d.code.contains("/MEM-"))
}

/// The archive, `height` tall: the list of documents and the one `open`, its text told since
/// `telling`. Up and down open the one before or after.
pub(super) fn draw(ui: &mut egui::Ui, open: &mut usize, telling: &mut Option<f64>, now: f64, height: f32) {
    let step = ui.input(|i| i.key_pressed(Key::ArrowDown) as isize - i.key_pressed(Key::ArrowUp) as isize);
    if step != 0 {
        let to = (*open as isize + step).clamp(0, DOCUMENTS.len() as isize - 1) as usize;
        if to != *open {
            *open = to;
            *telling = Some(now);
        }
    }
    *open = (*open).min(DOCUMENTS.len() - 1);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        ui.vertical(|ui| {
            ui.set_width(340.0);
            panel::well(ui, |ui| {
                let area = egui::ScrollArea::vertical().id_salt("archive_list").max_height(height - 4.0).auto_shrink([false, false]);
                ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Well, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, d) in DOCUMENTS.iter().enumerate() {
                        let response = row(ui, d, i == *open);
                        if i == *open && step != 0 {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() && i != *open {
                            *open = i;
                            *telling = Some(now);
                        }
                    }
                });
            });
        });
        sheet(ui, &DOCUMENTS[*open], telling, now, height);
    });
}

/// A side's seal: a disc in its colour with its mark (the archive's, a stack of records).
fn seal(painter: &Painter, c: Pos2, r: f32, side: Side) {
    let color = colour(side);
    painter.circle_filled(c, r, color.gamma_multiply(0.3));
    painter.circle_stroke(c, r, Stroke::new(1.5, color));
    painter.circle_stroke(c, r - 3.0, Stroke::new(0.8, color.gamma_multiply(0.6)));
    match side {
        Side::Arm | Side::Core => {
            let allegiance = if side == Side::Arm { Allegiance::Arm } else { Allegiance::Core };
            ta_ui::faction::paint(painter, Rect::from_center_size(c, vec2(r * 1.25, r * 1.25)), allegiance);
        }
        Side::Archive => {
            for k in 0..3 {
                let y = c.y + (k as f32 - 1.0) * r * 0.32;
                let half = r * (0.45 - 0.06 * k as f32);
                painter.line_segment([pos2(c.x - half, y), pos2(c.x + half, y)], Stroke::new(r * 0.16, AMBER_BRIGHT));
            }
        }
    }
}

/// A side's mark filling `rect` (the archive's, its stack of records).
fn watermark(painter: &Painter, rect: Rect, side: Side) {
    match side {
        Side::Arm => ta_ui::faction::paint(painter, rect, Allegiance::Arm),
        Side::Core => ta_ui::faction::paint(painter, rect, Allegiance::Core),
        Side::Archive => {
            let c = rect.center();
            let r = rect.width() * 0.5;
            painter.circle_stroke(c, r * 0.95, Stroke::new(r * 0.06, Color32::WHITE));
            for k in 0..3 {
                let y = c.y + (k as f32 - 1.0) * r * 0.32;
                let half = r * (0.5 - 0.07 * k as f32);
                painter.line_segment([pos2(c.x - half, y), pos2(c.x + half, y)], Stroke::new(r * 0.14, Color32::WHITE));
            }
        }
    }
}

/// A document's row in the list: its seal, its mark and date in its side's colour, its title.
fn row(ui: &mut egui::Ui, d: &Document, on: bool) -> egui::Response {
    let width = ui.available_width();
    let title = ui.painter().layout(t(d.title).to_string(), strong(13.0), TEXT_BRIGHT, width - 66.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, (title.size().y + 34.0).max(56.0)), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, on, t(d.title)));
    let painter = ui.painter();
    let color = colour(d.side);
    if on {
        painter.rect_filled(rect, 0.0, AMBER_DEEP.gamma_multiply(0.8));
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, AMBER);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, PANEL_HI.gamma_multiply(0.5));
    }
    seal(painter, pos2(rect.min.x + 28.0, rect.min.y + 28.0), 15.0, d.side);
    let head = format!("{}   {}", d.code, t(d.dateline));
    let head = painter.layout_job(LayoutJob::single_section(head, TextFormat { font_id: strong(10.5), color, extra_letter_spacing: tracking(1.0), ..Default::default() }));
    painter.galley(pos2(rect.min.x + 54.0, rect.min.y + 9.0), head, color);
    painter.galley(pos2(rect.min.x + 54.0, rect.min.y + 26.0), title, if on { AMBER_BRIGHT } else { TEXT_BRIGHT });
    painter.line_segment([pos2(rect.min.x + 8.0, rect.max.y - 0.5), pos2(rect.max.x - 8.0, rect.max.y - 0.5)], Stroke::new(1.0, LINE));
    w::focus_ring(ui, &response, rect);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The document open: its sheet, `height` tall, in the room left.
fn sheet(ui: &mut egui::Ui, d: &Document, telling: &mut Option<f64>, now: f64, height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let color = colour(d.side);
    {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, PAPER);
        // Old paper: lighter in the middle, the edges worn dark.
        w::soft_glow(&painter, rect.center(), 0.0, rect.width() * 0.6, Color32::from_rgba_unmultiplied(255, 240, 200, 10));
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 4.0)), 0.0, color);
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        // Its side's mark under the text, as a watermark.
        let mut mark = painter.clone();
        mark.set_opacity(0.05);
        let size = (rect.height() * 0.5).min(rect.width() * 0.4);
        watermark(&mark, Rect::from_center_size(rect.right_bottom() - vec2(size * 0.62, size * 0.62), vec2(size, size)), d.side);
    }
    let inner = rect.shrink2(vec2(26.0, 20.0)).with_min_y(rect.min.y + 22.0);
    let mut sheet = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    // The heading: its seal; its mark and date, its title; from whom, to whom.
    sheet.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let (mark, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
        seal(ui.painter(), mark.center(), 25.0, d.side);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - STAMP_ROOM).max(200.0));
            let head = format!("{}   ·   {}", d.code, t(d.dateline));
            ui.label(egui::RichText::new(head).font(strong(12.0)).color(color).extra_letter_spacing(tracking(1.6)));
            ui.add_space(2.0);
            let title = ui.painter().layout(t(d.title).to_string(), display(26.0), Color32::WHITE, ui.available_width());
            let (r, _) = ui.allocate_exact_size(title.size(), Sense::hover());
            ta_ui::metal::chrome(ui.painter(), r.min, title);
        });
    });
    sheet.add_space(10.0);
    // (A document's own words: "FROM" alone is where a map editor's copy comes from.)
    for (field, value) in [(n("FROM:"), d.from), (n("TO:"), d.to)] {
        sheet.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (label, _) = ui.allocate_exact_size(vec2(68.0, 18.0), Sense::hover());
            ui.painter().text(label.left_center(), egui::Align2::LEFT_CENTER, t(field), strong(11.0), STEEL);
            ui.label(egui::RichText::new(t(value)).font(strong(13.0)).color(TEXT).extra_letter_spacing(tracking(0.8)));
        });
    }
    sheet.add_space(10.0);
    {
        let y = sheet.cursor().min.y;
        let line = [pos2(inner.min.x, y), pos2(inner.max.x, y)];
        sheet.painter().extend(Shape::dashed_line(&line, Stroke::new(1.0, color.gamma_multiply(0.6)), 6.0, 4.0));
    }
    sheet.add_space(12.0);
    let left = (inner.max.y - sheet.cursor().min.y).max(60.0);
    let area = egui::ScrollArea::vertical().id_salt(("archive_text", d.code)).max_height(left).auto_shrink([false, false]);
    ta_ui::scroll::show(&mut sheet, area, ta_ui::scroll::Behind::Fill(PAPER), |ui| {
        ui.set_width(ui.available_width() - 12.0);
        let color = if d.side == Side::Core { Color32::from_rgb(232, 224, 214) } else { TEXT_BRIGHT };
        super::told_text(ui, t(d.text), telling, now, body(17.5), color, 27.0);
    });
    stamp(ui.painter(), pos2(rect.max.x - STAMP_ROOM * 0.5 - 18.0, rect.min.y + 58.0), t(d.stamp), color);
    if signal(d) {
        interference(&ui.painter_at(rect), rect.shrink(2.0), now);
    }
}

/// A stamp at `centre`, tilted: `text` in a double border, in `color`, inked unevenly.
fn stamp(painter: &Painter, centre: Pos2, text: &str, color: Color32) {
    let ink = color.gamma_multiply(0.85);
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: display(19.0), color: ink, extra_letter_spacing: tracking(3.0), ..Default::default() });
    job.wrap.max_width = STAMP_ROOM - 50.0;
    job.halign = Align::Center;
    let galley = painter.layout_job(job);
    let size = galley.size() + vec2(30.0, 16.0);
    let (sin, cos) = STAMP_TILT.sin_cos();
    let turn = |v: egui::Vec2| centre + vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
    for (inset, width) in [(0.0, 2.4), (4.5, 1.0)] {
        let half = size / 2.0 - vec2(inset, inset);
        let corners = vec![turn(vec2(-half.x, -half.y)), turn(vec2(half.x, -half.y)), turn(vec2(half.x, half.y)), turn(vec2(-half.x, half.y))];
        painter.add(Shape::closed_line(corners, Stroke::new(width, ink)));
    }
    // (Turned about its pivot, its top left: that put where its middle lands on `centre`.)
    let middle = galley.rect.center().to_vec2();
    let origin = centre - (turn(middle) - centre);
    painter.add(egui::epaint::TextShape { pos: origin, galley, underline: Stroke::NONE, fallback_color: ink, override_text_color: None, opacity_factor: 0.9, angle: STAMP_TILT });
}

/// A signal breaking up: now and then, for a moment, a band of noise across it.
fn interference(painter: &Painter, rect: Rect, now: f64) {
    const EVERY: f64 = 3.4;
    const LASTS: f64 = 0.22;
    let cycle = (now / EVERY).floor();
    let into = now - cycle * EVERY;
    painter.ctx().request_repaint();
    if into > LASTS {
        return;
    }
    let seed = (cycle as i64).unsigned_abs() as u32;
    let h = |k: u32| super::galaxy::hash(seed.wrapping_mul(7919).wrapping_add(k));
    let y = rect.min.y + rect.height() * (0.25 + 0.65 * h(1));
    let band = Rect::from_min_max(pos2(rect.min.x, y), pos2(rect.max.x, y + 6.0 + 16.0 * h(2)));
    painter.rect_filled(band, 0.0, Color32::from_black_alpha(120));
    for k in 0..60 {
        let x = band.min.x + band.width() * h(10 + k);
        let yy = band.min.y + band.height() * h(100 + k);
        let length = 4.0 + 40.0 * h(200 + k).powi(2);
        let tone = if h(300 + k) < 0.3 { CORE_RED } else { Color32::from_rgb(220, 220, 214) };
        painter.line_segment([pos2(x, yy), pos2((x + length).min(band.max.x), yy)], Stroke::new(1.0, tone.gamma_multiply(0.7)));
    }
}
