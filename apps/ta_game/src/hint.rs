//! The plate that a tip at the start of a battle and the coach's advice stand on: a lamp and
//! a caption, the words, a LOOK chip when the words are about a place or a unit (a click goes
//! there), a cross that puts them away and a line under them that stops hints of that kind for
//! good. It rises as it fades in, and goes pale when the pointer comes near it (not on it), so
//! that it never hides what the player is working at.

use egui::{Align2, Color32, Context, Id, Margin, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2, vec2};
use ta_ui::i18n::t;

use crate::theme::*;

/// A plate to show.
pub struct Plate<'a> {
    /// Its area's name.
    pub id: &'static str,
    /// Its lamp's colour, the colour of the line along its foot, and its caption (`n(..)`).
    pub lamp: Color32,
    pub foot: Color32,
    pub caption: &'static str,
    pub words: &'a str,
    /// Whether the words point somewhere (the LOOK chip, and a click on them goes there).
    pub linked: bool,
    /// What the cross says on hover, and the line that stops this kind of hint (translated).
    pub away: &'a str,
    pub never: &'a str,
    /// How opaque it is (fading in and out), how far it still has to rise (points), and what is
    /// left of its time (0 to 1: the line along its foot).
    pub opacity: f32,
    pub rise: f32,
    pub left: f32,
}

/// What the player did to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressed {
    /// Went to the place or the unit it is about.
    Look,
    /// Put it away (this once).
    Away,
    /// Asked never to see hints of this kind again.
    Never,
}

/// A plate as it was drawn (what the tests look at is not read otherwise).
#[cfg_attr(not(test), allow(dead_code))]
pub struct Shown {
    pub pressed: Option<Pressed>,
    /// The pointer is on it.
    pub on: bool,
    pub rect: Rect,
    /// How clear it was drawn for the pointer's nearness (1: not near).
    pub clear: f32,
    /// Where the LOOK chip stood.
    pub chip: Option<Rect>,
}

/// How near the pointer must come (points) for a plate to go pale, and how pale it goes.
const NEAR: f32 = 80.0;
const PALE: f32 = 0.3;

/// The LOOK chip's size (its word, translated, with room round it).
pub fn chip_size(painter: &Painter) -> Vec2 {
    let word = painter.layout_no_wrap(t("LOOK").to_string(), strong(10.5), Color32::WHITE);
    vec2((word.size().x + 16.0).max(40.0), 18.0)
}

/// The chip that says a notice or a hint goes somewhere: a small amber plate with LOOK on it.
pub fn paint_chip(painter: &Painter, rect: Rect, hot: bool, fade: f32) {
    let fill = if hot { Color32::from_rgb(70, 52, 30) } else { Color32::from_rgb(48, 38, 24) };
    painter.rect_filled(rect, 2.0, fill.gamma_multiply(fade));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, AMBER.gamma_multiply(if hot { 0.95 } else { 0.6 } * fade)), StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, t("LOOK"), strong(10.5), AMBER_BRIGHT.gamma_multiply(fade));
}

/// Shows `plate` low in the middle of `view`.
pub fn show(ctx: &Context, view: Rect, plate: &Plate) -> Shown {
    let id = Id::new(plate.id);
    let reduced = ta_ui::motion::reduced(ctx);
    // (Where it stood last frame: the pointer near it but not on it, and it goes pale.)
    let last = ctx.memory(|m| m.area_rect(id));
    let near = match (last, ctx.pointer_hover_pos()) {
        (Some(r), Some(at)) => (0.01..NEAR).contains(&r.distance_to_pos(at)),
        _ => false,
    };
    let clear = ctx.animate_value_with_time(id.with("clear"), if near { PALE } else { 1.0 }, if reduced { 0.0 } else { 0.18 });
    let content = ctx.content_rect();
    let offset = vec2(view.center().x - content.center().x, -(content.bottom() - view.bottom()) - 96.0 + plate.rise);
    let room = (view.width() - 48.0).clamp(240.0, 680.0);
    let mut pressed = None;
    let mut chip = None;
    let area = egui::Area::new(id).anchor(Align2::CENTER_BOTTOM, offset).show(ctx, |ui| {
        ui.set_opacity((plate.opacity * clear).max(0.05));
        let background = ui.painter().add(Shape::Noop);
        let shown = egui::Frame::new().inner_margin(Margin { left: 14, right: 10, top: 9, bottom: 11 }).show(ui, |ui| {
            ui.set_max_width(room);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (lamp, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                ui.painter().add(ta_ui::metal::lamp(lamp.center(), 3.5, plate.lamp, 1.0));
                ui.label(ta_ui::widgets::caption(plate.caption).color(plate.lamp));
                ui.vertical(|ui| {
                    ui.set_max_width((room - 170.0).max(120.0));
                    ui.spacing_mut().item_spacing.y = 5.0;
                    let words = RichText::new(plate.words).font(body(14.5)).color(TEXT_BRIGHT);
                    let said = ui.add(egui::Label::new(words).wrap().sense(if plate.linked { Sense::click() } else { Sense::hover() }));
                    if plate.linked && said.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        pressed = Some(Pressed::Look);
                    }
                    let never = ui.add(egui::Label::new(RichText::new(plate.never).font(body(11.5)).color(STEEL)).sense(Sense::click()));
                    if never.hovered() {
                        ui.painter().hline(never.rect.x_range(), never.rect.bottom() - 1.0, Stroke::new(1.0, STEEL));
                    }
                    if never.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        pressed = Some(Pressed::Never);
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if plate.linked {
                        let (rect, response) = ui.allocate_exact_size(chip_size(ui.painter()), Sense::click());
                        paint_chip(ui.painter(), rect, response.hovered(), 1.0);
                        chip = Some(rect);
                        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            pressed = Some(Pressed::Look);
                        }
                    }
                    if ta_ui::widgets::icon_button(ui, ta_ui::widgets::Glyph::Close, true).on_hover_text(plate.away).clicked() {
                        pressed = Some(Pressed::Away);
                    }
                });
            });
        });
        let r = shown.response.rect;
        let foot = Rect::from_min_max(Pos2::new(r.left() + 1.0, r.bottom() - 3.0), Pos2::new(r.right() - 1.0, r.bottom() - 1.0));
        ui.painter().set(
            background,
            Shape::Vec(vec![
                egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(140) }.as_shape(r, 3.0).into(),
                Shape::rect_filled(r.expand(1.0), 3.0, ta_ui::metal::STEEL_EDGE),
                ta_ui::metal::plate(ctx, r, PANEL),
                Shape::hline(r.x_range().shrink(1.0), r.top() + 1.0, Stroke::new(1.0, Color32::from_white_alpha(20))),
                Shape::rect_filled(foot, 0.0, Color32::from_black_alpha(120)),
                Shape::rect_filled(Rect::from_min_size(foot.min, vec2(foot.width() * plate.left.clamp(0.0, 1.0), foot.height())), 0.0, plate.foot),
            ]),
        );
    });
    let rect = area.response.rect;
    let on = ctx.pointer_hover_pos().is_some_and(|at| rect.contains(at));
    Shown { pressed, on, rect, clear, chip }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate(linked: bool) -> Plate<'static> {
        Plate { id: "test_plate", lamp: GREEN, foot: GREEN, caption: "COACH", words: "A factory has nothing to build.", linked, away: "Put it away", never: "Don't show this advice again", opacity: 1.0, rise: 0.0, left: 1.0 }
    }

    /// Runs frames at `time` seconds with the pointer where `pointer` says and `buttons` pressed.
    fn frame(ctx: &Context, time: f64, events: Vec<egui::Event>, linked: bool) -> Shown {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
        let input = egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
        let mut shown = None;
        let mut output = ctx.run_ui(input, |ui| shown = Some(show(ui.ctx(), screen, &plate(linked))));
        output.textures_delta.clear();
        output.drop_without_applying_deltas();
        shown.expect("drawn")
    }

    fn pointer(at: Pos2) -> Vec<egui::Event> {
        vec![egui::Event::PointerMoved(at)]
    }

    fn button(at: Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]
    }

    /// The plate stands low in the middle of the view, inside it, and goes pale when the pointer
    /// is near it and not on it.
    #[test]
    fn a_plate_goes_pale_as_the_pointer_comes_near() {
        let ctx = Context::default();
        crate::theme::install_compact(&ctx);
        let first = frame(&ctx, 0.0, Vec::new(), true);
        frame(&ctx, 0.1, Vec::new(), true);
        let still = frame(&ctx, 0.2, Vec::new(), true);
        assert!(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0)).contains_rect(still.rect), "{:?}", still.rect);
        assert!(still.rect.center().x > 600.0 && still.rect.center().x < 680.0 && still.rect.bottom() > 540.0, "low in the middle: {:?}", first.rect);
        assert!(still.clear > 0.99 && !still.on);
        // Near it, above (a clear 40 points over its top edge).
        let above = pointer(Pos2::new(still.rect.center().x, still.rect.top() - 40.0));
        let mut near = still;
        for i in 0..12 {
            near = frame(&ctx, 0.3 + i as f64 * 0.05, above.clone(), true);
        }
        assert!(near.clear < 0.35 && !near.on, "pale when the pointer is near: {}", near.clear);
        // On it: as clear as it can be (so that its buttons read).
        let on = pointer(near.rect.center());
        let mut over = near;
        for i in 0..12 {
            over = frame(&ctx, 1.0 + i as f64 * 0.05, on.clone(), true);
        }
        assert!(over.clear > 0.99 && over.on, "clear under the pointer: {}", over.clear);
        // Far away: clear.
        let far = pointer(Pos2::new(40.0, 40.0));
        for i in 0..12 {
            over = frame(&ctx, 2.0 + i as f64 * 0.05, far.clone(), true);
        }
        assert!(over.clear > 0.99 && !over.on);
    }

    /// Presses at `at` and lets go (the pointer comes first), as three frames; what the plate says
    /// was done.
    fn click(ctx: &Context, time: &mut f64, at: Pos2, linked: bool) -> Option<Pressed> {
        let mut result = None;
        for events in [pointer(at), button(at, true), button(at, false)] {
            *time += 0.05;
            result = result.or(frame(ctx, *time, events, linked).pressed);
        }
        result
    }

    /// The chip goes to the place, the cross puts the plate away, the line under the words
    /// stops its kind; without a link there is no chip.
    #[test]
    fn the_chip_the_cross_and_the_line_each_do_their_own() {
        let ctx = Context::default();
        crate::theme::install_compact(&ctx);
        let mut time = 0.0;
        let (mut rect, mut chip) = (Rect::NOTHING, None);
        for _ in 0..3 {
            time += 0.05;
            let shown = frame(&ctx, time, Vec::new(), true);
            (rect, chip) = (shown.rect, shown.chip);
        }
        let chip = chip.expect("a plate with a link has its chip");
        assert_eq!(click(&ctx, &mut time, chip.center(), true), Some(Pressed::Look));
        // The words are about the place too.
        assert_eq!(click(&ctx, &mut time, Pos2::new(rect.left() + 100.0, rect.top() + 18.0), true), Some(Pressed::Look));
        // The cross: the right end of the plate, level with the chip.
        assert_eq!(click(&ctx, &mut time, Pos2::new(rect.right() - 24.0, chip.center().y), true), Some(Pressed::Away));
        // The line under the words, at the plate's foot.
        assert_eq!(click(&ctx, &mut time, Pos2::new(rect.left() + 100.0, rect.bottom() - 14.0), true), Some(Pressed::Never));
        // Not about a place: no chip, and the words do nothing.
        time += 0.1;
        let mut plain = frame(&ctx, time, Vec::new(), false);
        for _ in 0..3 {
            time += 0.05;
            plain = frame(&ctx, time, Vec::new(), false);
        }
        assert!(plain.chip.is_none());
        assert_eq!(click(&ctx, &mut time, Pos2::new(plain.rect.left() + 100.0, plain.rect.top() + 18.0), false), None);
    }
}
