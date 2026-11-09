//! Clean-room decoder for the Smacker video format used by Total Annihilation's movies.
//!
//! TA's intro and campaign cutscenes are Smacker v2 (`SMK2`) files stored loose in the
//! install's `Data` directory under a `.zrb` extension (`1.zrb` … `5.zrb`). [`SmackerVideo`]
//! decodes them frame by frame into 8-bit palette-indexed images — with helpers that expand
//! a frame to RGBA at its display size — and interleaved 16-bit PCM for every audio track.
//! Presentation (texture upload, audio output, pacing) is left to the caller.
//!
//! The decoder is written from the publicly documented description of the format (the
//! multimedia.cx wiki "Smacker" page); no code from other implementations is used. Every
//! read is bounds-checked: malformed or truncated input yields a [`VideoError`], never a
//! panic.
//!
//! Internally the crate is split along the format's layers:
//! * `bits` — the LSB-first bit reader shared by every bitstream;
//! * `huffman` — Smacker's 8-bit and 16-bit ("big") Huffman trees, including the big
//!   trees' three-slot recent-value cache;
//! * `smacker` — the container: header, frame table, per-frame chunk layout and the public
//!   [`SmackerVideo`] / [`VideoFrame`] API;
//! * `palette`, `video`, `audio` — the three per-frame chunk codecs.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use ta_video::SmackerVideo;
//!
//! let mut video = SmackerVideo::open(std::fs::read("1.zrb")?)?;
//! println!("{}x{} @ {:?}/frame", video.width(), video.display_height(), video.frame_duration());
//! while let Some(frame) = video.next_frame()? {
//!     let rgba = frame.to_rgba(); // width × display_height × 4 bytes
//!     let pcm = frame.audio.first(); // interleaved i16 for audio_tracks()[0], if any
//!     // present `rgba`, queue `pcm`, then wait `video.frame_duration()`.
//!     # let _ = (rgba, pcm);
//! }
//! # Ok(())
//! # }
//! ```

mod audio;
mod bits;
mod huffman;
mod palette;
mod smacker;
mod video;

pub use audio::{AudioCodec, AudioTrackInfo};
pub use palette::Palette;
pub use smacker::{SmackerHeader, SmackerVersion, SmackerVideo, VideoFrame, YScale};

use thiserror::Error;

/// Errors raised while opening or decoding a Smacker file.
#[derive(Debug, Error)]
pub enum VideoError {
    #[error("not a Smacker file (signature \"{0}\")")]
    NotSmacker(String),
    #[error("unexpected end of data at offset {offset} (needed {needed} bytes)")]
    Eof { offset: usize, needed: usize },
    #[error("{0} bitstream ended early")]
    OutOfBits(&'static str),
    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
    /// A per-frame decoding failure. The decoder has already moved past the frame, so
    /// playback can continue with the next [`SmackerVideo::next_frame`] call.
    #[error("frame {frame}: {source}")]
    Frame {
        frame: usize,
        #[source]
        source: Box<VideoError>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl VideoError {
    pub(crate) fn invalid(what: &'static str, detail: impl Into<String>) -> Self {
        VideoError::Invalid { what, detail: detail.into() }
    }
}

pub type Result<T> = std::result::Result<T, VideoError>;

/// Bounds-checked little-endian field access for the container layer.
pub(crate) mod bytes {
    use crate::{Result, VideoError};

    #[inline]
    pub(crate) fn slice(data: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
        offset
            .checked_add(len)
            .and_then(|end| data.get(offset..end))
            .ok_or(VideoError::Eof { offset, needed: len })
    }

    #[inline]
    pub(crate) fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
        let b = slice(data, offset, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}
