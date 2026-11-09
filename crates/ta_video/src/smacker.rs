//! Smacker container: header, frame table, per-frame chunk layout and the public API.
//!
//! All integers are little-endian. A file is laid out as:
//!
//! ```text
//! offset  size  field
//!      0     4  signature "SMK2" (or "SMK4", which adds FULL-block modes)
//!      4     4  width in pixels
//!      8     4  height in stored rows (see flags)
//!     12     4  frame count, excluding the optional ring frame
//!     16     4  frame rate, signed: > 0 milliseconds per frame, < 0 units of 10 µs per
//!               frame, 0 means 10 fps
//!     20     4  flags: bit 0 ring frame appended, bit 1 Y-interlaced, bit 2 Y-doubled
//!     24    28  largest decoded audio chunk per track (7 × u32)
//!     52     4  size of the packed Huffman tree block
//!     56    16  in-memory sizes of the MMAP, MCLR, FULL and TYPE trees
//!     72    28  audio track format words (7 × u32, see the `audio` module)
//!    100     4  reserved
//!    104   4·N  frame sizes, N = frames (+1 with a ring frame); bits 0–1 are flags
//!               (bit 0: keyframe) and must be masked off
//!      …     N  frame types: bit 0 palette chunk present, bit k+1 audio chunk for track k
//!      …        packed Huffman trees (MMAP, MCLR, FULL, TYPE; see the `video` module)
//!      …        frames, back to back
//! ```
//!
//! A frame holds, in order: the palette chunk if flagged (see the `palette` module), one
//! chunk per flagged audio track (a `u32` length that includes itself, then the data),
//! and the video bitstream, which fills the rest of the frame.
//!
//! The ring frame, when present, transforms the last frame back into the first so that a
//! looping player can continue without a keyframe; [`SmackerVideo`] does not play it and
//! instead restarts from scratch on [`SmackerVideo::rewind`].
//!
//! "Y-interlaced" and "Y-doubled" files store half the display height: the player shows
//! each stored row on every other line (leaving the lines between black) or twice.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use crate::audio::{self, AudioTrackInfo};
use crate::bits::BitReader;
use crate::bytes::{slice, u32_at};
use crate::palette::{self, Palette};
use crate::video::{self, VideoTrees};
use crate::{Result, VideoError};

const MAX_TRACKS: usize = 7;
/// Largest frame (in pixels) accepted; Smacker content is far smaller.
const MAX_PIXELS: u64 = 4096 * 4096;

const FLAG_RING_FRAME: u32 = 0x01;
const FLAG_Y_INTERLACE: u32 = 0x02;
const FLAG_Y_DOUBLE: u32 = 0x04;
/// Frame-type bit announcing a palette chunk; bit `k + 1` announces audio track `k`.
const FRAME_PALETTE: u8 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmackerVersion {
    Smk2,
    Smk4,
}

/// Vertical presentation of the stored rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YScale {
    /// Rows are shown as stored.
    None,
    /// Each stored row fills every other display line; the lines between are blank.
    Interlace,
    /// Each stored row is shown twice.
    Double,
}

impl YScale {
    /// Display lines per stored row.
    pub fn factor(self) -> u32 {
        match self {
            YScale::None => 1,
            YScale::Interlace | YScale::Double => 2,
        }
    }
}

/// The fixed 104-byte file header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmackerHeader {
    pub version: SmackerVersion,
    pub width: u32,
    /// Stored rows per frame; see [`SmackerHeader::display_height`].
    pub height: u32,
    /// Number of frames, excluding the ring frame.
    pub frames: u32,
    /// Raw frame-rate field; see [`SmackerHeader::frame_duration`].
    pub frame_rate: i32,
    pub flags: u32,
    /// Largest decoded audio chunk of each track, in bytes.
    pub audio_sizes: [u32; MAX_TRACKS],
    /// Size of the packed Huffman tree block.
    pub trees_size: u32,
    /// In-memory sizes of the MMAP, MCLR, FULL and TYPE trees (informational).
    pub tree_sizes: [u32; 4],
    /// Raw audio format words.
    pub audio_formats: [u32; MAX_TRACKS],
}

impl SmackerHeader {
    /// Length of the fixed header in bytes.
    pub const LEN: usize = 104;

    pub fn parse(data: &[u8]) -> Result<SmackerHeader> {
        let signature = slice(data, 0, 4)?;
        let version = match signature {
            b"SMK2" => SmackerVersion::Smk2,
            b"SMK4" => SmackerVersion::Smk4,
            other => return Err(VideoError::NotSmacker(other.escape_ascii().to_string())),
        };
        slice(data, 0, Self::LEN)?;
        let field = |i: usize| u32_at(data, 4 * i);
        let per_track = |first: usize| -> Result<[u32; MAX_TRACKS]> {
            let mut words = [0; MAX_TRACKS];
            for (t, word) in words.iter_mut().enumerate() {
                *word = field(first + t)?;
            }
            Ok(words)
        };
        let header = SmackerHeader {
            version,
            width: field(1)?,
            height: field(2)?,
            frames: field(3)?,
            frame_rate: field(4)? as i32,
            flags: field(5)?,
            audio_sizes: per_track(6)?,
            trees_size: field(13)?,
            tree_sizes: [field(14)?, field(15)?, field(16)?, field(17)?],
            audio_formats: per_track(18)?,
        };
        let pixels = u64::from(header.width) * u64::from(header.height);
        if header.width == 0 || header.height == 0 || pixels > MAX_PIXELS {
            return Err(VideoError::invalid(
                "Smacker header",
                format!("unsupported frame size {}x{}", header.width, header.height),
            ));
        }
        Ok(header)
    }

    pub fn has_ring_frame(&self) -> bool {
        self.flags & FLAG_RING_FRAME != 0
    }

    pub fn y_scale(&self) -> YScale {
        if self.flags & FLAG_Y_INTERLACE != 0 {
            YScale::Interlace
        } else if self.flags & FLAG_Y_DOUBLE != 0 {
            YScale::Double
        } else {
            YScale::None
        }
    }

    /// Height of the presented picture, accounting for Y-interlacing / doubling.
    pub fn display_height(&self) -> u32 {
        self.height * self.y_scale().factor()
    }

    /// How long each frame is shown.
    pub fn frame_duration(&self) -> Duration {
        match self.frame_rate {
            rate if rate > 0 => Duration::from_millis(rate as u64),
            rate if rate < 0 => Duration::from_micros(u64::from(rate.unsigned_abs()) * 10),
            _ => Duration::from_millis(100),
        }
    }
}

/// Location and flags of one frame in the file.
#[derive(Clone, Copy, Debug)]
struct FrameEntry {
    offset: usize,
    size: usize,
    keyframe: bool,
    /// The frame-type byte: palette and audio-track chunk flags.
    contents: u8,
}

/// One decoded frame.
#[derive(Clone)]
pub struct VideoFrame {
    /// Frame number, from 0.
    pub index: usize,
    pub width: u32,
    /// Stored rows; `pixels` holds `width × height` indices.
    pub height: u32,
    pub y_scale: YScale,
    /// Whether the file marks this frame as a keyframe (a safe seek point).
    pub keyframe: bool,
    /// Whether this frame changed the palette.
    pub palette_changed: bool,
    /// Row-major palette indices.
    pub pixels: Vec<u8>,
    pub palette: Palette,
    /// Interleaved 16-bit PCM decoded from this frame, one vector per entry of
    /// [`SmackerVideo::audio_tracks`] (empty when the frame carries none for a track).
    ///
    /// Audio chunks are not frame-sized: encoders put roughly a second of pre-roll into the
    /// first frame and correspondingly leave the last frames silent. Queue every chunk back
    /// to back and start playback together with frame 0 to stay in sync.
    pub audio: Vec<Vec<i16>>,
}

impl fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoFrame")
            .field("index", &self.index)
            .field("size", &(self.width, self.height))
            .field("y_scale", &self.y_scale)
            .field("keyframe", &self.keyframe)
            .field("palette_changed", &self.palette_changed)
            .field("audio_samples", &self.audio.iter().map(Vec::len).collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl VideoFrame {
    pub fn display_height(&self) -> u32 {
        self.height * self.y_scale.factor()
    }

    /// RGBA at display size (`width × display_height`). Interlaced and doubled files are
    /// line-doubled, which is the usual way to present interlaced Smacker on modern
    /// displays.
    pub fn to_rgba(&self) -> Vec<u8> {
        self.expand(self.y_scale.factor() as usize, false)
    }

    /// RGBA at display size with the original player's scanlines: for Y-interlaced files
    /// every second line is black. Identical to [`VideoFrame::to_rgba`] otherwise.
    pub fn to_rgba_scanlines(&self) -> Vec<u8> {
        self.expand(self.y_scale.factor() as usize, self.y_scale == YScale::Interlace)
    }

    /// RGBA of the stored rows only (`width × height`), e.g. for GPU-side scaling.
    pub fn to_rgba_stored(&self) -> Vec<u8> {
        self.expand(1, false)
    }

    fn expand(&self, repeat: usize, blank_repeats: bool) -> Vec<u8> {
        let width = self.width as usize;
        if width == 0 {
            return Vec::new();
        }
        let lut = self.palette.map(|[r, g, b]| [r, g, b, 255]);
        let mut out = Vec::with_capacity(self.pixels.len() * 4 * repeat);
        for row in self.pixels.chunks_exact(width) {
            let start = out.len();
            for &p in row {
                out.extend_from_slice(&lut[usize::from(p)]);
            }
            for _ in 1..repeat {
                if blank_repeats {
                    for _ in 0..width {
                        out.extend_from_slice(&[0, 0, 0, 255]);
                    }
                } else {
                    out.extend_from_within(start..start + width * 4);
                }
            }
        }
        out
    }
}

/// A Smacker video held in memory, decoded frame by frame.
pub struct SmackerVideo {
    data: Vec<u8>,
    header: SmackerHeader,
    /// Every frame including the ring frame.
    frames: Vec<FrameEntry>,
    tracks: Vec<AudioTrackInfo>,
    trees: VideoTrees,
    /// The current picture; frames are coded as changes to it.
    pixels: Vec<u8>,
    palette: Palette,
    next: usize,
}

impl fmt::Debug for SmackerVideo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmackerVideo")
            .field("header", &self.header)
            .field("tracks", &self.tracks)
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl SmackerVideo {
    /// Parses a whole Smacker file held in memory.
    pub fn open(data: Vec<u8>) -> Result<SmackerVideo> {
        let header = SmackerHeader::parse(&data)?;
        let count = (header.frames as usize).saturating_add(usize::from(header.has_ring_frame()));
        // Frame sizes and types take five bytes per frame. Checking that they fit first
        // also bounds the frame table allocation by the file size.
        let table = slice(&data, SmackerHeader::LEN, count.saturating_mul(5))?;
        let (sizes, types) = table.split_at(count * 4);
        let trees_at = SmackerHeader::LEN + table.len();
        let trees_block = slice(&data, trees_at, header.trees_size as usize)?;
        let trees = VideoTrees::read(trees_block)?;

        let mut offset = trees_at + trees_block.len();
        let mut frames = Vec::with_capacity(count);
        for (&size, &contents) in sizes.as_chunks::<4>().0.iter().zip(types) {
            let raw = u32::from_le_bytes(size);
            let size = (raw & !3) as usize;
            frames.push(FrameEntry { offset, size, keyframe: raw & 1 != 0, contents });
            // Frames past the end of a truncated file fail when decoded, not here.
            offset = offset.saturating_add(size);
        }
        if offset > data.len() {
            log::warn!(
                "Smacker file is truncated: frames need {offset} bytes, file has {}",
                data.len()
            );
        }

        let tracks: Vec<AudioTrackInfo> = (0..MAX_TRACKS)
            .filter_map(|t| {
                AudioTrackInfo::from_header(t, header.audio_formats[t], header.audio_sizes[t])
            })
            .collect();
        for t in tracks.iter().filter(|t| !t.is_decodable()) {
            log::warn!("audio track {} uses unsupported Bink audio and will be silent", t.track);
        }

        let pixels = vec![0; header.width as usize * header.height as usize];
        Ok(SmackerVideo {
            data,
            header,
            frames,
            tracks,
            trees,
            pixels,
            palette: [[0; 3]; 256],
            next: 0,
        })
    }

    /// Reads and parses a Smacker file from disk.
    pub fn open_file(path: impl AsRef<Path>) -> Result<SmackerVideo> {
        SmackerVideo::open(std::fs::read(path)?)
    }

    pub fn header(&self) -> &SmackerHeader {
        &self.header
    }

    pub fn width(&self) -> u32 {
        self.header.width
    }

    /// Stored rows per frame.
    pub fn height(&self) -> u32 {
        self.header.height
    }

    /// Presented rows per frame (twice [`SmackerVideo::height`] for interlaced/doubled files).
    pub fn display_height(&self) -> u32 {
        self.header.display_height()
    }

    pub fn y_scale(&self) -> YScale {
        self.header.y_scale()
    }

    /// Number of frames (the ring frame is not counted).
    pub fn frame_count(&self) -> usize {
        self.header.frames as usize
    }

    pub fn frame_duration(&self) -> Duration {
        self.header.frame_duration()
    }

    /// Frames per second.
    pub fn frame_rate(&self) -> f64 {
        1.0 / self.frame_duration().as_secs_f64()
    }

    /// Total running time.
    pub fn duration(&self) -> Duration {
        self.frame_duration().saturating_mul(self.header.frames)
    }

    /// The audio tracks present in the file, in track order. [`VideoFrame::audio`] has one
    /// entry per element.
    pub fn audio_tracks(&self) -> &[AudioTrackInfo] {
        &self.tracks
    }

    /// Index of the next frame [`SmackerVideo::next_frame`] will decode.
    pub fn position(&self) -> usize {
        self.next
    }

    pub fn is_finished(&self) -> bool {
        self.next >= self.frame_count()
    }

    /// The most recently decoded picture (`width × height` palette indices).
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The palette of the most recently decoded frame.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Restarts playback from the first frame.
    pub fn rewind(&mut self) {
        self.next = 0;
        self.pixels.fill(0);
        self.palette = [[0; 3]; 256];
    }

    /// Decodes the next frame, or returns `None` after the last one.
    ///
    /// On error the decoder has still moved past the frame (and applied as much of it as
    /// it could), so calling `next_frame` again continues with the following frame.
    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        let index = self.next;
        if index >= self.frame_count() {
            return Ok(None);
        }
        self.next += 1;
        let (palette_changed, audio) = self
            .decode_frame(index)
            .map_err(|source| VideoError::Frame { frame: index, source: Box::new(source) })?;
        Ok(Some(VideoFrame {
            index,
            width: self.header.width,
            height: self.header.height,
            y_scale: self.header.y_scale(),
            keyframe: self.frames[index].keyframe,
            palette_changed,
            pixels: self.pixels.clone(),
            palette: self.palette,
            audio,
        }))
    }

    /// Applies frame `index` to the decoder state; returns whether the palette changed and
    /// the decoded audio per track.
    fn decode_frame(&mut self, index: usize) -> Result<(bool, Vec<Vec<i16>>)> {
        let entry = self.frames[index];
        let frame = slice(&self.data, entry.offset, entry.size)?;
        let mut at = 0usize;

        let palette_changed = entry.contents & FRAME_PALETTE != 0;
        if palette_changed {
            let len = usize::from(slice(frame, 0, 1)?[0]) * 4;
            if len == 0 {
                return Err(VideoError::invalid("palette chunk", "zero length"));
            }
            palette::apply(slice(frame, 1, len - 1)?, &mut self.palette)?;
            at = len;
        }

        let mut audio = vec![Vec::new(); self.tracks.len()];
        let mut audio_error = None;
        for track in 0..MAX_TRACKS {
            if u32::from(entry.contents) & (2 << track) == 0 {
                continue;
            }
            let len = u32_at(frame, at)? as usize;
            if len < 4 {
                return Err(VideoError::invalid("audio chunk", format!("length {len}")));
            }
            let chunk = slice(frame, at + 4, len - 4)?;
            at += len;
            if let Some(slot) = self.tracks.iter().position(|t| t.track == track) {
                // A damaged audio chunk must not stop the picture from updating: remember
                // the error and report it once the video has been decoded.
                if let Err(e) = audio::decode_chunk(chunk, &self.tracks[slot], &mut audio[slot]) {
                    audio_error.get_or_insert(e);
                }
            }
        }

        let video = frame.get(at..).unwrap_or_default();
        let mut bits = BitReader::new(video, "video");
        video::decode_frame(
            &mut bits,
            &mut self.trees,
            &mut self.pixels,
            self.header.width as usize,
            self.header.height as usize,
            self.header.version == SmackerVersion::Smk4,
        )?;
        // Frames are padded to four bytes, so a decoder in sync with the encoder leaves
        // fewer than 32 bits behind. More suggests damaged data.
        let unused = bits.bits_left();
        if unused >= 32 {
            log::debug!("frame {index}: {unused} bits of video data left unused");
        }
        match audio_error {
            Some(e) => Err(e),
            None => Ok((palette_changed, audio)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bits::BitWriter;
    use crate::huffman::testutil::{Shape, write_byte_tree};
    use crate::video::testutil::{Trees, descriptor, put};

    const W: u32 = 8;
    const H: u32 = 4;

    /// Builds a two-frame 8×4 SMK2 file with one 8-bit mono DPCM track.
    fn synthetic_file(flags: u32) -> Vec<u8> {
        let solid = |colour| descriptor(3, 0, colour);
        let mono = descriptor(0, 0, 0);
        let void = descriptor(2, 0, 0);
        let trees = Trees::new(&[0x00FF], &[0x0201], &[0], &[solid(1), mono, void]);

        // Frame 0: palette, audio, SOLID(1) + MONO. Frame 1: audio only, VOID + SOLID(1)
        // (the second block becomes solid 1).
        let mut palette = vec![0u8]; // length byte, patched below
        palette.extend([0x3F, 0x00, 0x00]); // entry 0: red
        palette.extend([0x00, 0x3F, 0x00]); // entry 1: green
        palette.extend([0x00, 0x00, 0x3F]); // entry 2: blue
        palette.extend([0xFF, 0xFF]); // keep entries 3..=255
        assert!(palette.len().is_multiple_of(4));
        palette[0] = (palette.len() / 4) as u8;

        let deltas = Shape::balanced(&[0x01, 0xFF]);
        let audio_chunk = |first: u8, steps: &[u16]| {
            let mut w = BitWriter::default();
            w.bits(0b001, 3);
            write_byte_tree(&mut w, &deltas);
            w.bits(u32::from(first), 8);
            for &d in steps {
                w.code(&deltas.code(d).unwrap());
            }
            let mut body = ((steps.len() + 1) as u32).to_le_bytes().to_vec();
            body.extend(w.finish());
            let mut chunk = ((body.len() + 4) as u32).to_le_bytes().to_vec();
            chunk.extend(body);
            chunk
        };

        let mut v0 = BitWriter::default();
        put(&mut v0, &trees.types, solid(1));
        put(&mut v0, &trees.types, mono);
        put(&mut v0, &trees.mclr, 0x0201);
        put(&mut v0, &trees.mmap, 0x00FF);
        let mut frame0 = palette.clone();
        frame0.extend(audio_chunk(0x80, &[0x01, 0x01]));
        frame0.extend(v0.finish());

        let mut v1 = BitWriter::default();
        put(&mut v1, &trees.types, void);
        put(&mut v1, &trees.types, solid(1));
        let mut frame1 = audio_chunk(0x40, &[0xFF]);
        frame1.extend(v1.finish());

        let mut frames = vec![frame0, frame1];
        if flags & FLAG_RING_FRAME != 0 {
            frames.push(frames[0].clone());
        }
        for f in &mut frames {
            while !f.len().is_multiple_of(4) {
                f.push(0);
            }
        }

        let tree_block = trees.serialise();
        let mut file = Vec::new();
        file.extend(b"SMK2");
        for v in [W, H, 2, (-3333i32) as u32, flags] {
            file.extend(v.to_le_bytes());
        }
        let mut audio_sizes = [0u32; 7];
        audio_sizes[0] = 3;
        for v in audio_sizes {
            file.extend(v.to_le_bytes());
        }
        file.extend((tree_block.len() as u32).to_le_bytes());
        for _ in 0..4 {
            file.extend(64u32.to_le_bytes());
        }
        let mut formats = [0u32; 7];
        formats[0] = 0xC000_5622; // compressed, present, 8-bit mono, 22050 Hz
        for v in formats {
            file.extend(v.to_le_bytes());
        }
        file.extend(0u32.to_le_bytes());
        assert_eq!(file.len(), SmackerHeader::LEN);
        for (i, f) in frames.iter().enumerate() {
            let keyframe = u32::from(i == 0);
            file.extend((f.len() as u32 | keyframe).to_le_bytes());
        }
        file.push(0b11); // palette + track 0
        file.push(0b10); // track 0
        if frames.len() == 3 {
            file.push(0b11);
        }
        file.extend(tree_block);
        for f in frames {
            file.extend(f);
        }
        file
    }

    #[test]
    fn decodes_synthetic_file() {
        let mut video = SmackerVideo::open(synthetic_file(FLAG_Y_INTERLACE)).unwrap();
        assert_eq!((video.width(), video.height(), video.display_height()), (8, 4, 8));
        assert_eq!(video.frame_count(), 2);
        assert_eq!(video.frame_duration(), Duration::from_micros(33_330));
        assert_eq!(video.audio_tracks().len(), 1);
        assert_eq!(video.audio_tracks()[0].sample_rate, 22050);

        let f0 = video.next_frame().unwrap().unwrap();
        assert!(f0.keyframe && f0.palette_changed);
        assert_eq!(f0.palette[0], [255, 0, 0]);
        assert_eq!(f0.palette[2], [0, 0, 255]);
        // Block 0 solid colour 1; block 1 MONO with colours (lo 1, hi 2) and mask 0x00FF,
        // which selects the high colour for rows 0–1.
        assert_eq!(f0.pixels[..8], [1, 1, 1, 1, 2, 2, 2, 2]);
        assert_eq!(f0.pixels[8..16], [1, 1, 1, 1, 2, 2, 2, 2]);
        assert_eq!(f0.pixels[16..], [1; 16]);
        assert_eq!(f0.audio, vec![vec![0, 0x100, 0x200]]);

        let rgba = f0.to_rgba();
        assert_eq!(rgba.len(), 8 * 8 * 4);
        assert_eq!(rgba[..4], [0, 255, 0, 255]); // colour 1 = green
        assert_eq!(rgba[32..36], [0, 255, 0, 255]); // doubled line
        let scan = f0.to_rgba_scanlines();
        assert_eq!(scan[32..36], [0, 0, 0, 255]); // blank interlace line
        assert_eq!(f0.to_rgba_stored().len(), 8 * 4 * 4);

        let f1 = video.next_frame().unwrap().unwrap();
        assert!(!f1.keyframe && !f1.palette_changed);
        assert_eq!(f1.pixels, [1; 32]);
        assert_eq!(f1.audio, vec![vec![-0x4000, -0x4100]]);
        assert!(video.next_frame().unwrap().is_none());

        video.rewind();
        let again = video.next_frame().unwrap().unwrap();
        assert_eq!(again.pixels, f0.pixels);
        assert_eq!(again.palette, f0.palette);
    }

    #[test]
    fn ring_frame_is_not_played() {
        let mut video = SmackerVideo::open(synthetic_file(FLAG_RING_FRAME)).unwrap();
        assert!(video.header().has_ring_frame());
        assert_eq!(video.y_scale(), YScale::None);
        assert_eq!(video.frame_count(), 2);
        let mut n = 0;
        while video.next_frame().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 2);
    }

    #[test]
    fn frame_rate_field() {
        let mut header = SmackerHeader::parse(&synthetic_file(0)).unwrap();
        header.frame_rate = 40;
        assert_eq!(header.frame_duration(), Duration::from_millis(40));
        header.frame_rate = 0;
        assert_eq!(header.frame_duration(), Duration::from_millis(100));
        header.frame_rate = i32::MIN;
        assert!(header.frame_duration() > Duration::from_secs(1000));
    }

    #[test]
    fn rejects_non_smacker_data() {
        assert!(matches!(SmackerVideo::open(b"RIFF....".to_vec()), Err(VideoError::NotSmacker(_))));
        assert!(SmackerVideo::open(b"SMK2".to_vec()).is_err());
        assert!(SmackerVideo::open(Vec::new()).is_err());
    }

    /// Every truncation and every single-byte corruption must produce errors or garbage
    /// pictures, never a panic.
    #[test]
    fn damaged_files_never_panic() {
        let file = synthetic_file(FLAG_Y_DOUBLE);
        let play = |data: Vec<u8>| {
            if let Ok(mut video) = SmackerVideo::open(data) {
                for _ in 0..video.frame_count() + 1 {
                    if let Ok(Some(frame)) = video.next_frame() {
                        let _ = frame.to_rgba();
                    }
                }
            }
        };
        for len in 0..file.len() {
            play(file[..len].to_vec());
        }
        for i in 0..file.len() {
            for x in [0x01u8, 0x80, 0xFF] {
                let mut damaged = file.clone();
                damaged[i] ^= x;
                play(damaged);
            }
        }
    }

    #[test]
    fn errors_skip_the_damaged_frame() {
        let mut file = synthetic_file(0);
        // Truncate the last byte of frame 1's video data.
        let len = file.len();
        file.truncate(len - 4);
        let mut video = SmackerVideo::open(file).unwrap();
        assert!(video.next_frame().unwrap().is_some());
        let err = video.next_frame().unwrap_err();
        assert!(matches!(err, VideoError::Frame { frame: 1, .. }), "{err}");
        assert!(video.next_frame().unwrap().is_none());
    }
}
