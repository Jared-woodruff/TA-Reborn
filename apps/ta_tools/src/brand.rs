//! `ta-tools brand <dir>`: TA Reborn's own art, from the emblem `tools/branding/emblem.py`
//! draws (`branding/emblem-*.png`, see `ta_ui::icon`) — the executables' icon (every size
//! Windows asks for, PNG-compressed) and the installer's side and header pictures, drawn here —
//! so the repository needs no hand-made image files. Run it after redrawing the emblem and commit
//! the results (`branding/`).

use std::path::Path;

use anyhow::{Context, Result};
use ta_ui::icon::emblem_rgba;

/// Sizes Windows picks from for the taskbar, Explorer views and shortcuts at every scale.
const ICON_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];

// Keep four source pixels for each pixel of the standard 96-DPI dialog. The installer
// resamples these masters to its actual controls; a 108-pixel logo must never be enlarged
// to cover a high-DPI screen. The largest emblem below is still within the 512-pixel master.
const INSTALLER_SCALE: u32 = 4;
const DEEP: [f32; 3] = [12.0, 16.0, 21.0];
const PANEL: [f32; 3] = [27.0, 34.0, 42.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];
const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
const BRASS: [f32; 3] = [216.0, 180.0, 86.0];
const BRASS_DIM: [f32; 3] = [142.0, 113.0, 48.0];

pub fn run(out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    write_ico(&out.join("ta-reborn.ico"))?;
    let welcome = welcome();
    welcome.write_bmp(&out.join("installer-welcome.bmp"))?;
    let header = header();
    header.write_bmp(&out.join("installer-header.bmp"))?;
    // For looking at: every icon size side by side, at 1:1 and enlarged.
    std::fs::write(out.join("icon-sheet.png"), png_rgb(&sheet())?)?;
    println!("wrote the icon ({} sizes) and the installer art to {}", ICON_SIZES.len(), out.display());
    Ok(())
}

fn png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec()).context("image size")?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

fn png_rgb(c: &Canvas) -> Result<Vec<u8>> {
    let rgba: Vec<u8> = c.pixels.iter().flat_map(|p| [p[0].round() as u8, p[1].round() as u8, p[2].round() as u8, 255]).collect();
    png(c.width, c.height, &rgba)
}

/// An .ico with a PNG image per size (Windows Vista and later read those at every size).
fn write_ico(path: &Path) -> Result<()> {
    let images: Vec<(u32, Vec<u8>)> = ICON_SIZES.iter().map(|&s| Ok((s, png(s, s, &emblem_rgba(s))?))).collect::<Result<_>>()?;
    let mut file = Vec::new();
    file.extend(0u16.to_le_bytes());
    file.extend(1u16.to_le_bytes());
    file.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        // 0 means 256.
        let edge = if *size >= 256 { 0 } else { *size as u8 };
        file.extend([edge, edge, 0, 0]);
        file.extend(1u16.to_le_bytes());
        file.extend(32u16.to_le_bytes());
        file.extend((data.len() as u32).to_le_bytes());
        file.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        file.extend(data);
    }
    std::fs::write(path, file).with_context(|| format!("writing {}", path.display()))
}

/// A small RGB drawing surface (floating point, sRGB values 0..255).
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 3]>,
}

impl Canvas {
    fn new(width: u32, height: u32, color: [f32; 3]) -> Canvas {
        Canvas { width, height, pixels: vec![color; (width * height) as usize] }
    }

    fn blend(&mut self, x: i32, y: i32, color: [f32; 3], alpha: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let p = &mut self.pixels[(y as u32 * self.width + x as u32) as usize];
        for k in 0..3 {
            p[k] += (color[k] - p[k]) * alpha.clamp(0.0, 1.0);
        }
    }

    /// Quiet gunmetal with a soft pool of warm light behind the emblem. Grain is low
    /// contrast and drawn at the master resolution, so it never becomes a coarse grid.
    fn metal(&mut self, light_x: f32, light_y: f32, radius: f32) {
        for y in 0..self.height {
            let t = y as f32 / (self.height - 1).max(1) as f32;
            for x in 0..self.width {
                let dx = (x as f32 - light_x) / radius;
                let dy = (y as f32 - light_y) / radius;
                let light = (-(dx * dx + dy * dy) * 2.0).exp();
                let grain = (hash(x as i32 / 3, y as i32) - 0.5) * 1.3;
                self.pixels[(y * self.width + x) as usize] = [0, 1, 2].map(|k| {
                    PANEL[k] + (DEEP[k] - PANEL[k]) * t + [24.0, 17.0, 5.0][k] * light + grain
                });
            }
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: [f32; 3], alpha: f32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.blend(xx, yy, color, alpha);
            }
        }
    }

    /// A fine inset frame, with a softly lit upper edge and a shaded lower edge.
    fn frame(&mut self, inset: i32, thickness: i32) {
        let (w, h) = (self.width as i32 - inset * 2, self.height as i32 - inset * 2);
        self.rect(inset, inset, w, thickness, WHITE, 0.12);
        self.rect(inset, inset, thickness, h, WHITE, 0.07);
        self.rect(inset, inset + h - thickness, w, thickness, BLACK, 0.65);
        self.rect(inset + w - thickness, inset, thickness, h, BLACK, 0.65);
    }

    /// Composites a straight-alpha RGBA image.
    fn image(&mut self, x: i32, y: i32, size: u32, rgba: &[u8]) {
        for yy in 0..size as i32 {
            for xx in 0..size as i32 {
                let i = ((yy as u32 * size + xx as u32) * 4) as usize;
                let a = rgba[i + 3] as f32 / 255.0;
                if a > 0.0 {
                    self.blend(x + xx, y + yy, [rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32], a);
                }
            }
        }
    }

    /// A 24-bit BMP (what NSIS wants for its pictures).
    fn write_bmp(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.bmp()).with_context(|| format!("writing {}", path.display()))
    }

    fn bmp(&self) -> Vec<u8> {
        let row = (self.width * 3).div_ceil(4) * 4;
        let size = 54 + row * self.height;
        let mut out = Vec::with_capacity(size as usize);
        out.extend(b"BM");
        out.extend(size.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(54u32.to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend((self.width as i32).to_le_bytes());
        out.extend((self.height as i32).to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(24u16.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend((row * self.height).to_le_bytes());
        out.extend(2835i32.to_le_bytes());
        out.extend(2835i32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        // Bottom-up rows of BGR, padded to four bytes.
        for y in (0..self.height).rev() {
            let start = out.len();
            for x in 0..self.width {
                let p = self.pixels[(y * self.width + x) as usize];
                out.extend([p[2].round() as u8, p[1].round() as u8, p[0].round() as u8]);
            }
            out.resize(start + row as usize, 0);
        }
        out
    }
}

/// The welcome and finish picture: a four-times-resolution master for a 164 × 314 slot.
/// A restrained brass rule and inset frame leave the emblem as the focal point.
fn welcome() -> Canvas {
    let s = INSTALLER_SCALE as i32;
    let mut c = Canvas::new(164 * INSTALLER_SCALE, 314 * INSTALLER_SCALE, PANEL);
    c.metal(82.0 * s as f32, 104.0 * s as f32, 105.0 * s as f32);
    c.frame(7 * s, s);
    // Fine brass corner accents echo the emblem's rim without competing with it.
    c.rect(18 * s, 22 * s, 25 * s, s, BRASS_DIM, 0.8);
    c.rect(121 * s, 22 * s, 25 * s, s, BRASS_DIM, 0.8);
    let size = 108 * INSTALLER_SCALE;
    c.image((c.width as i32 - size as i32) / 2, 58 * s, size, &emblem_rgba(size));
    c.rect(60 * s, 185 * s, 44 * s, s, BRASS, 0.95);
    c.rect(60 * s, 186 * s, 44 * s, s, BRASS_DIM, 0.5);
    c.rect(30 * s, 201 * s, 104 * s, s, WHITE, 0.06);
    // The lower panel stays calm: a small registration mark above the footer edge.
    c.rect(79 * s, 281 * s, 6 * s, s, BRASS_DIM, 0.65);
    c.rect(81 * s, 279 * s, 2 * s, 5 * s, BRASS_DIM, 0.65);
    c
}

/// The header picture: a four-times-resolution master for a 150 × 57 slot.
fn header() -> Canvas {
    let s = INSTALLER_SCALE as i32;
    let mut c = Canvas::new(150 * INSTALLER_SCALE, 57 * INSTALLER_SCALE, PANEL);
    c.metal(118.0 * s as f32, 27.0 * s as f32, 62.0 * s as f32);
    let size = 45 * INSTALLER_SCALE;
    c.image(c.width as i32 - size as i32 - 10 * s, 6 * s, size, &emblem_rgba(size));
    c.rect(12 * s, 14 * s, s, 29 * s, BRASS, 0.9);
    c.rect(13 * s, 14 * s, s, 29 * s, BRASS_DIM, 0.5);
    c.rect(22 * s, 28 * s, 51 * s, s, BRASS_DIM, 0.4);
    c
}

/// A repeatable noise value 0..1 for a pixel.
fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    (h & 0xffff) as f32 / 65535.0
}

/// Every icon size at 1:1, then the small ones enlarged ×4, on a dark and a light ground.
fn sheet() -> Canvas {
    let mut c = Canvas::new(1320, 992, [30.0, 34.0, 40.0]);
    c.rect(0, 496, 1320, 496, [236.0, 238.0, 240.0], 1.0);
    for top in [0, 496] {
        let mut x = 16;
        for &s in &ICON_SIZES {
            c.image(x, top + 16, s, &emblem_rgba(s));
            x += s as i32 + 16;
        }
        let mut x = 16;
        for &s in ICON_SIZES.iter().take(6) {
            let rgba = emblem_rgba(s);
            let big: Vec<u8> = (0..s * 4)
                .flat_map(|y| (0..s * 4).map(move |x| (((y / 4) * s + x / 4) * 4) as usize))
                .flat_map(|i| [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]])
                .collect();
            c.image(x, top + 288, s * 4, &big);
            x += (s * 4) as i32 + 16;
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_masters_are_high_resolution_bmps() {
        for (canvas, expected) in [(welcome(), (656, 1256)), (header(), (600, 228))] {
            let bmp = canvas.bmp();
            let field = |offset: usize| u32::from_le_bytes(bmp[offset..offset + 4].try_into().unwrap());
            assert_eq!(&bmp[..2], b"BM");
            assert_eq!((field(18), field(22)), expected);
            assert_eq!(u16::from_le_bytes(bmp[28..30].try_into().unwrap()), 24);
            assert_eq!(field(30), 0, "uncompressed RGB for Windows bitmap loading");
            assert_eq!(field(2) as usize, bmp.len());
            let stride = (field(18) * 3).div_ceil(4) * 4;
            // Check channel order and bottom-up row storage at both ends of the image.
            for (x, y) in [(0, 0), (canvas.width / 2, canvas.height / 2), (canvas.width - 1, canvas.height - 1)] {
                let source = canvas.pixels[(y * canvas.width + x) as usize].map(|p| p.round() as u8);
                let offset = (field(10) + (field(22) - 1 - y) * stride + x * 3) as usize;
                assert_eq!([bmp[offset + 2], bmp[offset + 1], bmp[offset]], source);
            }
        }
    }
}
