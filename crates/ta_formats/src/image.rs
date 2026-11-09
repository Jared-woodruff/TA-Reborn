//! Shared image types: TA's 256-colour palette and palette-indexed images.

use crate::{FormatError, Result};

/// A 256-entry RGB palette (`palettes/palette.pal` is 256 × `[r, g, b, 0]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub colors: [[u8; 3]; 256],
}

impl Default for Palette {
    fn default() -> Self {
        // Greyscale ramp; only used when the real palette is unavailable.
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8; 3];
        }
        Palette { colors }
    }
}

impl Palette {
    /// Parses a `.pal` file: 256 × 4 bytes (RGBX) or a raw 768-byte RGB table.
    pub fn parse(data: &[u8]) -> Result<Palette> {
        let stride = match data.len() {
            n if n >= 1024 => 4,
            n if n >= 768 => 3,
            n => return Err(FormatError::invalid("palette", format!("{n} bytes is too short"))),
        };
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            let o = i * stride;
            *c = [data[o], data[o + 1], data[o + 2]];
        }
        Ok(Palette { colors })
    }

    #[inline]
    pub fn rgb(&self, index: u8) -> [u8; 3] {
        self.colors[index as usize]
    }

    #[inline]
    pub fn rgba(&self, index: u8) -> [u8; 4] {
        let [r, g, b] = self.colors[index as usize];
        [r, g, b, 255]
    }
}

/// A palette-indexed image with a per-pixel opacity mask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major palette indices.
    pub pixels: Vec<u8>,
    /// Row-major opacity; `false` pixels are transparent.
    pub opaque: Vec<bool>,
}

impl IndexedImage {
    pub fn new(width: u32, height: u32, fill: u8) -> Self {
        let n = (width * height) as usize;
        IndexedImage { width, height, pixels: vec![fill; n], opaque: vec![false; n] }
    }

    /// Expands to RGBA8 using `palette`; transparent pixels become `[0, 0, 0, 0]`.
    pub fn to_rgba(&self, palette: &Palette) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for (&p, &o) in self.pixels.iter().zip(&self.opaque) {
            if o {
                out.extend_from_slice(&palette.rgba(p));
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        out
    }

    /// Expands to RGBA8 ignoring the mask (for textures, which TA always drew opaque).
    pub fn to_rgba_opaque(&self, palette: &Palette) -> Vec<u8> {
        self.pixels.iter().flat_map(|&p| palette.rgba(p)).collect()
    }
}
