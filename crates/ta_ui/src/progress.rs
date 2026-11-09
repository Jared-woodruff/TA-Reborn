//! Construction progress: a structure built along the rail, as TA builds. Ahead of the work the
//! frame's blueprint truss, faint; at the work the nanolathe's green lattice (the hologram TA
//! draws a unit in before it is metal), the beam streaming into it and sparks thrown back off
//! it; behind it finished brass plating, riveted. Work of unknown length is a build head
//! travelling to and fro along a caution-striped rail, nothing pretending to be done.
//!
//! Only active work moves, and not when it is complete, off screen or under reduced motion;
//! career and history meters stay quiet and keep their exact values. Nothing flashes: the
//! lattice glows steadily and the particles are small.
use std::time::Duration;

use egui::{Color32, Painter, Rect, Shape, Stroke, Ui, pos2, vec2};

use crate::{
    metal, motion,
    theme::{AMBER, AMBER_BRIGHT, AMBER_DIM, BG_DEEP, LINE_HI},
};

pub fn fraction(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

/// Paint inside an existing allocation. `None` is ongoing work of unknown length;
/// it never pretends that a particular percentage has completed.
pub fn paint(ui: &Ui, rect: Rect, value: Option<f32>, active: bool) {
    if !rect.is_finite() || !rect.is_positive() || !ui.is_rect_visible(rect) {
        return;
    }
    paint_at(ui.painter(), rect, value, active, ui.input(|i| i.time));
}

/// The nanolathe's green, a little softer than a lamp's.
const NANO: Color32 = Color32::from_rgb(132, 226, 104);
/// The sparks: white-hot to the weld's orange.
const SPARK: Color32 = Color32::from_rgb(255, 222, 150);

/// The same rail for full-window loading screens, on their own animation clock.
pub fn paint_at(painter: &Painter, rect: Rect, value: Option<f32>, active: bool, time: f64) {
    if !rect.is_finite() || !rect.is_positive() || !painter.clip_rect().intersects(rect) {
        return;
    }
    let ctx = painter.ctx();
    let value = value.map(fraction);
    let moving = active && value.is_none_or(|f| f < 1.0) && !motion::still(ctx);
    // (A clock that wraps slowly enough never to jump visibly.)
    let t = if moving && time.is_finite() { time.rem_euclid(3_600.0) as f32 } else { 0.0 };
    let p = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    p.add(metal::well(rect, BG_DEEP));
    p.rect_stroke(rect, 1.0, Stroke::new(1.0, LINE_HI), egui::StrokeKind::Inside);
    let inner = rect.shrink(if rect.height() >= 12.0 { 2.5 } else { 1.0 });
    if !inner.is_positive() {
        return;
    }
    let p = p.with_clip_rect(p.clip_rect().intersect(inner));
    let rail = Rail::new(inner);
    match value {
        Some(f) => rail.build(&p, f, active, moving, t),
        None => rail.busy(&p, moving, t),
    }
    if moving {
        motion::next_frame(ctx, Duration::from_millis(33));
    }
}

/// A rail's measures: its inside, its plates' width, and whether it is tall enough for detail
/// (rivets, the truss, the beam).
struct Rail {
    r: Rect,
    cell: f32,
    tall: bool,
}

impl Rail {
    fn new(r: Rect) -> Rail {
        let h = r.height();
        Rail { r, cell: (h * 1.9).clamp(9.0, 28.0), tall: h >= 7.0 }
    }

    fn x(&self, f: f32) -> f32 {
        self.r.left() + self.r.width() * f
    }

    /// The frame not yet built: a truss of chords and diagonals, faint (`alpha` of the green),
    /// from `x0` to `x1`; on a thin rail, a dotted line.
    fn truss(&self, p: &Painter, x0: f32, x1: f32, alpha: f32) {
        if x1 <= x0 || alpha <= 0.0 {
            return;
        }
        let color = NANO.gamma_multiply(alpha);
        let (top, bottom) = (self.r.top() + 0.5, self.r.bottom() - 0.5);
        if !self.tall {
            let y = self.r.center().y;
            let mut x = self.r.left() + (x0 - self.r.left()).div_euclid(4.0) * 4.0;
            while x < x1 {
                if x >= x0 {
                    p.hline(x..=(x + 1.5).min(x1), y, Stroke::new(1.0, color));
                }
                x += 4.0;
            }
            return;
        }
        let stroke = Stroke::new(1.0, color);
        p.hline(x0..=x1, top, stroke);
        p.hline(x0..=x1, bottom, stroke);
        // The diagonals on the plates' pitch, so the truss and the plating line up.
        let half = self.cell / 2.0;
        let mut k = ((x0 - self.r.left()) / half).floor() as i32;
        loop {
            let a = self.r.left() + k as f32 * half;
            if a >= x1 {
                break;
            }
            let (ya, yb) = if k.rem_euclid(2) == 0 { (bottom, top) } else { (top, bottom) };
            let seg = [pos2(a, ya), pos2(a + half, yb)];
            p.line_segment(seg, stroke);
            if k.rem_euclid(2) == 0 {
                p.vline(a, top..=bottom, Stroke::new(1.0, color.gamma_multiply(0.6)));
            }
            k += 1;
        }
    }

    /// Finished plating from `x0` to `x1`: brass, its seams on the plates' pitch, riveted where
    /// there is room; `alpha` for a plate still forming.
    fn plating(&self, p: &Painter, x0: f32, x1: f32, alpha: f32) {
        if x1 <= x0 {
            return;
        }
        let fill = Rect::from_min_max(pos2(x0, self.r.top()), pos2(x1, self.r.bottom()));
        p.add(metal::vgrad(fill, AMBER_BRIGHT.gamma_multiply(0.9 * alpha), AMBER_DIM.gamma_multiply(alpha)));
        p.hline(fill.x_range(), fill.top() + 0.5, Stroke::new(1.0, AMBER_BRIGHT.gamma_multiply(alpha)));
        p.hline(fill.x_range(), fill.bottom() - 0.5, Stroke::new(1.0, Color32::from_black_alpha((110.0 * alpha) as u8)));
        let plates = p.with_clip_rect(p.clip_rect().intersect(fill));
        let mut x = self.r.left() + self.cell;
        while x < x1 {
            if x > x0 {
                plates.vline(x, fill.y_range(), Stroke::new(1.5, Color32::from_black_alpha((130.0 * alpha) as u8)));
                plates.vline(x + 1.25, fill.y_range(), Stroke::new(0.75, Color32::from_white_alpha((50.0 * alpha) as u8)));
                if self.r.height() >= 11.0 {
                    for y in [fill.top() + 2.6, fill.bottom() - 2.6] {
                        let at = pos2(x + 3.2, y);
                        plates.circle_filled(at + vec2(0.3, 0.4), 1.05, Color32::from_black_alpha((120.0 * alpha) as u8));
                        plates.circle_filled(at, 0.95, AMBER_BRIGHT.gamma_multiply(alpha));
                    }
                }
            }
            x += self.cell;
        }
    }

    /// The work's edge at `x`: the nanolathe's lattice forming the next plate behind it, its
    /// glowing face, the lattice reaching on ahead.
    fn front(&self, p: &Painter, x: f32, glow: f32) {
        let r = self.r;
        // The plate forming: green lattice over it, the brass coming through.
        let forming = (self.cell * 0.9).min(x - r.left()).max(0.0);
        if forming > 0.5 {
            let band = Rect::from_min_max(pos2(x - forming, r.top()), pos2(x, r.bottom()));
            p.add(metal::hgrad(band, NANO.gamma_multiply(0.0), NANO.gamma_multiply(0.28 * glow)));
        }
        let ahead = (self.cell * 1.6).min(r.right() - x).max(0.0);
        if self.tall && ahead > 0.5 {
            // The lattice ahead of the work, fading as it reaches out.
            let steps = 6;
            for i in 0..steps {
                let a = x + ahead * i as f32 / steps as f32;
                let b = x + ahead * (i + 1) as f32 / steps as f32;
                let fade = 1.0 - (i as f32 + 0.5) / steps as f32;
                self.truss(p, a, b, 0.55 * fade * glow);
            }
        }
        // The face: a soft green glow and its bright edge.
        p.vline(x, r.y_range(), Stroke::new(if self.tall { 5.0 } else { 3.0 }, NANO.gamma_multiply(0.18 * glow)));
        p.vline(x, r.y_range(), Stroke::new(if self.tall { 1.6 } else { 1.0 }, NANO.gamma_multiply(0.95 * glow)));
    }

    /// The nanolathe's beam streaming into the face at `x` from up ahead (a builder off the
    /// rail), as a faint cone and particles running down it.
    fn beam(&self, p: &Painter, x: f32, t: f32) {
        if !self.tall {
            return;
        }
        let r = self.r;
        let reach = (self.cell * 2.2).min(r.right() - x).max(self.cell * 0.8);
        let source = pos2(x + reach, r.top() - r.height() * 0.6);
        p.add(Shape::convex_polygon(
            vec![source, pos2(x, r.top() + 0.5), pos2(x, r.bottom() - 0.5)],
            NANO.gamma_multiply(0.07),
            Stroke::NONE,
        ));
        const PARTICLES: usize = 9;
        for i in 0..PARTICLES {
            let phase = (t / 0.85 + i as f32 / PARTICLES as f32).fract();
            let lane = 0.14 + 0.72 * hash(i as u32 * 7 + 3);
            let end = pos2(x + 0.5, r.top() + r.height() * lane);
            let at = source + (end - source) * phase;
            let size = if r.height() >= 12.0 { 1.25 } else { 0.95 };
            p.circle_filled(at, size, NANO.gamma_multiply(0.25 + 0.75 * phase));
        }
    }

    /// Sparks thrown back off the face at `x` over the finished plating, falling as they cool.
    fn sparks(&self, p: &Painter, x: f32, t: f32) {
        if !self.tall {
            return;
        }
        let r = self.r;
        const SPARKS: usize = 6;
        for i in 0..SPARKS {
            let life = 0.55 + 0.35 * hash(i as u32 * 13 + 1);
            let phase = (t / life + hash(i as u32 * 5 + 9)).fract();
            let speed = self.cell * (0.9 + 0.9 * hash(i as u32 * 11 + 4));
            let rise = r.height() * (0.15 + 0.35 * hash(i as u32 * 3 + 7));
            let start = pos2(x - 1.0, r.top() + r.height() * (0.3 + 0.4 * hash(i as u32 * 17 + 2)));
            let at = start + vec2(-speed * phase, -rise * phase + r.height() * 1.4 * phase * phase);
            let tail = at + vec2(1.8, -0.6 + r.height() * 0.3 * phase);
            let cool = 1.0 - phase;
            let color = lerp(AMBER, SPARK, cool).gamma_multiply(cool.sqrt());
            p.line_segment([at, tail + vec2(0.6, 0.0)], Stroke::new(1.2, color));
        }
    }

    /// Work done to `f` (`active`: going on), the structure built that far: whole plates up to
    /// the last seam, and the plate the work is on rising from the bottom as the work crosses
    /// it (TA raises what it builds from the ground up), its lattice above it.
    fn build(&self, p: &Painter, f: f32, active: bool, moving: bool, t: f32) {
        let x = self.x(f);
        self.truss(p, x, self.r.right(), 0.12);
        if f >= 1.0 || !self.tall {
            self.plating(p, self.r.left(), x, 1.0);
            if f >= 1.0 {
                return;
            }
        } else {
            let seam = self.r.left() + ((x - self.r.left()) / self.cell).floor() * self.cell;
            self.plating(p, self.r.left(), seam, 1.0);
            if x > seam {
                let k = ((x - seam) / self.cell).clamp(0.0, 1.0);
                let top = self.r.bottom() - self.r.height() * (0.3 + 0.7 * k);
                let risen = Rect::from_min_max(pos2(seam, top), pos2(x, self.r.bottom()));
                p.add(metal::vgrad(risen, AMBER_BRIGHT.gamma_multiply(0.9), AMBER_DIM));
                p.vline(seam, risen.y_range(), Stroke::new(1.5, Color32::from_black_alpha(130)));
                // The lattice over the rest of the plate, and the line it is being built up to.
                let above = Rect::from_min_max(pos2(seam, self.r.top()), pos2(x, top));
                p.rect_filled(above, 0.0, NANO.gamma_multiply(0.12));
                p.hline(seam..=x, top, Stroke::new(1.0, NANO.gamma_multiply(0.8)));
            }
        }
        if active {
            // (The face breathes a little while it works: steady light, never a flash.)
            let glow = if moving { 0.85 + 0.15 * (t * std::f32::consts::TAU / 2.0).sin() } else { 0.85 };
            self.front(p, x, glow);
            if moving {
                self.beam(p, x, t);
                if f > 0.0 {
                    self.sparks(p, x, t);
                }
            }
        } else if f > 0.0 {
            // A quiet meter: where it stands, no work.
            p.vline(x, self.r.y_range(), Stroke::new(1.0, NANO.gamma_multiply(0.45)));
        }
    }

    /// Work of unknown length: the rail striped for a site at work, a build head travelling
    /// to and fro along it (still, in the middle, when nothing moves).
    fn busy(&self, p: &Painter, moving: bool, t: f32) {
        let r = self.r;
        // The caution stripes, marching slowly.
        let pitch = (r.height() * 2.0).max(6.0);
        let drift = if moving { (t * 10.0).rem_euclid(pitch) } else { 0.0 };
        let mut x = r.left() - pitch * 2.0 + drift;
        let lean = r.height();
        while x < r.right() + pitch {
            p.add(Shape::convex_polygon(
                vec![pos2(x, r.bottom()), pos2(x + pitch * 0.5, r.bottom()), pos2(x + pitch * 0.5 + lean, r.top()), pos2(x + lean, r.top())],
                AMBER_DIM.gamma_multiply(0.42),
                Stroke::NONE,
            ));
            x += pitch;
        }
        self.truss(p, r.left(), r.right(), 0.10);
        let sweep = if moving { 0.5 - 0.5 * (t * std::f32::consts::TAU / 3.6).cos() } else { 0.5 };
        let span = (self.cell * 2.0).min(r.width() * 0.4);
        let head = r.left() + span + (r.width() - 2.0 * span).max(0.0) * sweep;
        // A stretch of plating trailing the head, as if it lays it as it goes.
        let left = (head - span).max(r.left());
        self.plating(p, left, head, 0.85);
        self.front(p, head, 0.9);
        if moving {
            self.beam(p, head, t);
            self.sparks(p, head, t);
        }
    }
}

/// A fixed fraction from `i` (the particles' places: the same on every frame).
fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65_535.0
}

fn lerp(a: Color32, b: Color32, f: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// Where the face of the work stands in `rect` for `value` (tests, and anything drawn beside
/// the rail that must stay clear of it).
pub fn front_x(rect: Rect, value: f32) -> f32 {
    let inner = rect.shrink(if rect.height() >= 12.0 { 2.5 } else { 1.0 });
    inner.left() + inner.width() * fraction(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_animation_stops_when_complete_reduced_hidden_or_inactive() {
        let ctx = egui::Context::default();
        let run = |value, active, reduced, visible, time| {
            motion::set_reduced(&ctx, reduced);
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
                let rect = if visible { Rect::from_min_size(pos2(12.0, 12.0), vec2(220.0, 18.0)) } else { Rect::from_min_size(pos2(-500.0, -500.0), vec2(220.0, 18.0)) };
                paint(ui, rect, value, active);
            });
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        for i in 0..5 {
            run(Some(1.0), false, false, true, i as f64);
        }
        assert!(run(Some(0.5), true, false, true, 5.0) <= Duration::from_millis(34));
        assert!(run(None, true, false, true, 6.0) <= Duration::from_millis(34));
        for (value, active, reduced, visible) in [(Some(1.0), true, false, true), (Some(0.5), false, false, true), (None, true, true, true), (None, true, false, false)] {
            assert!(run(value, active, reduced, visible, 7.0) > Duration::from_secs(1));
        }
    }

    #[test]
    fn malformed_fractions_and_tiny_meters_are_bounded() {
        assert_eq!([fraction(f32::NAN), fraction(f32::INFINITY), fraction(-0.5), fraction(1.5)], [0.0, 0.0, 0.0, 1.0]);
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |ui| {
            for width in [0.0, 1.0, 8.0, 40.0, 600.0] {
                for height in [2.0, 5.0, 9.0, 22.0] {
                    for value in [Some(f32::NAN), Some(0.0), Some(0.5), Some(1.0), None] {
                        paint(ui, Rect::from_min_size(pos2(8.0, 8.0), vec2(width, height)), value, true);
                    }
                }
            }
        })
        .drop_without_applying_deltas();
    }

    /// The built plating ends exactly at the work's face: a fraction shown is the fraction done.
    #[test]
    fn the_plating_stops_at_the_fraction_done() {
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 20.0));
        let inner = rect.shrink(2.5);
        for f in [0.0, 0.25, 0.5, 0.999, 1.0] {
            assert!((front_x(rect, f) - (inner.left() + inner.width() * f)).abs() < 1e-4);
        }
        assert_eq!(front_x(rect, f32::NAN), inner.left());
    }
}
