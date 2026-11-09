//! ZSoft PCX decoder (8-bit indexed with trailing 256-colour palette, or 24-bit planar).
//! TA uses PCX for build pictures (`unitpics/*.pcx`), menu backgrounds and map previews.

use crate::bin::{u16_at, u8_at};
use crate::image::{IndexedImage, Palette};
use crate::{FormatError, Result};

// (One per image file: the palette kept inline costs nothing worth a box.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum PcxPixels {
    Indexed { image: IndexedImage, palette: Palette },
    Rgb(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct PcxImage {
    pub width: u32,
    pub height: u32,
    pub pixels: PcxPixels,
}

impl PcxImage {
    pub fn parse(data: &[u8]) -> Result<PcxImage> {
        if u8_at(data, 0)? != 0x0A {
            return Err(FormatError::invalid("PCX", "bad manufacturer byte"));
        }
        let encoding = u8_at(data, 2)?;
        let bpp = u8_at(data, 3)?;
        let (xmin, ymin) = (u16_at(data, 4)? as u32, u16_at(data, 6)? as u32);
        let (xmax, ymax) = (u16_at(data, 8)? as u32, u16_at(data, 10)? as u32);
        let planes = u8_at(data, 65)? as usize;
        let bytes_per_line = u16_at(data, 66)? as usize;
        if xmax < xmin || ymax < ymin || bpp != 8 || !(planes == 1 || planes == 3) || encoding > 1 {
            return Err(FormatError::invalid("PCX", format!("unsupported: {bpp} bpp, {planes} planes, encoding {encoding}")));
        }
        let width = xmax - xmin + 1;
        let height = ymax - ymin + 1;
        if bytes_per_line < width as usize {
            return Err(FormatError::invalid("PCX", format!("scanline has {bytes_per_line} bytes for {width} pixels")));
        }
        // The indexed palette is not part of the pixel stream. A truncated image
        // must not decode its marker/colours as pixels and appear to load successfully.
        let pal_off = if planes == 1 { data.len().checked_sub(769).filter(|&o| o >= 128 && data[o] == 0x0C) } else { None };
        let image_end = pal_off.unwrap_or(data.len());
        let pixels = &data[..image_end];
        let scan = bytes_per_line * planes;
        let total = scan * height as usize;
        if width as u64 * height as u64 > 16 << 20 || total > 64 << 20
            || total > pixels.len().saturating_sub(128).saturating_mul(if encoding == 1 { 63 } else { 1 })
        {
            return Err(FormatError::invalid("PCX", "image exceeds decode limit or cannot fit in the encoded data"));
        }

        let mut raw = Vec::with_capacity(total);
        let mut pos = 128;
        while raw.len() < total {
            let b = *pixels.get(pos).ok_or(FormatError::Eof { offset: pos, needed: 1 })?;
            pos += 1;
            if encoding == 1 && b >= 0xC0 {
                let count = (b & 0x3F) as usize;
                if count == 0 || count > total - raw.len() {
                    return Err(FormatError::invalid("PCX", "invalid pixel run length"));
                }
                let v = *pixels.get(pos).ok_or(FormatError::Eof { offset: pos, needed: 1 })?;
                pos += 1;
                raw.extend(std::iter::repeat_n(v, count));
            } else {
                raw.push(b);
            }
        }

        let (w, h) = (width as usize, height as usize);
        let pixels = if planes == 1 {
            let palette = match pal_off {
                Some(o) => Palette::parse(&data[o + 1..])?,
                None => Palette::default(),
            };
            let mut image = IndexedImage::new(width, height, 0);
            for y in 0..h {
                image.pixels[y * w..(y + 1) * w].copy_from_slice(&raw[y * scan..y * scan + w]);
            }
            image.opaque.fill(true);
            PcxPixels::Indexed { image, palette }
        } else {
            let mut rgb = vec![0u8; w * h * 3];
            for y in 0..h {
                for x in 0..w {
                    for c in 0..3 {
                        rgb[(y * w + x) * 3 + c] = raw[y * scan + c * bytes_per_line + x];
                    }
                }
            }
            PcxPixels::Rgb(rgb)
        };
        Ok(PcxImage { width, height, pixels })
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        match &self.pixels {
            PcxPixels::Indexed { image, palette } => image.to_rgba_opaque(palette),
            PcxPixels::Rgb(rgb) => rgb.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(planes: u8, stride: u16) -> Vec<u8> {
        let mut data = vec![0; 128];
        data[0] = 0x0A;
        data[3] = 8;
        data[8] = 1; // two pixels, one row
        data[65] = planes;
        data[66..68].copy_from_slice(&stride.to_le_bytes());
        data.extend(std::iter::repeat_n(7, planes as usize * stride as usize));
        data
    }

    #[test]
    fn indexed_palette_cannot_complete_a_truncated_pixel_stream() {
        for encoding in [0, 1] {
            let mut data = image(1, 2);
            data[2] = encoding;
            data.truncate(129); // only one of the two pixels is present
            data.push(0x0C);
            data.extend((0..768).map(|n| (n % 256) as u8));
            assert!(PcxImage::parse(&data).is_err(), "encoding {encoding} consumed palette bytes as pixels");
            data.insert(129, 7);
            let valid = PcxImage::parse(&data).unwrap();
            assert_eq!(valid.to_rgba(), [21, 22, 23, 255, 21, 22, 23, 255]);
        }
    }

    #[test]
    fn malformed_encoding_and_runs_are_rejected() {
        let mut data = image(1, 2);
        data[2] = 2;
        assert!(PcxImage::parse(&data).is_err());
        data[2] = 1;
        for payload in [&[0xC0, 7, 7, 7][..], &[0xC3, 7][..], &[0xC2][..]] {
            data.truncate(128);
            data.extend_from_slice(payload);
            assert!(PcxImage::parse(&data).is_err(), "accepted malformed run {payload:?}");
        }
        data.truncate(128);
        data.extend_from_slice(&[0xC2, 7]);
        assert_eq!(PcxImage::parse(&data).unwrap().to_rgba(), [7, 7, 7, 255, 7, 7, 7, 255]);
    }

    #[test]
    fn rejects_short_scanlines_in_both_pixel_formats() {
        for planes in [1, 3] {
            for stride in [0, 1] {
                assert!(PcxImage::parse(&image(planes, stride)).is_err());
            }
            for stride in [2, 4] {
                let decoded = PcxImage::parse(&image(planes, stride)).unwrap();
                assert_eq!(decoded.to_rgba(), [7, 7, 7, 255, 7, 7, 7, 255]);
            }
        }
    }
}
