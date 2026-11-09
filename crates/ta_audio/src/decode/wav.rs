//! RIFF WAVE decoding, written from the public Microsoft RIFF/WAVE specification.
//!
//! Supported codecs: integer PCM (8-bit unsigned, 16/24/32-bit signed), IEEE float (32/64-bit),
//! IMA ADPCM (`0x0011`), Microsoft ADPCM (`0x0002`), A-law (`0x0006`), µ-law (`0x0007`), and
//! `WAVE_FORMAT_EXTENSIBLE` wrapping any of those.
//!
//! The parser is lenient in the ways real files require: chunk sizes running past the end of
//! the file are clamped (TA's `cdoggy.wav` carries two stray trailing bytes), a `data` size of 0
//! or `0xFFFFFFFF` (streaming writers) means "to the end of the file", and a truncated last
//! ADPCM block is decoded as far as it goes.

use super::{DecodeError, SoundData};

/// Codec of a WAVE file's sample data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WavCodec {
    /// Integer PCM with the given container size in bits.
    Pcm { bits: u16 },
    /// IEEE float PCM.
    Float { bits: u16 },
    ImaAdpcm,
    MsAdpcm,
    ALaw,
    MuLaw,
}

impl std::fmt::Display for WavCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavCodec::Pcm { bits } => write!(f, "PCM {bits}-bit"),
            WavCodec::Float { bits } => write!(f, "float {bits}-bit"),
            WavCodec::ImaAdpcm => f.write_str("IMA ADPCM"),
            WavCodec::MsAdpcm => f.write_str("MS ADPCM"),
            WavCodec::ALaw => f.write_str("A-law"),
            WavCodec::MuLaw => f.write_str("µ-law"),
        }
    }
}

const TAG_PCM: u16 = 0x0001;
const TAG_MS_ADPCM: u16 = 0x0002;
const TAG_FLOAT: u16 = 0x0003;
const TAG_ALAW: u16 = 0x0006;
const TAG_MULAW: u16 = 0x0007;
const TAG_IMA_ADPCM: u16 = 0x0011;
const TAG_EXTENSIBLE: u16 = 0xFFFE;

/// The `fmt ` chunk.
#[derive(Clone, Debug)]
struct Fmt<'a> {
    tag: u16,
    channels: u16,
    rate: u32,
    block_align: u16,
    bits: u16,
    /// Bytes after `cbSize` (codec-specific).
    extra: &'a [u8],
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Decodes a complete RIFF WAVE file.
pub fn decode(b: &[u8]) -> Result<(SoundData, WavCodec), DecodeError> {
    let mut fmt_chunk: Option<&[u8]> = None;
    let mut data: Option<&[u8]> = None;
    let mut pos = 12usize;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let declared = le32(b, pos + 4) as usize;
        let start = pos + 8;
        let available = b.len() - start;
        if id == b"data" && (declared == 0 || declared == u32::MAX as usize) {
            // Written by a streaming encoder that never patched the size: data runs to the end.
            data.get_or_insert(&b[start..]);
            break;
        }
        let body = &b[start..start + declared.min(available)];
        match id {
            b"fmt " if fmt_chunk.is_none() => fmt_chunk = Some(body),
            b"data" if data.is_none() => data = Some(body),
            _ => {}
        }
        match start.checked_add(declared).and_then(|e| e.checked_add(declared & 1)) {
            Some(next) => pos = next,
            None => break,
        }
    }
    let fmt = parse_fmt(fmt_chunk.ok_or_else(|| DecodeError::Malformed("WAVE", "no fmt chunk".into()))?)?;
    let data = data.ok_or_else(|| DecodeError::Malformed("WAVE", "no data chunk".into()))?;
    let channels = fmt.channels as usize;

    let (samples, codec) = match fmt.tag {
        TAG_PCM => {
            let width = sample_width(&fmt)?;
            (decode_pcm(data, width)?, WavCodec::Pcm { bits: (width * 8) as u16 })
        }
        TAG_FLOAT => {
            let width = sample_width(&fmt)?;
            (decode_float(data, width)?, WavCodec::Float { bits: (width * 8) as u16 })
        }
        TAG_ALAW => (data.iter().map(|&x| alaw_to_i16(x) as f32 / 32768.0).collect(), WavCodec::ALaw),
        TAG_MULAW => (data.iter().map(|&x| ulaw_to_i16(x) as f32 / 32768.0).collect(), WavCodec::MuLaw),
        TAG_IMA_ADPCM => {
            if fmt.bits != 4 {
                return Err(DecodeError::UnsupportedBits(fmt.bits, "IMA ADPCM"));
            }
            (decode_ima_adpcm(data, channels, fmt.block_align as usize)?, WavCodec::ImaAdpcm)
        }
        TAG_MS_ADPCM => (decode_ms_adpcm(data, channels, fmt.block_align as usize, fmt.extra)?, WavCodec::MsAdpcm),
        tag => return Err(DecodeError::UnsupportedCodec(tag)),
    };
    Ok((SoundData::from_channels(fmt.rate, channels, samples)?, codec))
}

fn parse_fmt(c: &[u8]) -> Result<Fmt<'_>, DecodeError> {
    if c.len() < 14 {
        return Err(DecodeError::Malformed("WAVE", format!("fmt chunk of {} bytes", c.len())));
    }
    let mut fmt = Fmt {
        tag: le16(c, 0),
        channels: le16(c, 2),
        rate: le32(c, 4),
        block_align: le16(c, 12),
        bits: if c.len() >= 16 { le16(c, 14) } else { 8 },
        extra: &[],
    };
    if c.len() >= 18 {
        let cb = le16(c, 16) as usize;
        fmt.extra = &c[18..(18 + cb).min(c.len())];
    }
    if fmt.tag == TAG_EXTENSIBLE {
        // cbSize >= 22: valid bits (u16), channel mask (u32), sub-format GUID whose first two
        // bytes are the real format tag.
        if fmt.extra.len() < 8 {
            return Err(DecodeError::Malformed("WAVE", "short WAVE_FORMAT_EXTENSIBLE header".into()));
        }
        fmt.tag = le16(fmt.extra, 6);
    }
    if fmt.channels == 0 || fmt.channels > 32 {
        return Err(DecodeError::Malformed("WAVE", format!("{} channels", fmt.channels)));
    }
    if fmt.rate == 0 || fmt.rate > 768_000 {
        return Err(DecodeError::Malformed("WAVE", format!("sample rate {}", fmt.rate)));
    }
    Ok(fmt)
}

/// Bytes per sample for PCM/float: from the block alignment when consistent, else from the bit
/// depth rounded up to whole bytes.
fn sample_width(fmt: &Fmt) -> Result<usize, DecodeError> {
    let from_bits = (fmt.bits as usize).div_ceil(8);
    let ch = fmt.channels as usize;
    let from_align = if (fmt.block_align as usize).is_multiple_of(ch) { fmt.block_align as usize / ch } else { 0 };
    let width = if (1..=8).contains(&from_align) && from_align >= from_bits { from_align } else { from_bits };
    let ok = match fmt.tag {
        TAG_FLOAT => width == 4 || width == 8,
        _ => (1..=4).contains(&width),
    };
    if ok {
        Ok(width)
    } else {
        Err(DecodeError::UnsupportedBits(fmt.bits, if fmt.tag == TAG_FLOAT { "float PCM" } else { "PCM" }))
    }
}

fn decode_pcm(data: &[u8], width: usize) -> Result<Vec<f32>, DecodeError> {
    Ok(match width {
        1 => data.iter().map(|&s| (s as f32 - 128.0) / 128.0).collect(),
        2 => data.as_chunks::<2>().0.iter().map(|&s| i16::from_le_bytes(s) as f32 / 32768.0).collect(),
        3 => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|s| (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        4 => data.as_chunks::<4>().0.iter().map(|&s| i32::from_le_bytes(s) as f32 / 2_147_483_648.0).collect(),
        _ => return Err(DecodeError::UnsupportedBits((width * 8) as u16, "PCM")),
    })
}

fn decode_float(data: &[u8], width: usize) -> Result<Vec<f32>, DecodeError> {
    let clean = |x: f32| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 };
    Ok(match width {
        4 => data.as_chunks::<4>().0.iter().map(|&s| clean(f32::from_le_bytes(s))).collect(),
        8 => data.as_chunks::<8>().0.iter().map(|&s| clean(f64::from_le_bytes(s) as f32)).collect(),
        _ => return Err(DecodeError::UnsupportedBits((width * 8) as u16, "float PCM")),
    })
}

/// G.711 A-law to linear (ITU-T G.711; ±32256 full scale).
pub fn alaw_to_i16(a: u8) -> i16 {
    let a = a ^ 0x55;
    let mut t = ((a & 0x0F) as i32) << 4;
    let seg = (a >> 4) & 0x07;
    match seg {
        0 => t += 8,
        1 => t += 0x108,
        _ => {
            t += 0x108;
            t <<= seg - 1;
        }
    }
    (if a & 0x80 != 0 { t } else { -t }) as i16
}

/// G.711 µ-law to linear (ITU-T G.711; ±32124 full scale).
pub fn ulaw_to_i16(u: u8) -> i16 {
    const BIAS: i32 = 0x84;
    let u = !u;
    let mut t = (((u & 0x0F) as i32) << 3) + BIAS;
    t <<= (u >> 4) & 0x07;
    (if u & 0x80 != 0 { BIAS - t } else { t - BIAS }) as i16
}

const IMA_STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97,
    107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796,
    876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871,
    5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623,
    27086, 29794, 32767,
];
const IMA_INDEX_ADJUST: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// IMA/DVI ADPCM predictor state for one channel.
#[derive(Clone, Copy, Debug)]
struct ImaState {
    predictor: i32,
    index: i32,
}

impl ImaState {
    #[inline]
    fn expand(&mut self, nibble: u8) -> i16 {
        let step = IMA_STEPS[self.index as usize];
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 8 != 0 {
            diff = -diff;
        }
        self.predictor = (self.predictor + diff).clamp(-32768, 32767);
        self.index = (self.index + IMA_INDEX_ADJUST[(nibble & 7) as usize]).clamp(0, 88);
        self.predictor as i16
    }
}

/// IMA ADPCM as stored in WAVE files: each block starts with a 4-byte header per channel
/// (initial sample i16, step index u8, reserved u8), followed by groups of 4 bytes per channel,
/// each byte holding two samples low nibble first.
fn decode_ima_adpcm(data: &[u8], channels: usize, block_align: usize) -> Result<Vec<f32>, DecodeError> {
    if block_align < 4 * channels {
        return Err(DecodeError::Malformed("IMA ADPCM", format!("block size {block_align} for {channels} channels")));
    }
    let mut out = Vec::with_capacity(data.len() * 2 + 16);
    let mut states = vec![ImaState { predictor: 0, index: 0 }; channels];
    let mut chan_buf: Vec<Vec<i16>> = vec![Vec::with_capacity(8); channels];
    for block in data.chunks(block_align) {
        if block.len() < 4 * channels {
            break;
        }
        for (c, st) in states.iter_mut().enumerate() {
            let h = &block[c * 4..c * 4 + 4];
            st.predictor = i16::from_le_bytes([h[0], h[1]]) as i32;
            st.index = (h[2] as i32).clamp(0, 88);
            out.push(st.predictor as f32 / 32768.0);
        }
        // Payload: repeated [4 bytes ch0][4 bytes ch1]..., 8 samples per channel per group.
        for group in block[4 * channels..].chunks(4 * channels) {
            for (c, st) in states.iter_mut().enumerate() {
                let buf = &mut chan_buf[c];
                buf.clear();
                let start = (c * 4).min(group.len());
                let end = (c * 4 + 4).min(group.len());
                for &byte in &group[start..end] {
                    buf.push(st.expand(byte & 0x0F));
                    buf.push(st.expand(byte >> 4));
                }
            }
            // A short final group still holds samples. Emit only frames for
            // which every channel has supplied a sample, keeping interleaving.
            let frames = chan_buf.iter().map(Vec::len).min().unwrap_or(0);
            for i in 0..frames {
                for buf in &chan_buf {
                    out.push(buf[i] as f32 / 32768.0);
                }
            }
        }
    }
    Ok(out)
}

const MS_ADAPT: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230];
const MS_DEFAULT_COEFS: [(i32, i32); 7] =
    [(256, 0), (512, -256), (0, 0), (192, 64), (240, 0), (460, -208), (392, -232)];

/// Microsoft ADPCM predictor state for one channel.
#[derive(Clone, Copy, Debug, Default)]
struct MsState {
    coef: (i32, i32),
    delta: i32,
    sample1: i32,
    sample2: i32,
}

impl MsState {
    #[inline]
    fn expand(&mut self, nibble: u8) -> i16 {
        let signed = if nibble & 8 != 0 { nibble as i64 - 16 } else { nibble as i64 };
        // The specification divides by 256 (truncating towards zero), which is not the same as
        // an arithmetic shift for negative predictions.
        let predicted = (self.sample1 as i64 * self.coef.0 as i64 + self.sample2 as i64 * self.coef.1 as i64) / 256;
        let sample = (predicted + signed * self.delta as i64).clamp(-32768, 32767) as i32;
        self.sample2 = self.sample1;
        self.sample1 = sample;
        // Garbage input can grow the step size without bound; cap it like other decoders do.
        self.delta = ((MS_ADAPT[nibble as usize] * self.delta) >> 8).clamp(16, i32::MAX / 768);
        sample as i16
    }
}

/// Microsoft ADPCM: each block begins with per-channel arrays of predictor index (u8), initial
/// delta (i16), sample1 (i16) and sample2 (i16); sample2 then sample1 are the first two output
/// frames. The rest are nibbles, high nibble first, alternating channels in stereo.
/// Coefficient pairs come from the `fmt ` extension (or the 7 standard pairs).
fn decode_ms_adpcm(data: &[u8], channels: usize, block_align: usize, extra: &[u8]) -> Result<Vec<f32>, DecodeError> {
    if channels > 2 {
        return Err(DecodeError::Malformed("MS ADPCM", format!("{channels} channels")));
    }
    let header = 7 * channels;
    if block_align <= header {
        return Err(DecodeError::Malformed("MS ADPCM", format!("block size {block_align} for {channels} channels")));
    }
    let mut coefs: Vec<(i32, i32)> = MS_DEFAULT_COEFS.to_vec();
    if extra.len() >= 4 {
        let n = le16(extra, 2) as usize;
        let table: Vec<(i32, i32)> = extra[4..]
            .as_chunks::<4>()
            .0
            .iter()
            .take(n)
            .map(|c| (i16::from_le_bytes([c[0], c[1]]) as i32, i16::from_le_bytes([c[2], c[3]]) as i32))
            .collect();
        if !table.is_empty() {
            coefs = table;
        }
    }
    let mut out = Vec::with_capacity(data.len() * 2 + 16);
    let mut states = [MsState::default(); 2];
    for block in data.chunks(block_align) {
        if block.len() < header {
            break;
        }
        let rd16 = |o: usize| i16::from_le_bytes([block[o], block[o + 1]]) as i32;
        for (c, st) in states.iter_mut().enumerate().take(channels) {
            let predictor = block[c] as usize;
            st.coef = *coefs.get(predictor).ok_or_else(|| {
                DecodeError::Malformed("MS ADPCM", format!("predictor {predictor} of {}", coefs.len()))
            })?;
            st.delta = rd16(channels + 2 * c);
            st.sample1 = rd16(3 * channels + 2 * c);
            st.sample2 = rd16(5 * channels + 2 * c);
        }
        for st in states.iter().take(channels) {
            out.push(st.sample2 as f32 / 32768.0);
        }
        for st in states.iter().take(channels) {
            out.push(st.sample1 as f32 / 32768.0);
        }
        let mut c = 0;
        for &byte in &block[header..] {
            for nibble in [byte >> 4, byte & 0x0F] {
                out.push(states[c].expand(nibble) as f32 / 32768.0);
                c = (c + 1) % channels;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a canonical WAVE file.
    fn wav(tag: u16, channels: u16, rate: u32, bits: u16, block_align: u16, extra: &[u8], data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
        fmt.extend_from_slice(&block_align.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        if !extra.is_empty() {
            fmt.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            fmt.extend_from_slice(extra);
        }
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        out.extend_from_slice(&fmt);
        if fmt.len() % 2 == 1 {
            out.push(0);
        }
        out.extend_from_slice(b"LIST\x04\0\0\0INFO");
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        let riff = (out.len() - 8) as u32;
        out[4..8].copy_from_slice(&riff.to_le_bytes());
        out
    }

    #[test]
    fn decodes_ta_style_8bit_pcm() {
        let b = wav(1, 1, 11025, 8, 1, &[], &[128, 0, 255, 64]);
        let (s, codec) = decode(&b).unwrap();
        assert_eq!(codec, WavCodec::Pcm { bits: 8 });
        assert_eq!((s.sample_rate(), s.channels(), s.frames()), (11025, 1, 4));
        assert_eq!(s.samples(), &[0.0, -1.0, 127.0 / 128.0, -0.5]);
    }

    #[test]
    fn decodes_16_24_32_bit_and_float() {
        let d16: Vec<u8> = [0i16, 16384, -32768, 32767].iter().flat_map(|v| v.to_le_bytes()).collect();
        let (s, _) = decode(&wav(1, 2, 22050, 16, 4, &[], &d16)).unwrap();
        assert_eq!((s.channels(), s.frames()), (2, 2));
        assert_eq!(s.samples(), &[0.0, 0.5, -1.0, 32767.0 / 32768.0]);

        let d24 = [0x00, 0x00, 0x40, 0x00, 0x00, 0xC0]; // +0.5, -0.5
        let (s, codec) = decode(&wav(1, 1, 48000, 24, 3, &[], &d24)).unwrap();
        assert_eq!(codec, WavCodec::Pcm { bits: 24 });
        assert_eq!(s.samples(), &[0.5, -0.5]);

        let d32: Vec<u8> = [i32::MIN, 1 << 30].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(decode(&wav(1, 1, 48000, 32, 4, &[], &d32)).unwrap().0.samples(), &[-1.0, 0.5]);

        let df: Vec<u8> = [0.25f32, -2.0, f32::NAN].iter().flat_map(|v| v.to_le_bytes()).collect();
        let (s, codec) = decode(&wav(3, 1, 44100, 32, 4, &[], &df)).unwrap();
        assert_eq!(codec, WavCodec::Float { bits: 32 });
        assert_eq!(s.samples(), &[0.25, -1.0, 0.0]);
    }

    #[test]
    fn decodes_extensible_and_tolerates_bad_sizes() {
        let mut ext = vec![0u8; 22];
        ext[0..2].copy_from_slice(&16u16.to_le_bytes());
        ext[6..8].copy_from_slice(&1u16.to_le_bytes()); // KSDATAFORMAT_SUBTYPE_PCM
        let d: Vec<u8> = [1000i16, -1000].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut b = wav(0xFFFE, 1, 32000, 16, 2, &ext, &d);
        assert_eq!(decode(&b).unwrap().0.frames(), 2);
        // Data size larger than the file and two stray trailing bytes.
        let n = b.len();
        b[n - 8..n - 4].copy_from_slice(&1000u32.to_le_bytes());
        b.extend_from_slice(&[0x80, 0x80]);
        assert_eq!(decode(&b).unwrap().0.frames(), 3);
        // Streaming writers leave the data size at 0.
        let mut z = wav(1, 1, 8000, 8, 1, &[], &[1, 2, 3]);
        let n = z.len();
        z[n - 7..n - 3].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode(&z).unwrap().0.frames(), 3);
    }

    #[test]
    fn rejects_broken_headers() {
        assert!(matches!(decode(&wav(0x55, 1, 8000, 16, 2, &[], &[0, 0])), Err(DecodeError::UnsupportedCodec(0x55))));
        assert!(decode(&wav(1, 0, 8000, 16, 2, &[], &[0, 0])).is_err());
        assert!(decode(&wav(1, 1, 0, 16, 2, &[], &[0, 0])).is_err());
        assert!(decode(b"RIFF\x04\0\0\0WAVE").is_err());
    }

    #[test]
    fn g711_reference_values() {
        assert_eq!(ulaw_to_i16(0xFF), 0);
        assert_eq!(ulaw_to_i16(0x80), 32124);
        assert_eq!(ulaw_to_i16(0x00), -32124);
        assert_eq!(alaw_to_i16(0xD5), 8);
        assert_eq!(alaw_to_i16(0x55), -8);
        assert_eq!(alaw_to_i16(0xAA), 32256);
        assert_eq!(alaw_to_i16(0x2A), -32256);
        // Both laws are monotonic over their positive halves.
        let pos: Vec<i16> = (0x80u8..=0xFF).rev().map(ulaw_to_i16).collect();
        assert!(pos.windows(2).all(|w| w[0] <= w[1]));
    }

    fn to_i16(s: &SoundData) -> Vec<i32> {
        s.samples().iter().map(|&x| (x * 32768.0).round() as i32).collect()
    }

    #[test]
    fn ima_adpcm_follows_the_reference_algorithm() {
        // Mono block: header (predictor 0, index 0) + one group of nibbles 7,F,0,8,0,0,0,0
        // (low nibble first). Expected values worked out by hand from the IMA step table.
        let block = [0x00, 0x00, 0x00, 0x00, 0xF7, 0x80, 0x00, 0x00];
        let (s, _) = decode(&wav(0x11, 1, 8000, 4, 8, &[9, 0], &block)).unwrap();
        assert_eq!(to_i16(&s), [0, 11, -19, -15, -18, -15, -12, -10, -8]);

        // Stereo block: L header 100, R header -100, then 4 bytes of L nibbles, 4 of R nibbles.
        let block = [0x64, 0x00, 0, 0, 0x9C, 0xFF, 0, 0, 0, 0, 0, 0, 0x77, 0x77, 0x77, 0x77];
        let (s, _) = decode(&wav(0x11, 2, 8000, 4, 16, &[9, 0], &block)).unwrap();
        let right: Vec<i32> = to_i16(&s).chunks(2).map(|f| f[1]).collect();
        let left: Vec<i32> = to_i16(&s).chunks(2).map(|f| f[0]).collect();
        assert_eq!(left, [100; 9]);
        assert_eq!(right, [-100, -89, -59, 4, 140, 433, 1064, 2421, 5331]);
    }

    #[test]
    fn ima_adpcm_keeps_complete_frames_from_a_short_final_block() {
        let mono = [0, 0, 0, 0, 0xF7, 0x80, 0, 0];
        let mono_samples = [0, 11, -19, -15, -18, -15, -12, -10, -8];
        for bytes in 0..=4 {
            let mut data = mono.to_vec();
            data.extend_from_slice(&mono[..4 + bytes]);
            let (sound, _) = decode(&wav(0x11, 1, 8000, 4, 8, &[9, 0], &data)).unwrap();
            let expected: Vec<_> = mono_samples.into_iter().chain(mono_samples[..1 + bytes * 2].iter().copied()).collect();
            assert_eq!(to_i16(&sound), expected, "mono payload {bytes}");
        }
        let stereo = [100, 0, 0, 0, 156, 255, 0, 0, 0, 0, 0, 0, 0x77, 0x77, 0x77, 0x77];
        let right = [-100, -89, -59, 4, 140, 433, 1064, 2421, 5331];
        for bytes in 0..=8 {
            let (sound, _) = decode(&wav(0x11, 2, 8000, 4, 16, &[9, 0], &stereo[..8 + bytes])).unwrap();
            // Four-byte channel groups: only samples present for both channels
            // form an output frame. A partial right group can still supply frames.
            let frames = 1 + bytes.saturating_sub(4) * 2;
            let expected: Vec<_> = right[..frames].iter().flat_map(|&r| [100, r]).collect();
            assert_eq!(to_i16(&sound), expected, "stereo payload {bytes}");
        }
    }

    /// Reference vectors produced by ffmpeg 9 from a synthetic two-tone test signal:
    /// `ffmpeg -i ref.wav -c:a adpcm_ima_wav|adpcm_ms -block_size 256 x.wav`; the `*.s16` files
    /// are ffmpeg's own decode of those files (`-f s16le`).
    /// Asserts a bit-exact match with ffmpeg's decode (multi-block files, 256-byte blocks, so
    /// block headers, nibble order and channel interleaving are all exercised).
    fn assert_matches_ffmpeg(name: &str, codec: WavCodec, channels: u16) {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/");
        let w = std::fs::read(format!("{dir}{name}.wav")).unwrap();
        let r = std::fs::read(format!("{dir}{name}.s16")).unwrap();
        let (s, c) = decode(&w).unwrap();
        assert_eq!(c, codec);
        assert_eq!(s.channels(), channels);
        let expected: Vec<i32> = r.as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b) as i32).collect();
        let got = to_i16(&s);
        assert_eq!(got.len(), expected.len(), "{name}: sample count");
        if let Some(i) = got.iter().zip(&expected).position(|(a, b)| a != b) {
            panic!("{name}: sample {i} is {} but ffmpeg decodes {}", got[i], expected[i]);
        }
    }

    #[test]
    fn ms_adpcm_matches_ffmpeg() {
        assert_matches_ffmpeg("ms_mono", WavCodec::MsAdpcm, 1);
        assert_matches_ffmpeg("ms_stereo", WavCodec::MsAdpcm, 2);
    }

    #[test]
    fn ima_adpcm_matches_ffmpeg() {
        assert_matches_ffmpeg("ima_mono", WavCodec::ImaAdpcm, 1);
        assert_matches_ffmpeg("ima_stereo", WavCodec::ImaAdpcm, 2);
    }
}
