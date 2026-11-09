//! TA bitmap fonts (`fonts/*.fnt`, `guis/*.fnt`): the lettering of the in-game interface.
//!
//! Layout (little-endian): `{height: u16, _: u16}`, then 256 × `u16` glyph offsets (0 = no
//! glyph); each glyph is `{width: u8}` followed by `width × height` one-bit pixels, row by
//! row, most significant bit first, packed without row padding. Widths include the
//! spacing to the next letter.

use crate::bin::{Budget, slice, u16_at, u8_at};
use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub width: u32,
    /// Row-major coverage, `width × height`.
    pub pixels: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct Font {
    pub height: u32,
    /// Indexed by byte (Latin-1) code.
    pub glyphs: Vec<Option<Glyph>>,
}

impl Font {
    pub fn parse(data: &[u8]) -> Result<Font> {
        let height = u16_at(data, 0)? as u32;
        slice(data, 4, 512)?;
        let mut budget = Budget(16 << 20);
        let mut glyphs = Vec::with_capacity(256);
        for c in 0..256 {
            let offset = u16_at(data, 4 + c * 2)? as usize;
            if offset == 0 {
                glyphs.push(None);
                continue;
            }
            let width = u8_at(data, offset)? as u32;
            let bits = (width * height) as usize;
            let bytes = slice(data, offset + 1, bits.div_ceil(8))?;
            budget.take(bits, "FNT")?;
            let pixels = (0..bits).map(|i| bytes[i >> 3] & (0x80 >> (i & 7)) != 0).collect();
            glyphs.push(Some(Glyph { width, pixels }));
        }
        Ok(Font { height, glyphs })
    }

    /// The glyph drawn for `c` (characters outside Latin-1 have none).
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(c as usize).and_then(Option::as_ref)
    }

    /// Width of a line of text in font pixels.
    pub fn width(&self, text: &str) -> u32 {
        text.chars().filter_map(|c| self.glyph(c)).map(|g| g.width).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_glyph_bits_across_byte_boundaries() {
        // A 3-pixel-high font with two glyphs: ' ' (2 wide, empty) and '!' (3 wide).
        let mut data = vec![3, 0, 1, 0];
        data.resize(4 + 512, 0);
        let space = data.len();
        data.extend_from_slice(&[2, 0]);
        let bang = data.len();
        // Rows 010 / 111 / 010 → bits 010111010 → 0x5D, 0x00.
        data.extend_from_slice(&[3, 0b0101_1101, 0b0000_0000]);
        data[4 + 32 * 2..4 + 32 * 2 + 2].copy_from_slice(&(space as u16).to_le_bytes());
        data[4 + 33 * 2..4 + 33 * 2 + 2].copy_from_slice(&(bang as u16).to_le_bytes());
        let font = Font::parse(&data).unwrap();
        assert_eq!(font.height, 3);
        assert_eq!(font.glyph(' ').map(|g| g.width), Some(2));
        let g = font.glyph('!').unwrap();
        assert_eq!(g.pixels, [false, true, false, true, true, true, false, true, false]);
        assert!(font.glyph('A').is_none());
        assert_eq!(font.width("! !"), 8);
    }

    #[test]
    fn truncated_glyphs_are_errors() {
        let mut data = vec![8, 0, 1, 0];
        data.resize(4 + 512, 0);
        let at = data.len();
        data.extend_from_slice(&[8, 0xFF]);
        data[4 + 65 * 2..4 + 65 * 2 + 2].copy_from_slice(&(at as u16).to_le_bytes());
        assert!(Font::parse(&data).is_err());
    }
}
