//! GAF ("Graphics Animation Format") decoder.
//!
//! A GAF holds named entries, each a sequence of frames. TA uses it for unit textures
//! (`textures/*.gaf`, one entry per texture, extra frames = team colours), sprites and
//! explosions (`anims/*.gaf`), feature graphics and the whole 2D interface.
//!
//! Layout (all little-endian):
//! * header `{version = 0x00010100, entry_count, 0}` followed by `entry_count` pointers;
//! * entry `{frames: u16, 1: u16, 0: u32, name: [u8; 32]}` followed by `frames` ×
//!   `{frame_ptr: u32, duration: u32}`;
//! * frame `{width, height: u16, x_origin, y_origin: i16, transparency: u8, compressed: u8,
//!   subframes: u16, _: u32, data_ptr: u32, _: u32}`. With `subframes > 0`, `data_ptr`
//!   points at that many frame pointers whose images are layered; otherwise at pixels.
//! * compressed pixel rows: `u16 row_len` then control bytes — bit 0: skip `n >> 1`
//!   transparent pixels; bit 1: repeat next byte `(n >> 2) + 1` times; else copy
//!   `(n >> 2) + 1` literal bytes.
//!
//! [`Gaf::write`] writes that layout back (TA Reborn's own feature art, `assets/reborn/anims`):
//! every frame compressed, frames that repeat stored once.
//!
//! [`Gaf::parse`] decodes a whole file at once, every frame as painted: for TA's own art and
//! anything of its sizes (256 MiB of decoded frames at most). [`GafReader`] reads one frame at
//! a time, whole or reduced as its rows are read (`reader.rs`): a mod's explosions painted
//! thousands of pixels across (TA: Escalation's, up to 3240 x 3240, 238 MB a file) are drawn so
//! (`drawn.rs` plans how much each is reduced).

mod drawn;
mod reader;

pub use drawn::{AnimSize, EXPLOSION_TEXELS, MAX_DRAWN_SIDE, NATIVE_SIDE, Plan, explosion_budget, plan};
pub use reader::{EntryHeader, FrameHeader, GafReader, ReducedFrame};

use crate::bin::{Budget, fixed_str, i16_at, u16_at, u32_at, u8_at, slice, table};
use crate::image::IndexedImage;
use crate::{FormatError, Result};

const GAF_VERSION: u32 = 0x0001_0100;
const MAX_SUBFRAME_DEPTH: u32 = 4;
/// The most pixels one frame decoded whole may have.
const MAX_FRAME_PIXELS: usize = 16 << 20;
/// What [`Gaf::parse`] may hold of a file decoded whole (frames, their layers and their names).
/// Stock ur-buildings1.gaf needs about 150 MiB including its composite layers.
const WHOLE_BUDGET: usize = 256 << 20;

#[derive(Clone, Debug)]
pub struct GafFrame {
    pub image: IndexedImage,
    /// Pixel of the image that is placed at the sprite's position.
    pub origin_x: i32,
    pub origin_y: i32,
    pub transparency_index: u8,
    /// Per-frame value stored alongside the frame pointer (used as a duration by TA:K).
    pub duration: u32,
}

#[derive(Clone, Debug)]
pub struct GafEntry {
    pub name: String,
    pub frames: Vec<GafFrame>,
}

#[derive(Clone, Debug, Default)]
pub struct Gaf {
    pub entries: Vec<GafEntry>,
}

impl Gaf {
    pub fn parse(data: &[u8]) -> Result<Gaf> {
        let version = u32_at(data, 0)?;
        if version != GAF_VERSION {
            return Err(FormatError::invalid("GAF", format!("bad version {version:#010x}")));
        }
        let count = u32_at(data, 4)? as usize;
        if count > data.len() / 4 {
            return Err(FormatError::invalid("GAF", format!("implausible entry count {count}")));
        }
        table(data, 12, count, 4)?;
        let mut budget = Budget(WHOLE_BUDGET);
        budget.take(count * std::mem::size_of::<GafEntry>(), "GAF")?;
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let ptr = u32_at(data, 12 + i * 4)? as usize;
            entries.push(parse_entry(data, ptr, &mut budget)?);
        }
        Ok(Gaf { entries })
    }

    /// Case-insensitive entry lookup.
    pub fn entry(&self, name: &str) -> Option<&GafEntry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// The GAF as a file: entries in order, each frame's rows compressed (its transparent
    /// pixels skipped, runs of a colour repeated, the rest copied), a frame that repeats
    /// another (a held pose of an animation) pointing at the same data. Names must fit 31
    /// non-NUL Latin-1 bytes. Frames obey the reader's pixel budget, their sides fit `u16`
    /// and origins fit `i16`; each compressed row must fit its `u16` length field.
    pub fn write(&self) -> Result<Vec<u8>> {
        let invalid = |detail: String| FormatError::invalid("GAF", detail);
        let mut names = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            // The reader maps each name byte to one Latin-1 character. UTF-8 here
            // would silently change non-ASCII names on a write/read round trip.
            let name: Vec<u8> = e.name.chars().map(|c| u8::try_from(c as u32))
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| invalid(format!("entry {:?}: name is not Latin-1", e.name)))?;
            if name.len() > 31 || name.contains(&0) || e.frames.len() > u16::MAX as usize {
                return Err(invalid(format!("entry {:?} ({} frames)", e.name, e.frames.len())));
            }
            names.push(name);
        }
        let mut out = Vec::new();
        let put32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        put32(&mut out, GAF_VERSION);
        put32(&mut out, self.entries.len() as u32);
        put32(&mut out, 0);
        let pointers = out.len();
        out.resize(pointers + 4 * self.entries.len(), 0);
        // (Each distinct frame's header, by its contents.)
        let mut written: std::collections::HashMap<Vec<u8>, u32> = std::collections::HashMap::new();
        let mut frames_at = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            let mut ptrs = Vec::with_capacity(e.frames.len());
            for f in &e.frames {
                let img = &f.image;
                if img.width > u16::MAX as u32 || img.height > u16::MAX as u32 || img.width * img.height > 16 << 20
                    || img.pixels.len() != (img.width * img.height) as usize || img.opaque.len() != img.pixels.len() {
                    return Err(invalid(format!("{}: a frame of {}x{} pixels", e.name, img.width, img.height)));
                }
                let (Ok(ox), Ok(oy)) = (i16::try_from(f.origin_x), i16::try_from(f.origin_y)) else {
                    return Err(invalid(format!("{}: origin {},{}", e.name, f.origin_x, f.origin_y)));
                };
                let mut header = Vec::with_capacity(24);
                header.extend_from_slice(&(img.width as u16).to_le_bytes());
                header.extend_from_slice(&(img.height as u16).to_le_bytes());
                header.extend_from_slice(&ox.to_le_bytes());
                header.extend_from_slice(&oy.to_le_bytes());
                header.push(f.transparency_index);
                header.push(1);
                let rows = compress(img)?;
                let key = [header.as_slice(), rows.as_slice()].concat();
                let ptr = match written.get(&key) {
                    Some(&p) => p,
                    None => {
                        let p = out.len() as u32;
                        out.extend_from_slice(&header);
                        out.extend_from_slice(&0u16.to_le_bytes());
                        put32(&mut out, 0);
                        put32(&mut out, p + 24);
                        put32(&mut out, 0);
                        out.extend_from_slice(&rows);
                        written.insert(key, p);
                        p
                    }
                };
                ptrs.push((ptr, f.duration));
            }
            frames_at.push(ptrs);
        }
        for (i, ((e, ptrs), encoded_name)) in self.entries.iter().zip(&frames_at).zip(&names).enumerate() {
            let at = out.len() as u32;
            out[pointers + 4 * i..pointers + 4 * i + 4].copy_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&(e.frames.len() as u16).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            put32(&mut out, 0);
            let mut name = [0u8; 32];
            name[..encoded_name.len()].copy_from_slice(encoded_name);
            out.extend_from_slice(&name);
            for &(ptr, duration) in ptrs {
                put32(&mut out, ptr);
                put32(&mut out, duration);
            }
        }
        Ok(out)
    }
}

/// A frame's rows as TA compressed them: each a `u16` length and its control bytes (see the
/// module's notes), the transparent pixels at a row's end left out.
fn compress(img: &IndexedImage) -> Result<Vec<u8>> {
    let w = img.width as usize;
    let mut out = Vec::new();
    for y in 0..img.height as usize {
        let (pix, opaque) = (&img.pixels[y * w..(y + 1) * w], &img.opaque[y * w..(y + 1) * w]);
        let end = opaque.iter().rposition(|&o| o).map_or(0, |i| i + 1);
        let mut row = Vec::new();
        let mut x = 0;
        while x < end {
            if !opaque[x] {
                let n = opaque[x..end].iter().take_while(|&&o| !o).count().min(127);
                row.push(((n as u8) << 1) | 1);
                x += n;
                continue;
            }
            let run = pix[x..end].iter().zip(&opaque[x..end]).take_while(|&(&p, &o)| o && p == pix[x]).count();
            if run >= 3 {
                let n = run.min(64);
                row.push((((n - 1) as u8) << 2) | 2);
                row.push(pix[x]);
                x += n;
                continue;
            }
            // Literals, up to the next transparent pixel or run of three.
            let mut n = 0;
            while x + n < end && n < 64 && opaque[x + n] {
                let p = pix[x + n];
                if x + n + 2 < end && opaque[x + n + 1] && opaque[x + n + 2] && pix[x + n + 1] == p && pix[x + n + 2] == p {
                    break;
                }
                n += 1;
            }
            row.push(((n - 1) as u8) << 2);
            row.extend_from_slice(&pix[x..x + n]);
            x += n;
        }
        let len = u16::try_from(row.len()).map_err(|_| FormatError::invalid("GAF", "compressed row exceeds 65535 bytes"))?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&row);
    }
    Ok(out)
}

/// Reads only the entry names of a GAF (cheap indexing of texture files).
pub fn entry_names(data: &[u8]) -> Result<Vec<String>> {
    if u32_at(data, 0)? != GAF_VERSION {
        return Err(FormatError::invalid("GAF", "bad version"));
    }
    let count = u32_at(data, 4)? as usize;
    table(data, 12, count, 4)?;
    let mut budget = Budget(WHOLE_BUDGET);
    budget.take(count * std::mem::size_of::<String>(), "GAF")?;
    (0..count)
        .map(|i| {
            let ptr = u32_at(data, 12 + i * 4)? as usize;
            let name = fixed_str(data, ptr + 8, 32)?;
            budget.take(name.len(), "GAF")?;
            Ok(name)
        })
        .collect()
}

fn parse_entry(data: &[u8], ptr: usize, budget: &mut Budget) -> Result<GafEntry> {
    let frame_count = u16_at(data, ptr)? as usize;
    let name = fixed_str(data, ptr + 8, 32)?;
    budget.take(name.len(), "GAF")?;
    table(data, ptr + 40, frame_count, 8)?;
    budget.take(frame_count * std::mem::size_of::<GafFrame>(), "GAF")?;
    let mut frames = Vec::with_capacity(frame_count);
    for f in 0..frame_count {
        let rec = ptr + 40 + f * 8;
        let frame_ptr = u32_at(data, rec)? as usize;
        let duration = u32_at(data, rec + 4)?;
        let mut frame = parse_frame(data, frame_ptr, 0, budget)?;
        frame.duration = duration;
        frames.push(frame);
    }
    Ok(GafEntry { name, frames })
}

/// What a frame (or one of its layers) of `width` x `height` costs [`Gaf::parse`]'s budget: its
/// pixels and their mask, its rows (a frame no pixels wide may still have thousands of them) and
/// the frame itself.
fn whole_cost(width: u32, height: u32) -> usize {
    width as usize * height as usize * 2 + height as usize * 4 + std::mem::size_of::<GafFrame>()
}

fn parse_frame(data: &[u8], ptr: usize, depth: u32, budget: &mut Budget) -> Result<GafFrame> {
    if depth > MAX_SUBFRAME_DEPTH {
        return Err(FormatError::invalid("GAF", "subframe nesting too deep"));
    }
    let width = u16_at(data, ptr)? as u32;
    let height = u16_at(data, ptr + 2)? as u32;
    let origin_x = i16_at(data, ptr + 4)? as i32;
    let origin_y = i16_at(data, ptr + 6)? as i32;
    let transparency_index = u8_at(data, ptr + 8)?;
    let compressed = u8_at(data, ptr + 9)? != 0;
    let subframes = u16_at(data, ptr + 10)? as usize;
    let data_ptr = u32_at(data, ptr + 16)? as usize;
    let pixels = width as usize * height as usize;
    if pixels > MAX_FRAME_PIXELS {
        return Err(FormatError::invalid("GAF", "frame exceeds 16 million pixels"));
    }
    budget.take(whole_cost(width, height), "GAF")?;
    if pixels == 0 {
        // Nothing to draw: its data is not read (frames may share one block of it).
        return Ok(GafFrame { image: IndexedImage::new(width, height, transparency_index), origin_x, origin_y, transparency_index, duration: 0 });
    }
    if subframes == 0 && !compressed {
        slice(data, data_ptr, pixels)?;
    } else if subframes != 0 {
        table(data, data_ptr, subframes, 4)?;
    } else {
        table(data, data_ptr, height as usize, 2)?;
    }

    let mut image = IndexedImage::new(width, height, transparency_index);
    if subframes == 0 {
        if compressed {
            decode_compressed(data, data_ptr, &mut image)?;
        } else {
            let src = slice(data, data_ptr, (width * height) as usize)?;
            image.pixels.copy_from_slice(src);
            for (o, &p) in image.opaque.iter_mut().zip(src) {
                *o = p != transparency_index;
            }
        }
    } else {
        // Composite layers: each layer's origin is aligned with the parent's origin.
        for s in 0..subframes {
            let sub_ptr = u32_at(data, data_ptr + s * 4)? as usize;
            let layer = parse_frame(data, sub_ptr, depth + 1, budget)?;
            let dx = origin_x - layer.origin_x;
            let dy = origin_y - layer.origin_y;
            for ly in 0..layer.image.height as i32 {
                let y = ly + dy;
                if y < 0 || y >= height as i32 {
                    continue;
                }
                for lx in 0..layer.image.width as i32 {
                    let x = lx + dx;
                    if x < 0 || x >= width as i32 {
                        continue;
                    }
                    let li = (ly as u32 * layer.image.width + lx as u32) as usize;
                    if layer.image.opaque[li] {
                        let di = (y as u32 * width + x as u32) as usize;
                        image.pixels[di] = layer.image.pixels[li];
                        image.opaque[di] = true;
                    }
                }
            }
        }
    }
    Ok(GafFrame { image, origin_x, origin_y, transparency_index, duration: 0 })
}

#[cfg(test)]
mod run_tests {
    use super::*;

    /// Runs past a row's end are cut at it (the rest of the row is not read), whatever they say.
    #[test]
    fn runs_stop_at_the_row_end() {
        // Two rows of a 3-pixel-wide frame: a repeat of 64 (mask 0b1111_1110: 63 << 2 | 2) and
        // a literal of 4 bytes (3 << 2), each longer than the row.
        let mut data = Vec::new();
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&[(63 << 2) | 2, 7]);
        data.extend_from_slice(&5u16.to_le_bytes());
        data.extend_from_slice(&[3 << 2, 1, 2, 3, 4]);
        let mut image = IndexedImage::new(3, 2, 0);
        decode_compressed(&data, 0, &mut image).unwrap();
        assert_eq!(image.pixels, vec![7, 7, 7, 1, 2, 3]);
        assert!(image.opaque.iter().all(|&o| o));
    }
}

fn decode_compressed(data: &[u8], mut pos: usize, image: &mut IndexedImage) -> Result<()> {
    let w = image.width as usize;
    for y in 0..image.height as usize {
        let row_len = u16_at(data, pos)? as usize;
        pos += 2;
        let row = slice(data, pos, row_len)?;
        pos += row_len;
        unpack_row(row, &mut image.pixels[y * w..(y + 1) * w], &mut image.opaque[y * w..(y + 1) * w], pos)?;
    }
    Ok(())
}

/// A compressed row's control bytes (see the module's notes) drawn into `pixels` and `opaque`,
/// a row of its frame: runs past the row's end are cut at it, and past its end nothing more is
/// drawn (the rest of its bytes are not read). `pos` is where the row's bytes end (errors say).
fn unpack_row(row: &[u8], pixels: &mut [u8], opaque: &mut [bool], pos: usize) -> Result<()> {
    let w = pixels.len();
    let mut i = 0;
    let mut x = 0usize;
    while i < row.len() && x < w {
        let mask = row[i];
        i += 1;
        if mask & 1 != 0 {
            x += (mask >> 1) as usize;
        } else if mask & 2 != 0 {
            let count = (mask >> 2) as usize + 1;
            let v = *row.get(i).ok_or(FormatError::Eof { offset: pos, needed: 1 })?;
            i += 1;
            let n = count.min(w - x);
            pixels[x..x + n].fill(v);
            opaque[x..x + n].fill(true);
            x += count;
        } else {
            let count = (mask >> 2) as usize + 1;
            let run = row.get(i..i + count).ok_or(FormatError::Eof { offset: pos, needed: count })?;
            i += count;
            let n = count.min(w - x);
            pixels[x..x + n].copy_from_slice(&run[..n]);
            opaque[x..x + n].fill(true);
            x += count;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, origin: (i32, i32), at: impl Fn(u32, u32) -> Option<u8>) -> GafFrame {
        let mut image = IndexedImage::new(w, h, 9);
        for y in 0..h {
            for x in 0..w {
                if let Some(p) = at(x, y) {
                    image.pixels[(y * w + x) as usize] = p;
                    image.opaque[(y * w + x) as usize] = true;
                }
            }
        }
        GafFrame { image, origin_x: origin.0, origin_y: origin.1, transparency_index: 9, duration: 0 }
    }

    #[test]
    fn a_written_gaf_reads_back_as_it_was() {
        // Gaps longer than one skip reaches, runs longer than one repeat, literals, a pixel of the
        // transparent colour that is opaque, rows ending transparent, a frame repeated.
        let wide = frame(300, 5, (150, 4), |x, y| match (x, y) {
            (0..=139, 0) => None,
            (_, 0) => Some((x % 7) as u8 + 20),
            (_, 1) => Some(200),
            (0..=99, 2) => Some(9),
            (_, 2) => None,
            (_, 3) => (x % 3 != 0).then_some((x / 2 % 256) as u8),
            _ => None,
        });
        let small = frame(3, 2, (-1, 40), |x, y| (x + y != 2).then_some((x * 3 + y) as u8));
        let mut held = small.clone();
        held.duration = 4;
        let gaf = Gaf {
            entries: vec![
                GafEntry { name: "garland".into(), frames: vec![wide.clone(), small.clone(), held, small.clone()] },
                GafEntry { name: "garlandshad".into(), frames: vec![small.clone()] },
            ],
        };
        let bytes = gaf.write().unwrap();
        let back = Gaf::parse(&bytes).unwrap();
        assert_eq!(back.entries.len(), 2);
        for (a, b) in gaf.entries.iter().zip(&back.entries) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.frames.len(), b.frames.len());
            for (fa, fb) in a.frames.iter().zip(&b.frames) {
                assert_eq!(fa.image, fb.image);
                assert_eq!((fa.origin_x, fa.origin_y, fa.transparency_index, fa.duration), (fb.origin_x, fb.origin_y, fb.transparency_index, fb.duration));
            }
        }
        // The repeated frames are stored once: no bigger than the file with each frame once.
        let once = Gaf { entries: vec![GafEntry { name: "garland".into(), frames: vec![wide, small] }] }.write().unwrap();
        assert!(bytes.len() < once.len() + 100, "{} against {}", bytes.len(), once.len());
        assert_eq!(entry_names(&bytes).unwrap(), ["garland", "garlandshad"]);
    }

    #[test]
    fn names_too_long_are_refused() {
        let gaf = Gaf { entries: vec![GafEntry { name: "a".repeat(32), frames: vec![] }] };
        assert!(gaf.write().is_err());
    }

    #[test]
    fn entry_names_preserve_single_byte_characters_and_reject_unrepresentable_text() {
        for name in ["Métal", &"é".repeat(31)] {
            let gaf = Gaf { entries: vec![GafEntry { name: name.into(), frames: vec![] }] };
            let bytes = gaf.write().unwrap();
            assert_eq!(Gaf::parse(&bytes).unwrap().entries[0].name, name);
            assert_eq!(entry_names(&bytes).unwrap(), [name]);
        }
        for name in ["bad\0name", "界", &"é".repeat(32)] {
            let gaf = Gaf { entries: vec![GafEntry { name: name.into(), frames: vec![] }] };
            assert!(gaf.write().is_err(), "accepted unrepresentable name {name:?}");
        }
    }

    #[test]
    fn compressed_row_lengths_never_wrap() {
        // Literal packets add one byte per 64 pixels: these rows encode to exactly
        // 65,535 and 65,536 bytes, on either side of the u16 row-length limit.
        let make = |width| Gaf { entries: vec![GafEntry {
            name: "wide".into(), frames: vec![frame(width, 2, (0, 0), |x, y| Some(((x + y) % 251) as u8))],
        }] };
        let valid = make(64_526);
        let decoded = Gaf::parse(&valid.write().unwrap()).unwrap();
        assert_eq!(decoded.entries[0].frames[0].image, valid.entries[0].frames[0].image);
        assert!(make(64_527).write().is_err(), "a row length must not truncate to zero");
        // Width alone is not the limit: a maximum-width solid row still fits.
        let solid = Gaf { entries: vec![GafEntry { name: "solid".into(), frames: vec![frame(65_535, 1, (0, 0), |_, _| Some(7))] }] };
        assert_eq!(Gaf::parse(&solid.write().unwrap()).unwrap().entries[0].frames[0].image, solid.entries[0].frames[0].image);
    }

    #[test]
    fn composite_layers_align_clip_and_preserve_transparent_gaps() {
        let gaf = Gaf { entries: vec![GafEntry { name: "layers".into(), frames: vec![
            frame(3, 3, (1, 1), |_, _| None),
            frame(3, 3, (2, 2), |x, y| Some((40 + y * 3 + x) as u8)),
            frame(2, 2, (0, 0), |x, y| (x + y != 0).then_some((90 + y * 2 + x) as u8)),
        ] }] };
        let mut bytes = gaf.write().unwrap();
        let entry = u32_at(&bytes, 12).unwrap() as usize;
        let parent = u32_at(&bytes, entry + 40).unwrap() as usize;
        let layers = [u32_at(&bytes, entry + 48).unwrap(), u32_at(&bytes, entry + 56).unwrap()];
        let table = bytes.len() as u32;
        for layer in layers { bytes.extend_from_slice(&layer.to_le_bytes()); }
        bytes[parent + 10..parent + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[parent + 16..parent + 20].copy_from_slice(&table.to_le_bytes());
        let parsed = Gaf::parse(&bytes).unwrap();
        let image = &parsed.entries[0].frames[0].image;
        assert_eq!(image.pixels, [44, 45, 9, 47, 48, 91, 9, 92, 93]);
        assert_eq!(image.opaque, [true, true, false, true, true, true, false, true, true]);
        // A corrupt layer table pointing back to its parent is rejected before
        // recursion or allocation can grow without bound.
        bytes[table as usize..table as usize + 4].copy_from_slice(&(parent as u32).to_le_bytes());
        assert!(Gaf::parse(&bytes).unwrap_err().to_string().contains("nesting too deep"));
    }

    #[test]
    fn raw_frames_use_their_transparent_index_and_reject_truncation_and_huge_sizes() {
        let gaf = Gaf { entries: vec![GafEntry { name: "raw".into(), frames: vec![frame(3, 2, (0, 0), |_, _| None)] }] };
        let mut bytes = gaf.write().unwrap();
        let entry = u32_at(&bytes, 12).unwrap() as usize;
        let header = u32_at(&bytes, entry + 40).unwrap() as usize;
        let data = bytes.len() as u32;
        bytes[header + 9] = 0;
        bytes[header + 16..header + 20].copy_from_slice(&data.to_le_bytes());
        bytes.extend_from_slice(&[1, 9, 3, 9, 5, 6]);
        let parsed = Gaf::parse(&bytes).unwrap();
        assert_eq!(parsed.entries[0].frames[0].image.pixels, [1, 9, 3, 9, 5, 6]);
        assert_eq!(parsed.entries[0].frames[0].image.opaque, [true, false, true, false, true, true]);
        bytes.pop();
        assert!(Gaf::parse(&bytes).is_err());
        bytes[header..header + 4].fill(255);
        assert!(Gaf::parse(&bytes).unwrap_err().to_string().contains("16 million pixels"));
        bytes[header..header + 4].fill(0);
        let empty = Gaf::parse(&bytes).unwrap();
        assert!(empty.entries[0].frames[0].image.pixels.is_empty());
    }
}
