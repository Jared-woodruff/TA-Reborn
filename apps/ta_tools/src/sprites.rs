//! `gaf-pack`: paintings packed into a GAF in TA's palette, TA Reborn's own feature art
//! (`assets/reborn/anims/festive.gaf`, drawn by tools/features/sprites.py from the festive
//! models). Each pixel is dithered into the palette as the map generator dithers its tiles
//! (`PaletteLut::pattern`: by its colour and place alone), so a twinkling painting's frames
//! differ only where its lights do; a pixel half covered is kept or left as the same pattern
//! says, so the edges stay hard as TA's are. A shadow is TA's: dark pixels, as many of them as
//! the shadow is deep, that the game draws translucent.

use std::path::Path;

use anyhow::{Context, Result, bail};
use ta_formats::gaf::{Gaf, GafEntry, GafFrame};
use ta_formats::image::{IndexedImage, Palette};

use crate::mapgen::tiles::{BAYER, PaletteLut};

/// The palette index TA's paintings leave transparent.
const TRANSPARENT: u8 = 9;

/// Packs the entries of `manifest` (JSON: `entries`, each a `name`, `frames` of `png` paths
/// beside it and their `origin`, and `shadow` for a shadow) into the GAF `out`.
pub fn pack(manifest: &Path, out: &Path, palette: &Palette) -> Result<()> {
    let dir = manifest.parent().unwrap_or(Path::new("."));
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(manifest).with_context(|| manifest.display().to_string())?)?;
    // (The entries TA's maps paint with: not the system's colours at either end.)
    let lut = PaletteLut::new(palette.colors, |i| (16..240).contains(&i));
    let darkest = (16..240u8).min_by_key(|&i| palette.colors[i as usize].iter().map(|&v| v as u32).sum::<u32>()).unwrap_or(16);
    let mut gaf = Gaf::default();
    for e in doc["entries"].as_array().context("no entries")? {
        let name = e["name"].as_str().context("an entry without a name")?.to_string();
        let shadow = e["shadow"].as_bool().unwrap_or(false);
        let mut frames: Vec<GafFrame> = Vec::new();
        let mut first: Option<(image::RgbaImage, IndexedImage)> = None;
        for f in e["frames"].as_array().context("no frames")? {
            let png = dir.join(f["png"].as_str().context("a frame without a png")?);
            let rgba = image::open(&png).with_context(|| png.display().to_string())?.to_rgba8();
            let origin = f["origin"].as_array().filter(|o| o.len() == 2).and_then(|o| Some((o[0].as_i64()? as i32, o[1].as_i64()? as i32))).context("a frame without its origin")?;
            let (w, h) = rgba.dimensions();
            let mut img = IndexedImage::new(w, h, TRANSPARENT);
            for y in 0..h {
                for x in 0..w {
                    let p = rgba.get_pixel(x, y).0;
                    let i = (y * w + x) as usize;
                    // (Half covered: kept where the pattern says, a shadow as deep as it is.)
                    let t = (BAYER[(y as usize % 4) * 4 + x as usize % 4] as f32 + 0.5) / 16.0;
                    let a = p[3] as f32 / 255.0;
                    let kept = if shadow { a > 0.12 + 0.76 * t } else { a > 0.38 + 0.24 * t };
                    if !kept {
                        continue;
                    }
                    img.opaque[i] = true;
                    img.pixels[i] = if shadow {
                        darkest
                    } else if let Some((rgba0, img0)) = &first
                        && img0.opaque[i]
                        && rgba0.dimensions() == (w, h)
                        && rgba0.get_pixel(x, y).0.iter().zip(&p).all(|(a, b)| a.abs_diff(*b) <= 6)
                    {
                        // As the first frame where it is as it was (noise of the render aside).
                        img0.pixels[i]
                    } else {
                        lut.pattern([p[0], p[1], p[2]].map(|v| v as f32 / 255.0), x as usize, y as usize)
                    };
                }
            }
            if first.is_none() {
                first = Some((rgba, img.clone()));
            }
            frames.push(GafFrame { image: img, origin_x: origin.0, origin_y: origin.1, transparency_index: TRANSPARENT, duration: 0 });
        }
        if frames.is_empty() {
            bail!("{name}: no frames");
        }
        gaf.entries.push(GafEntry { name, frames });
    }
    let bytes = gaf.write()?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, &bytes)?;
    println!("{}: {} entries, {} frames, {} KB", out.display(), gaf.entries.len(), gaf.entries.iter().map(|e| e.frames.len()).sum::<usize>(), bytes.len() / 1024);
    Ok(())
}
