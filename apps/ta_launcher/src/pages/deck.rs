//! The Play page's command deck: the banner's map behind the words (graded down, drifting
//! slowly, swept by a radar's scan that lights the contacts it passes), and the ways into the
//! game as tiles, each with its own glyph and a line on what it is.
//!
//! Everything here moves on the launcher's ambient clock ([`crate::app::LauncherApp::ambient`]),
//! which stands still (and so draws nothing more) while the window is in the background, under
//! reduced motion, or after a minute with no one at it: the banner then holds where it was, with
//! no jump back to a resting pose.

use std::f64::consts::TAU;

use egui::text::LayoutJob;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shadow, Shape, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2};
use ta_ui::i18n::t;
use ta_ui::metal::{self, Slab};

use crate::theme::*;
use crate::widgets;

/// The radar's phosphor at `alpha`.
fn phosphor(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(120, 214, 92, alpha)
}

// ---- The banner's map ------------------------------------------------------------------------

/// The part of a picture `size` texels across that fills `rect` (texture coordinates): as much
/// as fits at `zoom` 1, less below it, its centre moved by `pan` (each axis -1 to 1 of the room
/// there is to move in).
pub(super) fn window(size: Vec2, rect: Rect, zoom: f32, pan: Vec2) -> Rect {
    let aspect = rect.width() / rect.height().max(1.0);
    let (w, h) = (size.x.max(1.0), size.y.max(1.0));
    // The largest crop of the picture's shape the banner's: its width and height in texels.
    let (cw, ch) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    let zoom = zoom.clamp(0.05, 1.0);
    let uv = vec2(cw * zoom / w, ch * zoom / h);
    let room = (vec2(1.0, 1.0) - uv) / 2.0;
    let centre = pos2(0.5 + room.x * pan.x.clamp(-1.0, 1.0), 0.5 + room.y * pan.y.clamp(-1.0, 1.0));
    Rect::from_center_size(centre, uv)
}

/// The map behind the banner, `time` seconds into its drift (the ambient clock): drawn darker
/// and warmer than painted (`level` of its brightness: the words over it read), creeping across
/// and in and out again so slowly it is only seen to have moved.
pub(super) fn paint_map(painter: &Painter, rect: Rect, texture: &egui::TextureHandle, level: f32, time: f64) {
    let zoom = 0.84 - 0.06 * (0.5 - 0.5 * (time * TAU / 53.0).cos()) as f32;
    let pan = vec2(0.45 * (time * TAU / 131.0).sin() as f32, 0.6 * (time * TAU / 89.0).sin() as f32);
    let uv = window(texture.size_vec2(), rect, zoom, pan);
    let tint = |c: f32| (c * level.clamp(0.0, 1.0)) as u8;
    painter.image(texture.id(), rect, uv, Color32::from_rgb(tint(176.0), tint(168.0), tint(150.0)));
}

/// The radar's period: the scan crosses in `SWEEP` seconds, then rests at the far side.
const SCAN: f64 = 9.0;
const SWEEP: f64 = 6.0;
/// The scan's contacts, as shares of the banner's width and height (on its right, where the
/// map is in the clear).
const CONTACTS: [(f32, f32); 6] = [(0.58, 0.32), (0.66, 0.71), (0.74, 0.22), (0.81, 0.58), (0.9, 0.36), (0.95, 0.77)];

/// The scan over the banner's map: a fine grid, the scan line with its afterglow, and contacts
/// that flare as it passes and fade after. Drawn under the words' shading (so it is faint
/// behind them). `moving`: the scan crosses (the ambient clock runs); else only the grid and the
/// contacts, dim. Returns whether it moved (so asks for more frames).
pub(super) fn paint_scan(painter: &Painter, rect: Rect, time: f64, moving: bool) -> bool {
    let p = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    // The grid: lines every 32 points, every fourth brighter.
    let step = 32.0;
    let (minor, major) = (Stroke::new(1.0, phosphor(9)), Stroke::new(1.0, phosphor(18)));
    let mut k = 1;
    let mut x = rect.min.x + step;
    while x < rect.max.x {
        p.vline(x.round() + 0.5, rect.y_range(), if k % 4 == 0 { major } else { minor });
        x += step;
        k += 1;
    }
    let (mut k, mut y) = (1, rect.min.y + step);
    while y < rect.max.y {
        p.hline(rect.x_range(), y.round() + 0.5, if k % 4 == 0 { major } else { minor });
        y += step;
        k += 1;
    }
    let cycle = time.rem_euclid(SCAN);
    let across = (cycle / SWEEP) as f32;
    if moving && across < 1.0 {
        let x = rect.min.x + rect.width() * across;
        let trail = Rect::from_min_max(pos2(x - 150.0, rect.min.y), pos2(x, rect.max.y));
        p.add(metal::hgrad(trail, phosphor(0), phosphor(24)));
        p.vline(x, rect.y_range(), Stroke::new(1.5, phosphor(95)));
    }
    for &(u, v) in &CONTACTS {
        let at = pos2(rect.min.x + rect.width() * u, rect.min.y + rect.height() * v);
        // (Seconds since the scan last passed it.)
        let passed = (cycle - SWEEP * u as f64).rem_euclid(SCAN) as f32;
        let glow = if moving { (1.0 - passed / 3.2).clamp(0.0, 1.0) } else { 0.0 };
        p.circle_filled(at, 2.0, phosphor((46.0 + 190.0 * glow) as u8));
        if glow > 0.0 {
            p.circle_stroke(at, 3.0 + 9.0 * (1.0 - glow), Stroke::new(1.0, phosphor((150.0 * glow) as u8)));
        }
    }
    moving
}

/// The banner's words read over any map: shaded behind them on the left, and along the top and
/// the foot.
pub(super) fn paint_shade(painter: &Painter, rect: Rect) {
    let words = Rect::from_min_max(rect.min, pos2(rect.min.x + rect.width() * 0.72, rect.max.y));
    widgets::gradient_h(painter, words, Color32::from_black_alpha(236), Color32::from_black_alpha(20));
    widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - rect.height() * 0.4), rect.max), Color32::TRANSPARENT, Color32::from_black_alpha(130));
    widgets::gradient_v(painter, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 28.0)), Color32::from_black_alpha(110), Color32::TRANSPARENT);
}

// ---- The ways into the game -------------------------------------------------------------------

/// A way into the game, as a tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Tutorial,
    Skirmish,
    Campaign,
    Scenarios,
    Conquest,
    Multiplayer,
}

impl Mode {
    /// TA's own sound for it, as its menus had them.
    fn sound(self) -> ta_ui::sound::UiSound {
        use ta_ui::sound::UiSound;
        match self {
            Mode::Skirmish => UiSound::Skirmish,
            Mode::Multiplayer => UiSound::Multiplayer,
            _ => UiSound::BigButton,
        }
    }
}

/// A way into the game as a tile `size` across: its glyph in a round well, its name, and (when
/// `hint` is given) a line or two on what it is, cut short where it runs on. Brass when it is the
/// way to go (`primary`). `label` and `hint` are translated here. It rises a little and lights
/// under the pointer, a gleam crossing it and a brass rule opening along its foot. Honours
/// `ui.add_enabled_ui(false, …)`.
pub(super) fn mode_tile(ui: &mut Ui, mode: Mode, label: &str, hint: Option<&str>, size: Vec2, primary: bool) -> Response {
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let name = t(label);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
    widgets::reveal_focus(ui, &response);
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), mode.sound());
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let ctx = ui.ctx().clone();
    let hot = ctx.animate_bool_with_time(response.id.with("hot"), enabled && (response.hovered() || response.has_focus()), 0.14);
    let down = enabled && response.is_pointer_button_down_on();
    let lift = if ta_ui::motion::reduced(&ctx) { 0.0 } else { 2.0 * hot } - if down { 1.0 } else { 0.0 };
    let r = rect.translate(vec2(0.0, -lift));
    // (Unavailable, egui fades it with the rest of what is disabled.)
    let painter = ui.painter().clone();
    painter.add(Shadow { offset: [0, (3.0 + 4.0 * hot) as i8], blur: (8.0 + 10.0 * hot) as u8, spread: 0, color: Color32::from_black_alpha((105.0 + 50.0 * hot) as u8) }.as_shape(r, CornerRadius::same(3)));
    let (ink, edge) = if primary {
        painter.add(metal::slab(&ctx, r, Slab::Brass, hot, down));
        (ON_AMBER, Color32::from_white_alpha(70))
    } else {
        painter.add(metal::slab(&ctx, r, Slab::Slot, hot, down));
        painter.add(metal::grime_over(&ctx, r.shrink(1.0), 0.6));
        // (The rim warms to brass under the pointer.)
        painter.rect_stroke(r.shrink(0.5), CornerRadius::same(2), Stroke::new(1.0, LINE.lerp_to_gamma(AMBER_DIM, hot)), egui::StrokeKind::Inside);
        (CREAM.lerp_to_gamma(TEXT_BRIGHT, hot), Color32::from_black_alpha(190))
    };
    // A gleam crosses it as the pointer comes, as it does the libraries' cards.
    if enabled && !ta_ui::motion::reduced(&ctx) && !ta_ui::motion::still(&ctx) {
        crate::library::gleam(ui, response.id, r.shrink(2.0), response.hovered() || response.has_focus());
    }
    // The brass rule along its foot, opening from the middle.
    if hot > 0.01 && !primary {
        let half = (r.width() - 16.0) * 0.5 * hot;
        let y = r.max.y - 2.5;
        painter.line_segment([pos2(r.center().x - half, y), pos2(r.center().x + half, y)], Stroke::new(2.0, AMBER.gamma_multiply(0.9)));
    }
    // The glyph in its well.
    let well_r = (r.height() * 0.27).clamp(13.0, 21.0);
    let well = pos2(r.min.x + 14.0 + well_r, r.center().y);
    if primary {
        painter.circle_filled(well, well_r, Color32::from_rgb(118, 88, 32));
        painter.circle_stroke(well, well_r, Stroke::new(1.0, Color32::from_rgb(78, 56, 18)));
        painter.circle_stroke(well, well_r + 1.0, Stroke::new(1.0, Color32::from_white_alpha(60)));
    } else {
        painter.circle_filled(well, well_r, INSET);
        painter.circle_stroke(well, well_r, Stroke::new(1.0, metal::STEEL_EDGE));
        painter.circle_stroke(well, well_r + 1.0, Stroke::new(1.0, Color32::from_white_alpha((14.0 + 22.0 * hot) as u8)));
        if hot > 0.01 {
            widgets::soft_glow(&painter, well, 0.0, well_r * 0.95, AMBER.gamma_multiply(0.22 * hot));
        }
    }
    let glyph_colour = if primary { Color32::from_rgb(250, 226, 160) } else { STEEL.lerp_to_gamma(AMBER_BRIGHT, hot) };
    let down_by = if down { vec2(0.0, 1.0) } else { Vec2::ZERO };
    paint_mode_glyph(&painter, mode, well + down_by, well_r / 19.0, glyph_colour);
    // Its name and what it is.
    let left = well.x + well_r + 12.0;
    let room = (r.max.x - left - 12.0).max(1.0);
    let title = widgets::fitted_galley(ui, |size| egui::RichText::new(name).font(strong(size)).extra_letter_spacing(ta_ui::i18n::tracking(1.0)), if r.height() >= 64.0 { 15.5 } else { 14.0 }, room, 0.75);
    let detail = hint.map(|hint| {
        let mut job = LayoutJob::simple(t(hint).to_string(), body(12.5), if primary { Color32::from_rgb(64, 46, 14) } else { STEEL }, room);
        job.wrap.max_rows = 2;
        job.wrap.break_anywhere = false;
        painter.layout_job(job)
    });
    let height = title.size().y + detail.as_ref().map_or(0.0, |d| 3.0 + d.size().y);
    let mut y = r.center().y - height / 2.0 + down_by.y;
    // (Lettered as the buttons are: a dark edge under light letters, a light one under dark.)
    let title_pos = pos2(left, y);
    if primary {
        painter.galley_with_override_text_color(title_pos + vec2(0.0, 1.0), title.clone(), edge);
    } else {
        painter.galley_with_override_text_color(title_pos + vec2(1.0, 1.0), title.clone(), edge);
    }
    painter.galley(title_pos, title.clone(), ink);
    y += title.size().y + 3.0;
    if let Some(detail) = detail {
        painter.galley(pos2(left, y), detail, STEEL);
    }
    widgets::focus_ring(ui, &response, r);
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// A mode's glyph, centred on `c`, `scale` 1 for a 38-point well: line drawings, original.
pub(super) fn paint_mode_glyph(painter: &Painter, mode: Mode, c: Pos2, scale: f32, colour: Color32) {
    let p = |x: f32, y: f32| c + vec2(x, y) * scale;
    let line = Stroke::new((1.7 * scale).max(1.2), colour);
    let thin = Stroke::new((1.2 * scale).max(1.0), colour);
    match mode {
        Mode::Tutorial => {
            // An open manual: two pages bowed from the spine, lines of type on each.
            painter.add(Shape::closed_line(vec![p(-1.0, -5.5), p(-5.0, -7.5), p(-10.5, -7.0), p(-10.5, 6.5), p(-5.0, 6.0), p(-1.0, 8.0)], line));
            painter.add(Shape::closed_line(vec![p(1.0, -5.5), p(5.0, -7.5), p(10.5, -7.0), p(10.5, 6.5), p(5.0, 6.0), p(1.0, 8.0)], line));
            for y in [-3.5, -0.5, 2.5] {
                painter.line_segment([p(-8.0, y), p(-3.5, y + 0.6)], thin);
                painter.line_segment([p(3.5, y + 0.6), p(8.0, y)], thin);
            }
        }
        Mode::Skirmish => {
            // Two blades crossed, their guards and grips.
            for d in [-1.0f32, 1.0] {
                painter.line_segment([p(-8.5 * d, -8.5), p(5.0 * d, 5.0)], line);
                painter.line_segment([p(2.0 * d, 7.5), p(7.5 * d, 2.0)], line);
                painter.line_segment([p(5.0 * d, 5.0), p(8.5 * d, 8.5)], Stroke::new((2.6 * scale).max(1.6), colour));
            }
        }
        Mode::Campaign => {
            // A flag on its staff, planted.
            painter.line_segment([p(-6.5, -9.0), p(-6.5, 9.0)], line);
            painter.add(Shape::convex_polygon(vec![p(-6.5, -9.0), p(8.5, -5.8), p(-6.5, -1.0)], colour, Stroke::NONE));
            painter.line_segment([p(-10.0, 9.0), p(-3.0, 9.0)], line);
        }
        Mode::Scenarios => {
            // A target: two rings, its centre, and the ticks of a sight.
            painter.circle_stroke(c, 8.0 * scale, line);
            painter.circle_stroke(c, 4.0 * scale, thin);
            painter.circle_filled(c, 1.6 * scale, colour);
            for (x, y) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
                painter.line_segment([p(8.0 * x, 8.0 * y), p(11.0 * x, 11.0 * y)], line);
            }
        }
        Mode::Conquest => {
            // A ringed world and a star.
            painter.circle_stroke(c, 6.0 * scale, line);
            let ring: Vec<Pos2> = (0..=32).map(|k| {
                let a = k as f32 / 32.0 * std::f32::consts::TAU;
                let (x, y) = (11.0 * a.cos(), 3.6 * a.sin());
                let tilt = -0.42f32;
                p(x * tilt.cos() - y * tilt.sin(), x * tilt.sin() + y * tilt.cos())
            }).collect();
            painter.add(Shape::line(ring, thin));
            painter.circle_filled(p(8.0, -8.0), 1.4 * scale, colour);
        }
        Mode::Multiplayer => {
            // Three stations linked.
            let nodes = [p(0.0, -7.5), p(-8.0, 6.0), p(8.0, 6.0)];
            for (i, &a) in nodes.iter().enumerate() {
                let b = nodes[(i + 1) % 3];
                painter.line_segment([a, b], thin);
            }
            for n in nodes {
                painter.circle_filled(n, 3.0 * scale, colour);
                painter.circle_stroke(n, 3.0 * scale, Stroke::new(1.0, Color32::from_black_alpha(120)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_maps_window_keeps_inside_the_picture_and_fills_the_banner() {
        let banner = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 250.0));
        for size in [vec2(1600.0, 1600.0), vec2(1600.0, 400.0), vec2(1600.0, 900.0), vec2(300.0, 1600.0)] {
            for zoom in [1.0, 0.84, 0.5] {
                for pan in [vec2(-1.0, -1.0), Vec2::ZERO, vec2(1.0, 1.0), vec2(0.4, -0.7)] {
                    let uv = window(size, banner, zoom, pan);
                    assert!(Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)).expand(1e-4).contains_rect(uv), "{size:?} {zoom} {pan:?}: {uv:?}");
                    // (The banner's shape, in texels.)
                    let texels = vec2(uv.width() * size.x, uv.height() * size.y);
                    assert!((texels.x / texels.y - 4.0).abs() < 0.01, "{size:?}: {texels:?}");
                }
            }
        }
    }
}
