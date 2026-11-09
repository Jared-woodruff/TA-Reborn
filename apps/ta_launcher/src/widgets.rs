//! The shared widgets live in `ta_ui`; this module keeps `crate::widgets::*` working.

pub use ta_ui::widgets::*;

/// A status lamp is geometry, so font fallback cannot move it off the text's centre.
pub fn status_label(ui: &mut egui::Ui, text: &str, color: egui::Color32, mark: char) -> egui::Response {
    use egui::{RichText, Sense, Stroke, vec2};
    let galley = wrapped_galley(ui, RichText::new(text).font(crate::theme::strong(10.0)).color(color), (ui.available_width() - 17.0).max(1.0));
    let height = galley.size().y.max(20.0);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 17.0, height), Sense::hover());
    let origin = rect.min + vec2(17.0, (height - galley.size().y) * 0.5);
    let first_line = galley.rows.first().map_or(galley.size().y * 0.5, |row| row.rect().center().y);
    let center = egui::pos2(rect.left() + 5.0, origin.y + first_line);
    let painter = ui.painter();
    match mark {
        '✓' => { painter.line_segment([center + vec2(-4., 0.), center + vec2(-1., 3.)], Stroke::new(1.5, color)); painter.line_segment([center + vec2(-1., 3.), center + vec2(5., -3.)], Stroke::new(1.5, color)); }
        '×' => { for direction in [-1.0, 1.0] { painter.line_segment([center + vec2(-3., -3. * direction), center + vec2(3., 3. * direction)], Stroke::new(1.5, color)); } }
        '!' => { painter.line_segment([center + vec2(0., -4.), center], Stroke::new(1.5, color)); painter.circle_filled(center + vec2(0., 3.), 1., color); }
        _ => { painter.circle_filled(center, 2.5, color); }
    }
    painter.galley(origin, galley, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    response
}

/// Work under way for no time anyone knows (a scan, an import): a construction rail at work
/// beside what is going on (`text`, translated).
pub fn busy_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (rail, _) = ui.allocate_exact_size(egui::vec2(84.0, 8.0), egui::Sense::hover());
        ta_ui::progress::paint(ui, rail, None, true);
        ui.add(egui::Label::new(egui::RichText::new(text).color(crate::theme::STEEL)).wrap());
    });
}

/// A quiet link in brass capitals (a folder to open, a search to clear), as the Play page's
/// folder links are; `label` is translated.
pub fn text_link(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.link(egui::RichText::new(ta_ui::i18n::t(label)).font(crate::theme::strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0)))
}
