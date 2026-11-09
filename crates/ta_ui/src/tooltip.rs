//! Hints on a plate of the launcher's metal (a brass edge along the top), anchored at the pointer
//! with room for the themed cursor and viewport-edge fallback; and side hints for a rail's items,
//! beside the item so that they cover neither it nor its neighbours.
use egui::{Context, Frame, Id, Margin, RectAlign, Response, Shape, Stroke, Tooltip, Ui, WidgetText};

use crate::{
    metal,
    theme::{AMBER_DIM, PANEL_HI, TEXT},
};

pub fn set_pointer_clearance(ctx: &Context, points: f32) {
    let points = if points.is_finite() { points.clamp(12.0, 160.0) } else { 24.0 };
    ctx.data_mut(|data| data.insert_temp(Id::new("ta_ui::tooltip_clearance"), points));
}

fn width(ctx: &Context) -> f32 {
    ctx.global_style().spacing.tooltip_width.min((ctx.content_rect().width() - 24.0).max(1.0))
}

fn tooltip(response: &Response, disabled: bool) -> Tooltip<'_> {
    let ctx = &response.ctx;
    let gap = ctx.data(|data| data.get_temp::<f32>(Id::new("ta_ui::tooltip_clearance"))).unwrap_or(24.0);
    let tip = if disabled { Tooltip::for_disabled(response) } else { Tooltip::for_enabled(response) }.width(width(ctx));
    themed(if ctx.pointer_hover_pos().is_some() { tip.at_pointer().gap(gap) } else { tip })
}

/// The popup without egui's own frame: [`plate`] paints the hint's.
fn themed(mut tip: Tooltip<'_>) -> Tooltip<'_> {
    tip.popup = tip.popup.frame(Frame::NONE);
    tip
}

/// A hint's plate round `content`: gunmetal, grimed as the cards are, a brass edge along its
/// top and a steel rim, over a soft shadow.
fn plate<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(Shape::Noop);
    let inner = Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 7, bottom: 7 }).show(ui, |ui| {
        ui.visuals_mut().override_text_color = Some(TEXT);
        content(ui)
    });
    let rect = inner.response.rect;
    let ctx = ui.ctx().clone();
    ui.painter().set(
        background,
        Shape::Vec(vec![
            egui::Shadow { offset: [0, 5], blur: 14, spread: 0, color: egui::Color32::from_black_alpha(170) }.as_shape(rect, 2.0).into(),
            metal::plate(&ctx, rect, PANEL_HI),
            Shape::rect_stroke(rect, 2.0, Stroke::new(1.0, metal::STEEL_EDGE), egui::StrokeKind::Inside),
            Shape::hline(rect.x_range().shrink(1.0), rect.top() + 1.25, Stroke::new(1.5, AMBER_DIM)),
        ]),
    );
    inner.inner
}

pub trait TooltipExt: Sized {
    fn hover_tip(self, text: impl Into<WidgetText>) -> Self;
    fn disabled_tip(self, text: impl Into<WidgetText>) -> Self;
    fn hover_tip_ui(self, content: impl FnOnce(&mut Ui)) -> Self;
    /// A hint beside the widget (to its right, else its left), its top level with the widget's:
    /// for a rail's or a list's items, where a hint under the pointer would cover the next item.
    fn side_tip(self, text: impl Into<WidgetText>) -> Self;
}

impl TooltipExt for Response {
    fn hover_tip(self, text: impl Into<WidgetText>) -> Self {
        self.hover_tip_ui(|ui| {
            ui.add(egui::Label::new(text).wrap());
        })
    }

    fn disabled_tip(self, text: impl Into<WidgetText>) -> Self {
        tooltip(&self, true).show(|ui| {
            plate(ui, |ui| {
                ui.add(egui::Label::new(text).wrap());
            })
        });
        self
    }

    fn hover_tip_ui(self, content: impl FnOnce(&mut Ui)) -> Self {
        tooltip(&self, false).show(|ui| plate(ui, content));
        self
    }

    fn side_tip(self, text: impl Into<WidgetText>) -> Self {
        side_hint(&self, text);
        self
    }
}

/// [`TooltipExt::side_tip`]'s hint, and where it stands when shown.
fn side_hint(response: &Response, text: impl Into<WidgetText>) -> Option<egui::Rect> {
    let mut tip = Tooltip::for_enabled(response).width(width(&response.ctx));
    tip.popup = tip.popup.align(RectAlign::RIGHT_START).align_alternatives(&[RectAlign::LEFT_START, RectAlign::BOTTOM_START]).gap(8.0);
    themed(tip)
        .show(|ui| {
            plate(ui, |ui| {
                ui.add(egui::Label::new(text).wrap());
            })
        })
        .map(|shown| shown.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, vec2};

    #[test]
    fn hints_follow_the_pointer_and_stay_in_the_viewport() {
        for (at, disabled) in [(egui::pos2(80.0, 60.0), false), (egui::pos2(780.0, 565.0), false), (egui::pos2(80.0, 60.0), true)] {
            let ctx = Context::default();
            ctx.all_styles_mut(|s| {
                s.interaction.tooltip_delay = 0.0;
                s.interaction.show_tooltips_only_when_still = false;
            });
            set_pointer_clearance(&ctx, 42.0);
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let mut hint = None;
            for frame in 0..6 {
                let input = egui::RawInput { screen_rect: Some(screen), time: Some(frame as f64), events: vec![egui::Event::PointerMoved(at)], ..Default::default() };
                ctx.run_ui(input, |ui| {
                    let response = ui.add_enabled(!disabled, egui::Button::new("A wide launcher control").min_size(vec2(780.0, 570.0)));
                    if let Some(shown) = tooltip(&response, disabled).show(|ui| plate(ui, |ui| ui.label("Pointer hint"))) {
                        hint = Some(shown.response.rect);
                    }
                })
                .drop_without_applying_deltas();
            }
            let hint = hint.expect("enabled and disabled controls retain their hints");
            assert!(screen.contains_rect(hint), "{hint:?}");
            assert!(hint.distance_to_pos(at) < 100.0, "hint must anchor to the pointer, not the far side of a wide widget: {hint:?} {at:?}");
            assert!(!hint.contains(at), "hint must not cover its pointer hotspot");
        }
    }

    /// A rail item's hint stands beside the item, level with it: clear of the items above and
    /// below it, and of the item itself.
    #[test]
    fn side_hints_stand_beside_their_item() {
        let ctx = Context::default();
        ctx.all_styles_mut(|s| {
            s.interaction.tooltip_delay = 0.0;
            s.interaction.show_tooltips_only_when_still = false;
        });
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let at = egui::pos2(60.0, 115.0);
        let (mut item, mut hint) = (Rect::NOTHING, None);
        for frame in 0..6 {
            let input = egui::RawInput { screen_rect: Some(screen), time: Some(frame as f64), events: vec![egui::Event::PointerMoved(at)], ..Default::default() };
            ctx.run_ui(input, |ui| {
                ui.allocate_space(vec2(200.0, 100.0));
                let response = ui.add_sized(vec2(200.0, 36.0), egui::Button::new("MODS"));
                item = response.rect;
                hint = side_hint(&response, "Ctrl+2").or(hint);
            })
            .drop_without_applying_deltas();
        }
        let hint = hint.expect("the side hint shows");
        assert!(hint.left() >= item.right(), "beside the item, not over it: {hint:?} {item:?}");
        assert!(hint.top() >= item.top() - 1.0 && hint.top() <= item.top() + 1.0, "level with the item: {hint:?} {item:?}");
    }
}
