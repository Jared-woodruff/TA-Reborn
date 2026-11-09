//! TA's pictures for the menus (a mission's glamour art). TA dithered its rendered pictures
//! into 256 colours, and egui draws a texture with one bilinear tap and no mipmaps, so the
//! dither showed as grain in the sky and on the metal wherever a picture was drawn. A
//! paletted picture is cleaned as it is read: an edge-keeping smoothing in Oklab (as the
//! units' paint is, `ta_render::model::remaster`), which mixes the dithered texels as the eye
//! does and keeps the edges and the shading TA painted.

use rayon::prelude::*;
use ta_formats::pcx::{PcxImage, PcxPixels};
use ta_vfs::Vfs;

/// The picture at `path` (a PCX), cleaned of its palette's dither.
pub fn read(vfs: &Vfs, path: &str) -> Option<egui::ColorImage> {
    let pcx = PcxImage::parse(&vfs.read_opt(path)?).ok()?;
    let (w, h) = (pcx.width as usize, pcx.height as usize);
    let mut rgba = pcx.to_rgba();
    if matches!(pcx.pixels, PcxPixels::Indexed { .. }) {
        dedither(w, h, &mut rgba);
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

/// How far apart in light (Oklab L) and in hue two texels may be and still be averaged: a
/// dither's neighbours are alike, an edge's are not.
const LIGHT: f32 = 0.055;
const HUE: f32 = 0.1;

/// Two passes of a 3 × 3 bilateral over `rgba` (sRGB, `w` × `h`), in Oklab.
fn dedither(w: usize, h: usize, rgba: &mut [u8]) {
    let mut lab: Vec<[f32; 3]> = rgba.as_chunks::<4>().0.iter().map(|p| to_lab([linear(p[0]), linear(p[1]), linear(p[2])])).collect();
    for _ in 0..2 {
        let src = lab.clone();
        lab.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let here = src[y * w + x];
                let (mut acc, mut total) = ([0.0f32; 3], 0.0f32);
                for dy in -1..=1isize {
                    for dx in -1..=1isize {
                        let (sx, sy) = ((x as isize + dx).clamp(0, w as isize - 1) as usize, (y as isize + dy).clamp(0, h as isize - 1) as usize);
                        let c = src[sy * w + sx];
                        let light = (c[0] - here[0]).powi(2) / (2.0 * LIGHT * LIGHT);
                        let hue = ((c[1] - here[1]).powi(2) + (c[2] - here[2]).powi(2)) / (2.0 * HUE * HUE);
                        let weight = (-((dx * dx + dy * dy) as f32) * 0.5 - light - hue).exp();
                        for k in 0..3 {
                            acc[k] += c[k] * weight;
                        }
                        total += weight;
                    }
                }
                *out = acc.map(|v| v / total);
            }
        });
    }
    for (p, c) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(lab) {
        let c = from_lab(c);
        for k in 0..3 {
            p[k] = srgb(c[k]);
        }
    }
}

fn linear(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let v = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (v * 255.0 + 0.5) as u8
}

fn to_lab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_47 * c[0] + 0.536_332_55 * c[1] + 0.051_445_99 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (l.max(0.0).cbrt(), m.max(0.0).cbrt(), s.max(0.0).cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_lab(c: [f32; 3]) -> [f32; 3] {
    let l = c[0] + 0.396_337_78 * c[1] + 0.215_803_76 * c[2];
    let m = c[0] - 0.105_561_346 * c[1] - 0.063_854_17 * c[2];
    let s = c[0] - 0.089_484_18 * c[1] - 1.291_485_5 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dither_smooths_and_an_edge_stays() {
        // A checker of two near greys beside a flat black: the checker evens out, the edge
        // between them stays.
        let (w, h) = (8, 4);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = if x >= 4 { 0 } else if (x + y) % 2 == 0 { 120 } else { 140 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        dedither(w, h, &mut rgba);
        let at = |x: usize, y: usize| rgba[(y * w + x) * 4] as i32;
        assert!((at(1, 1) - at(2, 1)).abs() <= 6, "the checker evens out: {} {}", at(1, 1), at(2, 1));
        assert!(at(5, 1) < 10 && at(3, 1) > 100, "the edge stays: {} {}", at(3, 1), at(5, 1));
    }
}
