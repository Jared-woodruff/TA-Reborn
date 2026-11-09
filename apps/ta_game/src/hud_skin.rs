//! The modern HUD's materials: the player's side's own panel metal from TA's interface
//! (`bitmaps/<side>guisidetile.pcx` and `<side>guitoptile.pcx`, the tiles TA fills wide
//! screens with), read from the install and tiled behind the command deck's plates and the
//! resource bar, with bevelled edges; `ta_ui::metal`'s plate when a side (a mod's) has none.
//! On them: steel bezels around the pictures, gunmetal buttons, recessed bars. The tiles are
//! quietened as they are read (see [`quieten`]) and drawn smoothly: as TA drew them, a texel to
//! a pixel, their palette's dithering is metal's grain; enlarged, it showed as coloured specks.

use egui::{Color32, Context, Rect, Shape, Stroke, TextureHandle, TextureOptions, TextureWrapMode, Vec2, pos2, vec2};
use ta_formats::pcx::PcxImage;
use ta_ui::metal;
use ta_vfs::Vfs;

/// A side's panel tiles, loaded when the side is first shown.
#[derive(Default)]
pub struct Skin {
    side: String,
    side_tile: Option<Tile>,
    top_tile: Option<Tile>,
}

struct Tile {
    texture: TextureHandle,
    size: Vec2,
}

impl Skin {
    /// The tiles of `side` (read again when the side changes).
    pub fn load(&mut self, ctx: &Context, vfs: &Vfs, side: &str) {
        let side = side.to_ascii_lowercase();
        if self.side != side {
            self.side_tile = tile(ctx, vfs, &format!("bitmaps/{side}guisidetile.pcx"), false);
            self.top_tile = tile(ctx, vfs, &format!("bitmaps/{side}guitoptile.pcx"), true);
            self.side = side;
        }
    }

    /// One of the command deck's plates over `rect`: TA's metal, lit along its top and left
    /// edges and shaded opposite, outlined, its shadow on the battlefield; riveted in its
    /// corners when `rivets`.
    pub fn deck(&self, ctx: &Context, rect: Rect, rivets: bool) -> Shape {
        let mut shapes = vec![
            egui::Shadow { offset: [0, 3], blur: 16, spread: 1, color: Color32::from_black_alpha(120) }.as_shape(rect, 3.0).into(),
            Shape::rect_filled(rect.expand(1.0), egui::CornerRadius::same(3), metal::STEEL_EDGE),
            tiled(ctx, self.side_tile.as_ref(), rect, true),
            Shape::hline(rect.x_range(), rect.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(48))),
            Shape::vline(rect.min.x + 0.5, rect.y_range(), Stroke::new(1.0, Color32::from_white_alpha(24))),
            Shape::hline(rect.x_range(), rect.max.y - 0.5, Stroke::new(1.0, Color32::from_black_alpha(160))),
            Shape::vline(rect.max.x - 0.5, rect.y_range(), Stroke::new(1.0, Color32::from_black_alpha(130))),
        ];
        if rivets {
            let inset = 4.5;
            for c in [rect.left_top() + vec2(inset, inset), rect.right_top() + vec2(-inset, inset), rect.left_bottom() + vec2(inset, -inset), rect.right_bottom() + vec2(-inset, -inset)] {
                shapes.push(metal::rivet(c, 1.8));
            }
        }
        Shape::Vec(shapes)
    }

    /// The resource bar's metal over `rect`: TA's tile keeps its own lower edge (stretched
    /// to the bar's height); the plate gets a bevel.
    pub fn top(&self, ctx: &Context, rect: Rect) -> Shape {
        let mut shapes = vec![tiled(ctx, self.top_tile.as_ref(), rect, false)];
        if self.top_tile.is_none() {
            shapes.push(Shape::hline(rect.x_range(), rect.max.y - 2.5, Stroke::new(1.0, Color32::from_white_alpha(40))));
            shapes.push(Shape::hline(rect.x_range(), rect.max.y - 1.0, Stroke::new(2.0, Color32::from_black_alpha(220))));
        }
        Shape::Vec(shapes)
    }
}

fn tiled(ctx: &Context, tile: Option<&Tile>, rect: Rect, repeat_down: bool) -> Shape {
    match tile {
        Some(t) => {
            // (Stretched down, its first and last rows' middles at the ends: smoothly drawn,
            // the texture wrapping, half a row of the other end would show at each.)
            let (v0, v1) = if repeat_down { (rect.min.y / t.size.y, rect.max.y / t.size.y) } else { (0.5 / t.size.y, 1.0 - 0.5 / t.size.y) };
            let uv = Rect::from_min_max(pos2(rect.min.x / t.size.x, v0), pos2(rect.max.x / t.size.x, v1));
            Shape::image(t.texture.id(), rect, uv, Color32::WHITE)
        }
        None => metal::plate(ctx, rect, crate::theme::PANEL),
    }
}

/// The tile on one of TA's tile sheets: the pixels that are not the sheet's backdrop colour
/// (the colour of its first pixel), quietened (`bevelled`: the resource bar's, with a bevel
/// along its foot).
fn tile(ctx: &Context, vfs: &Vfs, path: &str, bevelled: bool) -> Option<Tile> {
    let pcx = PcxImage::parse(&vfs.read_opt(path)?).ok()?;
    let (w, h) = (pcx.width as usize, pcx.height as usize);
    let rgba = pcx.to_rgba();
    let (x0, y0, x1, y1) = tile_bounds(&rgba, w, h)?;
    let (tw, th) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut colours = Vec::with_capacity(tw * th);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = (y * w + x) * 4;
            colours.push([rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32]);
        }
    }
    quieten(&mut colours, tw, th, bevelled);
    let pixels: Vec<u8> = colours.iter().flat_map(|c| [c[0], c[1], c[2], 255.0].map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
    let image = egui::ColorImage::from_rgba_unmultiplied([tw, th], &pixels);
    let options = TextureOptions { wrap_mode: TextureWrapMode::Repeat, ..TextureOptions::LINEAR };
    Some(Tile { texture: ctx.load_texture(path, image, options), size: vec2(tw as f32, th as f32) })
}

/// A tile (`w`×`h` colours) made a quiet material: each pixel's colour drawn most of the way
/// to the tile's own tint at its brightness (TA's palette dithering scatters the hue from pixel
/// to pixel: specks of green, blue and purple), and the dither softened, half way to its
/// neighbourhood's mean (the tile repeats across, and down unless `bevelled`). A `bevelled`
/// tile's foot (its last rows, from the first well brighter than its body) is a bevel: each of
/// its rows one even colour.
fn quieten(colours: &mut [[f32; 3]], w: usize, h: usize, bevelled: bool) {
    const CHROMA: f32 = 0.25;
    const SOFTEN: f32 = 0.5;
    let lum = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let row_lum = |y: usize| colours[y * w..(y + 1) * w].iter().map(|&c| lum(c)).sum::<f32>() / w as f32;
    let body_lum = (0..h * 3 / 4).map(row_lum).sum::<f32>() / (h * 3 / 4).max(1) as f32;
    let body = if bevelled { (h * 3 / 4..h).find(|&y| row_lum(y) > body_lum * 1.6 + 2.0).unwrap_or(h) } else { h };
    // The tint: the body's mean colour.
    let mut tint = [0.0f32; 3];
    for c in &colours[..body * w] {
        (0..3).for_each(|i| tint[i] += c[i]);
    }
    let n = (body * w).max(1) as f32;
    let tint = tint.map(|v| v / n);
    let tint_lum = lum(tint).max(1.0);
    for c in colours.iter_mut() {
        let k = lum(*c) / tint_lum;
        *c = std::array::from_fn(|i| tint[i] * k + (c[i] - tint[i] * k) * CHROMA);
    }
    let source = colours.to_vec();
    for y in 0..body {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for dy in [-1i32, 0, 1] {
                // (Across, the tile repeats; down, it repeats too, or stops at the body's ends.)
                let yy = if bevelled { (y as i32 + dy).clamp(0, body as i32 - 1) as usize } else { (y as i32 + dy).rem_euclid(h as i32) as usize };
                for dx in [-1i32, 0, 1] {
                    let xx = (x as i32 + dx).rem_euclid(w as i32) as usize;
                    (0..3).for_each(|i| sum[i] += source[yy * w + xx][i]);
                }
            }
            let c = &mut colours[y * w + x];
            (0..3).for_each(|i| c[i] = c[i] * (1.0 - SOFTEN) + sum[i] / 9.0 * SOFTEN);
        }
    }
    for y in body..h {
        let row = &mut colours[y * w..(y + 1) * w];
        let mut mean = [0.0f32; 3];
        row.iter().for_each(|c| (0..3).for_each(|i| mean[i] += c[i] / w as f32));
        row.fill(mean);
    }
}

/// The box (inclusive) of the pixels of an RGBA sheet `w`×`h` that differ from its first,
/// if it is big enough to tile.
fn tile_bounds(rgba: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let key = rgba.get(..3)?;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if &rgba[i..i + 3] != key {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    (x0 != usize::MAX && x1 >= x0 + 7 && y1 >= y0 + 7).then_some((x0, y0, x1, y1))
}

/// A steel bezel around a picture (see [`metal::bezel`]).
pub fn bezel(rect: Rect, width: f32, rivets: bool) -> Shape {
    metal::bezel(rect, width, rivets)
}

/// A recessed bar filled to `fraction` in `color` (lighter above, darker below), with a
/// tick every `ticks`-th of it.
pub fn gauge(rect: Rect, fraction: f32, color: Color32, ticks: u32) -> Shape {
    let mut shapes = vec![metal::well(rect, crate::theme::INSET)];
    let fill = Rect::from_min_size(rect.min, vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height())).shrink(2.0);
    if fill.width() > 0.0 {
        shapes.push(metal::vgrad(fill, color.lerp_to_gamma(Color32::WHITE, 0.25), color.lerp_to_gamma(Color32::BLACK, 0.35)));
        shapes.push(Shape::hline(fill.x_range(), fill.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(70))));
    }
    for k in 1..ticks {
        let x = rect.min.x + rect.width() * k as f32 / ticks as f32;
        shapes.push(Shape::vline(x, (rect.max.y - 4.0)..=(rect.max.y - 1.0), Stroke::new(1.0, Color32::from_black_alpha(150))));
    }
    Shape::Vec(shapes)
}

/// The colour of a health bar at `fraction`: TA's green, through yellow, to red.
pub fn health_color(fraction: f32) -> Color32 {
    let f = fraction.clamp(0.0, 1.0);
    if f > 0.5 {
        Color32::from_rgb(214, 196, 60).lerp_to_gamma(crate::theme::GREEN, (f - 0.5) * 2.0)
    } else {
        crate::theme::RED.lerp_to_gamma(Color32::from_rgb(214, 196, 60), f * 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_is_what_differs_from_the_backdrop() {
        // A 12 × 10 sheet of backdrop (1,2,3) with an 8 × 8 tile at (3, 2).
        let (w, h) = (12, 10);
        let mut rgba = [1u8, 2, 3, 255].repeat(w * h);
        for y in 2..10 {
            for x in 3..11 {
                let i = (y * w + x) * 4;
                rgba[i..i + 3].copy_from_slice(&[40, 40, 40]);
            }
        }
        assert_eq!(tile_bounds(&rgba, w, h), Some((3, 2, 10, 9)));
        assert_eq!(tile_bounds(&[1u8, 2, 3, 255].repeat(w * h), w, h), None, "a sheet of backdrop only");
    }

    #[test]
    fn a_tile_is_quietened_and_its_bevel_made_even() {
        // A grey body speckled with a purple and a green pixel in turn, over a bevel of two
        // rows of light grey dotted with blue.
        let (w, h) = (8, 12);
        let mut colours: Vec<[f32; 3]> = (0..w * h).map(|i| if i % 2 == 0 { [60.0, 20.0, 80.0] } else { [20.0, 60.0, 20.0] }).collect();
        for y in 10..12 {
            for x in 0..w {
                colours[y * w + x] = if x % 2 == 0 { [90.0, 90.0, 90.0] } else { [70.0, 70.0, 140.0] };
            }
        }
        quieten(&mut colours, w, h, true);
        let chroma = |c: &[f32; 3]| c.iter().cloned().fold(f32::MIN, f32::max) - c.iter().cloned().fold(f32::MAX, f32::min);
        assert!(colours[..10 * w].iter().all(|c| chroma(c) < 25.0), "the specks' hue is drawn in: {:?}", &colours[..2]);
        for y in 10..12 {
            assert!(colours[y * w..(y + 1) * w].windows(2).all(|p| p[0] == p[1]), "bevel row {y} is even");
        }
    }

    #[test]
    fn health_runs_green_to_red() {
        assert_eq!(health_color(1.0), crate::theme::GREEN);
        assert_eq!(health_color(0.0), crate::theme::RED);
        let half = health_color(0.5);
        assert!(half.r() > 150 && half.g() > 150, "yellow halfway: {half:?}");
    }
}
