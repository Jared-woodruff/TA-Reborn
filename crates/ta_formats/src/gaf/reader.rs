//! Reading a GAF one frame at a time ([`GafReader`]).
//!
//! [`GafReader::open`] reads the entries' names and their frames' headers (sizes, origins) and
//! nothing else. A frame is decoded when it is asked for: whole ([`GafReader::frame`], as
//! [`super::Gaf::parse`] decodes it) or reduced ([`GafReader::reduced`]), its rows read one at a
//! time, its composite layers' rows laid over each other as they are read, and each row summed
//! into the reduced frame's texels at once: no frame, nor any of its layers, is ever held whole.
//! [`GafReader::check`] reads a frame through and keeps nothing (the mod validator).
//!
//! What a hostile file can make a reader do is bounded by what it really holds, not by how big
//! its frames say they are:
//! * every frame and layer decoded costs its pixels, however many frames point at the same
//!   data: [`WORK_FLOOR`] pixels in all, or [`WORK_PER_BYTE`] for each byte of the file if that
//!   is more (Escalation's 238 MB antimatter blast decodes 330 million, every layer counted);
//! * the frames' headers are bounded as [`super::Gaf::parse`] bounds them (256 MiB), and so is
//!   one frame decoded whole (16 million pixels);
//! * a reduced frame is at most [`super::MAX_DRAWN_SIDE`] texels across and down;
//! * layers nest four deep at most, and while a frame is read its reader holds one of its rows
//!   and one row for each depth of layers.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{GAF_VERSION, GafEntry, GafFrame, MAX_DRAWN_SIDE, MAX_FRAME_PIXELS, MAX_SUBFRAME_DEPTH, WHOLE_BUDGET, parse_frame, unpack_row, whole_cost};
use crate::bin::{Budget, fixed_str, i16_at, slice, table, u16_at, u32_at, u8_at};
use crate::image::Palette;
use crate::{FormatError, Result};

/// Pixels any reader may decode, whatever its file's size (frames and their layers, each counted
/// every time it is read).
pub const WORK_FLOOR: u64 = 256 << 20;
/// Pixels a reader may decode for each byte of its file, where that is more than [`WORK_FLOOR`]:
/// a raw frame holds a byte a pixel, a compressed one rarely more than a few pixels a byte.
pub const WORK_PER_BYTE: u64 = 16;

/// A frame's header, read by [`GafReader::open`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub width: u32,
    pub height: u32,
    /// Pixel of the frame that is placed at the sprite's position.
    pub origin_x: i32,
    pub origin_y: i32,
    pub transparency_index: u8,
    /// The value stored beside the frame's pointer (see [`GafFrame::duration`]).
    pub duration: u32,
    /// Where its header is in the file.
    at: usize,
}

/// An entry's name and its frames' headers.
#[derive(Clone, Debug)]
pub struct EntryHeader {
    pub name: String,
    pub frames: Vec<FrameHeader>,
}

impl EntryHeader {
    /// Its frames' sizes, for planning how it is drawn.
    pub fn size(&self) -> super::AnimSize {
        super::AnimSize::of(self.frames.iter().map(|f| (f.width, f.height)))
    }
}

/// A GAF read frame by frame (see the module's notes). It may be shared between threads: each
/// frame is read on its own.
pub struct GafReader<'a> {
    data: &'a [u8],
    pub entries: Vec<EntryHeader>,
    /// Pixels it may still decode.
    work: AtomicU64,
}

/// A frame read reduced ([`GafReader::reduced`]): each texel `scale` x `scale` of its pixels,
/// drawn as big as the frame was painted (its texels `scale` pixels across, its origin where it
/// was).
#[derive(Clone, Debug, PartialEq)]
pub struct ReducedFrame {
    /// Texels across and down: `ceil(full_width / scale)` x `ceil(full_height / scale)`.
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, sRGB, straight alpha: a texel's colour is the mean, in linear light, of
    /// the opaque pixels it covers (black if none), its alpha the share of it they cover.
    pub rgba: Vec<u8>,
    /// Pixels to a texel (more than 1).
    pub scale: f64,
    /// The frame as painted: its size, and the pixel placed at the sprite's position.
    pub full_width: u32,
    pub full_height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub duration: u32,
}

impl<'a> GafReader<'a> {
    /// Reads the file's entries and their frames' headers.
    pub fn open(data: &'a [u8]) -> Result<GafReader<'a>> {
        let version = u32_at(data, 0)?;
        if version != GAF_VERSION {
            return Err(FormatError::invalid("GAF", format!("bad version {version:#010x}")));
        }
        let count = u32_at(data, 4)? as usize;
        if count > data.len() / 4 {
            return Err(FormatError::invalid("GAF", format!("implausible entry count {count}")));
        }
        table(data, 12, count, 4)?;
        // (As `Gaf::parse` bounds them: entries sharing a table of frames list it again.)
        let mut budget = Budget(WHOLE_BUDGET);
        budget.take(count * std::mem::size_of::<EntryHeader>(), "GAF")?;
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let ptr = u32_at(data, 12 + i * 4)? as usize;
            let frame_count = u16_at(data, ptr)? as usize;
            let name = fixed_str(data, ptr + 8, 32)?;
            budget.take(name.len(), "GAF")?;
            table(data, ptr + 40, frame_count, 8)?;
            budget.take(frame_count * std::mem::size_of::<FrameHeader>(), "GAF")?;
            let mut frames = Vec::with_capacity(frame_count);
            for f in 0..frame_count {
                let rec = ptr + 40 + f * 8;
                let at = u32_at(data, rec)? as usize;
                frames.push(FrameHeader {
                    width: u16_at(data, at)? as u32,
                    height: u16_at(data, at + 2)? as u32,
                    origin_x: i16_at(data, at + 4)? as i32,
                    origin_y: i16_at(data, at + 6)? as i32,
                    transparency_index: u8_at(data, at + 8)?,
                    duration: u32_at(data, rec + 4)?,
                    at,
                });
            }
            entries.push(EntryHeader { name, frames });
        }
        let work = WORK_FLOOR.max((data.len() as u64).saturating_mul(WORK_PER_BYTE));
        Ok(GafReader { data, entries, work: AtomicU64::new(work) })
    }

    /// Case-insensitive entry lookup (the first of a name, as [`super::Gaf::entry`]).
    pub fn entry(&self, name: &str) -> Option<&EntryHeader> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// A frame decoded whole, as [`super::Gaf::parse`] decodes it (at most 16 million pixels).
    pub fn frame(&self, f: &FrameHeader) -> Result<GafFrame> {
        self.charge_tree(f.at, 0)?;
        let mut frame = parse_frame(self.data, f.at, 0, &mut Budget(WHOLE_BUDGET))?;
        frame.duration = f.duration;
        Ok(frame)
    }

    /// A frame reduced as its rows are read: `scale` (1 or more) of its pixels to a texel
    /// across and down, coloured with `palette`, at most [`MAX_DRAWN_SIDE`] texels a side.
    pub fn reduced(&self, f: &FrameHeader, scale: f64, palette: &Palette) -> Result<ReducedFrame> {
        if !(scale.is_finite() && scale >= 1.0) {
            return Err(FormatError::invalid("GAF", format!("a frame reduced by {scale}")));
        }
        let (w, h) = (f.width as usize, f.height as usize);
        let side = |n: usize| (n as f64 / scale).ceil() as usize;
        let (tw, th) = (side(w), side(h));
        if tw > MAX_DRAWN_SIDE as usize || th > MAX_DRAWN_SIDE as usize {
            return Err(FormatError::invalid("GAF", format!("a {w} x {h} frame reduced by {scale:.2} is still {tw} x {th}, over {MAX_DRAWN_SIDE} a side")));
        }
        let mut out = ReducedFrame {
            width: tw as u32,
            height: th as u32,
            rgba: vec![0; tw * th * 4],
            scale,
            full_width: f.width,
            full_height: f.height,
            origin_x: f.origin_x,
            origin_y: f.origin_y,
            duration: f.duration,
        };
        if tw == 0 || th == 0 {
            return Ok(out);
        }
        self.charge_tree(f.at, 0)?;
        let mut rows = Rows::open(self.data, f.at)?;
        let mut scratch = rows.scratch();
        let (mut pixels, mut opaque) = (vec![0u8; w], vec![false; w]);
        let linear: Vec<[f32; 3]> = palette.colors.iter().map(|c| c.map(to_linear)).collect();
        // Where each column of pixels goes: the column of texels it starts in, its share there
        // (the rest in the next).
        let columns: Vec<(usize, f32)> = (0..w).map(|x| split(x, scale, tw)).collect();
        // One row of pixels summed across into texels (linear colour times share, and share);
        // and the two rows of texels a row of pixels falls in.
        let mut line = vec![[0f32; 4]; tw];
        let mut texels = [vec![[0f32; 4]; tw], vec![[0f32; 4]; tw]];
        let mut row_of_texels = 0usize;
        let area = (scale * scale) as f32;
        for y in 0..h {
            rows.row(&mut pixels, &mut opaque, &mut scratch)?;
            let (j, down) = split(y, scale, th);
            while row_of_texels < j {
                emit(&mut out.rgba, row_of_texels, &texels[0], area);
                texels.swap(0, 1);
                texels[1].fill([0.0; 4]);
                row_of_texels += 1;
            }
            if !opaque.contains(&true) {
                continue;
            }
            line.fill([0.0; 4]);
            for (x, (&p, _)) in pixels.iter().zip(&opaque).enumerate().filter(|(_, (_, o))| **o) {
                let [r, g, b] = linear[p as usize];
                let (i, across) = columns[x];
                let t = &mut line[i];
                *t = [t[0] + r * across, t[1] + g * across, t[2] + b * across, t[3] + across];
                if across < 1.0 {
                    let rest = 1.0 - across;
                    let t = &mut line[i + 1];
                    *t = [t[0] + r * rest, t[1] + g * rest, t[2] + b * rest, t[3] + rest];
                }
            }
            for (k, share) in [(0, down), (1, 1.0 - down)] {
                if share > 0.0 {
                    for (t, l) in texels[k].iter_mut().zip(&line) {
                        *t = [t[0] + l[0] * share, t[1] + l[1] * share, t[2] + l[2] * share, t[3] + l[3] * share];
                    }
                }
            }
        }
        emit(&mut out.rgba, row_of_texels, &texels[0], area);
        if row_of_texels + 1 < th {
            emit(&mut out.rgba, row_of_texels + 1, &texels[1], area);
        }
        Ok(out)
    }

    /// Reads a frame through, every row of it and of its layers, keeping nothing: whether it is
    /// sound, as [`GafReader::frame`] would find it. Returns what decoding it whole would cost
    /// [`super::Gaf::parse`]'s budget (`u64::MAX` if it or a layer is too big for that at all).
    pub fn check(&self, f: &FrameHeader) -> Result<u64> {
        self.charge_tree(f.at, 0)?;
        let mut rows = Rows::open(self.data, f.at)?;
        let mut scratch = rows.scratch();
        let (mut pixels, mut opaque) = (vec![0u8; rows.width], vec![false; rows.width]);
        while rows.y < rows.height {
            rows.row(&mut pixels, &mut opaque, &mut scratch)?;
        }
        rows.drain(&mut scratch)?;
        Ok(rows.whole)
    }

    /// Whether [`super::Gaf::parse`] could decode the file whole, given every frame's cost from
    /// [`GafReader::check`] (entry by entry, in order).
    pub fn whole(&self, costs: &[Vec<u64>]) -> bool {
        let entries = (self.entries.len() as u64).saturating_mul(std::mem::size_of::<GafEntry>() as u64);
        let total = self.entries.iter().zip(costs).fold(entries, |sum, (e, costs)| {
            let frames = (e.frames.len() as u64).saturating_mul(std::mem::size_of::<GafFrame>() as u64);
            costs.iter().fold(sum.saturating_add(e.name.len() as u64).saturating_add(frames), |s, &c| s.saturating_add(c))
        });
        total <= WHOLE_BUDGET as u64
    }

    /// Charges the work of decoding the frame at `at` and its layers: their pixels, and for a
    /// composite a step for each of its rows and layers (each layer is looked at every row),
    /// before any of it is read (layers nest no deeper than `Gaf::parse` takes them).
    fn charge_tree(&self, at: usize, depth: u32) -> Result<()> {
        if depth > MAX_SUBFRAME_DEPTH {
            return Err(FormatError::invalid("GAF", "subframe nesting too deep"));
        }
        let h = Header::read(self.data, at)?;
        let pixels = u64::from(h.width) * u64::from(h.height);
        let layers = if pixels == 0 { 0 } else { h.subframes };
        let work = 1 + pixels + u64::from(h.height) * layers as u64;
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(work))
            .map_err(|_| FormatError::invalid("GAF", "frames to decode exceed what the file's size allows"))?;
        if layers > 0 {
            table(self.data, h.data_ptr, layers, 4)?;
            for s in 0..layers {
                self.charge_tree(u32_at(self.data, h.data_ptr + s * 4)? as usize, depth + 1)?;
            }
        }
        Ok(())
    }
}

/// A frame's header fields, as `parse_frame` reads them.
struct Header {
    width: u32,
    height: u32,
    origin_x: i32,
    origin_y: i32,
    transparency: u8,
    compressed: bool,
    subframes: usize,
    data_ptr: usize,
}

impl Header {
    fn read(data: &[u8], at: usize) -> Result<Header> {
        Ok(Header {
            width: u16_at(data, at)? as u32,
            height: u16_at(data, at + 2)? as u32,
            origin_x: i16_at(data, at + 4)? as i32,
            origin_y: i16_at(data, at + 6)? as i32,
            transparency: u8_at(data, at + 8)?,
            compressed: u8_at(data, at + 9)? != 0,
            subframes: u16_at(data, at + 10)? as usize,
            data_ptr: u32_at(data, at + 16)? as usize,
        })
    }
}

/// A frame's rows, read one at a time from the top, its layers' rows laid over each other as
/// `parse_frame` lays their images (each layer's origin on the frame's; a layer's opaque pixels
/// over what is there).
struct Rows<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    origin_x: i32,
    origin_y: i32,
    transparency: u8,
    kind: Kind<'a>,
    /// Rows read.
    y: usize,
    /// What decoding it whole costs `Gaf::parse` (see [`GafReader::check`]).
    whole: u64,
    /// The widest layer at each depth under it (the rows they are read into, see `scratch`).
    widths: [usize; MAX_SUBFRAME_DEPTH as usize + 1],
}

enum Kind<'a> {
    /// No pixels: its data is not read (frames may share one block of it).
    Empty,
    /// Raw pixels from here, a row after another.
    Raw(usize),
    /// Compressed rows: the next one's length is here.
    Packed(usize),
    Layers(Vec<Rows<'a>>),
}

/// A row of a layer for each depth of layers: what one is read into before it is laid on its
/// frame's row.
type Scratch = Vec<(Vec<u8>, Vec<bool>)>;

impl<'a> Rows<'a> {
    fn open(data: &'a [u8], at: usize) -> Result<Rows<'a>> {
        Rows::open_at(data, at, 0)
    }

    /// Checks the extent of its data as `parse_frame` does before decoding (a layer's rows are
    /// read only as far as its frame's reach; `drain` reads the rest).
    fn open_at(data: &'a [u8], at: usize, depth: u32) -> Result<Rows<'a>> {
        if depth > MAX_SUBFRAME_DEPTH {
            return Err(FormatError::invalid("GAF", "subframe nesting too deep"));
        }
        let h = Header::read(data, at)?;
        let pixels = h.width as usize * h.height as usize;
        let mut whole = if pixels > MAX_FRAME_PIXELS { u64::MAX } else { whole_cost(h.width, h.height) as u64 };
        let mut widths = [0usize; MAX_SUBFRAME_DEPTH as usize + 1];
        let kind = if pixels == 0 {
            Kind::Empty
        } else if h.subframes == 0 && !h.compressed {
            slice(data, h.data_ptr, pixels)?;
            Kind::Raw(h.data_ptr)
        } else if h.subframes != 0 {
            table(data, h.data_ptr, h.subframes, 4)?;
            let mut layers = Vec::with_capacity(h.subframes);
            for s in 0..h.subframes {
                let layer = Rows::open_at(data, u32_at(data, h.data_ptr + s * 4)? as usize, depth + 1)?;
                whole = whole.saturating_add(layer.whole);
                widths[0] = widths[0].max(layer.width);
                for (d, &lw) in layer.widths.iter().enumerate().take(MAX_SUBFRAME_DEPTH as usize) {
                    widths[d + 1] = widths[d + 1].max(lw);
                }
                layers.push(layer);
            }
            Kind::Layers(layers)
        } else {
            table(data, h.data_ptr, h.height as usize, 2)?;
            Kind::Packed(h.data_ptr)
        };
        Ok(Rows {
            data,
            width: h.width as usize,
            height: h.height as usize,
            origin_x: h.origin_x,
            origin_y: h.origin_y,
            transparency: h.transparency,
            kind,
            y: 0,
            whole,
            widths,
        })
    }

    /// The rows its layers are read into, one for each depth (some none wide).
    fn scratch(&self) -> Scratch {
        self.widths.iter().map(|&w| (vec![0u8; w], vec![false; w])).collect()
    }

    /// Reads its next row into `pixels` and `opaque` (its width), its layers' rows through
    /// `scratch` (one row for each depth of them under it).
    fn row(&mut self, pixels: &mut [u8], opaque: &mut [bool], scratch: &mut [(Vec<u8>, Vec<bool>)]) -> Result<()> {
        let w = self.width;
        if self.y >= self.height {
            return Err(FormatError::invalid("GAF", "a frame read past its last row"));
        }
        match &mut self.kind {
            Kind::Empty => {
                pixels.fill(self.transparency);
                opaque.fill(false);
            }
            Kind::Raw(at) => {
                let row = &self.data[*at + self.y * w..*at + (self.y + 1) * w];
                pixels.copy_from_slice(row);
                for (o, &p) in opaque.iter_mut().zip(row) {
                    *o = p != self.transparency;
                }
            }
            Kind::Packed(pos) => {
                let len = u16_at(self.data, *pos)? as usize;
                *pos += 2;
                let row = slice(self.data, *pos, len)?;
                *pos += len;
                pixels.fill(self.transparency);
                opaque.fill(false);
                unpack_row(row, pixels, opaque, *pos)?;
            }
            Kind::Layers(layers) => {
                pixels.fill(self.transparency);
                opaque.fill(false);
                let Some(((lp, lo), deeper)) = scratch.split_first_mut() else {
                    return Err(FormatError::invalid("GAF", "subframe nesting too deep"));
                };
                for layer in layers.iter_mut() {
                    // (The layer's row this row of the frame shows, if any.)
                    let ly = self.y as i64 - i64::from(self.origin_y - layer.origin_y);
                    if ly < 0 || ly >= layer.height as i64 {
                        continue;
                    }
                    let lw = layer.width;
                    // Rows above the frame's top are read and passed over.
                    while (layer.y as i64) <= ly {
                        layer.row(&mut lp[..lw], &mut lo[..lw], deeper)?;
                    }
                    let dx = i64::from(self.origin_x - layer.origin_x);
                    let from = (-dx).max(0) as usize;
                    let to = (w as i64 - dx).clamp(0, lw as i64) as usize;
                    for lx in from..to.max(from) {
                        if lo[lx] {
                            let x = (lx as i64 + dx) as usize;
                            pixels[x] = lp[lx];
                            opaque[x] = true;
                        }
                    }
                }
            }
        }
        self.y += 1;
        Ok(())
    }

    /// Reads the rest of every layer under it (those of its rows its frame never showed), as
    /// `parse_frame`, decoding each layer whole, does.
    fn drain(&mut self, scratch: &mut [(Vec<u8>, Vec<bool>)]) -> Result<()> {
        if let Kind::Layers(layers) = &mut self.kind {
            let Some(((lp, lo), deeper)) = scratch.split_first_mut() else {
                return Err(FormatError::invalid("GAF", "subframe nesting too deep"));
            };
            for layer in layers.iter_mut() {
                let lw = layer.width;
                while layer.y < layer.height {
                    layer.row(&mut lp[..lw], &mut lo[..lw], deeper)?;
                }
                layer.drain(deeper)?;
            }
        }
        Ok(())
    }
}

/// The texel (along one side) pixel `n` starts in at `scale` pixels to a texel, and the share
/// of the pixel in it (the rest in the next one); `count` texels in all.
fn split(n: usize, scale: f64, count: usize) -> (usize, f32) {
    let start = n as f64 / scale;
    let texel = start.floor();
    let share = ((texel + 1.0).min((n + 1) as f64 / scale) - start) * scale;
    let i = (texel as usize).min(count.saturating_sub(1));
    // (Rounding must not leave a sliver for a texel past the last one.)
    if share >= 1.0 - 1e-6 || i + 1 >= count { (i, 1.0) } else { (i, share as f32) }
}

/// A row of texels (summed linear colour times share, and share) written as RGBA8: the mean
/// colour of what is opaque in each, its share of the texel's `area` (pixels) its alpha.
fn emit(rgba: &mut [u8], row: usize, texels: &[[f32; 4]], area: f32) {
    let w = texels.len();
    for (i, t) in texels.iter().enumerate() {
        let at = (row * w + i) * 4;
        if t[3] <= 0.0 {
            rgba[at..at + 4].fill(0);
            continue;
        }
        let colour = [t[0] / t[3], t[1] / t[3], t[2] / t[3]].map(to_srgb);
        rgba[at..at + 3].copy_from_slice(&colour);
        rgba[at + 3] = ((t[3] / area).clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

fn to_linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::super::{Gaf, GafEntry, GafFrame};
    use super::*;
    use crate::image::IndexedImage;

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
        GafFrame { image, origin_x: origin.0, origin_y: origin.1, transparency_index: 9, duration: 3 }
    }

    fn palette() -> Palette {
        let mut p = Palette::default();
        p.colors[1] = [255, 0, 0];
        p.colors[2] = [0, 0, 255];
        p.colors[3] = [255, 255, 255];
        p
    }

    #[test]
    fn frames_read_whole_one_at_a_time_are_those_parse_reads() {
        let gaf = Gaf { entries: vec![
            GafEntry { name: "boom".into(), frames: vec![frame(40, 30, (20, 15), |x, y| ((x + y) % 3 != 0).then_some((x * 7 + y) as u8)), frame(3, 2, (-1, 4), |x, _| Some(x as u8))] },
            GafEntry { name: "Other".into(), frames: vec![frame(1, 1, (0, 0), |_, _| None)] },
        ] };
        let bytes = gaf.write().unwrap();
        let whole = Gaf::parse(&bytes).unwrap();
        let reader = GafReader::open(&bytes).unwrap();
        assert_eq!(reader.entries.len(), 2);
        assert!(reader.entry("OTHER").is_some() && reader.entry("none").is_none());
        for (e, w) in reader.entries.iter().zip(&whole.entries) {
            assert_eq!(e.name, w.name);
            for (f, wf) in e.frames.iter().zip(&w.frames) {
                assert_eq!((f.width, f.height, f.origin_x, f.origin_y, f.duration), (wf.image.width, wf.image.height, wf.origin_x, wf.origin_y, wf.duration));
                let one = reader.frame(f).unwrap();
                assert_eq!((one.image.clone(), one.origin_x, one.origin_y, one.duration), (wf.image.clone(), wf.origin_x, wf.origin_y, wf.duration));
                reader.check(f).unwrap();
            }
        }
    }

    #[test]
    fn a_reduced_frame_averages_its_pixels_in_linear_light_and_keeps_its_place() {
        // 10 x 4: the left half red, the right half blue, the last row transparent.
        let gaf = Gaf { entries: vec![GafEntry { name: "half".into(), frames: vec![frame(10, 4, (5, 3), |x, y| (y < 3).then_some(if x < 5 { 1 } else { 2 }))] }] };
        let bytes = gaf.write().unwrap();
        let reader = GafReader::open(&bytes).unwrap();
        let f = reader.entries[0].frames[0];
        let r = reader.reduced(&f, 2.5, &palette()).unwrap();
        assert_eq!((r.width, r.height, r.full_width, r.full_height, r.origin_x, r.origin_y, r.duration), (4, 2, 10, 4, 5, 3, 3));
        let texel = |x: usize, y: usize| &r.rgba[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
        // Texel 0 (pixels 0 to 2.5, rows 0 to 2.5): all red, all opaque.
        assert_eq!(texel(0, 0), [255, 0, 0, 255]);
        // Texel 1 (pixels 2.5 to 5): red too; texel 2 (5 to 7.5) blue.
        assert_eq!(texel(1, 0), [255, 0, 0, 255]);
        assert_eq!(texel(2, 0), [0, 0, 255, 255]);
        // The lower row of texels (rows 2.5 to 5, of which 2.5 to 3 is painted, 3 to 4
        // transparent and 4 to 5 past the frame): a fifth covered, its colour as painted.
        assert_eq!(texel(0, 1), [255, 0, 0, 51]);
        // Mixed across (pixels straddling 2.5 split between texels): a texel half red half
        // blue is the mean in linear light, brighter than the mean of the bytes.
        let mixed = GafReader::open(&bytes).unwrap().reduced(&f, 10.0, &palette()).unwrap();
        assert_eq!((mixed.width, mixed.height), (1, 1));
        assert_eq!(mixed.rgba[..3], [188, 0, 188], "half of each in linear light");
        assert!((76..=77).contains(&mixed.rgba[3]), "30 of 100 pixels: {}", mixed.rgba[3]);
    }

    #[test]
    fn composite_layers_reduce_as_their_composed_frame_does() {
        // A frame of two layers (as `composite_layers_align_clip_and_preserve_transparent_gaps`
        // builds it), reduced by 1.5: the same texels as its composed image reduced.
        let gaf = Gaf { entries: vec![GafEntry { name: "layers".into(), frames: vec![
            frame(6, 6, (2, 2), |_, _| None),
            frame(6, 6, (4, 4), |x, y| Some((40 + y * 6 + x) as u8)),
            frame(4, 5, (0, 1), |x, y| (x + y != 0).then_some((90 + y * 4 + x) as u8)),
        ] }] };
        let mut bytes = gaf.write().unwrap();
        let entry = u32_at(&bytes, 12).unwrap() as usize;
        let parent = u32_at(&bytes, entry + 40).unwrap() as usize;
        let layers = [u32_at(&bytes, entry + 48).unwrap(), u32_at(&bytes, entry + 56).unwrap()];
        let table = bytes.len() as u32;
        for layer in layers {
            bytes.extend_from_slice(&layer.to_le_bytes());
        }
        bytes[parent + 10..parent + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[parent + 16..parent + 20].copy_from_slice(&table.to_le_bytes());
        let composed = Gaf::parse(&bytes).unwrap().entries[0].frames[0].clone();
        let mut palette = Palette::default();
        for (i, c) in palette.colors.iter_mut().enumerate() {
            *c = [i as u8, (i * 3 % 256) as u8, 255 - i as u8];
        }
        let reader = GafReader::open(&bytes).unwrap();
        let f = reader.entries[0].frames[0];
        assert_eq!(reader.frame(&f).unwrap().image, composed.image);
        let layered = reader.reduced(&f, 1.5, &palette).unwrap();
        // The composed image alone, as a frame of its own.
        let flat = Gaf { entries: vec![GafEntry { name: "flat".into(), frames: vec![composed] }] }.write().unwrap();
        let flat_reader = GafReader::open(&flat).unwrap();
        let plain = flat_reader.reduced(&flat_reader.entries[0].frames[0], 1.5, &palette).unwrap();
        assert_eq!(layered.rgba, plain.rgba);
        assert!(layered.rgba.chunks(4).any(|t| t[3] == 255) && layered.rgba.chunks(4).any(|t| t[3] == 0), "some covered, some not");
        reader.check(&f).unwrap();
    }

    #[test]
    fn work_is_bounded_by_what_the_file_holds() {
        // One compressed 4000 x 4000 frame of empty rows (8 KB), listed 40 times: 640 million
        // pixels from a file of a few dozen kilobytes.
        let empty = frame(4000, 4000, (0, 0), |_, _| None);
        let gaf = Gaf { entries: vec![GafEntry { name: "many".into(), frames: vec![empty; 40] }] };
        let bytes = gaf.write().unwrap();
        assert!(bytes.len() < 64 << 10, "{}", bytes.len());
        let reader = GafReader::open(&bytes).unwrap();
        let frames = reader.entries[0].frames.clone();
        let read = frames.iter().take_while(|f| reader.reduced(f, 4.0, &palette()).is_ok()).count();
        assert_eq!(read, (WORK_FLOOR / 16_000_000) as usize, "the floor's worth, then refused");
        let err = reader.reduced(&frames[39], 4.0, &palette()).unwrap_err();
        assert!(err.to_string().contains("exceed what the file's size allows"), "{err}");
    }

    #[test]
    fn reductions_are_bounded_in_size_and_scale() {
        let gaf = Gaf { entries: vec![GafEntry { name: "wide".into(), frames: vec![frame(5000, 2, (0, 0), |_, _| Some(3))] }] };
        let bytes = gaf.write().unwrap();
        let reader = GafReader::open(&bytes).unwrap();
        let f = reader.entries[0].frames[0];
        assert!(reader.reduced(&f, 2.0, &palette()).unwrap_err().to_string().contains("over 2048 a side"));
        assert_eq!(reader.reduced(&f, 2.5, &palette()).unwrap().width, 2000);
        for bad in [0.5, f64::NAN, f64::INFINITY] {
            assert!(reader.reduced(&f, bad, &palette()).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_whole_cost_is_what_parse_would_take() {
        let gaf = Gaf { entries: vec![GafEntry { name: "a".into(), frames: vec![frame(30, 20, (0, 0), |x, _| Some(x as u8)), frame(2, 2, (0, 0), |_, _| None)] }] };
        let bytes = gaf.write().unwrap();
        let reader = GafReader::open(&bytes).unwrap();
        let costs: Vec<u64> = reader.entries[0].frames.iter().map(|f| reader.check(f).unwrap()).collect();
        assert_eq!(costs, [whole_cost(30, 20) as u64, whole_cost(2, 2) as u64]);
        assert!(reader.whole(std::slice::from_ref(&costs)));
        assert!(!reader.whole(&[vec![WHOLE_BUDGET as u64]]));
    }
}
