//! The launcher's pages. Each is an `impl LauncherApp` block that draws into the central
//! column and requests work through [`crate::app::Action`]s.

mod about;
mod about_art;
mod benchmark;
mod deck;
mod getmods;
mod mods;
mod mod_order;
mod mod_report;
mod maps;
mod play;
mod settings;
mod skins;
mod skin_editor;

/// A built-in skin's name in the interface's language.
pub(crate) fn builtin_skin_name(id: &str) -> Option<String> { skins::builtin_name(id) }
mod update;

use egui::{Align, Color32, Layout, Response, RichText, Ui, vec2};

use crate::theme::*;
use ta_ui::i18n::t;
use crate::widgets::{self, caption};

/// Width of the caption column in [`field`] rows.
const FIELD_LABEL_WIDTH: f32 = 112.0;

/// Large page title, lettered as TA's are, with a one-line description underneath.
fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    ta_ui::metal::embossed_label(ui, RichText::new(t(title)).font(display(32.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.8)), TEXT_BRIGHT);
    ui.label(RichText::new(t(subtitle)).color(STEEL));
    ui.add_space(space::L);
}

/// A library page's heading: `title` (and `subtitle` where the page is tall enough, `short`
/// otherwise) with its buttons at its right, or under it where they do not fit. `labels` are the
/// buttons' words, to measure them; `buttons` adds them, as tall as it is told.
pub(crate) fn library_header(ui: &mut Ui, title: &str, subtitle: &str, short: bool, labels: &[&str], buttons: impl FnOnce(&mut Ui, f32)) {
    let height = if short { 28.0 } else { 32.0 };
    let width = ui.available_width();
    let room: f32 = labels.iter().map(|label| widgets::galley(ui, RichText::new(t(label)).font(strong(13.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 30.0).sum::<f32>()
        + labels.len().saturating_sub(1) as f32 * ui.spacing().item_spacing.x;
    if short {
        let heading = RichText::new(t(title)).font(display(22.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.4));
        let heading_width = widgets::galley(ui, heading.clone()).size().x;
        if width >= room + heading_width + space::M + 2.0 * ui.spacing().item_spacing.x {
            ui.horizontal(|ui| {
                ta_ui::metal::embossed_label(ui, heading, TEXT_BRIGHT);
                ui.add_space(space::M);
                buttons(ui, height);
            });
        } else {
            // Keep every action's label and click target inside a narrow page.
            ta_ui::metal::embossed_label(ui, heading, TEXT_BRIGHT);
            ui.horizontal_wrapped(|ui| buttons(ui, height));
        }
        ui.add_space(space::XS);
    } else if width >= room + 360.0 {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(width - room - space::L, 0.0), Layout::top_down(Align::Min), |ui| page_header(ui, title, subtitle));
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.horizontal(|ui| buttons(ui, height));
            });
        });
    } else {
        page_header(ui, title, subtitle);
        ui.horizontal_wrapped(|ui| buttons(ui, height));
        ui.add_space(space::S);
    }
}

/// Wrapped paragraph in body text.
fn paragraph(ui: &mut Ui, text: &str) {
    wrapped(ui, t(text), TEXT);
}

/// Body text that wraps at the available width.
fn wrapped(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.add(egui::Label::new(RichText::new(text).color(color)).wrap())
}

/// A path in monospace that wraps and can be selected and copied.
fn path_label(ui: &mut Ui, path: &std::path::Path, color: Color32) -> Response {
    ui.add(egui::Label::new(RichText::new(path.display().to_string()).font(mono(12.5)).color(color)).wrap())
}

/// A caption/value row: a fixed-width caption column and a value that wraps in the
/// remaining width (unlike a `Grid`, whose cells grow instead of wrapping).
fn field(ui: &mut Ui, label: &str, add_value: impl FnOnce(&mut Ui)) {
    field_with(ui, label, FIELD_LABEL_WIDTH, add_value);
}

/// [`field`] with a caption column this wide (rows alike: see [`caption_width`]).
fn field_with(ui: &mut Ui, label: &str, label_width: f32, add_value: impl FnOnce(&mut Ui)) {
    if ui.available_width() < 420.0 {
        ui.add(egui::Label::new(caption(label)).wrap());
        add_value(ui);
        ui.add_space(space::S);
        return;
    }
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(label_width, 18.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(label_width);
            ui.add_space(2.0);
            ui.label(caption(label));
        });
        ui.vertical(|ui| add_value(ui));
    });
    ui.add_space(space::XS);
}

/// A caption column wide enough for each of `labels` as translated (a long one widens the
/// column rather than breaking mid-word), at least [`FIELD_LABEL_WIDTH`] and at most 40% of
/// the room.
fn caption_width(ui: &Ui, labels: &[&str]) -> f32 {
    let widest = labels.iter().map(|l| widgets::galley(ui, caption(l)).size().x + 14.0).fold(FIELD_LABEL_WIDTH, f32::max);
    widest.min((ui.available_width() * 0.4).max(FIELD_LABEL_WIDTH))
}

/// Upper-cases the first letter of an error message from a library.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
