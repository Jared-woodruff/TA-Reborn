//! DirectDraw Surface textures, as Supreme Commander stores its pictures: the block-compressed
//! DXT1, DXT3 and DXT5, and uncompressed colour of 32, 24, 16 and 8 bits described by masks.
//! One mip level is decoded at a time: the converter asks for the level nearest the size it
//! paints at.

use crate::{Error, Result};

/// The largest side read (Supreme Commander's are 4096 at most).
const MAX_SIDE: u32 = 16384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Dxt1,
    Dxt3,
    Dxt5,
    /// Uncompressed, `bits` per texel, each channel where its mask says (alpha mask 0: opaque).
    Masked { bits: u32, masks: [u32; 4] },
}

/// A texture's header: its size, format and how many mip levels follow the first.
#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub format: Format,
    /// Where the first level's texels start.
    data: usize,
}

/// A decoded level: RGBA, row by row from the top.
#[derive(Clone, Debug, Default)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[u8; 4]>,
}

impl Image {
    /// The texel at (x, y), wrapped round the edges (textures repeat).
    #[inline]
    pub fn wrapped(&self, x: i64, y: i64) -> [u8; 4] {
        let (w, h) = (self.width as i64, self.height as i64);
        self.rgba[(y.rem_euclid(h) * w + x.rem_euclid(w)) as usize]
    }

    /// The texel at (x, y), clamped to the edges.
    #[inline]
    pub fn clamped(&self, x: i64, y: i64) -> [u8; 4] {
        let (w, h) = (self.width as i64, self.height as i64);
        self.rgba[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize]
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Result<Header> {
        let bad = |what: &str| Error::Format(format!("DDS: {what}"));
        if bytes.len() < 128 || &bytes[..4] != b"DDS " || u32_at(bytes, 4) != 124 {
            return Err(bad("not a DDS texture"));
        }
        let height = u32_at(bytes, 12);
        let width = u32_at(bytes, 16);
        let levels = u32_at(bytes, 28).max(1);
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(bad(&format!("bad size {width}x{height}")));
        }
        // The pixel format at 76: size, flags, FourCC, bits, then the R, G, B, A masks.
        let pf_flags = u32_at(bytes, 80);
        let fourcc = &bytes[84..88];
        let format = if pf_flags & 0x4 != 0 {
            match fourcc {
                b"DXT1" => Format::Dxt1,
                b"DXT3" => Format::Dxt3,
                b"DXT5" => Format::Dxt5,
                other => return Err(bad(&format!("unsupported compression {:?}", String::from_utf8_lossy(other)))),
            }
        } else {
            let bits = u32_at(bytes, 88);
            let alpha = if pf_flags & 0x1 != 0 { u32_at(bytes, 104) } else { 0 };
            let masks = [u32_at(bytes, 92), u32_at(bytes, 96), u32_at(bytes, 100), alpha];
            if !matches!(bits, 8 | 16 | 24 | 32) {
                return Err(bad(&format!("{bits}-bit texels")));
            }
            // (Luminance only: one mask, read into red, green and blue alike.)
            let masks = if pf_flags & 0x20000 != 0 { [masks[0], masks[0], masks[0], masks[3]] } else { masks };
            Format::Masked { bits, masks }
        };
        Ok(Header { width, height, levels, format, data: 128 })
    }

    /// The size of mip level `level` (each half the last, at least 1).
    pub fn level_size(&self, level: u32) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    fn level_bytes(&self, level: u32) -> usize {
        let (w, h) = self.level_size(level);
        match self.format {
            Format::Dxt1 => w.div_ceil(4) as usize * h.div_ceil(4) as usize * 8,
            Format::Dxt3 | Format::Dxt5 => w.div_ceil(4) as usize * h.div_ceil(4) as usize * 16,
            Format::Masked { bits, .. } => w as usize * h as usize * (bits / 8) as usize,
        }
    }

    /// The level whose larger side is the smallest still at least `side` (or the first).
    pub fn level_for(&self, side: u32) -> u32 {
        (0..self.levels).rev().find(|&l| { let (w, h) = self.level_size(l); w.max(h) >= side }).unwrap_or(0)
    }
}

/// Decodes mip level `level` of a texture (the last there is, when it has fewer).
pub fn decode(bytes: &[u8], level: u32) -> Result<Image> {
    let header = Header::parse(bytes)?;
    let level = level.min(header.levels - 1);
    let mut at = header.data;
    for l in 0..level {
        at += header.level_bytes(l);
    }
    let need = header.level_bytes(level);
    let Some(data) = bytes.get(at..at + need) else {
        // (A texture missing its smaller levels: its first, if that is there.)
        if level > 0 {
            return decode(bytes, 0);
        }
        return Err(Error::Format("DDS: texels missing".into()));
    };
    let (w, h) = header.level_size(level);
    let mut rgba = vec![[0u8; 4]; w as usize * h as usize];
    match header.format {
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => {
            let block_bytes = if header.format == Format::Dxt1 { 8 } else { 16 };
            let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
            for by in 0..bh {
                for bx in 0..bw {
                    let block = &data[(by * bw + bx) * block_bytes..][..block_bytes];
                    let texels = match header.format {
                        Format::Dxt1 => dxt1(block, true),
                        Format::Dxt3 => {
                            let mut t = dxt1(&block[8..], false);
                            for (i, px) in t.iter_mut().enumerate() {
                                let nibble = (block[i / 2] >> ((i % 2) * 4)) & 0xF;
                                px[3] = nibble * 17;
                            }
                            t
                        }
                        _ => {
                            let mut t = dxt1(&block[8..], false);
                            let alphas = dxt5_alpha(&block[..8]);
                            for (px, a) in t.iter_mut().zip(alphas) {
                                px[3] = a;
                            }
                            t
                        }
                    };
                    for (i, texel) in texels.iter().enumerate() {
                        let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                        if x < w as usize && y < h as usize {
                            rgba[y * w as usize + x] = *texel;
                        }
                    }
                }
            }
        }
        Format::Masked { bits, masks } => {
            let step = (bits / 8) as usize;
            for (i, px) in rgba.iter_mut().enumerate() {
                let mut v = 0u32;
                for b in 0..step {
                    v |= (data[i * step + b] as u32) << (8 * b);
                }
                let channel = |mask: u32| -> u8 {
                    if mask == 0 {
                        return 0;
                    }
                    let shift = mask.trailing_zeros();
                    let max = mask >> shift;
                    (((v & mask) >> shift) * 255 / max) as u8
                };
                *px = [channel(masks[0]), channel(masks[1]), channel(masks[2]), if masks[3] == 0 { 255 } else { channel(masks[3]) }];
            }
        }
    }
    Ok(Image { width: w, height: h, rgba })
}

/// A 565 colour's 8-bit channels.
fn rgb565(c: u16) -> [u16; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

/// A colour block's 16 texels (DXT1's, or the colour half of DXT3's and DXT5's, which always
/// use four colours: `punch` lets DXT1 use its transparent black).
fn dxt1(block: &[u8], punch: bool) -> [[u8; 4]; 16] {
    let (c0, c1) = (u16::from_le_bytes([block[0], block[1]]), u16::from_le_bytes([block[2], block[3]]));
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mut palette = [[0u8; 4]; 4];
    palette[0] = [a[0] as u8, a[1] as u8, a[2] as u8, 255];
    palette[1] = [b[0] as u8, b[1] as u8, b[2] as u8, 255];
    if c0 > c1 || !punch {
        for i in 0..3 {
            palette[2][i] = ((2 * a[i] + b[i] + 1) / 3) as u8;
            palette[3][i] = ((a[i] + 2 * b[i] + 1) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        for i in 0..3 {
            palette[2][i] = ((a[i] + b[i]) / 2) as u8;
        }
        palette[2][3] = 255;
        palette[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| palette[((bits >> (2 * i)) & 3) as usize])
}

/// A DXT5 alpha block's 16 values.
fn dxt5_alpha(block: &[u8]) -> [u8; 16] {
    let (a0, a1) = (block[0] as u16, block[1] as u16);
    let mut table = [0u8; 8];
    table[0] = a0 as u8;
    table[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u16 {
            table[i as usize + 1] = (((7 - i) * a0 + i * a1 + 3) / 7) as u8;
        }
    } else {
        for i in 1..5u16 {
            table[i as usize + 1] = (((5 - i) * a0 + i * a1 + 2) / 5) as u8;
        }
        table[6] = 0;
        table[7] = 255;
    }
    let mut bits = 0u64;
    for (i, &byte) in block[2..8].iter().enumerate() {
        bits |= (byte as u64) << (8 * i);
    }
    std::array::from_fn(|i| table[((bits >> (3 * i)) & 7) as usize])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A DDS file's header for `format` (FourCC, or uncompressed 32-bit BGRA).
    pub(crate) fn header(width: u32, height: u32, levels: u32, fourcc: Option<&[u8; 4]>) -> Vec<u8> {
        let mut h = vec![0u8; 128];
        h[..4].copy_from_slice(b"DDS ");
        h[4..8].copy_from_slice(&124u32.to_le_bytes());
        h[12..16].copy_from_slice(&height.to_le_bytes());
        h[16..20].copy_from_slice(&width.to_le_bytes());
        h[28..32].copy_from_slice(&levels.to_le_bytes());
        h[76..80].copy_from_slice(&32u32.to_le_bytes());
        match fourcc {
            Some(cc) => {
                h[80..84].copy_from_slice(&4u32.to_le_bytes());
                h[84..88].copy_from_slice(cc);
            }
            None => {
                h[80..84].copy_from_slice(&0x41u32.to_le_bytes());
                h[88..92].copy_from_slice(&32u32.to_le_bytes());
                h[92..96].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
                h[96..100].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
                h[100..104].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
                h[104..108].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
            }
        }
        h
    }

    #[test]
    fn uncompressed_texels_and_their_mip_levels_decode() {
        // 4x2 BGRA, then its 2x1 and 1x1 levels.
        let mut bytes = header(4, 2, 3, None);
        for i in 0..8u8 {
            bytes.extend_from_slice(&[i, 10 + i, 20 + i, 200]);
        }
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        bytes.extend_from_slice(&[9, 9, 9, 9]);
        let image = decode(&bytes, 0).unwrap();
        assert_eq!((image.width, image.height), (4, 2));
        assert_eq!(image.rgba[5], [25, 15, 5, 200]);
        assert_eq!(image.wrapped(-1, 2), image.rgba[3]);
        let small = decode(&bytes, 1).unwrap();
        assert_eq!((small.width, small.height, small.rgba[1]), (2, 1, [7, 6, 5, 8]));
        let header = Header::parse(&bytes).unwrap();
        assert_eq!((header.level_for(4), header.level_for(2), header.level_for(1)), (0, 1, 2));
    }

    #[test]
    fn dxt_blocks_decode_to_their_colours() {
        // One DXT1 block: red and blue, texel codes 0, 1, 2, 3 along the first row.
        let mut bytes = header(4, 4, 1, Some(b"DXT1"));
        bytes.extend_from_slice(&0xF800u16.to_le_bytes());
        bytes.extend_from_slice(&0x001Fu16.to_le_bytes());
        bytes.extend_from_slice(&[0b1110_0100, 0, 0, 0]);
        let image = decode(&bytes, 0).unwrap();
        assert_eq!(image.rgba[0], [255, 0, 0, 255]);
        assert_eq!(image.rgba[1], [0, 0, 255, 255]);
        assert_eq!(image.rgba[2], [170, 0, 85, 255]);
        assert_eq!(image.rgba[4], [255, 0, 0, 255]);
        // DXT5: alpha 255 and 0, interpolated along the first row.
        let mut bytes = header(4, 4, 1, Some(b"DXT5"));
        bytes.extend_from_slice(&[255, 0, 0b10_001_000, 0b0000_0110, 0, 0, 0, 0]);
        bytes.extend_from_slice(&0x07E0u16.to_le_bytes());
        bytes.extend_from_slice(&0x07E0u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        let image = decode(&bytes, 0).unwrap();
        assert_eq!(image.rgba[0], [0, 255, 0, 255]);
        assert_eq!(image.rgba[1][3], 0);
        assert_eq!(image.rgba[2][3], 219);
        assert!(decode(b"DDS nonsense", 0).is_err());
    }
}
