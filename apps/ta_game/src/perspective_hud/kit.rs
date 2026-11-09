//! The kit the cockpit's displays are drawn with: a pen that keeps one scale and one fade for a
//! whole display, and the shapes the displays are made of (segment bars, digits, plates, a clock's
//! sun and moon, the weather's symbols, an arrow for the wind).
//!
//! A display is drawn in its own unit: a millimetre of glass for the screens on a cockpit's
//! console, a pixel of a 1080-line picture for what is projected over the world. The pen turns
//! sizes (line widths, radii, letter heights) into points; positions are points already.

use std::f32::consts::{PI, TAU};

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::theme::{mono, strong};

/// The way `angle` (radians clockwise from straight up) points on the screen.
pub fn clock(angle: f32) -> Vec2 {
    vec2(angle.sin(), -angle.cos())
}

/// What the weather's symbol is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Sun,
    /// The sun on the horizon (dawn and dusk).
    Horizon,
    Moon,
    Cloud,
    Rain,
    Storm,
    Snow,
    Dust,
    Embers,
    Spores,
    Glitter,
    /// The ground's height.
    Peak,
}

/// A pen for one display: where it draws, how big its unit is, whether it draws over the world
/// (its lines and letters then wear a dark halo, to be read over sky and sand) and how far it has
/// come on (0..1).
#[derive(Clone, Copy)]
pub struct Pen<'a> {
    pub painter: &'a Painter,
    pub u: f32,
    pub halo: bool,
    pub on: f32,
}

impl<'a> Pen<'a> {
    pub fn new(painter: &'a Painter, u: f32, halo: bool, on: f32) -> Pen<'a> {
        Pen { painter, u, halo, on }
    }

    /// The same pen in a unit `k` times as big.
    pub fn scaled(&self, k: f32) -> Pen<'a> {
        Pen { u: self.u * k, ..*self }
    }

    /// Units in points.
    pub fn s(&self, units: f32) -> f32 {
        units * self.u
    }

    /// A colour as far as the display has come on.
    pub fn c(&self, col: Color32) -> Color32 {
        col.gamma_multiply(self.on)
    }

    fn shade(&self, alpha: f32) -> Color32 {
        Color32::from_black_alpha((alpha * 255.0 * self.on) as u8)
    }

    // ---- Letters -----------------------------------------------------------------------------

    /// Text in the displays' monospaced face, `size` units tall; the rectangle it took.
    pub fn text(&self, at: Pos2, align: Align2, text: impl Into<String>, size: f32, col: Color32) -> Rect {
        self.lettered(at, align, text.into(), mono(self.s(size)), col)
    }

    /// Text in the interface's semi-bold face (names).
    pub fn name(&self, at: Pos2, align: Align2, text: impl Into<String>, size: f32, col: Color32) -> Rect {
        self.lettered(at, align, text.into(), strong(self.s(size)), col)
    }

    fn lettered(&self, at: Pos2, align: Align2, text: String, font: FontId, col: Color32) -> Rect {
        let col = self.c(col);
        if self.halo {
            let d = (font.size * 0.06).max(0.8);
            let dark = Color32::from_black_alpha((col.a() as f32 * 0.6) as u8);
            for o in [vec2(d, 0.0), vec2(-d, 0.0), vec2(0.0, d), vec2(0.0, -d)] {
                self.painter.text(at + o, align, text.clone(), font.clone(), dark);
            }
        }
        self.painter.text(at, align, text, font, col)
    }

    /// How wide `text` is in the displays' face at `size` units.
    pub fn width(&self, text: &str, size: f32) -> f32 {
        self.painter.layout_no_wrap(text.to_string(), mono(self.s(size)), Color32::WHITE).size().x
    }

    /// `text` cut short (an ellipsis at its end) to fit `room` points at `size` units.
    pub fn fitted(&self, text: &str, size: f32, room: f32) -> String {
        let per = self.width("0", size).max(1.0);
        let fit = (room / per).floor().max(2.0) as usize;
        if text.chars().count() <= fit { text.to_string() } else { text.chars().take(fit - 1).collect::<String>() + "…" }
    }

    /// A figure of `places` digits, its leading zeros dim (an unlit digit's ghost), `size` units
    /// tall, its middle at `at`.
    pub fn digits(&self, at: Pos2, value: i32, places: usize, size: f32, lit: Color32, unlit: Color32) {
        let text = format!("{:0places$}", value.max(0));
        let one = self.width("0", size);
        let left = at.x - one * text.chars().count() as f32 * 0.5;
        let first = text.char_indices().find(|&(i, ch)| ch != '0' || i + 1 == text.len()).map_or(0, |(i, _)| i);
        for (i, ch) in text.chars().enumerate() {
            let col = if i < first { unlit } else { lit };
            self.text(pos2(left + one * (i as f32 + 0.5), at.y), Align2::CENTER_CENTER, ch.to_string(), size, col);
        }
    }

    // ---- Lines and shapes ---------------------------------------------------------------------

    pub fn line(&self, a: Pos2, b: Pos2, w: f32, col: Color32) {
        if self.halo {
            self.painter.line_segment([a, b], Stroke::new(self.s(w) + self.s(1.8), self.shade(0.32)));
        }
        self.painter.line_segment([a, b], Stroke::new(self.s(w), self.c(col)));
    }

    pub fn path(&self, points: Vec<Pos2>, w: f32, col: Color32) {
        if self.halo {
            self.painter.add(Shape::line(points.clone(), Stroke::new(self.s(w) + self.s(1.8), self.shade(0.32))));
        }
        self.painter.add(Shape::line(points, Stroke::new(self.s(w), self.c(col))));
    }

    /// A filled convex polygon.
    pub fn fill(&self, points: Vec<Pos2>, col: Color32) {
        self.painter.add(Shape::convex_polygon(points, self.c(col), Stroke::NONE));
    }

    pub fn dot(&self, at: Pos2, r: f32, col: Color32) {
        self.painter.circle_filled(at, self.s(r), self.c(col));
    }

    pub fn circle(&self, at: Pos2, r: f32, w: f32, col: Color32) {
        if self.halo {
            self.painter.circle_stroke(at, self.s(r), Stroke::new(self.s(w) + self.s(1.8), self.shade(0.32)));
        }
        self.painter.circle_stroke(at, self.s(r), Stroke::new(self.s(w), self.c(col)));
    }

    /// An arc about `at` from `from` to `to` (radians clockwise from straight up).
    pub fn arc(&self, at: Pos2, r: f32, from: f32, to: f32, w: f32, col: Color32) {
        let steps = (((to - from).abs() * self.s(r)) / 5.0).ceil().clamp(4.0, 120.0) as usize;
        self.path((0..=steps).map(|k| at + clock(from + (to - from) * k as f32 / steps as f32) * self.s(r)).collect(), w, col);
    }

    /// A rectangle's outline.
    pub fn frame(&self, r: Rect, w: f32, col: Color32) {
        if self.halo {
            self.painter.rect_stroke(r, 0.0, Stroke::new(self.s(w) + self.s(1.8), self.shade(0.32)), StrokeKind::Inside);
        }
        self.painter.rect_stroke(r, 0.0, Stroke::new(self.s(w), self.c(col)), StrokeKind::Inside);
    }

    /// A dark plate under what is written over the world, its edge a hairline of `edge`.
    pub fn plate(&self, r: Rect, edge: Color32, alpha: f32) {
        self.painter.rect_filled(r, self.s(2.0), self.shade(alpha));
        self.painter.rect_stroke(r, self.s(2.0), Stroke::new(self.s(1.0).max(1.0), self.c(edge).gamma_multiply(0.45)), StrokeKind::Inside);
    }

    /// A bar of `segments` lit up to `frac` (0..1), LED style.
    pub fn bar(&self, r: Rect, segments: usize, frac: f32, lit: Color32, unlit: Color32) {
        let gap = (r.width() / segments as f32 * 0.18).max(1.0);
        let w = (r.width() - gap * (segments - 1) as f32) / segments as f32;
        for k in 0..segments {
            let x = r.left() + k as f32 * (w + gap);
            let on = frac * segments as f32 > k as f32 + 0.02;
            self.painter.rect_filled(Rect::from_min_size(pos2(x, r.top()), vec2(w, r.height())), 0.0, self.c(if on { lit } else { unlit }));
        }
    }

    // ---- Symbols -------------------------------------------------------------------------------

    /// An arrow from `from` pointing `angle` (radians clockwise from straight up), `len` units
    /// long, its head `head` units across.
    pub fn arrow(&self, from: Pos2, angle: f32, len: f32, head: f32, w: f32, col: Color32) {
        let dir = clock(angle);
        let side = vec2(-dir.y, dir.x);
        let tip = from + dir * self.s(len);
        let base = tip - dir * self.s(head);
        self.line(from, base, w, col);
        self.fill(vec![tip, base + side * self.s(head * 0.5), base - side * self.s(head * 0.5)], col);
    }

    /// A symbol of the weather or the day, about `at`, `r` units across from its middle.
    pub fn glyph(&self, glyph: Glyph, at: Pos2, r: f32, col: Color32) {
        let w = (r * 0.16).max(0.9);
        let p = |x: f32, y: f32| at + vec2(x, y) * self.s(r);
        match glyph {
            Glyph::Sun => {
                self.dot(at, r * 0.42, col);
                for k in 0..8 {
                    let d = clock(k as f32 * TAU / 8.0);
                    self.line(at + d * self.s(r * 0.62), at + d * self.s(r * 0.95), w, col);
                }
            }
            Glyph::Horizon => {
                // Half a sun over a line, its rays above.
                self.painter.add(Shape::convex_polygon((0..=12).map(|k| at + vec2(0.0, r * 0.3 * self.u) + clock(-PI / 2.0 + PI * k as f32 / 12.0) * self.s(r * 0.46)).collect(), self.c(col), Stroke::NONE));
                for k in 1..6 {
                    let a = -PI / 2.0 + PI * k as f32 / 6.0;
                    self.line(at + vec2(0.0, r * 0.3 * self.u) + clock(a) * self.s(r * 0.66), at + vec2(0.0, r * 0.3 * self.u) + clock(a) * self.s(r * 0.95), w, col);
                }
                self.line(p(-0.95, 0.3), p(0.95, 0.3), w, col);
            }
            Glyph::Moon => {
                // The lune between the moon's disc and a disc set aside from it: a strip between
                // the discs' arcs, from one tip to the other.
                let (big, small, d) = (0.85f32, 0.72f32, 0.55f32);
                let x = (d * d + big * big - small * small) / (2.0 * d);
                let y = (big * big - x * x).max(0.0).sqrt();
                let phi = y.atan2(x);
                let m = |mx: f32, my: f32| at + vec2(mx, -my) * self.s(r);
                let steps = 14;
                let (up, low) = (y.atan2(x - d), (-y).atan2(x - d) + TAU);
                let mut mesh = egui::Mesh::default();
                for k in 0..=steps {
                    let t = k as f32 / steps as f32;
                    let a = phi + (TAU - 2.0 * phi) * t;
                    mesh.colored_vertex(m(big * a.cos(), big * a.sin()), self.c(col));
                    let b = up + (low - up) * t;
                    mesh.colored_vertex(m(d + small * b.cos(), small * b.sin()), self.c(col));
                }
                for k in 0..steps as u32 {
                    let i = k * 2;
                    mesh.add_triangle(i, i + 1, i + 2);
                    mesh.add_triangle(i + 1, i + 3, i + 2);
                }
                self.painter.add(Shape::mesh(mesh));
            }
            Glyph::Cloud => self.cloud(at, r, col),
            Glyph::Rain => {
                self.cloud(at + vec2(0.0, -r * 0.22 * self.u), r * 0.9, col);
                for k in -1..=1 {
                    self.line(p(k as f32 * 0.34, 0.34), p(k as f32 * 0.34 - 0.14, 0.82), w, col);
                }
            }
            Glyph::Storm => {
                self.cloud(at + vec2(0.0, -r * 0.22 * self.u), r * 0.9, col);
                self.path(vec![p(0.12, 0.24), p(-0.12, 0.54), p(0.1, 0.54), p(-0.1, 0.92)], w * 1.2, Color32::from_rgb(250, 220, 90));
            }
            Glyph::Snow => {
                for k in 0..3 {
                    let d = clock(k as f32 * PI / 3.0) * self.s(r * 0.9);
                    self.line(at - d, at + d, w, col);
                }
                self.dot(at, r * 0.12, col);
            }
            Glyph::Dust => {
                for (k, y) in [-0.5f32, 0.0, 0.5].into_iter().enumerate() {
                    let x0 = -0.9 + 0.2 * (k as f32 % 2.0);
                    self.path((0..=8).map(|i| p(x0 + 1.5 * i as f32 / 8.0, y + 0.1 * (i as f32 * 1.4 + k as f32 * 2.0).sin())).collect(), w, col);
                }
            }
            Glyph::Embers => {
                // A flame: two leaning strokes and a tongue.
                self.path(vec![p(0.0, 0.9), p(-0.5, 0.35), p(-0.3, -0.1), p(0.0, -0.95), p(0.15, -0.3), p(0.5, 0.1), p(0.4, 0.55), p(0.0, 0.9)], w, col);
                self.dot(p(0.02, 0.42), r * 0.2, col);
            }
            Glyph::Spores => {
                for (x, y, k) in [(-0.45, 0.25, 0.3), (0.2, -0.35, 0.4), (0.5, 0.45, 0.22), (-0.1, 0.62, 0.16)] {
                    self.circle(p(x, y), r * k, w, col);
                }
            }
            Glyph::Glitter => {
                // A four-pointed star, and a small one.
                for (c, k) in [(p(-0.15, 0.1), 0.85f32), (p(0.6, -0.6), 0.32)] {
                    let s = self.s(r * k);
                    self.fill(vec![c + vec2(0.0, -s), c + vec2(s * 0.28, 0.0), c + vec2(0.0, s), c + vec2(-s * 0.28, 0.0)], col);
                    self.fill(vec![c + vec2(-s, 0.0), c + vec2(0.0, -s * 0.28), c + vec2(s, 0.0), c + vec2(0.0, s * 0.28)], col);
                }
            }
            Glyph::Peak => {
                self.fill(vec![p(-1.0, 0.7), p(-0.35, -0.55), p(0.3, 0.7)], col);
                self.fill(vec![p(-0.05, 0.7), p(0.5, -0.1), p(1.0, 0.7)], col.gamma_multiply(0.7));
            }
        }
    }

    /// A cloud: three puffs on a flat base.
    fn cloud(&self, at: Pos2, r: f32, col: Color32) {
        let c = |x: f32, y: f32, k: f32| (at + vec2(x, y) * self.s(r), self.s(r * k));
        for (q, rad) in [c(-0.42, 0.1, 0.34), c(0.0, -0.12, 0.46), c(0.44, 0.12, 0.32)] {
            self.painter.circle_filled(q, rad, self.c(col));
        }
        self.painter.rect_filled(Rect::from_min_max(at + vec2(-0.42, 0.1) * self.s(r), at + vec2(0.44, 0.44) * self.s(r)), 0.0, self.c(col));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_angle_points_up_then_clockwise() {
        let close = |a: Vec2, b: Vec2| (a - b).length() < 1e-5;
        assert!(close(clock(0.0), vec2(0.0, -1.0)));
        assert!(close(clock(PI / 2.0), vec2(1.0, 0.0)));
        assert!(close(clock(PI), vec2(0.0, 1.0)));
    }
}
