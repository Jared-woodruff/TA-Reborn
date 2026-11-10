//! In-game UI theme: the shared look of `ta_ui`, the same as the launcher's (TA's metal,
//! pipes and gunmetal buttons, brass accents, Bahnschrift from the Windows font folder when
//! available).

pub use ta_ui::theme::*;

/// [`ta_ui::theme::install_compact`], with labels that only describe what they are beside, as
/// in the launcher: only actual text fields take a selection (a label showed the text cursor
/// under the pointer, the title's commander card among them).
pub fn install_compact(ctx: &egui::Context) -> bool {
    let system_font = ta_ui::theme::install_compact(ctx);
    ctx.all_styles_mut(|style| {
        style.interaction.selectable_labels = false;
        style.interaction.multi_widget_text_select = false;
    });
    system_font
}

/// Sets a context's zoom. `set_zoom_factor` waits for the next pass and then lays that pass out
/// on the size of the pass before, scaled, whatever size the pass is given: right for a context
/// that has drawn (a zoom changed in a window), wrong for one that has not, whose "size before"
/// is egui's placeholder of 10,000 points square (its first pass was laid out for that: the
/// classic interface made its art and letters nineteen times too large for it). A context that
/// has run no pass yet is given its zoom at once.
pub fn set_zoom(ctx: &egui::Context, zoom: f32) {
    if ctx.cumulative_pass_nr() == 0 {
        ctx.options_mut(|options| options.zoom_factor = zoom);
    } else if (ctx.zoom_factor() - zoom).abs() > 0.001 {
        ctx.set_zoom_factor(zoom);
    }
}

#[cfg(test)]
mod tests {
    /// The size a pass was laid out for.
    fn laid_out(ctx: &egui::Context, size: egui::Vec2) -> egui::Vec2 {
        let mut seen = egui::Vec2::ZERO;
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        ctx.run_ui(raw, |ui| seen = ui.ctx().content_rect().size()).drop_without_applying_deltas();
        seen
    }

    #[test]
    fn a_new_context_is_laid_out_for_its_window_at_any_zoom() {
        let size = egui::vec2(640.0, 360.0);
        // (egui's own way, for the record: a zoom waiting on a context that has not drawn lays
        // its first pass out for the placeholder, scaled.)
        let theirs = egui::Context::default();
        theirs.set_zoom_factor(1.25);
        assert!(laid_out(&theirs, size).x > 5000.0, "egui no longer does this: `set_zoom` may be simpler now");
        for zoom in [0.6, 1.0, 1.25, 2.5] {
            let ctx = egui::Context::default();
            super::set_zoom(&ctx, zoom);
            assert_eq!(laid_out(&ctx, size), size, "the first pass at zoom {zoom}");
            assert_eq!(ctx.zoom_factor(), zoom);
            // A zoom changed later keeps the window's size in pixels: its points scale with it.
            super::set_zoom(&ctx, zoom * 2.0);
            assert_eq!(laid_out(&ctx, size), size / 2.0, "the pass after a change from zoom {zoom}");
            assert_eq!(laid_out(&ctx, size), size, "and the next takes the size it is given");
        }
    }

    #[test]
    fn labels_are_passive_but_text_fields_remain_editable() {
        let ctx = egui::Context::default();
        super::install_compact(&ctx);
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
