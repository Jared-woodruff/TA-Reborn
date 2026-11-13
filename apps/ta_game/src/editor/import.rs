//! Pictures brought into the map, and its heights taken out: a grey picture as the ground's
//! heights (black low, white high), any picture as its ground's art (in TA's palette, cut into
//! tiles: the whole map, or a brush of tiles to paint with), and the heights as a grey picture.
//!
//! A picture is resampled a row at a time (a large map's art is never held whole), each pixel
//! the triangle-filtered mean of the picture under it, and put into the palette by a table of
//! the nearest colours, with Floyd–Steinberg dithering when asked.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{GrayImage, RgbaImage};
use ta_formats::image::Palette;
use ta_formats::tnt::TILE_SIZE;

use super::doc::{MapDoc, Tile};
use super::tools::TileBlock;

/// The largest file and picture read: bigger ones are refused.
const MAX_FILE: u64 = 256 << 20;
const MAX_SIDE: u32 = 16384;

/// Reads a picture (PNG or JPEG).
pub fn read(path: &Path) -> Result<RgbaImage> {
    let size = std::fs::metadata(path).with_context(|| path.display().to_string())?.len();
    if size > MAX_FILE {
        bail!(ta_ui::i18n::t("The picture is too large (256 MB at most)."));
    }
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    let picture = reader.decode().map_err(|e| anyhow::anyhow!(ta_ui::i18n::tf("Not a picture the editor can read ({error})", &[("error", &e)])))?;
    Ok(picture.to_rgba8())
}

/// The picture as the map's heights: `w` × `h` cells, black at `lowest`, white at `highest`
/// (a colour picture by its brightness).
pub fn heights_from(picture: &RgbaImage, w: u32, h: u32, lowest: u8, highest: u8) -> Vec<u8> {
    let sampler = Resampler::new(picture, w, h);
    let mut row = vec![[0.0f32; 3]; w as usize];
    let mut out = Vec::with_capacity((w * h) as usize);
    let (lo, hi) = (lowest as f32, highest as f32);
    for y in 0..h {
        sampler.row(y, &mut row);
        out.extend(row.iter().map(|c| {
            let brightness = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) / 255.0;
            (lo + (hi - lo) * brightness.clamp(0.0, 1.0)).round() as u8
        }));
    }
    out
}

/// The map's heights as a grey picture, a pixel a cell (black 0, white 255).
pub fn heights_picture(doc: &MapDoc) -> GrayImage {
    GrayImage::from_raw(doc.tnt.width, doc.tnt.height, doc.tnt.heights.clone()).unwrap_or_default()
}

/// The picture as a block of `tw` × `th` tiles of the map's tile set: resampled to their
/// 32 × 32 pixels each, put into the palette (dithered when `dither`), the tiles the set has
/// already shared. Fails, leaving the set as it was, when the tiles would run out.
pub fn art_tiles(doc: &mut MapDoc, picture: &RgbaImage, tw: u32, th: u32, quantizer: &Quantizer, dither: bool) -> Result<TileBlock> {
    if tw == 0 || th == 0 {
        return Ok(TileBlock { w: 0, h: 0, tiles: Vec::new() });
    }
    let pw = tw * TILE_SIZE as u32;
    let sampler = Resampler::new(picture, pw, th * TILE_SIZE as u32);
    let mut dither_rows = dither.then(|| Diffusion::new(pw as usize));
    let mut row = vec![[0.0f32; 3]; pw as usize];
    let mut band = vec![0u8; pw as usize * TILE_SIZE];
    let before = doc.tnt.tiles.len();
    let mut tiles = Vec::with_capacity((tw * th) as usize);
    for tz in 0..th {
        // A row of tiles at a time: its 32 rows of pixels, then its tiles.
        for y in 0..TILE_SIZE {
            sampler.row(tz * TILE_SIZE as u32 + y as u32, &mut row);
            let out = &mut band[y * pw as usize..(y + 1) * pw as usize];
            match &mut dither_rows {
                Some(diffusion) => diffusion.row(&row, out, quantizer),
                None => {
                    for (o, c) in out.iter_mut().zip(&row) {
                        *o = quantizer.nearest([c[0].round() as i32, c[1].round() as i32, c[2].round() as i32]);
                    }
                }
            }
        }
        for tx in 0..tw as usize {
            let mut tile: Tile = [0; TILE_SIZE * TILE_SIZE];
            for y in 0..TILE_SIZE {
                let from = y * pw as usize + tx * TILE_SIZE;
                tile[y * TILE_SIZE..(y + 1) * TILE_SIZE].copy_from_slice(&band[from..from + TILE_SIZE]);
            }
            match doc.intern_tile(&tile) {
                Some(index) => tiles.push(index),
                None => {
                    doc.truncate_tiles(before);
                    bail!(ta_ui::i18n::t("The map would run out of terrain tiles: make the picture a smaller brush, or start a smaller map."));
                }
            }
        }
    }
    Ok(TileBlock { w: tw, h: th, tiles })
}

/// The palette's nearest colours, a table of every colour at 6 bits a channel: the index whose
/// colour is nearest by the "redmean" distance (closer to the eye's than plain RGB).
pub struct Quantizer {
    table: Vec<u8>,
    colors: [[i32; 3]; 256],
}

impl Quantizer {
    pub fn new(palette: &Palette) -> Quantizer {
        let colors: [[i32; 3]; 256] = std::array::from_fn(|i| palette.colors[i].map(i32::from));
        let distance = |a: [i32; 3], b: [i32; 3]| {
            let mean = (a[0] + b[0]) / 2;
            let (dr, dg, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
            (((512 + mean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - mean) * db * db) >> 8)
        };
        let mut table = vec![0u8; 1 << 18];
        for r in 0..64 {
            for g in 0..64 {
                for b in 0..64 {
                    let c = [r * 4 + 2, g * 4 + 2, b * 4 + 2];
                    let best = (0..256).min_by_key(|&i| distance(c, colors[i])).unwrap_or(0);
                    table[((r << 12) | (g << 6) | b) as usize] = best as u8;
                }
            }
        }
        Quantizer { table, colors }
    }

    /// The palette index nearest to an RGB colour (channels clamped to 0..=255).
    pub fn nearest(&self, c: [i32; 3]) -> u8 {
        let k = |v: i32| (v.clamp(0, 255) >> 2) as usize;
        self.table[(k(c[0]) << 12) | (k(c[1]) << 6) | k(c[2])]
    }

    fn color(&self, index: u8) -> [i32; 3] {
        self.colors[index as usize]
    }
}

/// Floyd–Steinberg's error diffusion, a row at a time (the rows below take what the row above
/// could not show).
struct Diffusion {
    this: Vec<[f32; 3]>,
    next: Vec<[f32; 3]>,
}

impl Diffusion {
    fn new(width: usize) -> Diffusion {
        Diffusion { this: vec![[0.0; 3]; width + 2], next: vec![[0.0; 3]; width + 2] }
    }

    fn row(&mut self, row: &[[f32; 3]], out: &mut [u8], quantizer: &Quantizer) {
        for (x, (o, c)) in out.iter_mut().zip(row).enumerate() {
            let e = self.this[x + 1];
            let want = [c[0] + e[0], c[1] + e[1], c[2] + e[2]];
            let index = quantizer.nearest([want[0].round() as i32, want[1].round() as i32, want[2].round() as i32]);
            *o = index;
            let got = quantizer.color(index);
            // (The error kept within a channel's range, so a run of out-of-gamut colour can't
            // build it up without end.)
            let err: [f32; 3] = std::array::from_fn(|k| (want[k] - got[k] as f32).clamp(-96.0, 96.0));
            for (k, e) in err.iter().enumerate() {
                self.this[x + 2][k] += e * 7.0 / 16.0;
                self.next[x][k] += e * 3.0 / 16.0;
                self.next[x + 1][k] += e * 5.0 / 16.0;
                self.next[x + 2][k] += e / 16.0;
            }
        }
        std::mem::swap(&mut self.this, &mut self.next);
        self.next.iter_mut().for_each(|e| *e = [0.0; 3]);
    }
}

/// A picture resampled to `w` × `h`, a row at a time: each pixel the triangle-filtered mean of
/// the picture under it (as wide as the shrink, at least a pixel either way).
struct Resampler<'a> {
    picture: &'a RgbaImage,
    columns: Vec<Vec<(u32, f32)>>,
    rows: Vec<Vec<(u32, f32)>>,
}

impl<'a> Resampler<'a> {
    fn new(picture: &'a RgbaImage, w: u32, h: u32) -> Resampler<'a> {
        Resampler { picture, columns: weights(picture.width(), w), rows: weights(picture.height(), h) }
    }

    /// Row `y`, its colours as 0..=255 floats.
    fn row(&self, y: u32, out: &mut [[f32; 3]]) {
        for o in out.iter_mut() {
            *o = [0.0; 3];
        }
        let Some(rows) = self.rows.get(y as usize) else { return };
        for &(sy, wy) in rows {
            for (o, columns) in out.iter_mut().zip(&self.columns) {
                for &(sx, wx) in columns {
                    let p = self.picture.get_pixel(sx, sy).0;
                    let w = wx * wy;
                    o[0] += p[0] as f32 * w;
                    o[1] += p[1] as f32 * w;
                    o[2] += p[2] as f32 * w;
                }
            }
        }
    }
}

/// For each of `to` places along an axis, the source places and their weights (summing to one)
/// of a triangle filter `from` → `to`.
fn weights(from: u32, to: u32) -> Vec<Vec<(u32, f32)>> {
    let scale = from as f32 / to.max(1) as f32;
    let radius = scale.max(1.0);
    (0..to)
        .map(|i| {
            let centre = (i as f32 + 0.5) * scale - 0.5;
            let lo = (centre - radius).floor().max(0.0) as u32;
            let hi = ((centre + radius).ceil() as u32).min(from.saturating_sub(1));
            let mut taps: Vec<(u32, f32)> = (lo..=hi).map(|s| (s, (1.0 - (s as f32 - centre).abs() / radius).max(0.0))).filter(|&(_, w)| w > 0.0).collect();
            if taps.is_empty() {
                taps.push((centre.round().clamp(0.0, from.saturating_sub(1) as f32) as u32, 1.0));
            }
            let sum: f32 = taps.iter().map(|&(_, w)| w).sum();
            taps.iter_mut().for_each(|t| t.1 /= sum);
            taps
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let mut p = Palette::default();
        for (i, c) in p.colors.iter_mut().enumerate() {
            *c = [(i * 7 % 256) as u8, (i * 13 % 256) as u8, (i * 29 % 256) as u8];
        }
        p.colors[10] = [200, 30, 30];
        p.colors[11] = [30, 30, 200];
        p
    }

    #[test]
    fn a_palette_colour_finds_itself_and_a_picture_its_heights() {
        let p = palette();
        let q = Quantizer::new(&p);
        assert_eq!(q.color(q.nearest([200, 30, 30])), [200, 30, 30]);
        // Black at the lowest, white at the highest, a ramp between.
        let picture = RgbaImage::from_fn(64, 8, |x, _| { let v = (x * 255 / 63) as u8; image::Rgba([v, v, v, 255]) });
        let heights = heights_from(&picture, 16, 2, 20, 220);
        assert_eq!(heights.len(), 32);
        assert!(heights[0] < 30 && heights[15] > 210, "{heights:?}");
        assert!(heights[..16].windows(2).all(|w| w[0] <= w[1]), "rising: {heights:?}");
    }

    #[test]
    fn a_picture_becomes_tiles_shared_where_alike() {
        let p = palette();
        let q = Quantizer::new(&p);
        let new = super::super::doc::NewMap { name: "T".into(), width_steps: 1, height_steps: 1, sea_level: 0, height: 40, palette_map: "x".into() };
        let source = ta_formats::tnt::TntMap {
            width: 4, height: 4, sea_level: 0, tiles: vec![Box::new([0; TILE_SIZE * TILE_SIZE])], tile_map: vec![0; 4],
            heights: vec![40; 16], features: vec![ta_formats::tnt::FEATURE_NONE; 16], feature_names: Vec::new(), minimap: Default::default(),
            flags: Vec::new(),
        };
        let mut doc = MapDoc::new_blank(&new, &source, None);
        let before = doc.tnt.tiles.len();
        // Left half red, right half blue: two tiles a side, two kinds.
        let picture = RgbaImage::from_fn(128, 64, |x, _| if x < 64 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([30, 30, 200, 255]) });
        let block = art_tiles(&mut doc, &picture, 4, 2, &q, false).unwrap();
        assert_eq!((block.w, block.h, block.tiles.len()), (4, 2, 8));
        assert_eq!(doc.tnt.tiles.len(), before + 2, "alike tiles are one");
        assert_eq!(doc.tnt.tiles[block.tiles[0] as usize][0], 10);
        assert_eq!(doc.tnt.tiles[block.tiles[3] as usize][0], 11);
        // Dithered, a flat palette colour stays flat.
        let block = art_tiles(&mut doc, &picture, 4, 2, &q, true).unwrap();
        assert_eq!(doc.tnt.tiles[block.tiles[0] as usize].iter().filter(|&&i| i == 10).count(), TILE_SIZE * TILE_SIZE);
    }
}
