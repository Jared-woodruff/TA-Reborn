//! The About page's art: the nameplate's drawing board, the marks on the credits' plaques and the
//! other tiles (original line drawings, as the Play page's are), a plaque or a tile as it is drawn
//! (a mark in its well, a stamped role or a title, the names), and a plate built in by a
//! nano-lathe as it first comes into sight.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use egui::text::LayoutJob;
use egui::{Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Response, RichText, Sense, Shadow, Shape, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2};
use ta_ui::metal::{self, Slab};

use crate::theme::*;
use crate::widgets;

/// The nano-lathe's green (the loading screen's).
const NANO: Color32 = Color32::from_rgb(120, 236, 98);

// ---- The drawing board -----------------------------------------------------------------------

/// The nameplate's ground: a drawing board's grid in faint brass, every fourth line a little
/// stronger, and the construction lines of the emblem being built at `art` (its gauge's centre and
/// radius): a cross through it and two circles, as a draughtsman lays them out.
pub(super) fn drawing_board(painter: &Painter, rect: Rect, art: Option<(Pos2, f32)>) {
    let p = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    let line = |a: u8| Color32::from_rgba_unmultiplied(216, 180, 86, a);
    let step = 24.0;
    let (mut k, mut x) = (1, rect.min.x + step);
    while x < rect.max.x {
        p.vline(x.round() + 0.5, rect.y_range(), Stroke::new(1.0, line(if k % 4 == 0 { 15 } else { 7 })));
        x += step;
        k += 1;
    }
    let (mut k, mut y) = (1, rect.min.y + step);
    while y < rect.max.y {
        p.hline(rect.x_range(), y.round() + 0.5, Stroke::new(1.0, line(if k % 4 == 0 { 15 } else { 7 })));
        y += step;
        k += 1;
    }
    if let Some((c, r)) = art {
        let faint = Stroke::new(1.0, line(34));
        p.hline((c.x - r * 1.5)..=(c.x + r * 1.5), c.y, faint);
        p.vline(c.x, (c.y - r * 1.25)..=(c.y + r * 1.25), faint);
        p.circle_stroke(c, r * 1.32, Stroke::new(1.0, line(22)));
        // (Ticks where the circle crosses its axes, as on a drawing.)
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let at = c + vec2(dx, dy) * r * 1.32;
            let across = vec2(dy, dx) * 4.0;
            p.line_segment([at - across, at + across], Stroke::new(1.0, line(60)));
        }
    }
}

// ---- The marks -------------------------------------------------------------------------------

/// What a plaque or a tile shows in its well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// The project lead: a commander's star.
    Lead,
    /// Development: a slash between angle brackets.
    Dev,
    /// Quality assurance: a magnifier over a tick.
    Qa,
    /// The voices: a microphone on its stand.
    Voice,
    /// Music: two notes beamed together.
    Music,
    /// The original game: its disc, as it came in 1997.
    Disc,
    /// The website: a globe.
    Globe,
    /// The source: a branch of commits.
    Source,
    /// A store: a shopping bag.
    Store,
    /// Free software: a licence, a page with its corner turned down.
    Licence,
    /// Owned by others: a copyright seal.
    Seal,
    /// Written anew: a pen's nib.
    Nib,
    /// Modern hardware: a chip and its pins.
    Chip,
    /// The player's own copy, kept as it is: a padlock.
    Lock,
}

/// A mark, centred on `c`, `scale` 1 for a 38-point well (as the Play page's mode glyphs are).
pub(super) fn paint_mark(painter: &Painter, mark: Mark, c: Pos2, scale: f32, colour: Color32) {
    let p = |x: f32, y: f32| c + vec2(x, y) * scale;
    let line = Stroke::new((1.7 * scale).max(1.2), colour);
    let thin = Stroke::new((1.2 * scale).max(1.0), colour);
    let arc = |centre: Pos2, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        (0..=24).map(|k| {
            let a = from + (to - from) * k as f32 / 24.0;
            centre + vec2(a.cos(), a.sin()) * r * scale
        }).collect()
    };
    match mark {
        Mark::Lead => {
            // A five-pointed star, filled from its middle.
            let mut mesh = Mesh::default();
            mesh.colored_vertex(c, colour);
            for k in 0..10 {
                let a = -FRAC_PI_2 + k as f32 * PI / 5.0;
                let r = if k % 2 == 0 { 10.0 } else { 4.2 };
                mesh.colored_vertex(p(r * a.cos(), r * a.sin() + 0.6), colour);
            }
            for k in 0..10u32 {
                mesh.add_triangle(0, 1 + k, 1 + (k + 1) % 10);
            }
            painter.add(Shape::mesh(mesh));
        }
        Mark::Dev => {
            painter.add(Shape::line(vec![p(-4.5, -6.5), p(-10.5, 0.0), p(-4.5, 6.5)], line));
            painter.add(Shape::line(vec![p(4.5, -6.5), p(10.5, 0.0), p(4.5, 6.5)], line));
            painter.line_segment([p(2.2, -8.5), p(-2.2, 8.5)], line);
        }
        Mark::Qa => {
            let lens = p(-2.0, -2.0);
            painter.circle_stroke(lens, 7.0 * scale, line);
            painter.line_segment([p(3.2, 3.2), p(9.0, 9.0)], Stroke::new((2.8 * scale).max(1.8), colour));
            painter.add(Shape::line(vec![p(-5.6, -2.2), p(-3.0, 0.6), p(1.6, -4.6)], thin));
        }
        Mark::Voice => {
            // The capsule, its grille, the cradle round it and the stand.
            let capsule = Rect::from_min_max(p(-3.8, -10.0), p(3.8, 2.4));
            painter.rect_stroke(capsule, CornerRadius::same((3.8 * scale) as u8), line, egui::StrokeKind::Middle);
            for y in [-6.0, -3.0] {
                painter.line_segment([p(-1.8, y), p(1.8, y)], thin);
            }
            painter.add(Shape::line(arc(p(0.0, -1.5), 6.8, 0.0, PI), line));
            painter.line_segment([p(0.0, 5.3), p(0.0, 9.0)], line);
            painter.line_segment([p(-4.5, 9.0), p(4.5, 9.0)], line);
        }
        Mark::Music => {
            for head in [p(-5.6, 6.0), p(5.4, 3.8)] {
                painter.circle_filled(head, 2.9 * scale, colour);
            }
            painter.line_segment([p(-2.9, 6.0), p(-2.9, -6.5)], line);
            painter.line_segment([p(8.1, 3.8), p(8.1, -8.6)], line);
            painter.line_segment([p(-2.9, -6.5), p(8.1, -8.6)], Stroke::new((2.6 * scale).max(1.6), colour));
        }
        Mark::Disc => {
            painter.circle_stroke(c, 10.0 * scale, line);
            painter.circle_stroke(c, 2.8 * scale, thin);
            // A gleam across its face.
            painter.add(Shape::line(arc(c, 6.6, -2.5, -1.35), thin));
            painter.add(Shape::line(arc(c, 6.6, 0.64, 1.79), Stroke::new(thin.width, colour.gamma_multiply(0.55))));
        }
        Mark::Globe => {
            painter.circle_stroke(c, 9.5 * scale, line);
            painter.line_segment([p(-9.5, 0.0), p(9.5, 0.0)], thin);
            for y in [-5.0f32, 5.0] {
                let half = (9.5f32 * 9.5 - y * y).sqrt();
                painter.line_segment([p(-half, y), p(half, y)], thin);
            }
            let meridian: Vec<Pos2> = (0..=32).map(|k| {
                let a = k as f32 / 32.0 * TAU;
                p(4.2 * a.cos(), 9.5 * a.sin())
            }).collect();
            painter.add(Shape::closed_line(meridian, thin));
        }
        Mark::Source => {
            let (top, bottom, side) = (p(-5.0, -7.5), p(-5.0, 7.5), p(5.5, -3.5));
            painter.line_segment([top, bottom], line);
            let curve: Vec<Pos2> = (0..=16).map(|k| {
                let t = k as f32 / 16.0;
                let a = pos2(5.5, -1.0);
                let (b, d) = (pos2(5.5, 3.0), pos2(-5.0, 3.5));
                let q = |u: f32, v: f32| u + (v - u) * t;
                let (x1, y1) = (q(a.x, b.x), q(a.y, b.y));
                let (x2, y2) = (q(b.x, d.x), q(b.y, d.y));
                p(q(x1, x2), q(y1, y2))
            }).collect();
            painter.add(Shape::line(curve, line));
            for node in [top, bottom, side] {
                painter.circle_filled(node, 3.0 * scale, INSET);
                painter.circle_stroke(node, 3.0 * scale, line);
            }
        }
        Mark::Store => {
            let body = vec![p(-8.0, -3.0), p(8.0, -3.0), p(7.0, 9.0), p(-7.0, 9.0)];
            painter.add(Shape::closed_line(body, line));
            painter.add(Shape::line(arc(p(0.0, -3.0), 4.6, PI, TAU), line));
            painter.line_segment([p(-3.5, 2.5), p(3.5, 2.5)], thin);
        }
        Mark::Licence => {
            painter.add(Shape::closed_line(vec![p(-7.5, -9.5), p(4.0, -9.5), p(7.5, -6.0), p(7.5, 9.5), p(-7.5, 9.5)], line));
            painter.add(Shape::line(vec![p(4.0, -9.5), p(4.0, -6.0), p(7.5, -6.0)], thin));
            for y in [-3.0, 0.5, 4.0] {
                painter.line_segment([p(-4.5, y), p(4.5, y)], thin);
            }
            painter.line_segment([p(-4.5, 7.0), p(0.5, 7.0)], thin);
        }
        Mark::Seal => {
            painter.circle_stroke(c, 10.0 * scale, line);
            painter.circle_stroke(c, 7.6 * scale, Stroke::new(thin.width, colour.gamma_multiply(0.6)));
            painter.add(Shape::line(arc(c, 4.4, 0.75, TAU - 0.75), line));
        }
        Mark::Nib => {
            // A pen's nib writing down to the left: its shoulders, the point, the slit up from it
            // to the breather hole, and the line it has drawn.
            let (s, k) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
            let q = |x: f32, y: f32| p(x * k - y * s + 1.5, x * s + y * k - 2.0);
            painter.add(Shape::closed_line(vec![q(0.0, 10.0), q(-5.8, 0.5), q(-5.8, -4.0), q(-3.0, -8.5), q(3.0, -8.5), q(5.8, -4.0), q(5.8, 0.5)], line));
            painter.line_segment([q(0.0, 10.0), q(0.0, 1.6)], thin);
            painter.circle_stroke(q(0.0, -0.6), 1.9 * scale, thin);
            painter.line_segment([p(-10.5, 9.0), p(-2.5, 9.0)], thin);
        }
        Mark::Chip => {
            let die = Rect::from_min_max(p(-6.5, -6.5), p(6.5, 6.5));
            painter.rect_stroke(die, CornerRadius::same(1), line, egui::StrokeKind::Middle);
            painter.rect_stroke(die.shrink(3.6 * scale), CornerRadius::ZERO, thin, egui::StrokeKind::Middle);
            for k in [-3.6f32, 0.0, 3.6] {
                painter.line_segment([p(k, -6.5), p(k, -10.0)], thin);
                painter.line_segment([p(k, 6.5), p(k, 10.0)], thin);
                painter.line_segment([p(-6.5, k), p(-10.0, k)], thin);
                painter.line_segment([p(6.5, k), p(10.0, k)], thin);
            }
        }
        Mark::Lock => {
            let body = Rect::from_min_max(p(-7.0, -1.0), p(7.0, 9.5));
            painter.rect_stroke(body, CornerRadius::same((2.0 * scale) as u8), line, egui::StrokeKind::Middle);
            painter.add(Shape::line(arc(p(0.0, -1.0), 4.8, PI, TAU), line));
            painter.line_segment([p(-4.8, -1.0), p(-4.8, -2.0)], line);
            painter.line_segment([p(4.8, -1.0), p(4.8, -2.0)], line);
            painter.circle_filled(p(0.0, 3.4), 1.7 * scale, colour);
            painter.line_segment([p(0.0, 3.4), p(0.0, 6.4)], thin);
        }
    }
}

/// A mark in its round well (as the Play page's tiles have them): dark, rimmed in steel, lit
/// a little under the pointer (`hot`).
pub(super) fn mark_well(painter: &Painter, mark: Mark, centre: Pos2, radius: f32, hot: f32) {
    painter.circle_filled(centre, radius, INSET);
    painter.circle_stroke(centre, radius, Stroke::new(1.0, metal::STEEL_EDGE));
    painter.circle_stroke(centre, radius + 1.0, Stroke::new(1.0, Color32::from_white_alpha((14.0 + 22.0 * hot) as u8)));
    if hot > 0.01 {
        widgets::soft_glow(painter, centre, 0.0, radius * 0.95, AMBER.gamma_multiply(0.22 * hot));
    }
    paint_mark(painter, mark, centre, radius / 19.0, STEEL.lerp_to_gamma(AMBER_BRIGHT, 0.35 + 0.65 * hot));
}

// ---- Built in --------------------------------------------------------------------------------

/// How far a plate has been built in (0 nothing yet, 1 built) `delay` seconds after it first came
/// into sight on the page shown since `since`, over `BUILD_TIME`; built at once under reduced
/// motion. Asks for the frames it needs.
pub(super) fn built(ui: &Ui, id: egui::Id, rect: Rect, since: f64, delay: f32) -> f32 {
    const BUILD_TIME: f64 = 0.5;
    if ta_ui::motion::reduced(ui.ctx()) {
        return 1.0;
    }
    // (When it was first seen since the page came in: a new visit builds it again.)
    let key = id.with("about_built");
    let now = ui.input(|i| i.time);
    let visible = ui.is_rect_visible(rect);
    let seen = ui.ctx().data_mut(|d| {
        let stored = d.get_temp::<(f64, f64)>(key).filter(|&(visit, _)| visit == since);
        match stored {
            Some((_, seen)) => Some(seen),
            None if visible => {
                d.insert_temp(key, (since, now));
                Some(now)
            }
            None => None,
        }
    });
    let Some(seen) = seen else { return 0.0 };
    let t = ((now - seen - delay as f64) / BUILD_TIME).clamp(0.0, 1.0) as f32;
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    t
}

/// Over a plate being built in (`t` 0 to 1): the part still to come dark under the front rising
/// through it, its outline in the lathe's green, the front a bright line over a glow. Nothing once
/// it is built.
pub(super) fn build_over(painter: &Painter, rect: Rect, t: f32) {
    if t >= 1.0 {
        return;
    }
    let eased = t * t * (3.0 - 2.0 * t);
    let front = rect.max.y - rect.height() * eased;
    let todo = Rect::from_min_max(rect.min, pos2(rect.max.x, front));
    if todo.height() > 0.5 {
        painter.rect_filled(todo.expand(1.0), CornerRadius::same(2), Color32::from_rgba_unmultiplied(10, 11, 8, (235.0 * (1.0 - 0.25 * eased)) as u8));
        painter.rect_stroke(todo, CornerRadius::same(2), Stroke::new(1.0, NANO.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    }
    if front > rect.min.y + 0.5 {
        let glow = 12.0f32.min(rect.max.y - front);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(pos2(rect.min.x, front), NANO.gamma_multiply(0.32));
        mesh.colored_vertex(pos2(rect.max.x, front), NANO.gamma_multiply(0.32));
        mesh.colored_vertex(pos2(rect.max.x, front + glow), Color32::TRANSPARENT);
        mesh.colored_vertex(pos2(rect.min.x, front + glow), Color32::TRANSPARENT);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
        painter.hline(rect.x_range(), front, Stroke::new(1.6, Color32::from_rgb(196, 255, 176)));
    }
}

// ---- Plaques and tiles -----------------------------------------------------------------------

/// A plaque of the credits: a role stamped in brass over its names, a mark in its well; a line
/// under them (`detail`) in steel, or in brass with `link` (the plaque is then a link to it).
pub(super) struct Plaque<'a> {
    pub mark: Mark,
    /// The role, translated.
    pub role: &'a str,
    /// The names, as credited.
    pub names: Vec<&'a str>,
    pub detail: Option<&'a str>,
    pub link: Option<&'a str>,
    /// What a screen reader says of it.
    pub spoken: String,
    /// Its tooltip (a link's is its address).
    pub tip: Option<&'a str>,
}

/// The room a plaque `width` across has for its words.
pub(super) fn plaque_room(width: f32) -> f32 {
    (width - 15.0 - 2.0 * PLAQUE_WELL - 14.0 - 20.0).max(1.0)
}

/// The radius of a plaque's well.
const PLAQUE_WELL: f32 = 19.0;

/// Draws `plaque` filling `size`, its names `name_size` (every plaque of a roll alike), built in
/// as far as `built` says. A plaque with a link rises a little and lights under the pointer, a
/// gleam crossing it and a brass rule opening along its foot (as the Play page's tiles do); the
/// others stand still.
pub(super) fn plaque(ui: &mut Ui, plaque: &Plaque, size: Vec2, name_size: f32, built: f32) -> Response {
    let link = plaque.link.is_some();
    let (rect, response) = ui.allocate_exact_size(size, if link { Sense::click() } else { Sense::hover() });
    let spoken = plaque.spoken.clone();
    response.widget_info(|| WidgetInfo::labeled(if link { WidgetType::Link } else { WidgetType::Label }, true, spoken.clone()));
    if link {
        widgets::reveal_focus(ui, &response);
        if response.clicked() {
            ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Tab);
        }
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let ctx = ui.ctx().clone();
    let hot = if link { ctx.animate_bool_with_time(response.id.with("hot"), response.hovered() || response.has_focus(), 0.14) } else { 0.0 };
    let down = link && response.is_pointer_button_down_on();
    let lift = if ta_ui::motion::reduced(&ctx) { 0.0 } else { 2.0 * hot } - if down { 1.0 } else { 0.0 };
    let r = rect.translate(vec2(0.0, -lift));
    let painter = ui.painter().clone();
    painter.add(Shadow { offset: [0, (3.0 + 4.0 * hot) as i8], blur: (8.0 + 10.0 * hot) as u8, spread: 0, color: Color32::from_black_alpha((95.0 + 50.0 * hot) as u8) }.as_shape(r, CornerRadius::same(3)));
    painter.add(metal::slab(&ctx, r, Slab::Slot, hot, down));
    painter.add(metal::grime_over(&ctx, r.shrink(1.0), 0.6));
    painter.rect_stroke(r.shrink(0.5), CornerRadius::same(2), Stroke::new(1.0, LINE.lerp_to_gamma(AMBER_DIM, hot)), egui::StrokeKind::Inside);
    // Two rivets at its right end, as a plaque is held up.
    for y in [r.min.y + 9.0, r.max.y - 9.0] {
        painter.add(metal::rivet(pos2(r.max.x - 9.0, y), 2.4));
    }
    if link && !ta_ui::motion::reduced(&ctx) && !ta_ui::motion::still(&ctx) {
        crate::library::gleam(ui, response.id, r.shrink(2.0), response.hovered() || response.has_focus());
    }
    if hot > 0.01 {
        let half = (r.width() - 16.0) * 0.5 * hot;
        painter.line_segment([pos2(r.center().x - half, r.max.y - 2.5), pos2(r.center().x + half, r.max.y - 2.5)], Stroke::new(2.0, AMBER.gamma_multiply(0.9)));
    }
    let well_r = PLAQUE_WELL;
    let well = pos2(r.min.x + 15.0 + well_r, r.center().y);
    mark_well(&painter, plaque.mark, well, well_r, hot);
    // The words: the role, the names (smaller when they would not fit), the line under them.
    let left = well.x + well_r + 14.0;
    let room = plaque_room(r.width());
    let role = widgets::fitted_galley(ui, |s| RichText::new(ta_ui::i18n::upper(plaque.role)).font(strong(s)).extra_letter_spacing(ta_ui::i18n::tracking(1.6)), 11.5, room, 0.7);
    let names: Vec<_> = plaque.names.iter().map(|name| widgets::fitted_galley(ui, |s| RichText::new(*name).font(strong(s)), name_size, room, 0.8)).collect();
    let detail = plaque.detail.map(|d| {
        let mut job = LayoutJob::simple(d.to_string(), body(12.5), if link { AMBER.lerp_to_gamma(AMBER_BRIGHT, hot) } else { STEEL }, room);
        job.wrap.max_rows = 1;
        painter.layout_job(job)
    });
    let height = role.size().y + 4.0 + names.iter().map(|g| g.size().y + 1.0).sum::<f32>() + detail.as_ref().map_or(0.0, |d| 2.0 + d.size().y);
    let mut y = r.center().y - height / 2.0 + if down { 1.0 } else { 0.0 };
    metal::embossed(&painter, pos2(left, y), role.clone(), AMBER);
    y += role.size().y + 4.0;
    for name in names {
        painter.galley_with_override_text_color(pos2(left + 1.0, y + 1.0), name.clone(), Color32::from_black_alpha(170));
        painter.galley_with_override_text_color(pos2(left, y), name.clone(), TEXT_BRIGHT);
        y += name.size().y + 1.0;
    }
    if let Some(detail) = detail {
        let at = pos2(left, y + 2.0);
        if link && hot > 0.01 {
            painter.hline(at.x..=at.x + detail.size().x, at.y + detail.size().y - 1.0, Stroke::new(1.0, AMBER.gamma_multiply(hot)));
        }
        painter.galley(at, detail, STEEL);
    }
    build_over(&painter, rect, built);
    if link {
        widgets::focus_ring(ui, &response, r);
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A way out of the launcher as a tile (the website, the source, the stores): its mark in a well,
/// its name and its address under it; it rises and lights under the pointer, as the Play page's
/// tiles do. `label` is translated here; `address` is not. Honours `ui.add_enabled_ui(false, …)`.
pub(super) fn link_tile(ui: &mut Ui, mark: Mark, label: &str, address: &str, size: Vec2) -> Response {
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let name = ta_ui::i18n::t(label);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, enabled, format!("{name}: {address}")));
    widgets::reveal_focus(ui, &response);
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Tab);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let ctx = ui.ctx().clone();
    let hot = ctx.animate_bool_with_time(response.id.with("hot"), enabled && (response.hovered() || response.has_focus()), 0.14);
    let down = enabled && response.is_pointer_button_down_on();
    let lift = if ta_ui::motion::reduced(&ctx) { 0.0 } else { 2.0 * hot } - if down { 1.0 } else { 0.0 };
    let r = rect.translate(vec2(0.0, -lift));
    let painter = ui.painter().clone();
    painter.add(Shadow { offset: [0, (3.0 + 4.0 * hot) as i8], blur: (8.0 + 10.0 * hot) as u8, spread: 0, color: Color32::from_black_alpha((105.0 + 50.0 * hot) as u8) }.as_shape(r, CornerRadius::same(3)));
    painter.add(metal::slab(&ctx, r, Slab::Steel, hot, down));
    if enabled && !ta_ui::motion::reduced(&ctx) && !ta_ui::motion::still(&ctx) {
        crate::library::gleam(ui, response.id, r.shrink(2.0), response.hovered() || response.has_focus());
    }
    let well_r = (r.height() * 0.3).clamp(12.0, 17.0);
    let well = pos2(r.min.x + 10.0 + well_r, r.center().y);
    mark_well(&painter, mark, well, well_r, hot);
    let left = well.x + well_r + 11.0;
    let room = (r.max.x - left - 10.0).max(1.0);
    let title = widgets::fitted_galley(ui, |s| RichText::new(name).font(strong(s)).extra_letter_spacing(ta_ui::i18n::tracking(1.0)), 14.0, room, 0.72);
    // (An address too long for the tile keeps its site and its last part: "github.com/…/TA-Reborn".)
    let fits = |text: &str| painter.layout_no_wrap(text.to_string(), body(12.0), Color32::WHITE).size().x <= room;
    let shown = match address.split_once('/') {
        Some((site, path)) if !fits(address) => {
            let last = path.rsplit('/').next().unwrap_or(path);
            let short = format!("{site}/…/{last}");
            if fits(&short) { short } else { site.to_string() }
        }
        _ => address.to_string(),
    };
    let mut job = LayoutJob::simple(shown, body(12.0), Color32::WHITE, room);
    job.wrap.max_rows = 1;
    let under = painter.layout_job(job);
    let height = title.size().y + 2.0 + under.size().y;
    let y = r.center().y - height / 2.0 + if down { 1.0 } else { 0.0 };
    metal::slab_text(&painter, pos2(left + title.size().x / 2.0, y + title.size().y / 2.0), title.clone(), true, Slab::Steel.text(hot), down);
    let at = pos2(left, y + title.size().y + 2.0).round();
    // (Light on the gunmetal, a dark edge under it, as the lettering is.)
    painter.galley_with_override_text_color(at + vec2(0.0, 1.0), under.clone(), Color32::from_black_alpha(170));
    painter.galley_with_override_text_color(at, under, Color32::from_rgb(222, 218, 198).lerp_to_gamma(Color32::WHITE, 0.3 * hot));
    widgets::focus_ring(ui, &response, r);
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}
