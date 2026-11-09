//! The shared look lives in `ta_ui`; this module keeps `crate::theme::*` working.

pub use ta_ui::theme::*;

/// Launcher labels describe controls; only actual text fields accept text selection.
pub fn install(ctx: &egui::Context) -> bool {
    let system_font = ta_ui::theme::install(ctx);
    ctx.all_styles_mut(|style| {
        style.interaction.selectable_labels = false;
        style.interaction.multi_widget_text_select = false;
    });
    system_font
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels_are_passive_but_text_fields_remain_editable() {
        let ctx = egui::Context::default();
        super::install(&ctx);
        let mut input = String::from("Commander");
        let mut label = egui::Rect::NOTHING;
        let mut field = egui::Rect::NOTHING;
        let draw = |ui: &mut egui::Ui, input: &mut String, label: &mut egui::Rect, field: &mut egui::Rect| {
            *label = ui.label("Settings save automatically").rect;
            *field = ui.text_edit_singleline(input).rect;
        };
        ctx.run_ui(Default::default(), |ui| draw(ui, &mut input, &mut label, &mut field)).drop_without_applying_deltas();
        for (at, expected) in [(label.center(), egui::CursorIcon::Default), (field.center(), egui::CursorIcon::Text)] {
            let raw = egui::RawInput { events: vec![egui::Event::PointerMoved(at)], ..Default::default() };
            let mut output = ctx.run_ui(raw, |ui| draw(ui, &mut input, &mut label, &mut field));
            output.textures_delta.clear();
            assert_eq!(output.platform_output.cursor_icon, expected);
        }
        for pressed in [true, false] {
            let raw = egui::RawInput { events: vec![egui::Event::PointerMoved(field.center()), egui::Event::PointerButton { pos: field.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }], ..Default::default() };
            ctx.run_ui(raw, |ui| draw(ui, &mut input, &mut label, &mut field)).drop_without_applying_deltas();
        }
        let raw = egui::RawInput { events: vec![egui::Event::Text("X".into())], ..Default::default() };
        ctx.run_ui(raw, |ui| draw(ui, &mut input, &mut label, &mut field)).drop_without_applying_deltas();
        assert!(input.contains('X'));
    }
}
