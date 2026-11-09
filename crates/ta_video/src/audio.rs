//! Audio tracks: format descriptors and chunk decoding.
//!
//! A Smacker file carries up to seven audio tracks. Each is described in the header by a
//! 32-bit word:
//!
//! ```text
//! bit 31     data is compressed (Smacker DPCM, below)
//! bit 30     track has data
//! bit 29     16-bit samples (otherwise 8-bit)
//! bit 28     stereo (otherwise mono)
//! bits 27/26 Bink audio (RDFT/DCT) instead of DPCM — only used by late encoders
//! bits 0–23  sample rate in Hz
//! ```
//!
//! Frames carry audio in chunks of any length; a player simply queues them back to back.
//! Samples are interleaved (left first); 8-bit audio is unsigned, 16-bit is signed.
//!
//! Uncompressed chunks are raw PCM. A compressed chunk starts with a `u32` giving the
//! decoded size in bytes, followed by an LSB-first bitstream:
//!
//! 1. `present`, `stereo`, `16-bit` flags (1 bit each). Without `present` the chunk
//!    decodes to silence; the other flags must match the header.
//! 2. `1 << (stereo + 16-bit)` byte trees (see the `huffman` module): one per channel for
//!    8-bit audio, a low-byte and a high-byte tree per channel for 16-bit audio, ordered
//!    left then right.
//! 3. The first sample of each channel, right channel first: 8 bits each for 8-bit audio;
//!    for 16-bit audio a high byte followed by a low byte.
//! 4. For every further sample, a delta decoded with the channel's tree(s) (16-bit deltas
//!    are low byte then high byte) and added, with wrap-around, to the channel's previous
//!    sample. The first samples count toward the decoded size.

use crate::bits::BitReader;
use crate::bytes::u32_at;
use crate::huffman::ByteTree;
use crate::{Result, VideoError};

/// Refuse chunks that claim to decode to more than this many bytes.
const MAX_DECODED_BYTES: usize = 1 << 24;

const FLAG_COMPRESSED: u32 = 1 << 31;
const FLAG_16BIT: u32 = 1 << 29;
const FLAG_STEREO: u32 = 1 << 28;
const FLAG_BINK_RDFT: u32 = 1 << 27;
const FLAG_BINK_DCT: u32 = 1 << 26;
const RATE_MASK: u32 = 0x00FF_FFFF;

/// How a track's chunks are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    /// Raw PCM: unsigned 8-bit or signed little-endian 16-bit.
    Pcm,
    /// Smacker's Huffman-coded DPCM.
    SmackerDpcm,
    /// Bink audio. Not supported: such tracks are listed but decode to no samples.
    Bink,
}

/// Format of one audio track. Decoded output is always interleaved signed 16-bit PCM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrackInfo {
    /// Track number in the file (0–6).
    pub track: usize,
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits per stored sample (8 or 16).
    pub bits_per_sample: u16,
    pub codec: AudioCodec,
    /// Largest decoded chunk of the track in bytes, as declared by the header.
    pub max_chunk_bytes: u32,
}

impl AudioTrackInfo {
    /// Describes track `track` from its header format word, if the track exists.
    pub(crate) fn from_header(track: usize, format: u32, max_chunk_bytes: u32) -> Option<Self> {
        let sample_rate = format & RATE_MASK;
        if sample_rate == 0 {
            return None;
        }
        let codec = if format & (FLAG_BINK_RDFT | FLAG_BINK_DCT) != 0 {
            AudioCodec::Bink
        } else if format & FLAG_COMPRESSED != 0 {
            AudioCodec::SmackerDpcm
        } else {
            AudioCodec::Pcm
        };
        Some(AudioTrackInfo {
            track,
            sample_rate,
            channels: if format & FLAG_STEREO != 0 { 2 } else { 1 },
            bits_per_sample: if format & FLAG_16BIT != 0 { 16 } else { 8 },
            codec,
            max_chunk_bytes,
        })
    }

    /// Whether this crate can decode the track's samples.
    pub fn is_decodable(&self) -> bool {
        self.codec != AudioCodec::Bink
    }

    fn bytes_per_sample(&self) -> usize {
        usize::from(self.bits_per_sample / 8)
    }
}

/// Converts unsigned 8-bit PCM to signed 16-bit.
#[inline]
fn widen_u8(sample: u8) -> i16 {
    (i16::from(sample) - 128) << 8
}

/// Decodes one audio chunk (the bytes after its length field), appending the samples.
pub(crate) fn decode_chunk(chunk: &[u8], track: &AudioTrackInfo, out: &mut Vec<i16>) -> Result<()> {
    match track.codec {
        AudioCodec::Pcm => {
            if track.bits_per_sample == 16 {
                out.extend(chunk.as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b)));
            } else {
                out.extend(chunk.iter().map(|&b| widen_u8(b)));
            }
            Ok(())
        }
        AudioCodec::SmackerDpcm => decode_dpcm(chunk, track, out),
        AudioCodec::Bink => Ok(()),
    }
}

fn decode_dpcm(chunk: &[u8], track: &AudioTrackInfo, out: &mut Vec<i16>) -> Result<()> {
    let decoded_bytes = u32_at(chunk, 0)? as usize;
    if decoded_bytes > MAX_DECODED_BYTES {
        return Err(VideoError::invalid(
            "audio chunk",
            format!("implausible decoded size {decoded_bytes}"),
        ));
    }
    let stereo = track.channels == 2;
    let wide = track.bits_per_sample == 16;
    let channels = usize::from(track.channels);
    // Interleaved sample count.
    let samples = decoded_bytes / track.bytes_per_sample();

    let mut bits = BitReader::new(&chunk[4..], "audio");
    if !bits.bit()? {
        out.resize(out.len() + samples, 0);
        return Ok(());
    }
    let (chunk_stereo, chunk_wide) = (bits.bit()?, bits.bit()?);
    if chunk_stereo != stereo || chunk_wide != wide {
        return Err(VideoError::invalid(
            "audio chunk",
            format!(
                "chunk is {} {}-bit but track {} is {} {}-bit",
                if chunk_stereo { "stereo" } else { "mono" },
                if chunk_wide { 16 } else { 8 },
                track.track,
                if stereo { "stereo" } else { "mono" },
                track.bits_per_sample,
            ),
        ));
    }
    if !decoded_bytes.is_multiple_of(channels * track.bytes_per_sample()) {
        return Err(VideoError::invalid(
            "audio chunk",
            format!("{decoded_bytes} bytes is not a whole number of {channels}-channel samples"),
        ));
    }

    let tree_count = 1 << (u32::from(stereo) + u32::from(wide));
    let trees = (0..tree_count).map(|_| ByteTree::read(&mut bits)).collect::<Result<Vec<_>>>()?;

    out.reserve(samples);
    let first = samples.min(channels);
    if wide {
        let mut last = [0i16; 2];
        for sample in last[..channels].iter_mut().rev() {
            let hi = bits.byte()?;
            let lo = bits.byte()?;
            *sample = i16::from_be_bytes([hi, lo]);
        }
        out.extend_from_slice(&last[..first]);
        for i in first..samples {
            let ch = i % channels;
            let lo = trees[2 * ch].decode(&mut bits)?;
            let hi = trees[2 * ch + 1].decode(&mut bits)?;
            last[ch] = last[ch].wrapping_add(i16::from_le_bytes([lo, hi]));
            out.push(last[ch]);
        }
    } else {
        let mut last = [0u8; 2];
        for sample in last[..channels].iter_mut().rev() {
            *sample = bits.byte()?;
        }
        out.extend(last[..first].iter().map(|&s| widen_u8(s)));
        for i in first..samples {
            let ch = i % channels;
            // Adding the delta's raw byte modulo 256 equals adding it sign-extended.
            last[ch] = last[ch].wrapping_add(trees[ch].decode(&mut bits)?);
            out.push(widen_u8(last[ch]));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bits::BitWriter;
    use crate::huffman::testutil::{Shape, write_byte_tree};

    fn track(format: u32) -> AudioTrackInfo {
        AudioTrackInfo::from_header(0, format, 0).unwrap()
    }

    fn chunk(decoded_bytes: u32, body: Vec<u8>) -> Vec<u8> {
        let mut c = decoded_bytes.to_le_bytes().to_vec();
        c.extend(body);
        c
    }

    #[test]
    fn header_format_words() {
        // TA's movies: compressed, present, 8-bit, stereo, 22050 Hz.
        let t = track(0xD000_5622);
        assert_eq!((t.sample_rate, t.channels, t.bits_per_sample), (22050, 2, 8));
        assert_eq!(t.codec, AudioCodec::SmackerDpcm);
        let t = track(0x6000_AC44);
        assert_eq!((t.sample_rate, t.channels, t.bits_per_sample), (44100, 1, 16));
        assert_eq!(t.codec, AudioCodec::Pcm);
        assert_eq!(track(0xC800_5622).codec, AudioCodec::Bink);
        assert!(AudioTrackInfo::from_header(0, 0, 0).is_none());
    }

    #[test]
    fn dpcm_8bit_mono() {
        let deltas = Shape::balanced(&[0x00, 0x01, 0xFF, 0x10]);
        let mut w = BitWriter::default();
        w.bits(0b001, 3); // present, mono, 8-bit
        write_byte_tree(&mut w, &deltas);
        w.bits(0x80, 8); // first sample
        for d in [0x01u16, 0x01, 0xFF, 0x10, 0x00] {
            w.code(&deltas.code(d).unwrap());
        }
        let data = chunk(6, w.finish());
        let mut out = Vec::new();
        decode_chunk(&data, &track(0xC000_5622), &mut out).unwrap();
        let expected: Vec<i16> =
            [0x80u8, 0x81, 0x82, 0x81, 0x91, 0x91].iter().map(|&s| widen_u8(s)).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn dpcm_8bit_stereo_wraps_and_reads_right_channel_first() {
        let left = Shape::balanced(&[0x02, 0xFE]);
        let right = Shape::Leaf(0x05); // single leaf: zero-length codes
        let mut w = BitWriter::default();
        w.bits(0b011, 3); // present, stereo, 8-bit
        write_byte_tree(&mut w, &left);
        write_byte_tree(&mut w, &right);
        w.bits(0x10, 8); // right channel's first sample
        w.bits(0xFF, 8); // left channel's first sample
        for d in [0x02u16, 0xFE] {
            w.code(&left.code(d).unwrap());
        }
        let data = chunk(6, w.finish());
        let mut out = Vec::new();
        decode_chunk(&data, &track(0xD000_5622), &mut out).unwrap();
        let expected: Vec<i16> =
            [0xFFu8, 0x10, 0x01, 0x15, 0xFF, 0x1A].iter().map(|&s| widen_u8(s)).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn dpcm_16bit_stereo() {
        // Trees: left low, left high, right low, right high.
        let left_lo = Shape::balanced(&[0x00, 0x10, 0xF0]);
        let left_hi = Shape::balanced(&[0x00, 0xFF]);
        let right_lo = Shape::Leaf(0x01);
        let right_hi = Shape::Leaf(0x01);
        let mut w = BitWriter::default();
        w.bits(0b111, 3); // present, stereo, 16-bit
        for t in [&left_lo, &left_hi, &right_lo, &right_hi] {
            write_byte_tree(&mut w, t);
        }
        for v in [0x8000u16, 0x1234] {
            // right first, high byte then low byte
            w.bits(u32::from(v >> 8), 8);
            w.bits(u32::from(v & 0xFF), 8);
        }
        // Left deltas: +0x0010, -0x0010 (0xFFF0), +0x0000.
        for d in [0x0010u16, 0xFFF0, 0x0000] {
            w.code(&left_lo.code(d & 0xFF).unwrap());
            w.code(&left_hi.code(d >> 8).unwrap());
        }
        let data = chunk(16, w.finish()); // 4 stereo samples
        let mut out = Vec::new();
        decode_chunk(&data, &track(0xF000_AC44), &mut out).unwrap();
        let expected: [i16; 8] =
            [0x1234, i16::MIN, 0x1244, -0x7EFF, 0x1234, -0x7DFE, 0x1234, -0x7CFD];
        assert_eq!(out, expected);
    }

    #[test]
    fn absent_data_is_silence_and_mismatches_are_errors() {
        let mut out = Vec::new();
        decode_chunk(&chunk(8, vec![0]), &track(0xD000_5622), &mut out).unwrap();
        assert_eq!(out, vec![0; 8]);

        // Chunk claims 16-bit mono on an 8-bit stereo track.
        let mut w = BitWriter::default();
        w.bits(0b101, 3);
        assert!(decode_chunk(&chunk(8, w.finish()), &track(0xD000_5622), &mut out).is_err());
        // Truncated chunks fail cleanly.
        assert!(decode_chunk(&[1, 0], &track(0xD000_5622), &mut out).is_err());
        let mut w = BitWriter::default();
        w.bits(0b011, 3);
        assert!(decode_chunk(&chunk(8, w.finish()), &track(0xD000_5622), &mut out).is_err());
    }

    #[test]
    fn raw_pcm() {
        let mut out = Vec::new();
        decode_chunk(&[0x80, 0xFF, 0x00], &track(0x4000_5622), &mut out).unwrap();
        assert_eq!(out, vec![0, 0x7F00, -0x8000]);
        out.clear();
        decode_chunk(&[0x34, 0x12, 0xFF, 0xFF, 0x01], &track(0x6000_5622), &mut out).unwrap();
        assert_eq!(out, vec![0x1234, -1]);
    }
}
