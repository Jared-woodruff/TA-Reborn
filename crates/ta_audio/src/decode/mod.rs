//! Decoding sound files into PCM.
//!
//! [`decode`] handles every short sound the game uses, entirely in memory:
//!
//! * RIFF WAVE ([`wav`]): integer PCM (8/16/24/32-bit), IEEE float, IMA ADPCM, Microsoft
//!   ADPCM, A-law and µ-law,
//! * Humongous Entertainment `DIGI`/`TALK` containers (unsigned 8-bit PCM), used by one
//!   leftover file in TA (`sounds/sing.wav`),
//! * headerless unsigned 8-bit PCM at 11 025 Hz, accepted only when the bytes look like audio
//!   (`sounds/honk.wav`),
//! * Ogg Vorbis, through [`music`]'s decoder (TA Reborn's own spoken lines: the tutorial's).
//!
//! Streaming formats for music (MP3, Ogg Vorbis) are decoded by [`music`] through symphonia.

pub mod music;
pub mod wav;

pub use music::{MusicDecoder, decode_music};
pub use wav::WavCodec;

use thiserror::Error;

/// Decoded audio: interleaved `f32` samples in `[-1, 1]`, mono or stereo.
#[derive(Clone, PartialEq)]
pub struct SoundData {
    sample_rate: u32,
    channels: u16,
    samples: Vec<f32>,
}

impl std::fmt::Debug for SoundData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundData")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames())
            .finish()
    }
}

impl SoundData {
    /// Wraps interleaved samples. `channels` must be 1 or 2 and `sample_rate` non-zero; a
    /// trailing partial frame is dropped.
    pub fn new(sample_rate: u32, channels: u16, mut samples: Vec<f32>) -> Result<SoundData, DecodeError> {
        if !(1..=2).contains(&channels) {
            return Err(DecodeError::Malformed("sound", format!("{channels} channels (expected 1 or 2)")));
        }
        if sample_rate == 0 {
            return Err(DecodeError::Malformed("sound", "sample rate 0".into()));
        }
        samples.truncate(samples.len() / channels as usize * channels as usize);
        Ok(SoundData { sample_rate, channels, samples })
    }

    /// Mono sound from samples.
    pub fn mono(sample_rate: u32, samples: Vec<f32>) -> SoundData {
        SoundData { sample_rate: sample_rate.max(1), channels: 1, samples }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Interleaved samples.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// Length in seconds.
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    /// Largest absolute sample value.
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Root-mean-square level over all samples.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }

    /// Mean sample value (DC offset).
    pub fn dc_offset(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.samples.iter().map(|&s| s as f64).sum::<f64>() / self.samples.len() as f64) as f32
    }

    /// Bytes of sample memory held.
    pub fn memory_bytes(&self) -> usize {
        self.samples.len() * std::mem::size_of::<f32>()
    }

    /// Removes clicks at the edges: if the first or last frame is far from zero, a short linear
    /// fade (2 ms in, 5 ms out) is applied. Most TA sounds start and end on silence and are left
    /// untouched; a few are cut mid-waveform (e.g. `phaser.wav` ends at -0.55).
    pub fn declick(&mut self) {
        const THRESHOLD: f32 = 0.05;
        let ch = self.channels as usize;
        let frames = self.frames();
        if frames < 4 {
            return;
        }
        let edge = |i: usize, s: &[f32]| (0..ch).map(|c| s[i * ch + c].abs()).fold(0.0f32, f32::max);
        if edge(0, &self.samples) > THRESHOLD {
            let n = ((self.sample_rate as usize) / 500).clamp(2, frames / 2);
            for i in 0..n {
                let g = i as f32 / n as f32;
                for c in 0..ch {
                    self.samples[i * ch + c] *= g;
                }
            }
        }
        if edge(frames - 1, &self.samples) > THRESHOLD {
            let n = ((self.sample_rate as usize) / 200).clamp(2, frames / 2);
            for i in 0..n {
                let g = i as f32 / n as f32;
                let f = frames - 1 - i;
                for c in 0..ch {
                    self.samples[f * ch + c] *= g;
                }
            }
        }
    }

    /// Converts from arbitrary channel counts: 1 and 2 are kept, more channels keep the first
    /// two (front left/right in WAVE channel order).
    pub(crate) fn from_channels(
        sample_rate: u32,
        channels: usize,
        samples: Vec<f32>,
    ) -> Result<SoundData, DecodeError> {
        match channels {
            0 => Err(DecodeError::Malformed("sound", "0 channels".into())),
            1 | 2 => SoundData::new(sample_rate, channels as u16, samples),
            n => {
                let stereo = samples.chunks_exact(n).flat_map(|f| [f[0], f[1]]).collect();
                SoundData::new(sample_rate, 2, stereo)
            }
        }
    }
}

/// Container and codec a sound was stored in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceFormat {
    /// RIFF WAVE with the given codec.
    Wav(WavCodec),
    /// Humongous Entertainment `DIGI`/`TALK` container holding unsigned 8-bit PCM.
    HumongousDigi,
    /// Headerless unsigned 8-bit PCM, assumed to be 11 025 Hz mono.
    RawPcm8,
    /// Ogg Vorbis (decoded whole, as stereo).
    OggVorbis,
}

impl std::fmt::Display for SourceFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceFormat::Wav(c) => write!(f, "WAV {c}"),
            SourceFormat::HumongousDigi => f.write_str("Humongous DIGI (8-bit PCM)"),
            SourceFormat::RawPcm8 => f.write_str("headerless 8-bit PCM"),
            SourceFormat::OggVorbis => f.write_str("Ogg Vorbis"),
        }
    }
}

/// A decoded sound plus what it was stored as.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub sound: SoundData,
    pub format: SourceFormat,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("unrecognised sound data (no RIFF/WAVE or DIGI header)")]
    UnknownFormat,
    #[error("malformed {0}: {1}")]
    Malformed(&'static str, String),
    #[error("unsupported WAVE format tag {0:#06x}")]
    UnsupportedCodec(u16),
    #[error("unsupported {0}-bit samples for {1}")]
    UnsupportedBits(u16, &'static str),
    #[error("undecodable Ogg Vorbis: {0}")]
    Vorbis(String),
}

/// Sample rate TA used for all of its 8-bit effects; assumed for headerless data.
pub const TA_SAMPLE_RATE: u32 = 11_025;

/// Decodes a complete sound file held in memory.
pub fn decode(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        let (sound, codec) = wav::decode(bytes)?;
        return Ok(Decoded { sound, format: SourceFormat::Wav(codec) });
    }
    if bytes.len() >= 4 && &bytes[0..4] == b"OggS" {
        let sound = decode_music(bytes.to_vec(), Some("ogg")).map_err(|e| DecodeError::Vorbis(e.to_string()))?;
        return Ok(Decoded { sound, format: SourceFormat::OggVorbis });
    }
    if bytes.len() >= 8 && (&bytes[0..4] == b"DIGI" || &bytes[0..4] == b"TALK") {
        return Ok(Decoded { sound: decode_digi(bytes)?, format: SourceFormat::HumongousDigi });
    }
    if looks_like_pcm8(bytes) {
        return Ok(Decoded { sound: decode_raw_pcm8(bytes), format: SourceFormat::RawPcm8 });
    }
    Err(DecodeError::UnknownFormat)
}

/// Humongous Entertainment sound block: `DIGI` (big-endian size) containing `HSHD` (header,
/// sample rate as little-endian u16 at offset 6 of its payload) and `SDAT` (unsigned 8-bit PCM).
/// Block sizes include their 8-byte headers.
fn decode_digi(b: &[u8]) -> Result<SoundData, DecodeError> {
    let be32 = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
    let end = be32(4).clamp(8, b.len());
    let mut pos = 8;
    let mut rate = TA_SAMPLE_RATE;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= end {
        let id = &b[pos..pos + 4];
        let size = be32(pos + 4);
        if size < 8 {
            break;
        }
        let body = &b[pos + 8..(pos + size).min(end)];
        match id {
            b"HSHD" if body.len() >= 8 => {
                let r = u16::from_le_bytes([body[6], body[7]]) as u32;
                if (1000..=96_000).contains(&r) {
                    rate = r;
                }
            }
            b"SDAT" => data = Some(body),
            _ => {}
        }
        pos = pos.saturating_add(size);
    }
    let data = data.ok_or_else(|| DecodeError::Malformed("DIGI", "no SDAT block".into()))?;
    Ok(SoundData::mono(rate, data.iter().map(|&s| (s as f32 - 128.0) / 128.0).collect()))
}

/// Heuristic for headerless 8-bit PCM: enough bytes and strong sample-to-sample correlation
/// (sound is smooth; text and binary data are not).
fn looks_like_pcm8(b: &[u8]) -> bool {
    if b.len() < 256 {
        return false;
    }
    let mean = b.iter().map(|&x| x as f64).sum::<f64>() / b.len() as f64;
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for w in b.windows(2) {
        let (x0, x1) = (w[0] as f64 - mean, w[1] as f64 - mean);
        num += x0 * x1;
        den += x0 * x0;
    }
    den > 0.0 && num / den > 0.6
}

/// Headerless unsigned 8-bit PCM. The mean is removed because without a header there is no
/// trustworthy zero level (TA's `honk.wav` idles around 90 rather than 128).
fn decode_raw_pcm8(b: &[u8]) -> SoundData {
    let mean = b.iter().map(|&x| x as f32).sum::<f32>() / b.len().max(1) as f32;
    SoundData::mono(TA_SAMPLE_RATE, b.iter().map(|&s| ((s as f32 - mean) / 128.0).clamp(-1.0, 1.0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tutorial's spoken lines are Ogg Vorbis: decoded whole like any other short sound.
    #[test]
    fn decodes_ogg_vorbis() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/tone_440hz_quarter_second.ogg")).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.format, SourceFormat::OggVorbis);
        assert_eq!(decoded.sound.sample_rate(), 22_050);
        // A quarter of a second (the encoder's padding aside), and a tone, not silence.
        assert!((5_400..=5_700).contains(&decoded.sound.frames()), "{} frames", decoded.sound.frames());
        assert!(decoded.sound.samples().iter().any(|s| s.abs() > 0.1)); // (ffmpeg's tone peaks at 1/8)
        // Cut short: refused, never a crash.
        assert!(matches!(decode(&bytes[..bytes.len() / 3]), Err(DecodeError::Vorbis(_)) | Ok(_)));
        assert!(matches!(decode(b"OggS but nothing else"), Err(DecodeError::Vorbis(_))));
    }

    #[test]
    fn decodes_humongous_digi() {
        // DIGI { HSHD (rate 22050), SDAT [128, 255, 0] }
        let mut b = Vec::new();
        b.extend_from_slice(b"DIGI");
        b.extend_from_slice(&(8u32 + 24 + 11).to_be_bytes());
        b.extend_from_slice(b"HSHD");
        b.extend_from_slice(&24u32.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0x80, 0x80, 0xff, 0xff]);
        b.extend_from_slice(&22050u16.to_le_bytes());
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(b"SDAT");
        b.extend_from_slice(&11u32.to_be_bytes());
        b.extend_from_slice(&[128, 255, 0]);
        let d = decode(&b).unwrap();
        assert_eq!(d.format, SourceFormat::HumongousDigi);
        assert_eq!(d.sound.sample_rate(), 22050);
        assert_eq!(d.sound.samples(), &[0.0, 127.0 / 128.0, -1.0]);
    }

    #[test]
    fn raw_pcm8_needs_audio_like_data() {
        let tone: Vec<u8> = (0..2000).map(|i| (90.0 + 30.0 * (i as f32 * 0.05).sin()) as u8).collect();
        let d = decode(&tone).unwrap();
        assert_eq!(d.format, SourceFormat::RawPcm8);
        assert!(d.sound.dc_offset().abs() < 0.01);
        let text = b"[UNITINFO]\n{\n\tUnitName=ARMCOM;\n\tSide=ARM;\n}\n".repeat(20);
        assert_eq!(decode(&text).unwrap_err(), DecodeError::UnknownFormat);
        assert_eq!(decode(&[0u8; 10]).unwrap_err(), DecodeError::UnknownFormat);
    }

    #[test]
    fn declick_fades_hard_edges_only() {
        let mut s = SoundData::mono(11025, vec![0.8; 1000]);
        s.declick();
        assert_eq!(s.samples()[0], 0.0);
        assert!(s.samples()[999].abs() < 0.05);
        assert_eq!(s.samples()[500], 0.8);
        // Starts exactly on zero (sin(0)) and is cut at an arbitrary point.
        let clean: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        let mut c = SoundData::mono(11025, clean.clone());
        c.declick();
        assert_eq!(c.samples()[..900], clean[..900]);
    }

    #[test]
    fn channel_conversion() {
        let s = SoundData::from_channels(8000, 4, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]).unwrap();
        assert_eq!(s.channels(), 2);
        assert_eq!(s.samples(), &[1.0, 2.0, 5.0, 6.0]);
        assert!(SoundData::new(8000, 3, vec![]).is_err());
        assert_eq!(SoundData::new(8000, 2, vec![1.0, 2.0, 3.0]).unwrap().frames(), 1);
    }
}
