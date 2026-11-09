//! Total Annihilation's materials, drawn: the grimy dark metal of its menus, steel pipe
//! frames with joints and bolts, recessed wells, bevelled gunmetal, brass and rust slabs,
//! indicator lamps and outlined titles. All of it is painted or generated here (no TA art
//! is shipped); the look follows TA's own screens (the battle room, the load game dialog,
//! the main menu's framed slots, the in-game buttons).
//!
//! The surfaces are returned as [`Shape`]s, so a panel can paint them behind contents laid
//! out first (`Painter::set` on a slot reserved with `Shape::Noop`); the text is painted
//! straight away.

use std::sync::Arc;

use egui::epaint::CircleShape;
use egui::{
    Color32, Context, CornerRadius, Galley, Id, Mesh, Painter, Pos2, Rangef, Rect, Response, RichText, Sense, Shape,
    Stroke, StrokeKind, TextureHandle, TextureId, TextureOptions, TextureWrapMode, Ui, pos2, vec2,
};

use crate::theme::*;

/// The steel of the pipes and slabs: its shine, body, shadow and outline.
pub const STEEL_SHINE: Color32 = Color32::from_rgb(214, 212, 204);
pub const STEEL_BODY: Color32 = Color32::from_rgb(128, 126, 118);
pub const STEEL_SHADE: Color32 = Color32::from_rgb(58, 56, 51);
pub const STEEL_EDGE: Color32 = Color32::from_rgb(16, 15, 13);

// ---- Grime ---------------------------------------------------------------------------------

/// Size of the fine grime texture (it tiles), at a texel a point.
const GRIME: usize = 256;

/// Size of the stains' texture (it tiles), at `STAIN_POINTS` points a texel: it repeats every
/// 2048 points, wider than any panel, so no panel shows its stains twice.
const STAINS: usize = 512;
const STAIN_POINTS: f32 = 4.0;

/// The scratches: hashed per square of this many points, so none recurs.
const SCRATCH_CELL: f32 = 128.0;

/// The fine grime laid over plates: brushing and grain, as black and white of low opacity (so
/// it darkens and lightens any colour beneath). Generated once per context; it tiles, anchored
/// to the screen so neighbouring plates match. (Its stains and scratches are laid separately,
/// see [`grime_over`]: in one small tile they repeated across a panel in a grid.)
pub fn grime(ctx: &Context) -> TextureId {
    cached(ctx, "ta_ui::metal::grime", grime_image)
}

/// The broad stains and mottling laid over plates, a texture of their own (see [`STAINS`]).
fn stains(ctx: &Context) -> TextureId {
    cached(ctx, "ta_ui::metal::stains", stain_image)
}

/// A tiling texture made once per context.
fn cached(ctx: &Context, key: &str, make: fn() -> egui::ColorImage) -> TextureId {
    let id = Id::new(key);
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture.id();
    }
    let options = TextureOptions { wrap_mode: TextureWrapMode::Repeat, ..TextureOptions::LINEAR };
    let texture = ctx.load_texture(key, make(), options);
    let texture_id = texture.id();
    ctx.data_mut(|d| d.insert_temp(id, texture));
    texture_id
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    (h & 0xffff) as f32 / 65535.0
}

/// Smooth noise on a lattice of `cell_x` by `cell_y` pixels that wraps every `size`.
fn noise(x: f32, y: f32, cell_x: f32, cell_y: f32, size: usize, seed: u32) -> f32 {
    let (u, v) = (x / cell_x, y / cell_y);
    let (periods_x, periods_y) = ((size as f32 / cell_x) as i32, (size as f32 / cell_y) as i32);
    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let at = |dx: i32, dy: i32| hash((x0 + dx).rem_euclid(periods_x), (y0 + dy).rem_euclid(periods_y), seed);
    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
    let (sx, sy) = (s(fx), s(fy));
    (a + (b - a) * sx) * (1.0 - sy) + (c + (d - c) * sx) * sy
}

fn grime_image() -> egui::ColorImage {
    let mut light = vec![0f32; GRIME * GRIME];
    for y in 0..GRIME {
        for x in 0..GRIME {
            let (fx, fy) = (x as f32, y as f32);
            // Fine brushing along the plate, and grain.
            let brushing = noise(fx, fy, 64.0, 2.0, GRIME, 4) * 0.6 + noise(fx, fy, 16.0, 1.0, GRIME, 5) * 0.4;
            let grain = hash(x as i32, y as i32, 6);
            light[y * GRIME + x] = (brushing - 0.5) * 0.22 + (grain - 0.5) * 0.22;
        }
    }
    light_image(GRIME, &light)
}

fn stain_image() -> egui::ColorImage {
    let mut light = vec![0f32; STAINS * STAINS];
    for y in 0..STAINS {
        for x in 0..STAINS {
            let (fx, fy) = (x as f32, y as f32);
            // Stains (broad) and mottling: lattices of 64, 32 and 16 points.
            let stains = noise(fx, fy, 16.0, 16.0, STAINS, 1) * 0.5 + noise(fx, fy, 8.0, 8.0, STAINS, 2) * 0.3 + noise(fx, fy, 4.0, 4.0, STAINS, 3) * 0.2;
            light[y * STAINS + x] = (stains - 0.5) * 1.2;
        }
    }
    light_image(STAINS, &light)
}

/// Light (lighter or darker than the plate) as white or black of low opacity.
fn light_image(size: usize, light: &[f32]) -> egui::ColorImage {
    let mut rgba = Vec::with_capacity(size * size * 4);
    for &l in light {
        if l >= 0.0 {
            rgba.extend_from_slice(&[255, 255, 255, (l * 30.0).min(24.0) as u8]);
        } else {
            rgba.extend_from_slice(&[0, 0, 0, (-l * 100.0).min(72.0) as u8]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([size, size], &rgba)
}

/// The grime over `rect` (`strength` 0 to 1), for a surface already filled: the stains, the
/// fine brushing and grain, and a few scratches, none of them repeating across a panel.
pub fn grime_over(ctx: &Context, rect: Rect, strength: f32) -> Shape {
    let strength = strength.clamp(0.0, 1.0);
    let a = (strength * 255.0).round() as u8;
    let tint = Color32::from_rgba_premultiplied(a, a, a, a);
    let uv = |size: f32| Rect::from_min_max(pos2(rect.min.x / size, rect.min.y / size), pos2(rect.max.x / size, rect.max.y / size));
    let mut shapes = vec![Shape::image(stains(ctx), rect, uv(STAINS as f32 * STAIN_POINTS), tint), Shape::image(grime(ctx), rect, uv(GRIME as f32), tint)];
    scratches(&mut shapes, rect, strength);
    Shape::Vec(shapes)
}

/// Short faint strokes, mostly along the brushing: a few in each square of [`SCRATCH_CELL`]
/// (anchored to the screen as the grime is), each whole inside `rect`.
fn scratches(shapes: &mut Vec<Shape>, rect: Rect, strength: f32) {
    let inside = rect.shrink(2.0);
    if !inside.is_positive() {
        return;
    }
    let cells = |a: f32, b: f32| (a / SCRATCH_CELL).floor() as i32..=(b / SCRATCH_CELL).floor() as i32;
    for j in cells(rect.min.y, rect.max.y) {
        for i in cells(rect.min.x, rect.max.x) {
            let count = 1 + (hash(i, j, 30) * 3.0) as u32;
            for m in 0..count {
                let k = 40 + 8 * m;
                let start = pos2((i as f32 + hash(i, j, k)) * SCRATCH_CELL, (j as f32 + hash(i, j, k + 1)) * SCRATCH_CELL);
                let length = 5.0 + hash(i, j, k + 2) * 18.0;
                let end = start + vec2(length, length * (hash(i, j, k + 3) - 0.5) * 0.35);
                if inside.contains(start) && inside.contains(end) {
                    let alpha = ((5.0 + 4.0 * hash(i, j, k + 4)) * strength) as u8;
                    shapes.push(Shape::line_segment([start, end], Stroke::new(1.0, Color32::from_white_alpha(alpha))));
                }
            }
        }
    }
}

// ---- Gradients -----------------------------------------------------------------------------

/// `rect` filled with a vertical gradient.
pub fn vgrad(rect: Rect, top: Color32, bottom: Color32) -> Shape {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    Shape::mesh(mesh)
}

/// `rect` filled with a horizontal gradient.
pub fn hgrad(rect: Rect, left: Color32, right: Color32) -> Shape {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    Shape::mesh(mesh)
}

// ---- Plates --------------------------------------------------------------------------------

/// A metal plate: `fill` with the grime over it and a faint light along its top edge.
pub fn plate(ctx: &Context, rect: Rect, fill: Color32) -> Shape {
    Shape::Vec(vec![
        Shape::rect_filled(rect, CornerRadius::same(2), fill),
        grime_over(ctx, rect, 1.0),
        Shape::hline(rect.x_range(), rect.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(10))),
    ])
}

/// A plate with a bevelled edge (lit above and on the left, shaded opposite) and a rivet in
/// each corner: the lighter frame, for tiles in a grid. `lit` (0 to 1) brightens it.
pub fn riveted_plate(ctx: &Context, rect: Rect, fill: Color32, lit: f32) -> Shape {
    let mut shapes = vec![Shape::rect_filled(rect.expand(1.0), CornerRadius::same(3), STEEL_EDGE)];
    shapes.push(plate(ctx, rect, fill.lerp_to_gamma(Color32::WHITE, 0.05 * lit)));
    shapes.extend(bevel(rect.shrink(0.5), Color32::from_white_alpha((22.0 + 30.0 * lit) as u8), Color32::from_black_alpha(110)));
    let inset = 7.0;
    for c in [rect.left_top() + vec2(inset, inset), rect.right_top() + vec2(-inset, inset), rect.left_bottom() + vec2(inset, -inset), rect.right_bottom() + vec2(-inset, -inset)] {
        shapes.push(rivet(c, 2.4));
    }
    Shape::Vec(shapes)
}

/// Light along the top and left edges of `rect`, shade along the others.
fn bevel(rect: Rect, light: Color32, shade: Color32) -> [Shape; 4] {
    [
        Shape::hline(rect.x_range(), rect.min.y + 0.5, Stroke::new(1.0, light)),
        Shape::vline(rect.min.x + 0.5, rect.y_range(), Stroke::new(1.0, light.gamma_multiply(0.7))),
        Shape::hline(rect.x_range(), rect.max.y - 0.5, Stroke::new(1.0, shade)),
        Shape::vline(rect.max.x - 0.5, rect.y_range(), Stroke::new(1.0, shade.gamma_multiply(0.8))),
    ]
}

/// The backdrop of a screen: TA's tiled plates of dark metal, some browner, some greyer or
/// darker (one warm gunmetal: a bluer plate read as a stray patch of another colour), with
/// seams between them and faint etched traces, all under the grime. Anchored to the screen.
pub fn backdrop(ctx: &Context, rect: Rect, base: Color32) -> Shape {
    const W: f32 = 168.0;
    const H: f32 = 124.0;
    let mut shapes = vec![Shape::rect_filled(rect, CornerRadius::ZERO, base)];
    let (i0, i1) = ((rect.min.x / W).floor() as i32, (rect.max.x / W).ceil() as i32);
    let (j0, j1) = ((rect.min.y / H).floor() as i32, (rect.max.y / H).ceil() as i32);
    let etch = Stroke::new(1.0, Color32::from_black_alpha(60));
    let etch_lit = Stroke::new(1.0, Color32::from_white_alpha(9));
    for j in j0..j1 {
        for i in i0..i1 {
            let whole = Rect::from_min_size(pos2(i as f32 * W, j as f32 * H), vec2(W, H));
            let tile = whole.intersect(rect);
            if tile.width() <= 0.0 || tile.height() <= 0.0 {
                continue;
            }
            let h = hash(i, j, 11);
            let tint = if h < 0.3 {
                Color32::from_rgba_unmultiplied(84, 58, 30, 30)
            } else if h < 0.55 {
                Color32::from_rgba_unmultiplied(44, 42, 38, 24)
            } else if h < 0.75 {
                Color32::from_black_alpha(34)
            } else {
                Color32::from_rgba_unmultiplied(60, 56, 40, 16)
            };
            shapes.push(Shape::rect_filled(tile, CornerRadius::ZERO, tint));
            // Etched traces: a run along the plate that turns twice, ending in a ring.
            for k in 0..2 {
                if hash(i, j, 12 + k) < 0.35 {
                    continue;
                }
                let r = |n: u32| hash(i, j, 20 + 10 * k + n);
                let start = pos2(whole.min.x + 14.0 + r(0) * (W - 28.0), whole.min.y + 12.0 + r(1) * (H - 24.0));
                let run = (r(2) - 0.5) * W * 0.9;
                let drop = (r(3) - 0.5) * H * 0.8;
                let tail = (r(4) - 0.5) * W * 0.5;
                let inner = whole.shrink(8.0);
                let points: Vec<Pos2> = [start, start + vec2(run, 0.0), start + vec2(run, drop), start + vec2(run + tail, drop)]
                    .iter()
                    .map(|p| p.clamp(inner.min, inner.max))
                    .collect();
                shapes.push(Shape::line(points.iter().map(|p| *p + vec2(1.0, 1.0)).collect(), etch_lit));
                shapes.push(Shape::circle_stroke(points[3], 2.0, etch));
                shapes.push(Shape::line(points, etch));
            }
            // Seams along the plate's top and left edges.
            if whole.min.y >= rect.min.y {
                shapes.push(Shape::hline(tile.x_range(), whole.min.y + 0.5, Stroke::new(1.0, Color32::from_black_alpha(130))));
                shapes.push(Shape::hline(tile.x_range(), whole.min.y + 1.5, Stroke::new(1.0, Color32::from_white_alpha(10))));
            }
            if whole.min.x >= rect.min.x {
                shapes.push(Shape::vline(whole.min.x + 0.5, tile.y_range(), Stroke::new(1.0, Color32::from_black_alpha(130))));
                shapes.push(Shape::vline(whole.min.x + 1.5, tile.y_range(), Stroke::new(1.0, Color32::from_white_alpha(8))));
            }
        }
    }
    shapes.push(grime_over(ctx, rect, 1.0));
    shapes.extend(backdrop_light(ctx.content_rect(), rect));
    Shape::Vec(shapes)
}

/// The light across a screen's backdrop, over the part of it in `rect`: a broad warm light
/// from above the screen, a little left of its middle, and the screen's edges and corners in
/// shade. Worked out where the screen is, so the backdrops of a screen's panels (each drawn
/// on its own) light as one.
fn backdrop_light(screen: Rect, rect: Rect) -> [Shape; 2] {
    // (The light, added; the shade, laid over: a grid fine enough for their smooth falls.)
    const COLUMNS: usize = 16;
    const ROWS: usize = 12;
    let cuts = |lo: f32, hi: f32, from: f32, to: f32, n: usize| {
        let mut at = vec![lo];
        at.extend((1..n).map(|k| egui::emath::lerp(from..=to, k as f32 / n as f32)).filter(|&x| x > lo + 0.5 && x < hi - 0.5));
        at.push(hi);
        at
    };
    let xs = cuts(rect.min.x, rect.max.x, screen.min.x, screen.max.x, COLUMNS);
    let ys = cuts(rect.min.y, rect.max.y, screen.min.y, screen.max.y, ROWS);
    let (w, h) = (screen.width().max(1.0), screen.height().max(1.0));
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let (mut light, mut shade) = (Mesh::default(), Mesh::default());
    for &y in &ys {
        for &x in &xs {
            let (u, v) = ((x - screen.min.x) / w, (y - screen.min.y) / h);
            // From above the screen: as far off as the screen is tall.
            let d = ((u - 0.4) * w / h).hypot(v + 0.35) / 1.25;
            let lit = 1.0 - smooth(0.25, 1.0, d);
            light.colored_vertex(pos2(x, y), Color32::from_rgba_premultiplied((10.0 * lit) as u8, (9.5 * lit) as u8, (8.0 * lit) as u8, 0));
            // The edges' shade, deepest in the corners.
            let r = ((u - 0.5) / 0.5).hypot((v - 0.48) / 0.52) / std::f32::consts::SQRT_2;
            shade.colored_vertex(pos2(x, y), Color32::from_black_alpha((110.0 * smooth(0.35, 1.0, r)) as u8));
        }
    }
    let n = xs.len() as u32;
    for j in 0..ys.len() as u32 - 1 {
        for i in 0..n - 1 {
            let at = j * n + i;
            for mesh in [&mut light, &mut shade] {
                mesh.add_triangle(at, at + 1, at + n);
                mesh.add_triangle(at + 1, at + n + 1, at + n);
            }
        }
    }
    [Shape::mesh(shade), Shape::mesh(light)]
}

/// A recessed well (lists, maps, text): dark, shadowed along its top and left, outlined in
/// black and lit along its lower edge.
pub fn well(rect: Rect, fill: Color32) -> Shape {
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), 8.0_f32.min(rect.height())));
    let left = Rect::from_min_size(rect.min, vec2(6.0_f32.min(rect.width()), rect.height()));
    Shape::Vec(vec![
        Shape::rect_filled(rect, CornerRadius::same(2), fill),
        vgrad(top, Color32::from_black_alpha(130), Color32::TRANSPARENT),
        hgrad(left, Color32::from_black_alpha(100), Color32::TRANSPARENT),
        Shape::rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, STEEL_EDGE), StrokeKind::Inside),
        Shape::hline((rect.min.x + 2.0)..=(rect.max.x - 1.0), rect.max.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(20))),
        Shape::vline(rect.max.x + 0.5, (rect.min.y + 2.0)..=rect.max.y, Stroke::new(1.0, Color32::from_white_alpha(12))),
    ])
}

/// An engraved seam across a plate (a rule): a dark line with a light edge below.
pub fn seam(x: Rangef, y: f32) -> Shape {
    Shape::Vec(vec![
        Shape::hline(x, y - 0.5, Stroke::new(1.0, Color32::from_black_alpha(150))),
        Shape::hline(x, y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(16))),
    ])
}

/// An engraved seam down a plate: a dark line with a light edge to its right.
pub fn seam_down(x: f32, y: Rangef) -> Shape {
    Shape::Vec(vec![
        Shape::vline(x - 0.5, y, Stroke::new(1.0, Color32::from_black_alpha(150))),
        Shape::vline(x + 0.5, y, Stroke::new(1.0, Color32::from_white_alpha(16))),
    ])
}

// ---- Pipes and bolts -----------------------------------------------------------------------

/// A straight steel pipe from `a` to `b` (horizontal or vertical), `width` thick, lit from
/// the top left like a cylinder.
pub fn pipe(a: Pos2, b: Pos2, width: f32) -> Shape {
    let w = width;
    let horizontal = (b.y - a.y).abs() < (b.x - a.x).abs();
    let rect = if horizontal {
        Rect::from_min_max(pos2(a.x.min(b.x), a.y - w / 2.0), pos2(a.x.max(b.x), a.y + w / 2.0))
    } else {
        Rect::from_min_max(pos2(a.x - w / 2.0, a.y.min(b.y)), pos2(a.x + w / 2.0, a.y.max(b.y)))
    };
    let mut shapes = vec![Shape::rect_filled(rect.expand(0.8), CornerRadius::same(1), STEEL_EDGE)];
    if horizontal {
        let shine = rect.min.y + rect.height() * 0.35;
        shapes.push(vgrad(Rect::from_min_max(rect.min, pos2(rect.max.x, shine)), STEEL_BODY, STEEL_SHINE));
        shapes.push(vgrad(Rect::from_min_max(pos2(rect.min.x, shine), rect.max), STEEL_SHINE, STEEL_SHADE));
    } else {
        let shine = rect.min.x + rect.width() * 0.35;
        shapes.push(hgrad(Rect::from_min_max(rect.min, pos2(shine, rect.max.y)), STEEL_BODY, STEEL_SHINE));
        shapes.push(hgrad(Rect::from_min_max(pos2(shine, rect.min.y), rect.max), STEEL_SHINE, STEEL_SHADE));
    }
    Shape::Vec(shapes)
}

/// A collar around a pipe at `c` (a band a little wider than it).
fn collar(c: Pos2, horizontal_pipe: bool, w: f32) -> Shape {
    let (along, across) = (5.0, w + 3.0);
    let (r, body, edge) = if horizontal_pipe {
        let r = Rect::from_center_size(c, vec2(along, across));
        (r, vgrad(r, STEEL_SHINE, STEEL_SHADE), Shape::vline(r.min.x + 1.0, r.y_range(), Stroke::new(1.0, Color32::from_white_alpha(90))))
    } else {
        let r = Rect::from_center_size(c, vec2(across, along));
        (r, hgrad(r, STEEL_SHINE, STEEL_SHADE), Shape::hline(r.x_range(), r.min.y + 1.0, Stroke::new(1.0, Color32::from_white_alpha(90))))
    };
    Shape::Vec(vec![Shape::rect_filled(r.expand(0.8), CornerRadius::same(1), STEEL_EDGE), body, edge])
}

/// A square joint plate where pipes meet or cross, `size` across, with a bolt in it.
pub fn joint(c: Pos2, size: f32) -> Shape {
    joint_plated(c, size, Color32::from_rgb(150, 148, 140), Color32::from_rgb(78, 76, 70))
}

/// A [`joint`] plated in `color` (brass, or red on a warning): an accent on a part that is
/// hardware, never a level.
pub fn accent_joint(c: Pos2, size: f32, color: Color32) -> Shape {
    joint_plated(c, size, color.lerp_to_gamma(Color32::WHITE, 0.25), color.lerp_to_gamma(Color32::BLACK, 0.45))
}

/// A joint plate shaded from `top` to `bottom`, its bolt steel.
fn joint_plated(c: Pos2, size: f32, top: Color32, bottom: Color32) -> Shape {
    let r = Rect::from_center_size(c, vec2(size, size));
    let mut shapes = vec![Shape::rect_filled(r.expand(1.0), CornerRadius::same(2), STEEL_EDGE), vgrad(r, top, bottom)];
    shapes.extend(bevel(r, Color32::from_white_alpha(110), Color32::from_black_alpha(120)));
    shapes.push(bolt(c, size * 0.26));
    Shape::Vec(shapes)
}

/// A steel pipe frame inside `rect`: `width` thick, its ends running past the corners where
/// joint plates hold them, collars along long runs. The frame keeps inside `rect`; its inner
/// edge is [`pipe_inset`] in from it.
pub fn pipe_frame(rect: Rect, width: f32) -> Shape {
    let w = width.max(2.0);
    let over = (w * 1.1).max(3.0);
    let path = pipe_frame_path(rect, width);
    let mut shapes = Vec::new();
    for x in [path.min.x, path.max.x] {
        shapes.push(pipe(pos2(x, path.min.y - over), pos2(x, path.max.y + over), w));
    }
    for y in [path.min.y, path.max.y] {
        shapes.push(pipe(pos2(path.min.x - over, y), pos2(path.max.x + over, y), w));
    }
    let runs = |length: f32| ((length / 300.0).floor() as usize).min(6);
    let (nx, ny) = (runs(path.width()), runs(path.height()));
    for i in 1..=nx {
        let x = path.min.x + path.width() * i as f32 / (nx + 1) as f32;
        shapes.push(collar(pos2(x, path.min.y), true, w));
        shapes.push(collar(pos2(x, path.max.y), true, w));
    }
    for i in 1..=ny {
        let y = path.min.y + path.height() * i as f32 / (ny + 1) as f32;
        shapes.push(collar(pos2(path.min.x, y), false, w));
        shapes.push(collar(pos2(path.max.x, y), false, w));
    }
    for corner in [path.left_top(), path.right_top(), path.left_bottom(), path.right_bottom()] {
        shapes.push(joint(corner, pipe_frame_joint(width)));
    }
    Shape::Vec(shapes)
}

/// The line a [`pipe_frame`]'s pipes run along: its corners are where its joints sit.
pub fn pipe_frame_path(rect: Rect, width: f32) -> Rect {
    let w = width.max(2.0);
    rect.shrink((w * 1.1).max(3.0) + w / 2.0)
}

/// The size of a [`pipe_frame`]'s corner joints.
pub fn pipe_frame_joint(width: f32) -> f32 {
    width.max(2.0) + 6.0
}

/// How far in from a [`pipe_frame`]'s rect its inner edge is.
pub fn pipe_inset(width: f32) -> f32 {
    let w = width.max(2.0);
    (w * 1.1).max(3.0) + w + 1.0
}

/// A horizontal pipe along `x` with a joint at each end: a rule between the parts of a
/// panel. `accent` plates both joints in that colour, so the rule is capped alike at each
/// end and nothing along it reads as a level.
pub fn pipe_rule(x: Rangef, y: f32, width: f32, accent: Option<Color32>) -> Shape {
    let mut shapes = vec![pipe(pos2(x.min, y), pos2(x.max, y), width)];
    for c in [pos2(x.min + 2.0, y), pos2(x.max - 2.0, y)] {
        shapes.push(match accent {
            Some(color) => accent_joint(c, width + 5.0, color),
            None => joint(c, width + 5.0),
        });
    }
    Shape::Vec(shapes)
}

/// A steel bezel around a picture (a map, a briefing, the minimap): a raised rim `width`
/// wide just outside `rect`, riveted at the corners when `rivets`.
pub fn bezel(rect: Rect, width: f32, rivets: bool) -> Shape {
    let ring = rect.expand(width / 2.0 + 1.0);
    let mut shapes = vec![
        Shape::rect_stroke(ring, CornerRadius::same(2), Stroke::new(width + 2.0, STEEL_EDGE), StrokeKind::Middle),
        Shape::rect_stroke(ring, CornerRadius::same(2), Stroke::new(width, Color32::from_rgb(92, 90, 84)), StrokeKind::Middle),
        Shape::hline(ring.x_range(), ring.min.y - width / 2.0 + 0.5, Stroke::new(1.0, Color32::from_white_alpha(80))),
        Shape::vline(ring.min.x - width / 2.0 + 0.5, ring.y_range(), Stroke::new(1.0, Color32::from_white_alpha(50))),
        Shape::hline(ring.x_range(), ring.max.y + width / 2.0 - 0.5, Stroke::new(1.0, Color32::from_black_alpha(120))),
        Shape::rect_stroke(rect.expand(0.5), CornerRadius::ZERO, Stroke::new(1.0, Color32::from_black_alpha(200)), StrokeKind::Middle),
    ];
    if rivets {
        for c in [ring.left_top(), ring.right_top(), ring.left_bottom(), ring.right_bottom()] {
            shapes.push(rivet(c, (width * 0.45).max(1.8)));
        }
    }
    Shape::Vec(shapes)
}

/// A bolt head: a steel disc with a shine, a slot and a shadow.
pub fn bolt(center: Pos2, radius: f32) -> Shape {
    let slot = vec2(radius * 0.62, -radius * 0.24);
    Shape::Vec(vec![
        Shape::circle_filled(center + vec2(0.6, 0.9), radius, Color32::from_black_alpha(150)),
        Shape::Circle(CircleShape { center, radius, fill: STEEL_BODY, stroke: Stroke::new(1.0, STEEL_EDGE) }),
        Shape::circle_filled(center + vec2(-radius * 0.28, -radius * 0.3), radius * 0.45, STEEL_SHINE.gamma_multiply(0.8)),
        Shape::line_segment([center - slot, center + slot], Stroke::new(1.0, STEEL_SHADE)),
    ])
}

/// A rivet: a small domed steel head.
pub fn rivet(center: Pos2, radius: f32) -> Shape {
    Shape::Vec(vec![
        Shape::circle_filled(center + vec2(0.5, 0.8), radius, Color32::from_black_alpha(140)),
        Shape::Circle(CircleShape { center, radius, fill: Color32::from_rgb(98, 96, 88), stroke: Stroke::new(0.8, STEEL_EDGE) }),
        Shape::circle_filled(center + vec2(-radius * 0.3, -radius * 0.3), radius * 0.4, Color32::from_white_alpha(120)),
    ])
}

// ---- Buttons -------------------------------------------------------------------------------

/// What a button is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slab {
    /// Gunmetal with cream lettering: TA's own buttons.
    Steel,
    /// Brass with dark lettering, for the main action.
    Brass,
    /// Rusted red, for what cannot be undone.
    Rust,
    /// A dark slot rimmed in steel (TA's menu slots), for rows of choices.
    Slot,
}

impl Slab {
    /// The colour of text on it (`hot` 0 to 1 when pointed at).
    pub fn text(self, hot: f32) -> Color32 {
        match self {
            Slab::Steel => CREAM.lerp_to_gamma(Color32::from_rgb(255, 250, 214), hot),
            Slab::Brass => ON_AMBER,
            Slab::Rust => Color32::from_rgb(250, 226, 212),
            Slab::Slot => TEXT.lerp_to_gamma(AMBER_BRIGHT, hot),
        }
    }

    /// Whether its lettering is light (and so outlined in black, as TA's is).
    pub fn light_text(self) -> bool {
        self != Slab::Brass
    }
}

/// A button's body, bevelled like TA's (lit along its top and left edges, shaded along the
/// others, a dark outline, scuffed): `hot` 0 to 1 lights it, `down` presses it in (and, as
/// TA's do, lights it up).
pub fn slab(ctx: &Context, rect: Rect, kind: Slab, hot: f32, down: bool) -> Shape {
    let r = CornerRadius::same(2);
    let (top, bottom) = match kind {
        Slab::Steel => (Color32::from_rgb(102, 103, 98), Color32::from_rgb(58, 59, 55)),
        Slab::Brass => (Color32::from_rgb(230, 196, 110), Color32::from_rgb(150, 114, 45)),
        Slab::Rust => (Color32::from_rgb(142, 62, 47), Color32::from_rgb(82, 32, 25)),
        Slab::Slot => (Color32::from_rgb(33, 32, 27), Color32::from_rgb(22, 21, 18)),
    };
    let lift = if down { 0.2 } else { 0.12 * hot };
    let (top, bottom) = (top.lerp_to_gamma(Color32::WHITE, lift), bottom.lerp_to_gamma(Color32::WHITE, lift));
    let body = rect.shrink(0.5);
    let mut shapes = vec![Shape::rect_filled(rect.expand(1.0), r, STEEL_EDGE), vgrad(body, top, bottom)];
    if kind == Slab::Slot {
        // Recessed: shadow at the top, a steel rim, a lit lower lip.
        shapes.push(vgrad(Rect::from_min_size(body.min, vec2(body.width(), 6.0_f32.min(body.height()))), Color32::from_black_alpha(110), Color32::TRANSPARENT));
        shapes.push(Shape::rect_stroke(body, r, Stroke::new(1.0, STEEL_SHADE.lerp_to_gamma(AMBER_DIM, hot)), StrokeKind::Inside));
        shapes.push(Shape::hline((body.min.x + 2.0)..=(body.max.x - 2.0), body.max.y - 1.0, Stroke::new(1.0, Color32::from_white_alpha(16))));
        return Shape::Vec(shapes);
    }
    // Scuffs and brushing, then the bevel: pressed, the light and shade swap sides.
    shapes.push(grime_over(ctx, body, if kind == Slab::Brass { 0.5 } else { 0.9 }));
    let (lit, shade) = (Color32::from_white_alpha(if kind == Slab::Brass { 120 } else { 72 }), Color32::from_black_alpha(120));
    let (upper, lower) = if down { (shade, lit) } else { (lit, shade) };
    shapes.extend(bevel(body.shrink(0.5), upper, lower));
    Shape::Vec(shapes)
}

/// A gunmetal slab shaped as an arrow plate (TA's previous and next buttons): pointed on the
/// left when `left`, else on the right.
pub fn arrow_slab(rect: Rect, left: bool, hot: f32, down: bool) -> Shape {
    let d = rect.height() * 0.5;
    let (x0, x1, y0, y1, cy) = (rect.min.x, rect.max.x, rect.min.y, rect.max.y, rect.center().y);
    let outline: Vec<Pos2> = if left {
        vec![pos2(x0, cy), pos2(x0 + d, y0), pos2(x1, y0), pos2(x1, y1), pos2(x0 + d, y1)]
    } else {
        vec![pos2(x0, y0), pos2(x1 - d, y0), pos2(x1, cy), pos2(x1 - d, y1), pos2(x0, y1)]
    };
    let lift = if down { 0.2 } else { 0.12 * hot };
    let top = Color32::from_rgb(106, 107, 102).lerp_to_gamma(Color32::WHITE, lift);
    let bottom = Color32::from_rgb(56, 57, 53).lerp_to_gamma(Color32::WHITE, lift);
    // A fan from the centre, shaded top to bottom.
    let shade = |p: Pos2| top.lerp_to_gamma(bottom, ((p.y - y0) / (y1 - y0)).clamp(0.0, 1.0));
    let mut mesh = Mesh::default();
    let c = rect.center();
    mesh.colored_vertex(c, shade(c));
    for p in &outline {
        mesh.colored_vertex(*p, shade(*p));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    // The outline, then light along the edges of the upper half (they face the light).
    let lit = Stroke::new(1.0, Color32::from_white_alpha(if down { 24 } else { 84 }));
    let mut shapes = vec![Shape::convex_polygon(outline.clone(), STEEL_EDGE, Stroke::new(2.0, STEEL_EDGE)), Shape::mesh(mesh)];
    for (a, b) in outline.iter().zip(outline.iter().cycle().skip(1)) {
        if a.y <= cy + 0.1 && b.y <= cy + 0.1 {
            shapes.push(Shape::line_segment([*a + vec2(0.0, 1.0), *b + vec2(0.0, 1.0)], lit));
        }
    }
    Shape::Vec(shapes)
}

/// A button's lettering, centred on `center` in `color`: light letters outlined in black
/// (TA's stencil caps), dark ones engraved (a light edge below). Pressed buttons' lettering
/// sinks a point.
pub fn slab_text(painter: &Painter, center: Pos2, galley: Arc<Galley>, light: bool, color: Color32, down: bool) {
    let pos = (center - galley.size() / 2.0 + vec2(0.0, if down { 1.0 } else { 0.0 })).round();
    if light {
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.0, 1.0)] {
            painter.galley_with_override_text_color(pos + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(190));
        }
    } else {
        painter.galley_with_override_text_color(pos + vec2(0.0, 1.0), galley.clone(), Color32::from_white_alpha(80));
    }
    painter.galley_with_override_text_color(pos, galley, color);
}

/// An indicator lamp: dark glass when off, lit and glowing when on (`on` 0 to 1).
pub fn lamp(center: Pos2, radius: f32, color: Color32, on: f32) -> Shape {
    faded_lamp(center, radius, color, on, 1.0)
}

/// A [`lamp`] drawn at `opacity` (0 to 1) as a whole, its rim and glass too: one that comes and
/// goes (over the battle) fades in and out rather than showing as a bare dark ball.
pub fn faded_lamp(center: Pos2, radius: f32, color: Color32, on: f32, opacity: f32) -> Shape {
    let opacity = opacity.clamp(0.0, 1.0);
    let mut shapes = vec![Shape::circle_filled(center, radius + 1.5, STEEL_EDGE.gamma_multiply(opacity))];
    if on > 0.0 {
        for (grow, alpha) in [(3.0, 0.07), (2.2, 0.14), (1.6, 0.26)] {
            shapes.push(Shape::circle_filled(center, radius * grow, color.gamma_multiply(alpha * on * opacity)));
        }
    }
    let glass = Color32::from_rgb(20, 26, 18).lerp_to_gamma(color, 0.15 + 0.85 * on).gamma_multiply(opacity);
    shapes.push(Shape::Circle(CircleShape { center, radius, fill: glass, stroke: Stroke::new(1.0, Color32::from_black_alpha((160.0 * opacity) as u8)) }));
    shapes.push(Shape::circle_filled(center + vec2(-radius * 0.3, -radius * 0.35), radius * 0.38, Color32::from_white_alpha(((50.0 + 130.0 * on) * opacity) as u8)));
    Shape::Vec(shapes)
}

// ---- Lettering -----------------------------------------------------------------------------

/// A soft dark halo round what `shape` paints (at an offset, in a colour), out to `radius`:
/// copies of it in black on rings round it, fainter outwards as a blur fades, `strength` how
/// dark (1: some 40% black along a letter's edge). Lettering straight on the battle reads
/// over bright ground with it as over the night, and the halo follows the letters' own shapes
/// (a pool behind them showed its edge over bright ground).
pub fn halo(painter: &Painter, radius: f32, strength: f32, shape: impl Fn(egui::Vec2, Color32) -> Shape) {
    const RINGS: usize = 6;
    let sigma = radius / 2.5;
    // (A ring's share: a blur's weight at its radius by how much ground the ring covers.)
    let weight = |r: f32| r * (-(r * r) / (2.0 * sigma * sigma)).exp();
    let rings: Vec<(f32, usize)> = (1..=RINGS).map(|k| (radius * k as f32 / RINGS as f32, 6 + 4 * k)).collect();
    let total: f32 = rings.iter().map(|&(r, _)| weight(r)).sum();
    let mut shapes = Vec::new();
    for (k, &(r, n)) in rings.iter().enumerate() {
        let alpha = 1.2 * strength * weight(r) / total / n as f32;
        let color = Color32::from_black_alpha((alpha * 255.0).round().clamp(1.0, 255.0) as u8);
        for i in 0..n {
            // (Each ring turned half a step on from the last, so their copies do not line up.)
            let a = std::f32::consts::TAU * (i as f32 + 0.5 * k as f32) / n as f32;
            shapes.push(shape(vec2(a.cos(), a.sin()) * r, color));
        }
    }
    painter.add(Shape::Vec(shapes));
}

/// Text as TA writes its titles: light, outlined in black, with a drop shadow.
pub fn embossed(painter: &Painter, pos: Pos2, galley: Arc<Galley>, color: Color32) {
    painter.galley_with_override_text_color(pos + vec2(2.0, 2.5), galley.clone(), Color32::from_black_alpha(140));
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.0, 1.0), (-1.0, 1.0)] {
        painter.galley_with_override_text_color(pos + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(215));
    }
    painter.galley_with_override_text_color(pos, galley, color);
}

/// [`embossed`] text as a label.
pub fn embossed_label(ui: &mut Ui, text: RichText, color: Color32) -> Response {
    let galley = crate::widgets::galley(ui, text);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(2.0, 2.0), Sense::hover());
    embossed(ui.painter(), rect.min + vec2(1.0, 0.0), galley, color);
    response
}

/// Small text straight on the battle (a footer's), with a thin dark rim round each letter and
/// a soft shadow, so it reads over pale ground and passing units.
pub fn outlined_label(ui: &mut Ui, text: RichText, color: Color32) -> Response {
    let galley = crate::widgets::galley(ui, text);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(2.0, 2.0), Sense::hover());
    outlined(ui.painter(), rect.min + vec2(1.0, 1.0), galley, color);
    response
}

/// [`outlined_label`]'s lettering at `pos`.
pub fn outlined(painter: &Painter, pos: Pos2, galley: Arc<Galley>, color: Color32) {
    painter.galley_with_override_text_color(pos + vec2(0.5, 1.5), galley.clone(), Color32::from_black_alpha(120));
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        painter.galley_with_override_text_color(pos + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(150));
    }
    painter.galley_with_override_text_color(pos, galley, color);
}

/// Text in polished steel (the title screen's lockup, as TA's logo is): bright above, a
/// bright horizon across the middle of the capitals, dark just below it, lighter at the foot;
/// outlined and shadowed.
pub fn chrome(painter: &Painter, pos: Pos2, galley: Arc<Galley>) {
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley_with_override_text_color(pos + vec2(2.0, 3.5), galley.clone(), Color32::from_black_alpha(170));
    for (dx, dy) in [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
        painter.galley_with_override_text_color(pos + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(225));
    }
    // Down the capitals (0 at their top, 1 at the baseline): the sky fading towards the
    // horizon, the horizon, then the ground, lightening towards the foot.
    let shade = |t: f32| -> u8 {
        let v = if t < 0.45 {
            252.0 - 40.0 * (t / 0.45)
        } else if t < 0.5 {
            255.0
        } else {
            100.0 + 80.0 * ((t - 0.5) / 0.5).clamp(0.0, 1.0)
        };
        v as u8
    };
    let (top, base) = cap_band(&galley);
    let c = base - top;
    const STEPS: usize = 24;
    for k in 0..STEPS {
        let (t0, t1) = (k as f32 / STEPS as f32, (k + 1) as f32 / STEPS as f32);
        // (The first and last bands reach past the capitals, for the letters that do.)
        let y0 = if k == 0 { rect.min.y - 4.0 } else { pos.y + top + t0 * c };
        let y1 = if k + 1 == STEPS { rect.max.y + 4.0 } else { pos.y + top + t1 * c };
        let v = shade((t0 + t1) / 2.0);
        let band = painter.with_clip_rect(Rect::from_min_max(pos2(rect.min.x - 4.0, y0), pos2(rect.max.x + 4.0, y1)));
        band.galley_with_override_text_color(pos, galley.clone(), Color32::from_rgb(v, v, v.saturating_sub(4)));
    }
}

/// Where the capitals of a line of text are: the top and the foot of its first letter's ink,
/// relative to the galley (an estimate when it has no letter). (The ink, not the baseline:
/// the theme moves the glyphs down in their line.)
pub fn cap_band(galley: &Galley) -> (f32, f32) {
    for placed in &galley.rows {
        for glyph in &placed.row.glyphs {
            if glyph.chr.is_alphanumeric() && !glyph.uv_rect.is_nothing() {
                let top = placed.pos.y + glyph.pos.y + glyph.uv_rect.offset.y;
                return (top, top + glyph.uv_rect.size.y);
            }
        }
    }
    let h = galley.size().y;
    (h * 0.1, h * 0.7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grime_tiles_and_stays_faint() {
        for (image, size) in [(grime_image(), GRIME), (stain_image(), STAINS)] {
            assert_eq!(image.size, [size, size]);
            let alpha = |x: usize, y: usize| image.pixels[y * size + x].a() as i32;
            assert!(image.pixels.iter().all(|p| p.a() <= 72), "faint enough to lay over any plate");
            // Wrapping noise: across the edge where it tiles, no more change than inside.
            let seam: i32 = (0..size).map(|y| (alpha(0, y) - alpha(size - 1, y)).abs()).sum();
            let inside: i32 = (0..size).map(|y| (alpha(100, y) - alpha(101, y)).abs()).sum();
            assert!(seam < inside * 3 + size as i32, "no seam where it tiles ({seam} vs {inside})");
        }
    }

    #[test]
    fn a_pipe_frame_keeps_inside_its_rect() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
        let bounds = pipe_frame(rect, 5.0).visual_bounding_rect();
        assert!(rect.expand(1.5).contains_rect(bounds), "{bounds:?} in {rect:?}");
        assert!(pipe_inset(5.0) > 5.0);
    }
}
