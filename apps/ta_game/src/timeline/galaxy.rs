//! The chronicle's galaxy: TA's galaxy as a chapter tells it stood (`lore::Sky`), seen from
//! above its plane at a slant and turning slowly. Its stars wound on two arms round a bright
//! middle; CORE's space round the middle in its red, the ARM's share of the rim in its blue,
//! the front between them flaring with battles; the worlds of the war in their owners'
//! colours, the burned ones smouldering in smudges of ash; the fleets under way; what the
//! chapter marks, named; and a key to it all at the side. From one chapter to the next it
//! eases: its numbers move between the two, its fleets and marks fade over. All of it painted
//! here (nothing of TA's art).

use std::f32::consts::{PI, TAU};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use ta_ui::i18n::{n, t, tracking};
use ta_ui::widgets as w;

use crate::lore::{Mark, Ships, Sky};
use crate::planet::Planets;
use crate::theme::*;

/// CORE's red and the ARM's blue.
pub const CORE_RED: Color32 = Color32::from_rgb(236, 92, 58);
pub const ARM_BLUE: Color32 = Color32::from_rgb(92, 156, 255);
/// The battles' fire and the burned worlds' embers.
const FIRE: Color32 = Color32::from_rgb(255, 150, 70);

/// The disc's height on the screen for its width (seen from above at a slant).
const TILT: f32 = 0.44;
/// Where the ARM's share of the rim begins, round from the galaxy's own east.
const ARM_FROM: f32 = 0.06 * TAU;
/// How far the arms wind from the middle to the rim, and how fast the galaxy turns.
const WIND: f32 = 4.6;
const SPIN: f32 = 0.012;
const STARS: u32 = 3200;
const WORLDS: u32 = 46;
const NEBULAE: u32 = 40;

/// The marks' places on the galaxy: how far out (of its radius), how far round.
const CORE_PRIME: (f32, f32) = (0.0, 0.0);
const EMPYRREAN: (f32, f32) = (0.9, ARM_FROM + 0.075 * TAU);
const GATES: [(f32, f32); 6] = [(0.36, 0.9), (0.6, 1.95), (0.44, 3.05), (0.7, 3.95), (0.3, 4.9), (0.64, 5.75)];
/// The Core Contingency's worlds: where, their names, the kind of world each is.
const CONTINGENCY: [(f32, f32, &str, &str); 4] = [(0.63, 2.25, n("HYDROSS"), "water"), (0.78, 3.15, n("LUSCH"), "lush"), (0.57, 4.3, n("TEMBLOR"), "red"), (0.82, 5.3, n("GELIDUS"), "ice")];
const KROGOTH: (f32, f32) = (0.32, 3.55);
/// Commanders landing: CORE's in its space, the ARM's in theirs (as the war's end has them).
const COMMANDERS: [(f32, f32); 8] = [(0.28, 2.6), (0.7, 0.9), (0.4, 4.2), (0.86, 1.6), (0.2, 5.4), (0.78, 2.05), (0.45, 3.4), (0.92, 0.6)];

/// A number from 0 to 1 for `k`, the same every frame.
pub(super) fn hash(k: u32) -> f32 {
    let mut x = k.wrapping_mul(0x9E37_79B9) ^ 0x5BD1_E995;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFF_FFFF) as f32 / 16_777_215.0
}

fn smooth(from: f32, to: f32, x: f32) -> f32 {
    let s = ((x - from) / (to - from)).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a.clamp(0.0, 255.0) as u8)
}

fn faded(c: Color32, a: f32) -> Color32 {
    rgba(c.r(), c.g(), c.b(), a)
}

/// A chapter's galaxy part of the way to the next: its numbers.
#[derive(Clone, Copy)]
struct Mix {
    core: f32,
    arm: f32,
    war: f32,
    ruin: f32,
    light: f32,
}

impl Mix {
    fn of(a: &Sky, b: &Sky, f: f32) -> Mix {
        let l = |x: f32, y: f32| x + (y - x) * f;
        Mix { core: l(a.core, b.core), arm: l(a.arm, b.arm), war: l(a.war, b.war), ruin: l(a.ruin, b.ruin), light: l(a.light, b.light) }
    }

    /// Where the ARM's space begins, in from the rim (never inside CORE's).
    fn arm_inner(&self) -> f32 {
        (1.0 - self.arm).max(self.core)
    }

    fn span(&self) -> f32 {
        self.arm * TAU
    }

    fn arm_holds(&self, r: f32, a: f32) -> bool {
        self.arm > 0.005 && r >= self.arm_inner() && (a - ARM_FROM).rem_euclid(TAU) <= self.span()
    }

    /// The line the war is fought along: half way across the ground between them, or where
    /// they meet.
    fn front(&self) -> f32 {
        let inner = self.arm_inner();
        if inner > self.core { (self.core + inner) * 0.5 } else { inner }
    }
}

/// Where the galaxy is on the screen.
#[derive(Clone, Copy)]
struct View {
    centre: Pos2,
    radius: f32,
    spin: f32,
    /// How large its marks are (1 for a galaxy 600 points across).
    k: f32,
}

impl View {
    /// The point `r` of the radius out from the middle, `a` round.
    fn at(&self, r: f32, a: f32) -> Pos2 {
        let a = a + self.spin;
        pos2(self.centre.x + self.radius * r * a.cos(), self.centre.y + self.radius * r * a.sin() * TILT)
    }
}

/// The width of the key at the galaxy's right.
const KEY_WIDTH: f32 = 200.0;

/// Paints the galaxy in `rect`, `f` of the way from the chapter whose galaxy is `from` to the
/// one whose is `to`, `time` seconds into the interface's clock; its key at the right.
pub fn paint(painter: &Painter, rect: Rect, from: &Sky, to: &Sky, f: f32, time: f32, planets: &mut Planets) {
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    crate::conquest_ui::paint_space(&painter, rect, time);
    let radius = (rect.width() * 0.47).min(rect.height() * 0.45 / TILT);
    let view = View { centre: rect.center() + vec2(0.0, rect.height() * 0.02), radius, spin: time * SPIN, k: (radius / 300.0).clamp(0.75, 1.6) };
    let m = Mix::of(from, to, f);
    let presence = |mark: Mark| match (from.marks.contains(&mark), to.marks.contains(&mark)) {
        (true, true) => 1.0,
        (true, false) => 1.0 - f,
        (false, true) => f,
        (false, false) => 0.0,
    };
    // A haze over the whole disc, then its owners' space, its stars, the ash of its burned
    // worlds and the light of its middle.
    ellipse_glow(&painter, view.centre, radius * 1.12, radius * 1.12 * TILT, rgba(130, 140, 196, 34.0 * m.light));
    territories(&painter, &view, &m);
    stars(&painter, &view, &m);
    ashes(&painter, &view, &m);
    ellipse_glow(&painter, view.centre, radius * 0.36, radius * 0.36 * TILT * 1.3, rgba(255, 206, 150, 110.0 * m.light));
    ellipse_glow(&painter, view.centre, radius * 0.12, radius * 0.12 * TILT * 1.5, rgba(255, 240, 214, 230.0 * m.light));
    let archive = presence(Mark::Archive);
    let sweep = (time / 7.0).fract() * TAU;
    if archive > 0.0 {
        // The archive: every world read in turn as its sweep goes round.
        polar(&painter, &view, sweep - 1.1, sweep, &[(0.0, 0.0), (0.12, 0.5), (1.0, 0.8), (1.06, 0.0)], faded(AMBER, 70.0 * archive), |u| u * u);
        painter.line_segment([view.at(0.03, sweep), view.at(1.04, sweep)], Stroke::new(1.4 * view.k, faded(AMBER_BRIGHT, 200.0 * archive)));
    }
    worlds(&painter, &view, &m, time, (sweep, archive));
    front(&painter, &view, &m, time);
    if from.ships == to.ships {
        ships(&painter, &view, &m, to.ships, 1.0, time);
    } else {
        ships(&painter, &view, &m, from.ships, 1.0 - f, time);
        ships(&painter, &view, &m, to.ships, f, time);
    }
    let bounds = rect.shrink(6.0);
    for mark in [Mark::Gates, Mark::Contingency, Mark::Krogoth, Mark::Commanders, Mark::Empyrrean, Mark::CorePrime] {
        let alpha = presence(mark);
        if alpha > 0.01 {
            let mut faded = painter.clone();
            faded.set_opacity(painter.opacity() * alpha);
            paint_mark(&faded, &view, bounds, mark, time, planets);
        }
    }
    let key = Rect::from_min_max(pos2(rect.max.x - KEY_WIDTH, rect.min.y), rect.max);
    paint_key(&painter, key, &m, presence(Mark::Commanders));
}

/// A soft glow over an ellipse `rx` by `ry` round `c`: `color` (its alpha the peak) in the
/// middle, nothing at the edge.
fn ellipse_glow(painter: &Painter, c: Pos2, rx: f32, ry: f32, color: Color32) {
    const RINGS: u32 = 10;
    const SEGMENTS: u32 = 72;
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, color);
    for i in 1..=RINGS {
        let s = i as f32 / RINGS as f32;
        let fade = (1.0 - s * s).powi(2);
        for k in 0..SEGMENTS {
            let angle = k as f32 / SEGMENTS as f32 * TAU;
            mesh.colored_vertex(c + vec2(angle.cos() * rx * s, angle.sin() * ry * s), rgba(r, g, b, a as f32 * fade));
        }
    }
    let at = |ring: u32, k: u32| 1 + ring * SEGMENTS + k % SEGMENTS;
    for k in 0..SEGMENTS {
        mesh.add_triangle(0, at(0, k), at(0, k + 1));
    }
    for ring in 0..RINGS - 1 {
        for k in 0..SEGMENTS {
            mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
            mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A band of the disc from `a0` round to `a1`, its `rings` (how far out, how strong) from the
/// inside out, in `color` (its alpha the strongest), `along` its strength from one end to the
/// other (0 to 1).
fn polar(painter: &Painter, view: &View, a0: f32, a1: f32, rings: &[(f32, f32)], color: Color32, along: impl Fn(f32) -> f32) {
    if rings.len() < 2 || a1 <= a0 {
        return;
    }
    let segments = ((a1 - a0) / (TAU / 120.0)).ceil().max(2.0) as u32;
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let mut mesh = Mesh::default();
    for &(out, strength) in rings {
        for k in 0..=segments {
            let u = k as f32 / segments as f32;
            mesh.colored_vertex(view.at(out, a0 + (a1 - a0) * u), rgba(r, g, b, a as f32 * strength * along(u)));
        }
    }
    let row = segments + 1;
    for ring in 0..rings.len() as u32 - 1 {
        for k in 0..segments {
            let (p, q) = (ring * row + k, (ring + 1) * row + k);
            mesh.add_triangle(p, q, q + 1);
            mesh.add_triangle(p, q + 1, p + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// An arc of the disc `out` from the middle, from `a0` round to `a1`.
fn arc(view: &View, out: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let segments = ((a1 - a0).abs() / (TAU / 160.0)).ceil().max(2.0) as u32;
    (0..=segments).map(|k| view.at(out, a0 + (a1 - a0) * k as f32 / segments as f32)).collect()
}

/// CORE's space round the middle, brighter towards its border; the ARM's share of the rim,
/// brightest along its front.
fn territories(painter: &Painter, view: &View, m: &Mix) {
    let c = m.core;
    polar(painter, view, 0.0, TAU, &[(0.0, 0.4), (c * 0.65, 0.45), (c * 0.9, 0.65), (c, 0.8), (c + 0.03, 0.3), (c + 0.08, 0.0)], faded(CORE_RED, 46.0), |_| 1.0);
    painter.extend(Shape::dashed_line(&arc(view, c, 0.0, TAU), Stroke::new(1.0, faded(CORE_RED, 90.0)), 6.0, 5.0));
    if m.arm > 0.005 {
        let inner = m.arm_inner();
        let span = m.span();
        let mid = (inner + 0.03 + 1.0) * 0.5;
        // (Its ends fade over a few degrees: the ARM holds no straight-edged wedge.)
        let ends = (0.12 / span.max(0.12)).min(0.4);
        let along = |u: f32| smooth(0.0, ends, u) * smooth(1.0, 1.0 - ends, u);
        polar(painter, view, ARM_FROM, ARM_FROM + span, &[(inner - 0.012, 0.0), (inner + 0.02, 1.0), (mid, 0.65), (1.0, 0.55), (1.08, 0.0)], faded(ARM_BLUE, 92.0), along);
        let edge = arc(view, inner, ARM_FROM + span * ends * 0.5, ARM_FROM + span * (1.0 - ends * 0.5));
        painter.add(Shape::line(edge, Stroke::new(1.4, faded(ARM_BLUE, 190.0))));
    }
}

/// Where a star of an arm lies: `r` out, wound round from the arm's start, spread about it.
fn on_arm(arm: u32, r: f32, spread: f32) -> f32 {
    arm as f32 * PI + r * WIND + spread
}

/// The clouds of the arms: soft glows of blue, rose and violet along them, under the stars.
fn nebulae(painter: &Painter, view: &View, m: &Mix) {
    for j in 0..NEBULAE {
        let h = |k: u32| hash(31_000 + j * 11 + k);
        let r = 0.18 + 0.78 * h(1);
        let a = on_arm(j % 2, r, (h(2) - 0.5) * 0.3);
        let [cr, cg, cb] = match (h(3) * 3.0) as u32 {
            0 => [90, 130, 255],
            1 => [255, 104, 168],
            _ => [150, 110, 255],
        };
        let reach = (10.0 + 22.0 * h(4)) * view.k;
        w::soft_glow(painter, view.at(r, a), 0.0, reach, rgba(cr, cg, cb, (26.0 + 34.0 * h(5)) * m.light));
    }
}

/// The stars: a bright middle, a dim disc, and two arms wound round them, bright with young
/// stars; a few of the brightest glinting.
fn stars(painter: &Painter, view: &View, m: &Mix) {
    nebulae(painter, view, m);
    let mut mesh = Mesh::default();
    mesh.reserve_vertices(STARS as usize * 4);
    mesh.reserve_triangles(STARS as usize * 2);
    let size = (view.radius / 320.0).clamp(0.8, 1.5);
    let mut glints = Vec::new();
    for i in 0..STARS {
        let h = |k: u32| hash(i.wrapping_mul(13).wrapping_add(k));
        let (r, a, [cr, cg, cb], bright) = match i % 10 {
            0 | 1 => (0.22 * h(1).powf(1.6), TAU * h(2), [255, 222, 178], 1.0),
            2 | 3 => (0.08 + 0.92 * h(1).sqrt(), TAU * h(2), [210, 214, 236], 0.55),
            _ => {
                let r = 0.08 + 0.92 * h(1).powf(0.9);
                let spread = (h(2) + h(3) + h(4) - 1.5) * 0.26 * (1.0 - 0.3 * r);
                let color = if h(5) < 0.07 {
                    [255, 172, 198]
                } else if h(5) < 0.55 {
                    [184, 208, 255]
                } else {
                    [230, 234, 252]
                };
                (r, on_arm(i % 2, r, spread), color, 1.0)
            }
        };
        let p = view.at(r, a);
        let half = (0.35 + h(6).powi(4) * 0.95) * size;
        let alpha = (50.0 + 190.0 * h(7)) * bright * m.light * (1.0 - 0.55 * smooth(0.84, 1.0, r));
        let color = rgba(cr, cg, cb, alpha);
        let base = mesh.vertices.len() as u32;
        for d in [vec2(-half, -half), vec2(half, -half), vec2(half, half), vec2(-half, half)] {
            mesh.colored_vertex(p + d, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
        if i % 41 == 0 && h(8) > 0.4 {
            glints.push((p, (2.5 + 3.0 * h(9)) * size, rgba(cr, cg, cb, 150.0 * m.light)));
        }
    }
    painter.add(Shape::mesh(mesh));
    for (p, reach, color) in glints {
        painter.line_segment([p - vec2(reach, 0.0), p + vec2(reach, 0.0)], Stroke::new(0.8, color));
        painter.line_segment([p - vec2(0.0, reach), p + vec2(0.0, reach)], Stroke::new(0.8, color));
    }
}

/// A world of the war: where it is, and the share of the galaxy burned before it burns.
fn world(i: u32) -> (f32, f32, f32) {
    (0.16 + 0.8 * hash(9000 + i * 5), TAU * hash(9001 + i * 5), hash(9002 + i * 5))
}

/// The burned worlds' ash: dark smudges over the stars round them.
fn ashes(painter: &Painter, view: &View, m: &Mix) {
    for i in 0..WORLDS {
        let (r, a, burns) = world(i);
        if burns < m.ruin {
            let strength = smooth(burns, burns + 0.06, m.ruin);
            w::soft_glow(painter, view.at(r, a), 0.0, 22.0 * view.k, Color32::from_black_alpha((120.0 * strength) as u8));
        }
    }
}

/// The worlds: each a point ringed in its owner's colour (CORE's, the ARM's, nobody's), the
/// burned ones embers; read by the archive's sweep as it passes (`sweep`: where it is, how
/// much it shows).
fn worlds(painter: &Painter, view: &View, m: &Mix, time: f32, sweep: (f32, f32)) {
    for i in 0..WORLDS {
        let (r, a, burns) = world(i);
        let p = view.at(r, a);
        if burns < m.ruin {
            let glow = 0.5 + 0.5 * (time * (0.6 + hash(9003 + i * 5)) + i as f32).sin();
            painter.circle_filled(p, 1.7 * view.k, rgba(110 + (90.0 * glow) as u8, 40, 18, 230.0));
            continue;
        }
        let [cr, cg, cb] = if m.arm_holds(r, a) {
            [166, 210, 255]
        } else if r <= m.core {
            [255, 186, 156]
        } else {
            [180, 180, 186]
        };
        painter.circle_filled(p, 1.6 * view.k, rgba(cr, cg, cb, 225.0));
        painter.add(Shape::ellipse_stroke(p, vec2(3.4, 3.4 * TILT * 1.6) * view.k, Stroke::new(0.7, rgba(cr, cg, cb, 70.0))));
        let (at, shown) = sweep;
        if shown > 0.0 {
            let behind = (at - a).rem_euclid(TAU);
            let lit = shown * (1.0 - behind / 1.4).max(0.0);
            if lit > 0.0 {
                w::soft_glow(painter, p, 0.0, 9.0 * view.k, faded(AMBER_BRIGHT, 200.0 * lit));
            }
        }
    }
}

/// The front: a faint fire along it, battles flaring and dying up and down it.
fn front(painter: &Painter, view: &View, m: &Mix, time: f32) {
    if m.war < 0.01 || m.arm < 0.01 {
        return;
    }
    let span = m.span();
    let line = m.front();
    let depth = (m.arm_inner() - m.core).max(0.0) + 0.05;
    painter.add(Shape::line(arc(view, line, ARM_FROM + span * 0.04, ARM_FROM + span * 0.96), Stroke::new(1.6 * view.k, faded(FIRE, 80.0 * m.war))));
    let battles = (m.war * 34.0) as u32;
    for i in 0..battles {
        let rate = 0.35 + 0.5 * hash(7000 + i * 3);
        let clock = time * rate + hash(7001 + i * 3) * 10.0;
        let phase = clock.fract();
        if phase > 0.4 {
            continue;
        }
        // (Each flares somewhere new each time.)
        let cycle = clock.floor() as u32;
        let a = ARM_FROM + span * (0.04 + 0.92 * hash(7002u32.wrapping_add(i * 3).wrapping_add(cycle.wrapping_mul(101))));
        let r = line + (hash(7003u32.wrapping_add(i * 7).wrapping_add(cycle.wrapping_mul(31))) - 0.5) * depth;
        let life = phase / 0.4;
        let p = view.at(r, a);
        let strength = (1.0 - life) * m.war.min(1.0);
        w::soft_glow(painter, p, 0.0, (4.0 + 12.0 * life) * view.k, faded(FIRE, 210.0 * strength));
        painter.circle_filled(p, 1.6 * view.k * (1.0 - life * 0.5), rgba(255, 244, 214, 255.0 * strength));
    }
}

/// A ship's streak from `tail` to `head`: nothing at its tail, `color` at its head.
fn streak(mesh: &mut Mesh, tail: Pos2, head: Pos2, width: f32, color: Color32) {
    let along = head - tail;
    if along.length_sq() < 0.01 {
        return;
    }
    let side = along.normalized().rot90() * width * 0.5;
    let base = mesh.vertices.len() as u32;
    let clear = faded(color, 0.0);
    mesh.colored_vertex(tail - side * 0.3, clear);
    mesh.colored_vertex(tail + side * 0.3, clear);
    mesh.colored_vertex(head + side, color);
    mesh.colored_vertex(head - side, color);
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

/// The fleets under way: CORE's collection ships out from the middle; the refusers out to the
/// rim; both sides' fleets to and fro across the front.
fn ships(painter: &Painter, view: &View, m: &Mix, kind: Ships, alpha: f32, time: f32) {
    if alpha <= 0.01 {
        return;
    }
    // In and out at the ends of their runs.
    let ends = |u: f32| smooth(0.0, 0.08, u) * (1.0 - smooth(0.86, 1.0, u)) * alpha;
    let mut mesh = Mesh::default();
    let width = 1.6 * view.k;
    match kind {
        Ships::None => {}
        Ships::Collection => {
            for i in 0..26 {
                let a = TAU * hash(5000 + i);
                let u = (time * (0.035 + 0.025 * hash(5100 + i)) + hash(5200 + i)).fract();
                let r = 0.06 + u * 0.9;
                streak(&mut mesh, view.at(r - 0.06, a), view.at(r, a), width, rgba(255, 150, 104, 230.0 * ends(u)));
            }
        }
        Ships::Fleeing => {
            for i in 0..30 {
                let (r0, a0) = (0.2 + 0.45 * hash(5300 + i), TAU * hash(5400 + i));
                let a1 = ARM_FROM + m.span() * (0.1 + 0.8 * hash(5500 + i));
                // (The short way round.)
                let turn = (a1 - a0 + PI).rem_euclid(TAU) - PI;
                let place = |u: f32| view.at(r0 + (0.94 - r0) * u, a0 + turn * u);
                let u = (time * (0.03 + 0.02 * hash(5600 + i)) + hash(5700 + i)).fract();
                streak(&mut mesh, place((u - 0.07).max(0.0)), place(u), width, rgba(206, 226, 255, 230.0 * ends(u)));
            }
        }
        Ships::War => {
            let line = m.front();
            for i in 0..36 {
                let core = i % 2 == 0;
                let a = ARM_FROM + m.span() * (0.06 + 0.88 * hash(5800 + i));
                let (r0, color) = if core { ((m.core - 0.24).max(0.05), [255, 140, 96]) } else { ((m.arm_inner() + 0.24).min(1.02), [150, 196, 255]) };
                let u = (time * (0.07 + 0.05 * hash(5900 + i)) + hash(6000 + i)).fract();
                let r = r0 + (line - r0) * u;
                let tail = r0 + (line - r0) * (u - 0.1).max(0.0);
                let [cr, cg, cb] = color;
                streak(&mut mesh, view.at(tail, a), view.at(r, a), width, rgba(cr, cg, cb, 230.0 * ends(u)));
            }
        }
    }
    if !mesh.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}

/// An ellipse lying in the galaxy's plane, `r` across, round `p`.
fn flat_ring(painter: &Painter, p: Pos2, r: f32, stroke: Stroke) {
    painter.add(Shape::ellipse_stroke(p, vec2(r, r * TILT * 1.4), stroke));
}

/// One of the chapter's marks.
fn paint_mark(painter: &Painter, view: &View, bounds: Rect, mark: Mark, time: f32, planets: &mut Planets) {
    let k = view.k;
    let mut planet = |p: Pos2, r: f32, kind: &str, name: &str| {
        planets.paint(painter, p, r, kind, crate::planet::seed_of(name), || crate::conquest_ui::paint_planet(painter, p, r, crate::conquest_ui::planet_color(kind)));
    };
    match mark {
        Mark::CorePrime => {
            let p = view.at(CORE_PRIME.0, CORE_PRIME.1);
            let pulse = 0.5 + 0.5 * (time * 1.6).sin();
            w::soft_glow(painter, p, 0.0, 26.0 * k, faded(CORE_RED, 70.0 + 50.0 * pulse));
            planet(p, 8.0 * k, "metal", "core prime");
            flat_ring(painter, p, (13.0 + 3.0 * pulse) * k, Stroke::new(1.4, faded(CORE_RED, 220.0)));
            label(painter, bounds, p, 14.0 * k, t("CORE PRIME"), CORE_RED, k);
        }
        Mark::Empyrrean => {
            let p = view.at(EMPYRREAN.0, EMPYRREAN.1);
            let pulse = 0.5 + 0.5 * (time * 1.3 + 1.0).sin();
            w::soft_glow(painter, p, 0.0, 22.0 * k, faded(ARM_BLUE, 60.0 + 50.0 * pulse));
            planet(p, 7.0 * k, "green", "empyrrean");
            flat_ring(painter, p, (11.5 + 2.5 * pulse) * k, Stroke::new(1.4, faded(ARM_BLUE, 230.0)));
            label(painter, bounds, p, 12.0 * k, t("EMPYRREAN"), ARM_BLUE, k);
        }
        Mark::Gates => {
            let places: Vec<Pos2> = GATES.iter().map(|&(r, a)| view.at(r, a)).collect();
            for pair in places.windows(2) {
                painter.extend(Shape::dashed_line(pair, Stroke::new(1.0, faded(AMBER, 120.0)), 5.0 * k, 4.0 * k));
            }
            for (i, &p) in places.iter().enumerate() {
                let r = 5.0 * k;
                let pulse = 0.5 + 0.5 * (time * 2.2 + i as f32 * 1.7).sin();
                w::soft_glow(painter, p, 0.0, r * 2.4, rgba(150, 210, 255, 110.0 + 90.0 * pulse));
                painter.circle_filled(p, r + 1.5 * k, Color32::from_black_alpha(160));
                // A ring of brass in two turning halves round its bright eye.
                for half in 0..2 {
                    let a0 = time * 0.9 + half as f32 * PI + 0.3;
                    let points: Vec<Pos2> = (0..=10).map(|s| {
                        let b = a0 + (PI - 0.6) * s as f32 / 10.0;
                        p + vec2(b.cos(), b.sin()) * r
                    }).collect();
                    painter.add(Shape::line(points, Stroke::new(1.7 * k, AMBER)));
                }
                painter.circle_filled(p, 1.7 * k, rgba(226, 244, 255, 255.0));
            }
            label(painter, bounds, places[1], 7.0 * k, t("THE GALACTIC GATES"), AMBER_BRIGHT, k);
        }
        Mark::Contingency => {
            for (i, &(r, a, name, kind)) in CONTINGENCY.iter().enumerate() {
                let p = view.at(r, a);
                let pulse = 0.5 + 0.5 * (time * 1.5 + i as f32 * 2.1).sin();
                w::soft_glow(painter, p, 0.0, 18.0 * k, faded(AMBER, 40.0 + 40.0 * pulse));
                planet(p, 6.5 * k, kind, name);
                painter.extend(Shape::dashed_line(&ellipse_points(p, 11.0 * k), Stroke::new(1.1, faded(AMBER, 200.0)), 3.0 * k, 3.0 * k));
                label(painter, bounds, p, 11.0 * k, t(name), AMBER_BRIGHT, k);
            }
        }
        Mark::Krogoth => {
            let p = view.at(KROGOTH.0, KROGOTH.1);
            // The ground shaking as it walks: rings running out from it.
            for j in 0..3 {
                let phase = (time / 1.8 + j as f32 / 3.0).fract();
                flat_ring(painter, p, (8.0 + 40.0 * phase) * k, Stroke::new(1.5 * (1.0 - phase), faded(RED, 200.0 * (1.0 - phase).powf(1.5))));
            }
            w::soft_glow(painter, p, 0.0, 20.0 * k, faded(RED, 90.0));
            planet(p, 6.5 * k, "lava", "krogoth");
            reticle(painter, p, 12.0 * k, k, time, RED);
            label(painter, bounds, p, 14.0 * k, t("THE KROGOTH"), RED, k);
        }
        Mark::Commanders => {
            for (i, &(r, a)) in COMMANDERS.iter().enumerate() {
                let p = view.at(r, a);
                // Each drops onto its world from orbit, and lands in a flash of its nanolathe.
                let cycle = 5.5;
                let s = (time + i as f32 * 0.69).rem_euclid(cycle);
                let d = 3.6 * k;
                if s < 0.6 {
                    let u = s / 0.6;
                    let at = p - vec2(0.0, 48.0 * k * (1.0 - u).powi(2));
                    painter.line_segment([at - vec2(0.0, 14.0 * k), at], Stroke::new(1.2 * k, faded(AMBER_BRIGHT, 160.0 * u)));
                    diamond(painter, at, d, u);
                } else {
                    let u = (s - 0.6) / 1.1;
                    if u < 1.0 {
                        flat_ring(painter, p, (4.0 + 18.0 * u) * k, Stroke::new(1.4, faded(AMBER_BRIGHT, 230.0 * (1.0 - u))));
                        w::soft_glow(painter, p, 0.0, 12.0 * k, faded(AMBER_BRIGHT, 160.0 * (1.0 - u)));
                    }
                    diamond(painter, p, d, 1.0);
                }
            }
        }
        Mark::Archive => {}
    }
}

/// Points round an ellipse lying in the galaxy's plane (for a dashed ring).
fn ellipse_points(p: Pos2, r: f32) -> Vec<Pos2> {
    (0..=40).map(|s| {
        let a = s as f32 / 40.0 * TAU;
        p + vec2(a.cos() * r, a.sin() * r * TILT * 1.4)
    }).collect()
}

/// A Commander: a small brass diamond edged dark.
fn diamond(painter: &Painter, c: Pos2, d: f32, alpha: f32) {
    let points = vec![c + vec2(0.0, -d * 1.3), c + vec2(d, 0.0), c + vec2(0.0, d * 1.3), c + vec2(-d, 0.0)];
    painter.add(Shape::convex_polygon(points, faded(AMBER_BRIGHT, 255.0 * alpha), Stroke::new(1.0, Color32::from_black_alpha((200.0 * alpha) as u8))));
}

/// Four brackets turning slowly round `p`.
fn reticle(painter: &Painter, p: Pos2, reach: f32, k: f32, time: f32, color: Color32) {
    for q in 0..4 {
        let a = time * 0.6 + q as f32 * TAU / 4.0;
        let points: Vec<Pos2> = (0..=6).map(|s| {
            let b = a - 0.3 + 0.6 * s as f32 / 6.0;
            p + vec2(b.cos(), b.sin()) * reach
        }).collect();
        painter.add(Shape::line(points, Stroke::new(2.0 * k.sqrt(), color)));
    }
}

/// A name beside what it names (at `p`, `r` round): lettered on a dark plate, up and to the
/// right of it, or on whichever other side keeps it inside `bounds`, a line out to it.
fn label(painter: &Painter, bounds: Rect, p: Pos2, r: f32, text: &str, color: Color32, k: f32) {
    let job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: strong(11.5 * k.sqrt()), color, extra_letter_spacing: tracking(1.4), ..Default::default() });
    let galley = painter.layout_job(job);
    let size = galley.size();
    let gap = r + 8.0 * k;
    let sides = [vec2(1.0, -1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)];
    let place = |side: egui::Vec2| {
        let corner = p + side * gap * std::f32::consts::FRAC_1_SQRT_2;
        let min = pos2(if side.x > 0.0 { corner.x } else { corner.x - size.x }, if side.y > 0.0 { corner.y } else { corner.y - size.y });
        (Rect::from_min_size(min, size), corner)
    };
    let (rect, corner) = sides.iter().map(|&side| place(side)).find(|(rect, _)| bounds.contains_rect(rect.expand(5.0))).unwrap_or_else(|| place(sides[0]));
    let plate = rect.expand2(vec2(5.0, 2.0));
    painter.line_segment([p + (corner - p).normalized() * r, corner], Stroke::new(1.0, faded(color, 150.0)));
    painter.rect_filled(plate, 2.0, Color32::from_black_alpha(170));
    painter.rect_stroke(plate, 2.0, Stroke::new(1.0, faded(color, 90.0)), egui::StrokeKind::Inside);
    painter.galley(rect.min, galley, color);
}

/// The key beside the galaxy: what its colours and marks are, those it shows now.
fn paint_key(painter: &Painter, rect: Rect, m: &Mix, commanders: f32) {
    let entries: [(&str, f32, u8); 5] = [
        (n("CORE"), 1.0, 0),
        (n("THE ARM"), smooth(0.0, 0.05, m.arm), 1),
        (n("THE FRONT"), smooth(0.0, 0.1, m.war), 2),
        (n("BURNED WORLDS"), smooth(0.0, 0.05, m.ruin), 3),
        (n("COMMANDERS"), commanders, 4),
    ];
    let mut y = rect.max.y - 10.0;
    for &(name, shown, swatch) in entries.iter().rev() {
        if shown < 0.01 {
            continue;
        }
        let mut faded_painter = painter.clone();
        faded_painter.set_opacity(painter.opacity() * shown);
        let p = &faded_painter;
        let galley = p.layout_job(LayoutJob::single_section(t(name).to_string(), TextFormat { font_id: strong(11.0), color: TEXT, extra_letter_spacing: tracking(1.2), ..Default::default() }));
        let row = Rect::from_min_size(pos2(rect.min.x + 12.0, y - galley.size().y), vec2(rect.width() - 24.0, galley.size().y));
        let c = pos2(row.min.x + 7.0, row.center().y);
        match swatch {
            0 => {
                p.circle_filled(c, 6.0, faded(CORE_RED, 110.0));
                p.circle_stroke(c, 6.0, Stroke::new(1.0, CORE_RED));
            }
            1 => {
                p.circle_filled(c, 6.0, faded(ARM_BLUE, 110.0));
                p.circle_stroke(c, 6.0, Stroke::new(1.0, ARM_BLUE));
            }
            2 => {
                w::soft_glow(p, c, 0.0, 8.0, faded(FIRE, 220.0));
                p.circle_filled(c, 2.0, rgba(255, 244, 214, 255.0));
            }
            3 => {
                p.circle_filled(c, 6.0, Color32::from_black_alpha(200));
                p.circle_filled(c, 2.0, rgba(190, 52, 22, 255.0));
            }
            _ => diamond(p, c, 4.0, 1.0),
        }
        p.galley(pos2(row.min.x + 20.0, row.min.y), galley, TEXT);
        y = row.min.y - 7.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lore::CHAPTERS;

    fn mix(sky: &Sky) -> Mix {
        Mix::of(sky, sky, 1.0)
    }

    #[test]
    fn the_arm_holds_the_rim_and_never_inside_core() {
        for c in &CHAPTERS {
            let m = mix(&c.sky);
            assert!(m.arm_inner() >= m.core, "{}", c.title);
            if m.arm > 0.0 {
                // Empyrrean is the ARM's from the day they reach it.
                assert!(m.arm_holds(EMPYRREAN.0, EMPYRREAN.1), "{}", c.title);
                assert!(!m.arm_holds(0.1, EMPYRREAN.1), "{}", c.title);
            } else {
                assert!(!m.arm_holds(0.99, EMPYRREAN.1), "{}", c.title);
            }
        }
    }

    #[test]
    fn the_front_lies_between_them() {
        for c in &CHAPTERS {
            let m = mix(&c.sky);
            assert!(m.front() >= m.core - 1e-6 && m.front() <= m.arm_inner() + 1e-6, "{}", c.title);
        }
    }

    #[test]
    fn the_chapters_ease_into_each_other() {
        let (a, b) = (&CHAPTERS[0].sky, &CHAPTERS[6].sky);
        let half = Mix::of(a, b, 0.5);
        assert!((half.core - (a.core + b.core) / 2.0).abs() < 1e-6);
        assert!((Mix::of(a, b, 0.0).ruin - a.ruin).abs() < 1e-6 && (Mix::of(a, b, 1.0).ruin - b.ruin).abs() < 1e-6);
    }

    #[test]
    fn hashes_spread_over_zero_to_one() {
        let values: Vec<f32> = (0..2000).map(hash).collect();
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.03, "{mean}");
    }
}
