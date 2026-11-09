//! How the radial menu is painted (its shapes are `shape.rs`; where things go and when is
//! `hud/radial.rs`). Every plate is a bevelled piece of gunmetal lit from the top left, as the
//! rest of the interface is: a face shaded like a dome, a grain laid in it, a lit edge above and
//! a shaded one below, a shadow on what lies under it. What is set is lit amber along its inner
//! edge, what is under the pointer is brass and risen, what cannot be taken back is rust, what
//! would do nothing is dull. Round it runs a steel rim with a rivet between the plates and a lamp
//! where the pointer is; in the middle sits the unit on its seat, with its health round it.

use std::sync::Arc;

use egui::{Align2, Color32, Context, Galley, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2 as EVec2, pos2, vec2};
use ta_ui::metal;

use super::shape::{self, Sector, facing, polar};
use crate::hud::UnitPic;
use crate::hud_icons::{self, Glyph};
use crate::radial::{self, Entry, Id, Tone};
use crate::theme::*;

/// The colours of metal and energy, as the deck's costs have them.
const METAL: Color32 = Color32::from_rgb(170, 184, 196);
const ENERGY: Color32 = Color32::from_rgb(242, 212, 60);

/// Where the ring is and how big, this frame: what every plate is measured from.
#[derive(Clone, Copy, Debug)]
pub struct Space {
    pub centre: Pos2,
    /// The interface's size for the ring (1 at 720 points high, up to 1.5).
    pub scale: f32,
    /// The radii of its inner and outer edge as drawn now (they swell as it opens).
    pub r0: f32,
    pub r1: f32,
    /// How opaque the whole of it is (0 to 1).
    pub alpha: f32,
    /// Reduced flashes: no bright pulses, a dimmer glow.
    pub calm: bool,
}

impl Space {
    /// How far from the middle the menu's parts may go: the room the menu asks of the screen (the
    /// middle is kept this far from the edges of the battlefield).
    pub fn reach(&self) -> f32 {
        (radial::OUTER + radial::MARGIN) * self.scale
    }

    /// A length in points at this size.
    fn pt(&self, v: f32) -> f32 {
        v * self.scale
    }

    /// The size of lettering that grows with the ring, but more slowly than it does.
    fn text(&self, v: f32) -> f32 {
        v * self.scale.sqrt()
    }
}

/// How one plate is drawn this frame: the values that move (see `radial_motion`).
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// How lit it is: under the pointer or the stick, or the choice just made (0 to 1; a little
    /// over while the light swings past, when the plate rises a little more than it lights).
    pub hot: f32,
    /// How far it is pressed (0 to 1): the flash on it (which reduced flashes leaves out) ...
    pub press: f32,
    /// ... and how far it sinks (0 to 1; none with reduced motion).
    pub sink: f32,
    /// How refused it is (0 to 1): tinted rust for a moment, whether or not it shakes.
    pub warn: f32,
    /// How opaque it is (0 to 1).
    pub alpha: f32,
    /// How far out from the middle it has come: 1 in place, less near the hub.
    pub grow: f32,
    /// How far it has turned from its place (radians, clockwise).
    pub swing: f32,
    /// How far it is pushed out (points at the smallest size) besides what the pointer's light gives.
    pub lift: f32,
    /// How far it is shaken to the side (points): a choice that cannot be made.
    pub nudge: f32,
    /// How far its count has popped (0 to 1): a unit queued.
    pub pop: f32,
}

impl Look {
    pub const REST: Look = Look { hot: 0.0, press: 0.0, sink: 0.0, warn: 0.0, alpha: 1.0, grow: 1.0, swing: 0.0, lift: 0.0, nudge: 0.0, pop: 0.0 };
}

/// The colours a plate is made in: its faces from the hub outwards, unlit and lit, and its inks.
struct Skin {
    inner: [f32; 3],
    outer: [f32; 3],
    lit_inner: [f32; 3],
    lit_outer: [f32; 3],
    /// The glow round it when it is lit.
    glow: [f32; 3],
    /// The line round it, unlit and lit.
    edge: Color32,
    edge_lit: Color32,
    /// The lettering on it, unlit and lit (dark on the brass, light on the rust).
    ink: Color32,
    ink_lit: Color32,
    /// Whether the lit face is bright enough for dark lettering.
    dark_ink_lit: bool,
}

impl Skin {
    fn of(e: &Entry, depth: usize) -> Skin {
        // (A ring opened from another is a little cooler and darker than the first: one level in.)
        let deep = 1.0 - 0.1 * depth.min(2) as f32;
        let tint = |c: [f32; 3]| [c[0] * deep, c[1] * deep, (c[2] * deep + 2.0 * depth.min(2) as f32).min(255.0)];
        if e.disabled {
            Skin { inner: [26.0, 26.0, 25.0], outer: [40.0, 40.0, 38.0], lit_inner: [34.0, 34.0, 32.0], lit_outer: [50.0, 50.0, 47.0], glow: [90.0, 90.0, 84.0], edge: Color32::from_rgb(34, 33, 30), edge_lit: Color32::from_rgb(74, 72, 66), ink: Color32::from_rgb(122, 120, 110), ink_lit: Color32::from_rgb(150, 147, 134), dark_ink_lit: false }
        } else if e.tone == Tone::Danger {
            Skin {
                inner: [68.0, 27.0, 21.0],
                outer: [114.0, 46.0, 35.0],
                lit_inner: [134.0, 48.0, 36.0],
                lit_outer: [204.0, 80.0, 60.0],
                glow: [240.0, 90.0, 64.0],
                edge: Color32::from_rgb(40, 14, 10),
                edge_lit: Color32::from_rgb(255, 150, 120),
                ink: Color32::from_rgb(250, 226, 212),
                ink_lit: Color32::from_rgb(255, 244, 236),
                dark_ink_lit: false,
            }
        } else {
            Skin {
                inner: tint([24.0, 24.0, 22.0]),
                outer: tint([76.0, 76.0, 69.0]),
                lit_inner: [108.0, 78.0, 26.0],
                lit_outer: [234.0, 192.0, 98.0],
                glow: [242.0, 200.0, 96.0],
                edge: Color32::from_rgb(14, 13, 11),
                edge_lit: AMBER_BRIGHT,
                ink: CREAM,
                ink_lit: Color32::from_rgb(34, 24, 8),
                dark_ink_lit: true,
            }
        }
    }

    /// The colour of the face at a point `v` of the way out, with `hot` of the pointer's light, and the dome's light on it.
    fn face(&self, v: f32, hot: f32, dome: f32, set: bool) -> [f32; 3] {
        let t = v.clamp(0.0, 1.0).powf(1.25);
        let mut c = [0.0; 3];
        for k in 0..3 {
            let unlit = self.inner[k] + (self.outer[k] - self.inner[k]) * t;
            let lit = self.lit_inner[k] + (self.lit_outer[k] - self.lit_inner[k]) * t;
            c[k] = unlit + (lit - unlit) * hot;
        }
        if set {
            // (What is switched on or set is a little warmer all over.)
            c[0] += 9.0 * (1.0 - hot);
            c[1] += 5.0 * (1.0 - hot);
        }
        // The dome: the side towards the light brighter, the far side darker, less where it is bright already.
        let amp = 15.0 * (1.0 - 0.3 * hot);
        for k in c.iter_mut() {
            *k = (*k + dome * amp).clamp(0.0, 255.0);
        }
        c
    }
}

fn rgb(c: [f32; 3], alpha: f32) -> Color32 {
    Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8).gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// Light that adds to what is under it (the glow round a lit plate).
fn glow(c: [f32; 3], strength: f32) -> Color32 {
    let k = strength.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied((c[0] * k) as u8, (c[1] * k) as u8, (c[2] * k) as u8, 0)
}

/// `c` with its alpha scaled by `k`.
fn fade(c: Color32, k: f32) -> Color32 {
    c.gamma_multiply(k.clamp(0.0, 1.0))
}

// ---- The plates ------------------------------------------------------------------------------

/// One plate of the ring, with its glyph or picture, its words, its badges and its key.
#[allow(clippy::too_many_arguments)]
pub fn wedge(p: &Painter, ctx: &Context, sp: &Space, i: usize, count: usize, e: &Entry, pic: Option<UnitPic>, look: &Look, depth: usize) {
    let alpha = sp.alpha * look.alpha;
    if alpha <= 0.01 {
        return;
    }
    let skin = Skin::of(e, depth);
    // Where it stands: its sector turned by what it is doing, risen by the light on it.
    let a = radial::angle(i, count) + look.swing;
    let width = std::f32::consts::TAU / count.max(1) as f32;
    let (a0, a1) = if count == 1 { (a - 3.0, a + 3.0) } else { (a - width / 2.0, a + width / 2.0) };
    let strength = if e.disabled { 0.25 } else { 1.0 };
    let hot = look.hot.clamp(0.0, 1.0) * strength;
    // It rises towards the viewer as the light comes on: larger all round (and over its
    // neighbours' edges), its shadow further off; pressed, it goes back down.
    let rise = sp.pt(5.0) * look.hot.clamp(0.0, 1.3) * strength + sp.pt(look.lift) - sp.pt(2.5) * look.sink;
    let offset = vec2(look.nudge * sp.scale, 0.0);
    let centre = sp.centre + offset;
    let (r0, r1) = (sp.r0 * look.grow - 0.45 * rise, sp.r1 * look.grow + 0.55 * rise);
    let sector = Sector { centre, r0, r1, a0, a1, gap: (sp.pt(3.2) - 0.9 * rise).max(-sp.pt(4.0)), corner: sp.pt(4.5) };
    let tess = sector.tessellate();

    // The shadow it casts on what is under it: further off the more it has risen.
    let off = vec2(1.4, 2.6) * sp.scale * (1.0 + 1.4 * hot);
    let under = Sector { centre: centre + off, ..sector }.tessellate();
    let shade = (if e.disabled { 0.30 } else { 0.50 }) * alpha;
    let mut mesh = under.fill(|_, _, _| Color32::from_black_alpha((255.0 * shade) as u8));
    mesh.append(under.strip((0.0, sp.pt(4.0 + 5.0 * hot)), |_, _| (Color32::from_black_alpha((255.0 * shade * 0.8) as u8), Color32::TRANSPARENT)));
    p.add(Shape::mesh(mesh));

    // The pointer's light, round it, before the plate goes over it.
    if hot > 0.01 {
        let k = hot * if sp.calm { 0.38 } else { 0.62 } * alpha;
        p.add(Shape::mesh(tess.strip((sp.pt(-1.0), sp.pt(9.0)), |_, n| (glow(skin.glow, k * (0.55 + 0.45 * facing(n).max(0.0) + 0.2)), glow(skin.glow, 0.0)))));
    }

    // Its face: shaded like a dome from the hub outwards and from the light across.
    let body = tess.fill(|at, _, v| {
        let d = (at - sp.centre).normalized();
        rgb(skin.face(v, hot, facing(d), e.lit), alpha * 0.97)
    });
    p.add(Shape::mesh(body));
    // The grain in the metal, laid in the plate's own shape (faint where it is lit).
    let grain = ((1.0 - 0.55 * hot) * if e.disabled { 0.5 } else { 0.9 }) * alpha;
    let a8 = (grain.clamp(0.0, 1.0) * 255.0) as u8;
    p.add(Shape::mesh(tess.textured(metal::grime(ctx), 256.0, Color32::from_rgba_premultiplied(a8, a8, a8, a8))));

    // What is set: a lit line along its inner edge.
    if e.lit && !e.disabled {
        let inner = Sector { r1: r0 + sp.pt(2.4), corner: 0.0, ..sector }.tessellate();
        let lamp = [150.0, 230.0, 110.0];
        p.add(Shape::mesh(inner.fill(|_, _, v| glow(lamp, (0.55 - 0.3 * v) * alpha * (1.0 - 0.6 * hot)))));
    }

    // Its edge, a chamfer: lit on the sides that face the light, shaded on the others (a hard
    // band, then a soft one inside it), and a line round it.
    let shine = if e.disabled { 0.35 } else { 1.0 };
    let chamfer = |n: egui::Vec2, soft: f32| {
        let f = facing(n);
        let light = (f.max(0.0).powf(0.8) * (0.42 + 0.22 * hot) * shine).min(0.8) * soft;
        let dark = (-f).max(0.0).powf(0.8) * 0.62 * soft;
        if light >= dark { Color32::from_white_alpha((255.0 * light * alpha) as u8) } else { Color32::from_black_alpha((255.0 * dark * alpha) as u8) }
    };
    p.add(Shape::mesh(tess.strip((0.0, -sp.pt(1.3)), |_, n| (chamfer(n, 1.0), chamfer(n, 1.0)))));
    p.add(Shape::mesh(tess.strip((-sp.pt(1.3), -sp.pt(3.6)), |_, n| (chamfer(n, 0.5), Color32::TRANSPARENT))));
    let line = fade(skin.edge, alpha).lerp_to_gamma(fade(skin.edge_lit, alpha), hot);
    p.add(Shape::closed_line(tess.outline_points(), Stroke::new(if hot > 0.5 { 1.6 } else { 1.2 }, line)));
    if look.warn > 0.01 {
        // A refusal: the plate takes a rust tint for a moment (dimmer for those who asked for no flashes).
        let k = look.warn * alpha * if sp.calm { 0.55 } else { 1.0 };
        p.add(Shape::mesh(tess.fill(|_, _, _| Color32::from_rgba_unmultiplied(206, 74, 52, (96.0 * k) as u8))));
    }
    if look.press > 0.01 && !sp.calm {
        // The flash of a press: a brief white over the plate.
        p.add(Shape::mesh(tess.fill(|_, _, _| glow([255.0, 244.0, 214.0], 0.22 * look.press * alpha))));
    }

    // Its contents, fitted to the room the plate has.
    let ink = fade(if hot > 0.5 { skin.ink_lit } else { skin.ink }, alpha);
    let ink = if hot > 0.01 && hot <= 0.5 { skin.ink.lerp_to_gamma(skin.ink_lit, hot * 2.0).gamma_multiply(alpha) } else { ink };
    content(p, sp, a, (r0, r1), width, e, pic, &skin, hot, look.alpha * sp.alpha, ink, look.pop);

    // Its key, on the rim outside; and a lamp where it is set.
    if let Some(key) = &e.key {
        key_cap(p, sp, polar(centre, a, r1 + sp.pt(14.0 - 1.0 * (1.0 - hot))), key, hot, alpha);
    }
}

/// The glyph or picture, the words, the cost and the badges of a plate.
#[allow(clippy::too_many_arguments)]
fn content(p: &Painter, sp: &Space, a: f32, (r0, r1): (f32, f32), width: f32, e: &Entry, pic: Option<UnitPic>, skin: &Skin, hot: f32, alpha: f32, ink: Color32, pop: f32) {
    let centre = sp.centre;
    let rm = (r0 + r1) / 2.0 - sp.pt(3.0);
    let at = polar(centre, a, rm);
    // The room a line of the word has: the plate's width where it faces up or down, its depth where
    // it faces sideways (a ring of five or fewer is wider than it is deep, and a long word on a side
    // plate would run under the middle).
    let across = (2.0 * rm * (width / 2.0).sin()).max(sp.pt(46.0)) - 8.0;
    let depth = (r1 - r0) - sp.pt(16.0);
    let chord = across.min(1.0 / ((a.sin() / depth).powi(2) + (a.cos() / across).powi(2)).sqrt());
    // (What leads to another ring has a mark after its word: room for it.)
    let marker = if e.opens { sp.pt(9.0) } else { 0.0 };
    let label = fitted(p, &e.label, sp.text(10.5), chord - marker, ink);
    let extra = if e.cost.is_some() { sp.pt(12.5) } else if e.detail.is_some() { sp.pt(11.0) } else { 0.0 };
    let gap = sp.pt(2.0);
    let mut icon = if e.picture.is_some() { sp.pt(34.0) } else { sp.pt(26.0) };
    let room = (r1 - r0) - sp.pt(18.0);
    let block = icon + gap + label.size().y + extra;
    if block > room {
        icon = (icon - (block - room)).max(sp.pt(18.0));
    }
    // (As the plate rises its picture rises with it and its word settles under it.)
    let lift = sp.pt(1.8) * hot;
    let top = at.y - (icon + gap + label.size().y + extra) / 2.0 - sp.pt(2.0) - lift;
    let icon_rect = Rect::from_center_size(pos2(at.x, top + icon / 2.0), EVec2::splat(icon * (1.0 + 0.1 * hot)));
    if let Some(pic) = pic {
        pic.paint(p, icon_rect, Color32::WHITE.gamma_multiply(alpha * if e.disabled { 0.5 } else { 1.0 }));
        p.add(crate::hud::pic_frame(icon_rect, hot > 0.5));
    } else if let Some(glyph) = e.glyph {
        stamp(p, icon_rect, glyph, ink, alpha, skin.dark_ink_lit && hot > 0.5);
    }
    let label_at = pos2(at.x - (label.size().x + marker) / 2.0, top + icon + gap + sp.pt(1.2) * hot);
    outlined(p, label_at.round(), label.clone(), ink, alpha, !(skin.dark_ink_lit && hot > 0.5));
    if e.opens {
        // A small arrowhead after the word: it opens a ring of its own.
        let tip = pos2(label_at.x + label.size().x + sp.pt(7.5), label_at.y + label.size().y / 2.0);
        let (h, w) = (sp.pt(3.1), sp.pt(4.2));
        let (fill, edge) = if skin.dark_ink_lit && hot > 0.5 { (Color32::from_rgb(40, 28, 8), Color32::from_white_alpha(70)) } else { (AMBER, Color32::from_black_alpha(170)) };
        p.add(Shape::convex_polygon(vec![tip, tip + vec2(-w, -h), tip + vec2(-w, h)], fade(fill, alpha), Stroke::new(1.0, fade(edge, alpha))));
    }
    let below = label_at.y + label.size().y + sp.pt(1.0);
    if let Some((metal_cost, energy_cost)) = e.cost {
        cost_chip(p, pos2(at.x, below), (metal_cost, energy_cost), alpha, sp);
    } else if let Some(detail) = &e.detail {
        p.text(pos2(at.x, below), Align2::CENTER_TOP, detail, strong(sp.text(9.0)), fade(STEEL, alpha));
    }
    // How many are queued, or how many choices there are within: a roundel on the picture's corner.
    if e.count > 0 {
        let queued = matches!(e.id, Id::Produce(_));
        // (A unit just queued: the count swells and settles.)
        let radius = sp.pt(if queued { 8.0 } else { 7.0 }) * (1.0 + 0.45 * pop);
        badge(p, icon_rect.right_top() + vec2(sp.pt(1.0), sp.pt(1.0)), radius, &count_text(e.count), alpha, queued, sp);
    }
    // On or the setting it has: a lamp on the inner edge.
    if e.lit && !e.disabled {
        p.add(metal::faded_lamp(polar(centre, a, r0 + sp.pt(7.5)), sp.pt(2.6), GREEN, 1.0, alpha));
    }
}

fn count_text(n: usize) -> String {
    if n > 99 { "99+".to_string() } else { n.to_string() }
}

/// `text` in lettering `size` points high on one line in `room` points: a smaller size (down to
/// four fifths) where it does not fit at that, and past that a second line.
pub fn fitted(painter: &Painter, text: &str, size: f32, room: f32, color: Color32) -> Arc<Galley> {
    for k in [1.0, 0.92, 0.84] {
        let galley = painter.layout_no_wrap(text.to_string(), strong(size * k), color);
        if galley.size().x <= room {
            return galley;
        }
    }
    painter.layout(text.to_string(), strong(size * 0.84), color, room)
}

/// Lettering with the dark edge TA's has (and its own shadow below): `thick` for light letters on dark.
fn outlined(p: &Painter, at: Pos2, galley: Arc<Galley>, ink: Color32, alpha: f32, thick: bool) {
    if thick {
        let edge = Color32::from_black_alpha((205.0 * alpha) as u8);
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.0, 1.0)] {
            p.galley_with_override_text_color(at + vec2(dx, dy), galley.clone(), edge);
        }
    } else {
        // (Dark letters on brass are engraved: a light edge below.)
        p.galley_with_override_text_color(at + vec2(0.0, 1.0), galley.clone(), Color32::from_white_alpha((90.0 * alpha) as u8));
    }
    p.galley_with_override_text_color(at, galley, ink);
}

/// A die-stamped glyph: a dark edge below it, or on the brass a light one.
fn stamp(p: &Painter, rect: Rect, glyph: Glyph, ink: Color32, alpha: f32, on_brass: bool) {
    if on_brass {
        hud_icons::paint(p, rect.translate(vec2(0.8, 1.0)), glyph, Color32::from_white_alpha((120.0 * alpha) as u8));
    } else {
        hud_icons::paint(p, rect.translate(vec2(1.0, 1.0)), glyph, Color32::from_black_alpha((180.0 * alpha) as u8));
    }
    hud_icons::paint(p, rect, glyph, ink);
}

/// A metal figure and an energy figure on a small dark chip, as the deck's build pictures have
/// them: the costs of a unit or a building.
fn cost_chip(p: &Painter, top: Pos2, (metal_cost, energy_cost): (f32, f32), alpha: f32, sp: &Space) {
    let size = sp.text(9.5);
    let short = |v: f32| if v >= 10_000.0 { format!("{:.0}k", v / 1000.0) } else { format!("{v:.0}") };
    let (m, en) = (p.layout_no_wrap(short(metal_cost), strong(size), METAL), p.layout_no_wrap(short(energy_cost), strong(size), ENERGY));
    let pad = sp.pt(3.5);
    let gap = sp.pt(4.0);
    let w = m.size().x + en.size().x + gap + 2.0 * pad;
    let h = m.size().y.max(en.size().y) + sp.pt(1.5);
    let chip = Rect::from_min_size(pos2(top.x - w / 2.0, top.y), vec2(w, h));
    p.rect_filled(chip.translate(vec2(0.0, 1.0)), 3.0, Color32::from_white_alpha((16.0 * alpha) as u8));
    p.rect_filled(chip, 3.0, Color32::from_black_alpha((170.0 * alpha) as u8));
    p.rect_stroke(chip, 3.0, Stroke::new(1.0, Color32::from_black_alpha((210.0 * alpha) as u8)), StrokeKind::Inside);
    let y = chip.center().y;
    let x = chip.min.x + pad;
    p.galley_with_override_text_color(pos2(x, y - m.size().y / 2.0), m.clone(), fade(METAL, alpha));
    p.galley_with_override_text_color(pos2(x + m.size().x + gap, y - en.size().y / 2.0), en, fade(ENERGY, alpha));
}

/// A brass roundel with a figure in it (how many are queued): or, for what is not a queue, a steel one.
fn badge(p: &Painter, c: Pos2, r: f32, text: &str, alpha: f32, brass: bool, sp: &Space) {
    p.circle_filled(c + vec2(0.7, 1.2), r + 0.6, Color32::from_black_alpha((150.0 * alpha) as u8));
    p.circle_filled(c, r + 1.0, fade(metal::STEEL_EDGE, alpha));
    let (body, shine) = if brass { (Color32::from_rgb(168, 128, 52), Color32::from_rgb(238, 206, 122)) } else { (Color32::from_rgb(84, 84, 78), Color32::from_rgb(150, 150, 140)) };
    p.circle_filled(c, r, fade(body, alpha));
    p.circle_filled(c + vec2(-r * 0.22, -r * 0.28), r * 0.72, fade(shine, 0.55 * alpha));
    p.circle_filled(c + vec2(r * 0.12, r * 0.2), r * 0.62, fade(body, 0.85 * alpha));
    let ink = if brass { Color32::from_rgb(34, 24, 8) } else { TEXT_BRIGHT };
    let galley = p.layout_no_wrap(text.to_string(), display(sp.text(if text.len() > 2 { 9.5 } else { 11.0 })), ink);
    let at = c - galley.size() / 2.0 + vec2(0.0, 0.5);
    if brass {
        p.galley_with_override_text_color(at + vec2(0.0, 0.8), galley.clone(), Color32::from_white_alpha((90.0 * alpha) as u8));
    }
    p.galley_with_override_text_color(at, galley, fade(ink, alpha));
}

/// A key as a small cap centred at `at` (moved in where a long key would go out of the room the
/// menu has): sunk into the rim, lit brass with its plate.
pub fn key_cap(p: &Painter, sp: &Space, at: Pos2, key: &str, hot: f32, alpha: f32) {
    let text: String = key.split('+').map(|part| part.trim().to_uppercase()).collect::<Vec<_>>().join(" ");
    if text.chars().count() > 8 {
        return;
    }
    let ink = Color32::from_rgb(222, 214, 188).lerp_to_gamma(Color32::from_rgb(255, 238, 180), hot);
    let galley = p.layout_no_wrap(text, strong(sp.text(9.5)), ink);
    // (A lit key swells a little.)
    let mut cap = Rect::from_center_size(at, (galley.size() + vec2(9.0, 3.5) * sp.scale.sqrt()) * (1.0 + 0.1 * hot));
    let room = Rect::from_center_size(sp.centre, EVec2::splat(2.0 * sp.reach()));
    cap = cap.translate(vec2((room.min.x - cap.min.x).max(0.0) - (cap.max.x - room.max.x).max(0.0), (room.min.y - cap.min.y).max(0.0) - (cap.max.y - room.max.y).max(0.0)));
    p.rect_filled(cap.translate(vec2(0.0, 1.0)), 3.0, Color32::from_white_alpha((18.0 * alpha) as u8));
    let fill = Color32::from_black_alpha(196).lerp_to_gamma(Color32::from_rgb(78, 58, 20), hot);
    p.rect_filled(cap, 3.0, fade(fill, alpha));
    p.rect_stroke(cap, 3.0, Stroke::new(1.0, fade(metal::STEEL_SHADE.lerp_to_gamma(AMBER_BRIGHT, hot), alpha)), StrokeKind::Inside);
    p.hline(cap.shrink2(vec2(3.0, 0.0)).x_range(), cap.min.y + 1.5, Stroke::new(1.0, Color32::from_white_alpha((20.0 * alpha) as u8)));
    p.galley_with_override_text_color(cap.center() - galley.size() / 2.0, galley, fade(ink, alpha));
}

// ---- The rim and the hub -----------------------------------------------------------------------

/// A steel tube bent into a circle, as the colour at `angle` (clockwise from up) and across its
/// width `w` (-1 at the inner edge, 1 at the outer): lit on the side that faces the light, with a
/// glint, dark along its two edges.
fn tube(angle: f32, w: f32, body: Color32) -> Color32 {
    let out = vec2(angle.sin(), -angle.cos());
    let nz = (1.0 - w * w).max(0.0).sqrt();
    let n = [out.x * w, out.y * w, nz];
    let l = norm([shape::LIGHT.x * 0.78, shape::LIGHT.y * 0.78, 0.62]);
    let diffuse = dot(n, l).clamp(0.0, 1.0);
    let half = norm([l[0], l[1], l[2] + 1.0]);
    let spec = dot(n, half).clamp(0.0, 1.0).powf(20.0);
    let shade = 0.26 + 0.74 * diffuse;
    let edge = 1.0 - 0.62 * w.abs().powi(6);
    let c = |v: u8| ((v as f32 * shade + 150.0 * spec) * edge).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(c(body.r()), c(body.g()), c(body.b()))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// A rivet head with the light on it.
fn rivet(p: &Painter, c: Pos2, r: f32, alpha: f32) {
    p.circle_filled(c + vec2(0.5, 0.9), r, Color32::from_black_alpha((150.0 * alpha) as u8));
    p.circle_filled(c, r + 0.6, fade(metal::STEEL_EDGE, alpha));
    p.circle_filled(c, r, fade(Color32::from_rgb(112, 110, 102), alpha));
    p.circle_filled(c + vec2(-r * 0.3, -r * 0.32), r * 0.45, Color32::from_white_alpha((150.0 * alpha) as u8));
}

/// The rim round the plates: a steel tube with a rivet where two plates meet, a lamp over each
/// plate that is lit (`lamps`: where it reaches round the ring, and how strongly) and a bead of
/// light riding it to the plate the pointer is on (`bead`: its angle and how bright). `depth` rings
/// opened from rings add a thin brass band each outside.
pub fn bezel(p: &Painter, sp: &Space, count: usize, lamps: &[(f32, f32, f32)], bead: Option<(f32, f32)>, depth: usize) {
    let (a, s) = (sp.alpha, sp.scale);
    let (r_in, r_out) = (sp.r1 + 2.0 * s, sp.r1 + 7.0 * s);
    let body = metal::STEEL_BODY;
    let mut tube_mesh = shape::ring(sp.centre, (r_in, r_out), (0.0, std::f32::consts::TAU), 6, |ang, w| tube(ang, w, body).gamma_multiply(a));
    // (A shadow under it, towards the bottom right.)
    let under = shape::ring(sp.centre + vec2(1.2, 2.2) * s, (r_in - 0.5, r_out + 0.5), (0.0, std::f32::consts::TAU), 1, |_, _| Color32::from_black_alpha((120.0 * a) as u8));
    p.add(Shape::mesh(under));
    tube_mesh.indices.shrink_to_fit();
    p.add(Shape::mesh(tube_mesh));
    for k in 1..=depth.min(2) {
        let r = r_out + 3.0 * s * k as f32;
        p.add(Shape::mesh(shape::ring(sp.centre, (r - 1.0 * s, r + 1.0 * s), (0.0, std::f32::consts::TAU), 2, |ang, w| tube(ang, w, Color32::from_rgb(176, 138, 62)).gamma_multiply(a))));
    }
    if count > 1 {
        let step = std::f32::consts::TAU / count as f32;
        for k in 0..count {
            rivet(p, polar(sp.centre, (k as f32 - 0.5) * step, (r_in + r_out) / 2.0), 1.7 * s, a);
        }
    }
    for &(lo, hi, strength) in lamps {
        let k = strength * if sp.calm { 0.7 } else { 1.0 } * a;
        let mesh = shape::ring(sp.centre, (r_in + 1.3 * s, r_out - 1.3 * s), (lo, hi), 4, |_, w| glow([255.0, 214.0, 120.0], k * (1.0 - w * w * 0.8)));
        p.add(Shape::mesh(mesh));
    }
    if let Some((angle, strength)) = bead {
        // The bead: a small bright lamp on the rim (its halo is left out for those who asked for no flashes).
        let c = polar(sp.centre, angle, (r_in + r_out) / 2.0);
        let k = strength.clamp(0.0, 1.0) * a;
        if !sp.calm {
            for (reach, light) in [(4.4, 0.10), (3.2, 0.17), (2.2, 0.30)] {
                p.circle_filled(c, 1.6 * s * reach, glow([255.0, 214.0, 120.0], light * k));
            }
        }
        p.circle_filled(c, 2.3 * s, fade(Color32::from_rgb(255, 226, 150), k));
        p.circle_filled(c + vec2(-0.5, -0.6) * s, 1.1 * s, Color32::from_white_alpha((230.0 * k) as u8));
    }
}

/// A ring of light that spreads from `at` and thins as it goes (`t` 0 to 1): the choice that was
/// made, going. (Not drawn with reduced flashes: it is a pulse of light.)
pub fn ripple(p: &Painter, sp: &Space, at: Pos2, t: f32) {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 || sp.calm {
        return;
    }
    let spread = 1.0 - (1.0 - t).powi(3);
    let fall = (1.0 - t).powi(2);
    p.circle_stroke(at, sp.pt(7.0 + 44.0 * spread), Stroke::new(sp.text(2.4) * (1.0 - 0.6 * t), glow([255.0, 220.0, 130.0], 0.55 * fall)));
}

/// The soft shade under the whole ring: darker near it, gone by the time it has a hand's breadth.
pub fn pool(p: &Painter, sp: &Space) {
    let a = sp.alpha;
    let r = sp.r1 + 8.0 * sp.scale;
    let stops = [
        (0.0, Color32::from_black_alpha((120.0 * a) as u8)),
        (r, Color32::from_black_alpha((120.0 * a) as u8)),
        (r + 14.0 * sp.scale, Color32::from_black_alpha((78.0 * a) as u8)),
        (r + 30.0 * sp.scale, Color32::from_black_alpha((34.0 * a) as u8)),
        (r + 48.0 * sp.scale, Color32::TRANSPARENT),
    ];
    p.add(Shape::mesh(shape::disc(sp.centre + vec2(0.0, 3.0 * sp.scale), &stops)));
}

/// What the middle of the menu shows.
pub struct Hub<'a> {
    pub picture: Option<UnitPic>,
    pub name: &'a str,
    /// The share of health left (none: no ring).
    pub health: Option<f32>,
    /// In a ring opened from another: the ring's name, how many plates the ring before had and which
    /// of them opened this one.
    pub back: Option<(&'a str, usize, Option<usize>)>,
    /// Which page of how many, the page the dots were on a moment ago and how far they have come
    /// from it (0 to 1).
    pub page: Option<(usize, usize)>,
    pub page_from: Option<usize>,
    pub page_mix: f32,
    /// How much the hub is lit, when the pointer is over it (it goes back).
    pub hot: f32,
}

/// The hub: the seat, and on it the unit (or the way back).
pub fn hub(p: &Painter, sp: &Space, h: &Hub, face_alpha: f32) {
    hub_seat(p, sp, h.hot);
    hub_face(p, sp, h, face_alpha);
}

/// The hub's seat: a steel ring, and in it the well.
pub fn hub_seat(p: &Painter, sp: &Space, hot: f32) {
    let (a, s) = (sp.alpha, sp.scale);
    let r = (sp.r0 - 5.0 * s).max(8.0);
    let c = sp.centre;
    p.add(Shape::mesh(shape::ring(c + vec2(0.8, 1.6) * s, (r - 1.0, r + 4.5 * s), (0.0, std::f32::consts::TAU), 1, |_, _| Color32::from_black_alpha((130.0 * a) as u8))));
    p.add(Shape::mesh(shape::ring(c, (r, r + 4.0 * s), (0.0, std::f32::consts::TAU), 5, |ang, w| tube(ang, w, metal::STEEL_BODY).gamma_multiply(a))));
    let well = Color32::from_rgb(40, 39, 34).lerp_to_gamma(Color32::from_rgb(76, 62, 28), hot);
    p.add(Shape::mesh(shape::disc(c, &[(0.0, well.gamma_multiply(a)), (r * 0.72, Color32::from_rgb(28, 27, 23).lerp_to_gamma(Color32::from_rgb(52, 42, 18), hot).gamma_multiply(a)), (r, Color32::from_rgb(12, 11, 9).gamma_multiply(a))])));
}

/// What is on the hub's seat: the unit and its health and name, or the way back, and the dots of a
/// long ring's pages; `face_alpha` fades it (one face goes as the next comes).
pub fn hub_face(p: &Painter, sp: &Space, h: &Hub, face_alpha: f32) {
    let (a, s) = (sp.alpha * face_alpha, sp.scale);
    let r = (sp.r0 - 5.0 * s).max(8.0);
    let c = sp.centre;
    // Its health, round the well: a track and what is left of it (on the unit's own ring, not on
    // the way back).
    if let Some(health) = h.health.filter(|_| h.back.is_none()) {
        let (ri, ro) = (r - 6.5 * s, r - 3.2 * s);
        p.add(Shape::mesh(shape::ring(c, (ri, ro), (0.0, std::f32::consts::TAU), 2, |_, w| Color32::from_black_alpha((150.0 * (1.0 - 0.4 * w.abs()) * a) as u8))));
        let colour = if health > 0.6 { GREEN } else if health > 0.3 { AMBER } else { RED };
        if health > 0.004 {
            p.add(Shape::mesh(shape::ring(c, (ri + 0.4, ro - 0.4), (0.0, std::f32::consts::TAU * health), 3, |_, w| {
                let lift = 1.0 - 0.55 * w.abs();
                Color32::from_rgb((colour.r() as f32 * lift) as u8, (colour.g() as f32 * lift) as u8, (colour.b() as f32 * lift) as u8).gamma_multiply(a)
            })));
        }
    }
    if let Some((ring, parts, from)) = h.back {
        // A way back: a small dial of the ring before with the plate this ring came from lit, the arrow
        // over it, and the ring's name.
        let dial = Pos2::new(c.x, c.y - 12.0 * s);
        let (di, dr) = (9.0 * s, 18.5 * s);
        let step = std::f32::consts::TAU / parts.max(1) as f32;
        p.add(Shape::mesh(shape::ring(dial, (di - 1.0, dr + 1.0), (0.0, std::f32::consts::TAU), 1, |_, _| Color32::from_black_alpha((190.0 * a) as u8))));
        for k in 0..parts.max(1) {
            let lit = from == Some(k);
            let (lo, hi) = if parts <= 1 { (0.0, std::f32::consts::TAU) } else { (k as f32 * step - step / 2.0 + 0.06, k as f32 * step + step / 2.0 - 0.06) };
            let (inner, outer) = if lit { (Color32::from_rgb(180, 136, 52), Color32::from_rgb(240, 204, 118)) } else { (Color32::from_rgb(58, 57, 52), Color32::from_rgb(98, 96, 88)) };
            p.add(Shape::mesh(shape::ring(dial, (di, dr), (lo, hi), 1, |_, w| if w < 0.0 { inner.gamma_multiply(a) } else { outer.gamma_multiply(a) })));
        }
        stamp(p, Rect::from_center_size(dial, EVec2::splat(13.0 * s)), Glyph::Back, fade(CREAM, a.min(1.0)), a, false);
        let name = hud_icons::lettering(p, ring, sp.text(9.5), r * 1.5, fade(CREAM, a));
        let at = pos2(c.x - name.size().x / 2.0, c.y + 14.0 * s);
        outlined(p, at.round(), name, fade(CREAM, a), a, true);
        let caption = p.layout_no_wrap(ta_ui::i18n::t("BACK").to_string(), strong(sp.text(8.0)), fade(STEEL, a));
        p.galley(pos2(c.x - caption.size().x / 2.0, c.y + 26.0 * s), caption, fade(STEEL, a));
    } else {
        if let Some(pic) = h.picture {
            let size = 36.0 * s;
            let rect = Rect::from_center_size(pos2(c.x, c.y - 8.0 * s), EVec2::splat(size));
            pic.paint(p, rect, Color32::WHITE.gamma_multiply(a));
            p.add(crate::hud::pic_frame(rect, false));
        }
        if !h.name.is_empty() {
            let chord = 2.0 * (r * r - (24.0 * s).powi(2)).max(1.0).sqrt() - 4.0;
            let name = fitted_name(p, h.name, sp.text(9.5), chord.max(30.0), fade(CREAM, a));
            let at = pos2(c.x - name.size().x / 2.0, c.y + 14.0 * s);
            outlined(p, at.round(), name, fade(CREAM, a), a, true);
        }
    }
    // The pages of a ring too long for one: a dot to each, the one it is on lit.
    if let Some((at, of)) = h.page {
        let y = c.y + r - 1.0 * s;
        let spacing = 6.0 * s;
        let x0 = c.x - spacing * (of as f32 - 1.0) / 2.0;
        for k in 0..of.min(9) {
            let now = if k == at.min(of - 1) { 1.0 } else { 0.0 };
            // (The light moves from the page it was on to the one it is on.)
            let lit = match h.page_from {
                Some(from) if from != at => {
                    let was = if k == from { 1.0 } else { 0.0 };
                    was + (now - was) * h.page_mix.clamp(0.0, 1.0)
                }
                _ => now,
            };
            p.add(metal::faded_lamp(pos2(x0 + spacing * k as f32, y), 1.6 * s, AMBER, lit, a));
        }
    }
}

/// A name in the hub: as large as fits on a line, else two.
fn fitted_name(p: &Painter, text: &str, size: f32, room: f32, color: Color32) -> Arc<Galley> {
    for k in [1.0, 0.9, 0.8] {
        let galley = p.layout_no_wrap(text.to_string(), strong(size * k), color);
        if galley.size().x <= room {
            return galley;
        }
    }
    p.layout(text.to_string(), strong(size * 0.8), color, room)
}

// ---- The line of words ---------------------------------------------------------------------

/// What the plate under the ring says, on a plate of its own: its words over a dark ground with a
/// steel edge and a brass strip. When the words change (`mix`, 0 to 1, is how far the change has
/// come) the new words slide up into the plate as the old slide out, and the plate takes the size
/// of the new on the way; with no words before (or none after) the plate itself comes (or goes).
/// The words slide `slide` points (at the smallest size) as they change: none with reduced motion,
/// when they only cross over.
pub fn plate(p: &Painter, sp: &Space, cur: &str, prev: &str, mix: f32, wrap: f32, slide: f32) {
    let m = mix.clamp(0.0, 1.0);
    let e = m * m * (3.0 - 2.0 * m);
    let prev = if m < 1.0 { prev } else { "" };
    if cur.is_empty() && prev.is_empty() {
        return;
    }
    // (The old words go sooner than the new come, so that the two are not read over each other, and the
    // plate has the size of the new words before they are clear: they do not show cut at its sides.)
    let (k_cur, k_prev) = (((m - 0.3) / 0.7).clamp(0.0, 1.0), (1.0 - m * 2.2).clamp(0.0, 1.0));
    let grown = (m * 1.7).min(1.0);
    let grown = grown * grown * (3.0 - 2.0 * grown);
    let layout = |text: &str, k: f32| (!text.is_empty()).then(|| p.layout(text.to_string(), body(sp.text(12.5)), fade(TEXT_BRIGHT, sp.alpha * k), wrap));
    let (g_cur, g_prev) = (layout(cur, k_cur), layout(prev, k_prev));
    let size = match (&g_cur, &g_prev) {
        (Some(c), Some(o)) => vec2(o.size().x + (c.size().x - o.size().x) * grown, o.size().y + (c.size().y - o.size().y) * grown),
        (Some(c), None) => c.size(),
        (None, Some(o)) => o.size(),
        (None, None) => return,
    };
    // The plate's own opacity: what it was, to what it is (it comes with its first words, goes with its last).
    let a = sp.alpha * (if prev.is_empty() { 0.0 } else { 1.0 } + ((if cur.is_empty() { 0.0 } else { 1.0 }) - (if prev.is_empty() { 0.0 } else { 1.0 })) * e);
    if a <= 0.01 {
        return;
    }
    let top = sp.centre.y + sp.r1 + sp.pt(31.0);
    let rect = Rect::from_min_size(pos2(sp.centre.x - size.x / 2.0, top), size).expand2(vec2(11.0, 5.5));
    p.rect_filled(rect.translate(vec2(1.0, 2.0)), 4.0, Color32::from_black_alpha((90.0 * a) as u8));
    p.add(metal::vgrad(rect, Color32::from_rgba_unmultiplied(34, 33, 29, (232.0 * a) as u8), Color32::from_rgba_unmultiplied(16, 15, 13, (238.0 * a) as u8)));
    p.rect_stroke(rect, 3.0, Stroke::new(1.0, fade(metal::STEEL_SHADE, a)), StrokeKind::Outside);
    p.hline(rect.shrink2(vec2(3.0, 0.0)).x_range(), rect.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha((24.0 * a) as u8)));
    // A brass strip down its left edge, as a plate in the deck has.
    let strip = Rect::from_min_max(rect.min + vec2(0.5, 2.0), pos2(rect.min.x + 3.5, rect.max.y - 2.0));
    p.add(metal::vgrad(strip, fade(Color32::from_rgb(232, 196, 108), a), fade(Color32::from_rgb(150, 114, 45), a)));
    let words = p.with_clip_rect(rect.shrink(1.0).intersect(p.clip_rect()));
    if let Some(g) = g_prev {
        words.galley(pos2(sp.centre.x - g.size().x / 2.0, top - sp.pt(slide) * e), g, fade(TEXT_BRIGHT, sp.alpha * k_prev));
    }
    if let Some(g) = g_cur {
        words.galley(pos2(sp.centre.x - g.size().x / 2.0, top + sp.pt(slide) * (1.0 - e)), g, fade(TEXT_BRIGHT, sp.alpha * k_cur));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, glyph: Glyph) -> Entry {
        Entry::new(Id::Info, glyph, label, "what it does")
    }

    /// Runs `draw` in one frame of a 1280 by 720 screen: the bounds of everything it painted.
    fn painted(draw: impl FnOnce(&Painter, &Context)) -> Vec<Rect> {
        let ctx = Context::default();
        crate::theme::install_compact(&ctx);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
        let mut draw = Some(draw);
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), max_texture_side: Some(8192), ..Default::default() }, |ui| {
            if let Some(draw) = draw.take() {
                draw(ui.painter(), ui.ctx());
            }
        });
        let shapes = std::mem::take(&mut out.shapes);
        out.drop_without_applying_deltas();
        shapes.iter().map(|s| s.shape.visual_bounding_rect()).filter(|r| r.is_finite()).collect()
    }

    /// The whole of the menu, plates risen and keys and all, keeps inside the room round its
    /// middle that the menu asks of the screen (`reach` in `Hud::radial_menu`), at every size and
    /// however many plates; the soft shade under it, which is not a part that can be cut, goes beyond.
    #[test]
    fn the_whole_ring_stays_inside_the_room_the_menu_asks_the_screen_for() {
        for scale in [1.0f32, 1.33, 1.5] {
            for count in [1usize, 2, 3, 6, 9] {
                let centre = pos2(640.0, 360.0);
                let sp = Space { centre, scale, r0: radial::INNER * scale, r1: radial::OUTER * scale, alpha: 1.0, calm: false };
                let entries: Vec<Entry> = (0..count)
                    .map(|i| {
                        let mut e = entry(&format!("Choice number {i}"), Glyph::Move);
                        e.key = Some("ALT F".into());
                        e.opens = i % 2 == 0;
                        e.cost = (i % 3 == 0).then_some((1234.0, 56789.0));
                        e.count = i;
                        e
                    })
                    .collect();
                let rects = painted(|p, ctx| {
                    bezel(p, &sp, count, &[(0.0, 1.0, 1.0)], Some((0.4, 1.0)), 2);
                    for (i, e) in entries.iter().enumerate() {
                        // (The first lit and pressed, risen past its place as a spring swings, the second turned and shrunk.)
                        let look = match i {
                            0 => Look { hot: 1.2, press: 0.5, pop: 1.0, ..Look::REST },
                            1 => Look { grow: 0.6, swing: 0.8, alpha: 0.5, nudge: 3.5, ..Look::REST },
                            _ => Look::REST,
                        };
                        wedge(p, ctx, &sp, i, count, e, None, &look, 1);
                    }
                    let hub_of = Hub { picture: None, name: "Construction KBot × 12", health: Some(0.5), back: None, page: Some((1, 3)), page_from: Some(0), page_mix: 0.5, hot: 0.0 };
                    hub(p, &sp, &hub_of, 1.0);
                    let back = Hub { back: Some(("BUILD", 9, Some(2))), ..hub_of };
                    hub(p, &sp, &back, 1.0);
                    ripple(p, &sp, polar(centre, 0.0, 90.0 * scale), 0.4);
                });
                let room = Rect::from_center_size(centre, vec2(2.0 * sp.reach(), 2.0 * sp.reach()));
                for r in &rects {
                    assert!(room.expand(0.5).contains_rect(*r), "scale {scale}, {count} plates: {r:?} reaches out of {room:?}");
                }
                assert!(rects.len() > count * 8, "it painted something");
                // (The shade is wider, and soft all the way out.)
                let shade = painted(|p, _| pool(p, &sp));
                let widest = shade.iter().fold(Rect::NOTHING, |a, b| a.union(*b));
                assert!(widest.width() < 2.0 * (sp.r1 + 60.0 * scale), "the shade is a pool, not a floodlight: {widest:?}");
            }
        }
    }

    /// The line under the ring says nothing with no words, comes with its first and goes with its last,
    /// and between two sets of words stays between their sizes, under the ring.
    #[test]
    fn the_line_under_the_ring_changes_from_one_set_of_words_to_the_next() {
        let sp = Space { centre: pos2(640.0, 360.0), scale: 1.0, r0: radial::INNER, r1: radial::OUTER, alpha: 1.0, calm: false };
        let shapes = |cur: &str, prev: &str, mix: f32| painted(|p, _| plate(p, &sp, cur, prev, mix, 330.0, 5.0));
        let bounds = |cur: &str, prev: &str, mix: f32| shapes(cur, prev, mix).iter().fold(Rect::NOTHING, |a, b| a.union(*b));
        assert!(shapes("", "", 0.5).is_empty(), "no words, no plate");
        assert!(!shapes("Hold fire", "", 1.0).is_empty());
        assert!(!shapes("", "Hold fire", 0.3).is_empty(), "going");
        assert!(shapes("", "Hold fire", 1.0).is_empty(), "gone");
        let (short, long) = ("Hold fire: never shoots", "Attack (A): click an enemy to destroy it, or the ground to shell it; drag a circle for everything in it");
        let (small, large) = (bounds(short, "", 1.0), bounds(long, "", 1.0));
        for k in 0..=10 {
            let mid = bounds(long, short, k as f32 / 10.0);
            assert!(mid.top() >= sp.centre.y + sp.r1, "under the ring: {mid:?}");
            assert!(mid.width() >= small.width() - 8.0 && mid.width() <= large.width() + 8.0, "step {k}: {mid:?} between {small:?} and {large:?}");
        }
    }

    /// The words under the ring slide as they change, and with a slide of nothing (reduced motion) they
    /// only cross over: their places do not change from one step to the next.
    #[test]
    fn the_words_slide_only_when_they_are_asked_to() {
        let sp = Space { centre: pos2(640.0, 360.0), scale: 1.0, r0: radial::INNER, r1: radial::OUTER, alpha: 1.0, calm: false };
        // The top of each line of words painted, at several steps of the change.
        let tops = |slide: f32| -> Vec<Vec<f32>> {
            [0.3f32, 0.5, 0.7, 0.9]
                .iter()
                .map(|&mix| {
                    let ctx = Context::default();
                    crate::theme::install_compact(&ctx);
                    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
                    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| plate(ui.painter(), &sp, "Fight: move there", "Attack: shell it", mix, 330.0, slide));
                    let shapes = std::mem::take(&mut out.shapes);
                    out.drop_without_applying_deltas();
                    shapes.iter().filter_map(|s| if let Shape::Text(t) = &s.shape { Some(t.pos.y) } else { None }).collect()
                })
                .collect()
        };
        let slid = tops(5.0);
        let still = tops(0.0);
        let spread = |steps: &[Vec<f32>]| {
            let all: Vec<f32> = steps.iter().filter_map(|t| t.first().copied()).collect();
            all.iter().copied().fold(f32::MIN, f32::max) - all.iter().copied().fold(f32::MAX, f32::min)
        };
        assert!(spread(&slid) > 1.0, "the words slide when asked: {slid:?}");
        assert!(spread(&still) < 1e-3, "and stay where they are with no slide: {still:?}");
    }

    /// What a frame of a full ring costs to paint and to tessellate, and how many vertices go to the
    /// graphics card: at rest, and in a change of ring (the ring that is going under the one that is
    /// coming, as the interface draws them). Run with `--ignored --nocapture`; the figures are the PR's.
    #[test]
    #[ignore]
    fn what_a_frame_of_the_ring_costs() {
        use std::time::{Duration, Instant};
        let ctx = Context::default();
        crate::theme::install_compact(&ctx);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
        let count = 9;
        let sp = Space { centre: pos2(640.0, 360.0), scale: 1.0, r0: radial::INNER, r1: radial::OUTER, alpha: 1.0, calm: false };
        let entries: Vec<Entry> = (0..count)
            .map(|i| {
                let mut e = entry(&format!("Choice {i}"), Glyph::Move);
                e.key = Some("ALT F".into());
                e.cost = Some((120.0, 2410.0));
                e.count = i;
                e
            })
            .collect();
        // (RING_PARTS=bare: plates with no words, glyph, key or cost; =none: no plates; for finding what a frame is spent on.)
        let parts = std::env::var("RING_PARTS").unwrap_or_default();
        let entries: Vec<Entry> = if parts == "bare" {
            entries.into_iter().map(|e| Entry { label: String::new(), glyph: None, key: None, cost: None, count: 0, ..e }).collect()
        } else if parts == "none" {
            Vec::new()
        } else {
            entries
        };
        for rings in [1usize, 2] {
            let (mut build, mut tessellate, mut vertices, mut triangles) = (Duration::ZERO, Duration::ZERO, 0usize, 0usize);
            let frames = 200;
            for frame in 0..frames + 20 {
                let t0 = Instant::now();
                let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                    let p = ui.painter();
                    pool(p, &sp);
                    bezel(p, &sp, count, &[(0.0, 1.0, 1.0)], Some((0.4, 1.0)), 1);
                    for _ in 0..rings {
                        for (i, e) in entries.iter().enumerate() {
                            wedge(p, ui.ctx(), &sp, i, count, e, None, &Look { hot: if i == 2 { 1.0 } else { 0.0 }, ..Look::REST }, 0);
                        }
                    }
                    hub(p, &sp, &Hub { picture: None, name: "Stumpy", health: Some(0.8), back: None, page: None, page_from: None, page_mix: 1.0, hot: 0.0 }, 1.0);
                    plate(p, &sp, "Attack (A): click an enemy to destroy it", "", 1.0, 330.0, 5.0);
                });
                let built = t0.elapsed();
                let t1 = Instant::now();
                let primitives = ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
                let tessellated = t1.elapsed();
                out.drop_without_applying_deltas();
                if frame >= 20 {
                    build += built;
                    tessellate += tessellated;
                    for prim in &primitives {
                        if let egui::epaint::Primitive::Mesh(m) = &prim.primitive {
                            vertices += m.vertices.len();
                            triangles += m.indices.len() / 3;
                        }
                    }
                }
            }
            println!(
                "{} ring{} of {count}: {:.0} us to paint, {:.0} us to tessellate, {} vertices and {} triangles a frame",
                rings,
                if rings == 1 { "" } else { "s (a change)" },
                build.as_secs_f64() * 1e6 / frames as f64,
                tessellate.as_secs_f64() * 1e6 / frames as f64,
                vertices / frames,
                triangles / frames
            );
        }
    }

    /// Lit, set, dangerous and dead plates are told apart by their metal: the pointer's light is brass,
    /// what is set a little warmer, the danger rust and what would do nothing duller than any.
    #[test]
    fn a_lit_plate_is_brass_a_set_one_warmer_and_a_disabled_one_dull() {
        let plain = entry("A", Glyph::Info);
        let mut danger = plain.clone();
        danger.tone = Tone::Danger;
        let mut off = plain.clone();
        off.disabled = true;
        let luma = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
        let steel = Skin::of(&plain, 0);
        assert!(luma(steel.face(0.5, 1.0, 0.0, false)) > luma(steel.face(0.5, 0.0, 0.0, false)) + 40.0, "lit brass is brighter");
        let lit = steel.face(0.5, 1.0, 0.0, false);
        assert!(lit[0] > lit[2] * 1.8, "and it is brass, not grey");
        let rust = Skin::of(&danger, 0).face(0.5, 0.0, 0.0, false);
        assert!(rust[0] > rust[1] * 1.8 && rust[0] > rust[2] * 1.8, "danger is rust: {rust:?}");
        let dull = Skin::of(&off, 0).face(0.5, 0.0, 0.0, false);
        assert!(luma(dull) < luma(steel.face(0.5, 0.0, 0.0, false)), "dead is duller than steel");
        assert!(steel.face(0.5, 0.0, 0.0, true)[0] > steel.face(0.5, 0.0, 0.0, false)[0], "what is set is warmer");
        assert!(luma(steel.face(0.5, 0.0, 1.0, false)) > luma(steel.face(0.5, 0.0, -1.0, false)), "the dome's lit side is brighter");
        // A ring opened from another is a little darker than the first.
        assert!(luma(Skin::of(&plain, 2).face(0.5, 0.0, 0.0, false)) < luma(steel.face(0.5, 0.0, 0.0, false)));
    }

    /// The faces are valid colours at the ends of every range they are asked for.
    #[test]
    fn the_faces_stay_valid_colours() {
        for e in [entry("A", Glyph::Info), Entry { tone: Tone::Danger, ..entry("B", Glyph::Info) }, Entry { disabled: true, ..entry("C", Glyph::Info) }] {
            let skin = Skin::of(&e, 1);
            for v in [-1.0, 0.0, 0.5, 1.0, 3.0] {
                for hot in [0.0, 0.5, 1.0] {
                    for dome in [-1.0, 0.0, 1.0] {
                        let c = skin.face(v, hot, dome, true);
                        assert!(c.iter().all(|x| (0.0..=255.0).contains(x)), "{c:?}");
                    }
                }
            }
        }
    }

    /// A steel ring is lit where it faces the light: the outer side of its upper left, the inner
    /// side of its lower right; and dark along its edges.
    #[test]
    fn a_steel_tube_is_lit_on_the_side_that_faces_the_light() {
        let body = metal::STEEL_BODY;
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        let up_left = std::f32::consts::TAU * 0.875;
        assert!(lum(tube(up_left, 0.6, body)) > lum(tube(up_left, -0.6, body)));
        assert!(lum(tube(up_left + std::f32::consts::PI, -0.6, body)) > lum(tube(up_left + std::f32::consts::PI, 0.6, body)));
        assert!(lum(tube(up_left, 1.0, body)) < lum(tube(up_left, 0.0, body)), "the edge is dark");
    }

    /// A long word takes a smaller size before a second line, and a short one keeps its size.
    #[test]
    fn a_long_word_takes_a_smaller_size_and_then_a_second_line() {
        let mut found = Vec::new();
        painted(|p, _| {
            for room in [200.0, 80.0, 40.0] {
                found.push(fitted(p, "SELF-DESTRUCT", 10.5, room, Color32::WHITE));
            }
        });
        assert_eq!(found[0].rows.len(), 1);
        assert_eq!(found[1].rows.len(), 1, "smaller, on one line");
        assert!(found[1].size().x <= 80.0 && found[1].size().y < found[0].size().y + 0.1);
        assert!(found[2].rows.len() > 1, "two lines where it cannot fit");
    }
}
