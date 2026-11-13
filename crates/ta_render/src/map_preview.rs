//! A map's picture for the menus' map pickers: drawn from the map's own art at the size it is
//! shown, not TA's stored minimap (252 pixels at most, each pixel snapped to the nearest
//! palette colour: flat greens, olive deserts, pink speckle, stray pixels, blurred when
//! blown up). The art over the minimap's window is averaged in linear light (a box filter,
//! so dithering becomes the colour it stands for), and the heights add a little relief, lit
//! from the north-west as TA's art is.

use ta_formats::image::Palette;
use ta_formats::tnt::{CELL_SIZE, TILE_SIZE, TntMap, minimap_window};

use crate::util::{linear_to_srgb, srgb_to_linear_lut};

/// A map's picture.
pub struct MapPicture {
    pub width: usize,
    pub height: usize,
    /// Opaque sRGB pixels, row by row.
    pub rgba: Vec<u8>,
    /// The ground it shows in world pixels, from the map's top left corner (the minimap's
    /// window: start positions are placed on it as on the minimap).
    pub area: (f32, f32),
}

/// How strongly the heights shade the picture (0: the art as painted).
const RELIEF: f32 = 0.35;

/// The picture of `map`'s minimap window, `long_side` pixels along its longer side (at most
/// the art's own size).
pub fn render(map: &TntMap, palette: &Palette, long_side: u32) -> MapPicture {
    render_cells(map, palette, long_side, minimap_window(map.width, map.height))
}

/// The picture of the whole of `map` (the battle's minimap: the minimap's window leaves out
/// ground the battle shows), `long_side` pixels along its longer side.
pub fn render_whole(map: &TntMap, palette: &Palette, long_side: u32) -> MapPicture {
    render_cells(map, palette, long_side, (map.width, map.height))
}

/// The picture of the `ww` × `wh` cells from `map`'s top left.
fn render_cells(map: &TntMap, palette: &Palette, long_side: u32, (ww, wh): (u32, u32)) -> MapPicture {
    let art_w = (ww as usize * CELL_SIZE).min(map.width_px() as usize).max(1);
    let art_h = (wh as usize * CELL_SIZE).min(map.height_px() as usize).max(1);
    let long = (long_side as usize).clamp(1, art_w.max(art_h));
    let (out_w, out_h) = if art_w >= art_h { (long, (long * art_h / art_w).max(1)) } else { ((long * art_w / art_h).max(1), long) };
    // The art in blocks of `r` pixels (each the mean of its block), as fine as needed for a
    // block across an output pixel at least (a cell's 16 at most: the window is whole cells);
    // the blocks' means, weighted by how much of each an output pixel covers, are the art's.
    let mut r = 16;
    while r > 1 && (art_w / r < out_w || art_h / r < out_h) {
        r /= 2;
    }
    let blocks = blocks(map, palette, art_w, art_h, r);
    let (bw, bh) = (art_w.div_ceil(r), art_h.div_ceil(r));
    let picture = area_resample(&blocks, bw, bh, out_w, out_h);
    let shade = relief(map, art_w, art_h, out_w, out_h);
    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    for (c, f) in picture.iter().zip(&shade) {
        rgba.extend([linear_to_srgb(c[0] * f), linear_to_srgb(c[1] * f), linear_to_srgb(c[2] * f), 255]);
    }
    MapPicture { width: out_w, height: out_h, rgba, area: (art_w as f32, art_h as f32) }
}

/// The art of the window `art_w` × `art_h` in blocks of `r` × `r` pixels, each its mean in
/// linear light; each tile's blocks worked out once, however often it is laid.
fn blocks(map: &TntMap, palette: &Palette, art_w: usize, art_h: usize, r: usize) -> Vec<[f32; 3]> {
    let lut = srgb_to_linear_lut();
    let colour: Vec<[f32; 3]> = palette.colors.iter().map(|c| [lut[c[0] as usize], lut[c[1] as usize], lut[c[2] as usize]]).collect();
    let per = TILE_SIZE / r;
    let mut tiles: Vec<Option<Vec<[f32; 3]>>> = vec![None; map.tiles.len()];
    let tiles_across = map.width_px() as usize / TILE_SIZE;
    let (bw, bh) = (art_w.div_ceil(r), art_h.div_ceil(r));
    let mut out = vec![[0.0f32; 3]; bw * bh];
    for ty in 0..art_h.div_ceil(TILE_SIZE) {
        for tx in 0..art_w.div_ceil(TILE_SIZE) {
            let index = map.tile_map.get(ty * tiles_across + tx).copied().unwrap_or(0) as usize;
            let Some(tile) = map.tiles.get(index) else { continue };
            let means = tiles[index].get_or_insert_with(|| {
                let mut means = vec![[0.0f32; 3]; per * per];
                for y in 0..TILE_SIZE {
                    for x in 0..TILE_SIZE {
                        let c = colour[tile[y * TILE_SIZE + x] as usize];
                        let m = &mut means[(y / r) * per + x / r];
                        m[0] += c[0];
                        m[1] += c[1];
                        m[2] += c[2];
                    }
                }
                let n = (r * r) as f32;
                means.iter_mut().for_each(|m| m.iter_mut().for_each(|v| *v /= n));
                means
            });
            for by in 0..per {
                let y = ty * per + by;
                if y >= bh {
                    break;
                }
                for bx in 0..per {
                    let x = tx * per + bx;
                    if x < bw {
                        out[y * bw + x] = means[by * per + bx];
                    }
                }
            }
        }
    }
    out
}

/// `src` (`sw` × `sh`) down to `dw` × `dh`, each pixel the mean of what it covers (parts of
/// pixels counted in part); across, then down.
fn area_resample(src: &[[f32; 3]], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<[f32; 3]> {
    // Each output pixel's spans of input pixels: (first, weights).
    let spans = |s: usize, d: usize| -> Vec<(usize, Vec<f32>)> {
        let scale = s as f32 / d as f32;
        (0..d)
            .map(|o| {
                let (a, b) = (o as f32 * scale, ((o + 1) as f32 * scale).min(s as f32));
                let first = (a.floor() as usize).min(s - 1);
                let last = ((b.ceil() as usize).max(first + 1)).min(s);
                let weights: Vec<f32> = (first..last).map(|i| (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0)).collect();
                let total: f32 = weights.iter().sum::<f32>().max(1e-6);
                (first, weights.into_iter().map(|w| w / total).collect())
            })
            .collect()
    };
    let (across, down) = (spans(sw, dw), spans(sh, dh));
    let mut rows = vec![[0.0f32; 3]; dw * sh];
    for y in 0..sh {
        for (x, (first, weights)) in across.iter().enumerate() {
            let mut c = [0.0f32; 3];
            for (k, w) in weights.iter().enumerate() {
                let s = src[y * sw + first + k];
                c[0] += s[0] * w;
                c[1] += s[1] * w;
                c[2] += s[2] * w;
            }
            rows[y * dw + x] = c;
        }
    }
    let mut out = vec![[0.0f32; 3]; dw * dh];
    for (y, (first, weights)) in down.iter().enumerate() {
        for x in 0..dw {
            let mut c = [0.0f32; 3];
            for (k, w) in weights.iter().enumerate() {
                let s = rows[(first + k) * dw + x];
                c[0] += s[0] * w;
                c[1] += s[1] * w;
                c[2] += s[2] * w;
            }
            out[y * dw + x] = c;
        }
    }
    out
}

/// How much brighter or darker each output pixel is for the slope under it: lit from the
/// north-west, flat ground (and the sea's bed) as painted.
fn relief(map: &TntMap, art_w: usize, art_h: usize, out_w: usize, out_h: usize) -> Vec<f32> {
    let cell = CELL_SIZE as f32;
    // The heights as a smooth field, in world pixels (a cell's height at its middle).
    let height = |x: f32, z: f32| -> f32 {
        let (fx, fz) = ((x / cell - 0.5).max(0.0), (z / cell - 0.5).max(0.0));
        let (x0, z0) = (fx.floor() as u32, fz.floor() as u32);
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let h = |x: u32, z: u32| map.height_at(x, z) as f32;
        let top = h(x0, z0) + (h(x0 + 1, z0) - h(x0, z0)) * tx;
        let bottom = h(x0, z0 + 1) + (h(x0 + 1, z0 + 1) - h(x0, z0 + 1)) * tx;
        top + (bottom - top) * tz
    };
    let sea = map.sea_level as f32;
    let light = [-0.55f32, 0.75, -0.55];
    let len = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
    let light = light.map(|v| v / len);
    let (sx, sz) = (art_w as f32 / out_w as f32, art_h as f32 / out_h as f32);
    let step = sx.max(sz).max(8.0);
    let mut out = Vec::with_capacity(out_w * out_h);
    for oy in 0..out_h {
        for ox in 0..out_w {
            let (x, y) = ((ox as f32 + 0.5) * sx, (oy as f32 + 0.5) * sz);
            // (The art draws ground h high h/2 further up: the ground under this pixel.)
            let mut z = y;
            for _ in 0..2 {
                z = y + height(x, z) * 0.5;
            }
            let h = height(x, z);
            if h <= sea {
                out.push(1.0);
                continue;
            }
            let gx = (height(x + step, z) - height(x - step, z)) / (2.0 * step);
            let gz = (height(x, z + step) - height(x, z - step)) / (2.0 * step);
            let n = [-gx, 1.0, -gz];
            let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / nl / light[1];
            out.push((1.0 + RELIEF * (lit - 1.0)).clamp(0.55, 1.35));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map `cells` cells square, every tile the same `tile`, flat at `height`.
    fn map(cells: u32, tile: [u8; TILE_SIZE * TILE_SIZE], height: u8) -> TntMap {
        let tiles_across = (cells / 2) as usize;
        TntMap {
            width: cells,
            height: cells,
            sea_level: 0,
            tiles: vec![Box::new(tile)],
            tile_map: vec![0; tiles_across * tiles_across],
            heights: vec![height; (cells * cells) as usize],
            features: vec![0; (cells * cells) as usize],
            feature_names: Vec::new(),
            minimap: ta_formats::image::IndexedImage::new(1, 1, 0),
            flags: Vec::new(),
        }
    }

    #[test]
    fn dithering_becomes_the_colour_it_stands_for() {
        let mut palette = Palette::default();
        palette.colors[1] = [255, 0, 0];
        palette.colors[2] = [0, 0, 255];
        // A checkerboard of two colours: a picture of one even colour, their mean in light.
        let tile: [u8; TILE_SIZE * TILE_SIZE] = std::array::from_fn(|i| if (i % TILE_SIZE + i / TILE_SIZE).is_multiple_of(2) { 1 } else { 2 });
        let p = render(&map(64, tile, 40), &palette, 100);
        let mean = linear_to_srgb(0.5);
        assert!(p.rgba.chunks(4).all(|c| c[0].abs_diff(mean) <= 1 && c[1] == 0 && c[2].abs_diff(mean) <= 1), "{:?}", &p.rgba[..8]);
        // The minimap's window (two cells off the right, eight off the foot), 100 across.
        assert_eq!((p.width, p.height), (100, 100 * 56 / 62));
        assert_eq!(p.area, (62.0 * 16.0, 56.0 * 16.0));
    }

    #[test]
    fn the_whole_map_for_the_battle() {
        let p = render_whole(&map(64, [3; TILE_SIZE * TILE_SIZE], 0), &Palette::default(), 128);
        assert_eq!((p.width, p.height), (128, 128));
        assert_eq!(p.area, (64.0 * 16.0, 64.0 * 16.0));
    }

    #[test]
    fn never_larger_than_the_art() {
        // Eight cells: a window six across, one down (96 by 16 pixels of art).
        let p = render(&map(8, [3; TILE_SIZE * TILE_SIZE], 0), &Palette::default(), 4000);
        assert_eq!((p.width, p.height), (96, 16));
    }
}
