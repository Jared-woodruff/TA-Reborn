//! How long a music file plays, for the jukebox, from its bytes: an MP3 by its Xing, Info or
//! VBRI header, else by its frames counted (TA's own have no such header); Ogg Vorbis and Opus
//! by the last page's granule position over the rate; FLAC by its STREAMINFO; WAV by its data
//! over its byte rate; M4A by its movie header. Nothing is decoded.

use std::path::Path;

use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// A file's duration without loading its audio payload into memory. The container reader
/// seeks to its metadata; large WAV/FLAC collections remain cheap to browse in the jukebox.
pub fn file_seconds(path: &Path) -> Option<f64> {
    let file = std::fs::File::open(path).ok()?;
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    crate::guarded(|| source_seconds(Box::new(file), &extension)).flatten()
}

fn source_seconds(source: Box<dyn MediaSource>, extension: &str) -> Option<f64> {
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let stream = MediaSourceStream::new(source, Default::default());
    let probed = symphonia::default::get_probe().format(
        &hint, stream, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default(),
    ).ok()?;
    let params = &probed.format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?.codec_params;
    let frames = params.n_frames?;
    let duration = if let Some(base) = params.time_base {
        let time = base.calc_time(frames);
        time.seconds as f64 + time.frac
    } else {
        frames as f64 / params.sample_rate? as f64
    };
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

/// The file's length in seconds, `None` when it cannot be told.
pub fn seconds(bytes: &[u8]) -> Option<f64> {
    let s = if bytes.starts_with(b"OggS") {
        ogg(bytes)
    } else if bytes.starts_with(b"fLaC") {
        flac(bytes)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        wav(bytes)
    } else if bytes.get(4..8) == Some(b"ftyp") {
        mp4(bytes)
    } else {
        mp3(bytes)
    };
    s.filter(|s| s.is_finite() && *s > 0.0)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// An MPEG audio frame's header: its version (1, 2 or 2.5 as 25), layer, sample rate, size in
/// bytes and samples, and whether it is mono.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    version: u8,
    layer: u8,
    rate: u32,
    size: usize,
    samples: u32,
    mono: bool,
}

fn frame(h: &[u8]) -> Option<Frame> {
    if h.len() < 4 || h[0] != 0xff || h[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = match (h[1] >> 3) & 3 {
        0 => 25,
        2 => 2,
        3 => 1,
        _ => return None,
    };
    let layer = match (h[1] >> 1) & 3 {
        1 => 3,
        2 => 2,
        3 => 1,
        _ => return None,
    };
    let bitrate_index = (h[2] >> 4) as usize;
    let rate_index = ((h[2] >> 2) & 3) as usize;
    if bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    const V1: [[u32; 15]; 3] = [
        [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
        [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    ];
    const V2: [[u32; 15]; 2] = [[0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256], [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]];
    let kbps = if version == 1 { V1[layer as usize - 1][bitrate_index] } else { V2[(layer != 1) as usize][bitrate_index] };
    let rate = [44_100, 48_000, 32_000][rate_index] / if version == 1 { 1 } else if version == 2 { 2 } else { 4 };
    let pad = ((h[2] >> 1) & 1) as usize;
    let bits = kbps as usize * 1000;
    let (size, samples) = match layer {
        1 => ((12 * bits / rate as usize + pad) * 4, 384),
        2 => (144 * bits / rate as usize + pad, 1152),
        _ if version == 1 => (144 * bits / rate as usize + pad, 1152),
        _ => (72 * bits / rate as usize + pad, 576),
    };
    Some(Frame { version, layer, rate, size, samples, mono: h[3] >> 6 == 3 })
}

fn mp3(b: &[u8]) -> Option<f64> {
    // Past an ID3v2 tag (and its footer), to the first frame.
    let mut at = 0;
    if b.starts_with(b"ID3") && b.len() >= 10 {
        let size = b[6..10].iter().fold(0usize, |s, &x| (s << 7) | (x & 0x7f) as usize);
        at = 10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 };
    }
    // (The first header followed by another where its size says: not a stray sync.)
    let (start, first) = (at..b.len().saturating_sub(4)).take(64 * 1024).find_map(|i| {
        let f = frame(&b[i..])?;
        let next = frame(b.get(i + f.size..)?)?;
        (next.version == f.version && next.layer == f.layer && next.rate == f.rate).then_some((i, f))
    })?;
    // A VBR header: the frame count, the frame holding it not counted.
    if first.layer == 3 {
        let side = match (first.version == 1, first.mono) {
            (true, false) => 32,
            (true, true) | (false, false) => 17,
            (false, true) => 9,
        };
        let xing = start + 4 + side;
        if matches!(b.get(xing..xing + 4), Some(b"Xing" | b"Info")) && be32(b, xing + 4)? & 1 != 0 {
            return Some(be32(b, xing + 8)? as f64 * first.samples as f64 / first.rate as f64);
        }
        if b.get(start + 36..start + 40) == Some(b"VBRI") {
            return Some(be32(b, start + 36 + 14)? as f64 * first.samples as f64 / first.rate as f64);
        }
    }
    // Else every frame counted, past junk to the next sync, to a trailing tag.
    let (mut i, mut samples) = (start, 0u64);
    while i + 4 <= b.len() {
        match frame(&b[i..]).filter(|f| f.version == first.version && f.layer == first.layer && f.rate == first.rate && i + f.size <= b.len()) {
            Some(f) => {
                samples += f.samples as u64;
                i += f.size;
            }
            None if b[i..].starts_with(b"TAG") || b[i..].starts_with(b"APETAGEX") => break,
            None => i += 1,
        }
    }
    Some(samples as f64 / first.rate as f64)
}

fn ogg(b: &[u8]) -> Option<f64> {
    // The first page's packet says what it is and its rate (Opus always counts at 48 kHz,
    // after its pre-skip).
    let segments = *b.get(26)? as usize;
    let packet = 27 + segments;
    let (rate, skip) = if b.get(packet..packet + 7) == Some(b"\x01vorbis") {
        (le32(b, packet + 12)? as f64, 0.0)
    } else if b.get(packet..packet + 8) == Some(b"OpusHead") {
        (48_000.0, le16(b, packet + 10)? as f64)
    } else if b.get(packet..packet + 5) == Some(b"\x7fFLAC") {
        // (FLAC in Ogg: its STREAMINFO follows the mapping header.)
        let info = packet + 13 + 4;
        ((be64(b, info + 10)? >> 44) as f64, 0.0)
    } else {
        return None;
    };
    // The last page's granule position: the samples up to its end.
    let from = b.len().saturating_sub(256 * 1024);
    let last = (from..b.len().saturating_sub(14)).rev().find(|&i| &b[i..i + 4] == b"OggS")?;
    let granule = i64::from_le_bytes(b.get(last + 6..last + 14)?.try_into().ok()?);
    (granule > 0 && rate > 0.0).then(|| (granule as f64 - skip).max(0.0) / rate)
}

fn flac(b: &[u8]) -> Option<f64> {
    // STREAMINFO, the first block: the rate in 20 bits, then the channels, the bits a sample
    // and the samples in all in 36.
    if b.get(4)? & 0x7f != 0 {
        return None;
    }
    let packed = be64(b, 8 + 10)?;
    let rate = (packed >> 44) as f64;
    let total = (packed & 0xf_ffff_ffff) as f64;
    (rate > 0.0 && total > 0.0).then(|| total / rate)
}

fn wav(b: &[u8]) -> Option<f64> {
    let (mut at, mut byte_rate) = (12, None);
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = le32(b, at + 4)? as usize;
        match id {
            b"fmt " => byte_rate = le32(b, at + 8 + 8),
            // (A data chunk written while streaming may not know its size: the rest of the file.)
            b"data" => {
                let data = if size == 0 || size == u32::MAX as usize { b.len() - at - 8 } else { size.min(b.len() - at - 8) };
                return Some(data as f64 / byte_rate.filter(|&r| r > 0)? as f64);
            }
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    None
}

fn mp4(b: &[u8]) -> Option<f64> {
    // The movie header in the movie box: its time scale and duration.
    fn boxes(b: &[u8], mut at: usize, end: usize, want: &[u8; 4]) -> Option<(usize, usize)> {
        while at + 8 <= end {
            let size = be32(b, at)? as usize;
            let (head, size) = match size {
                1 => (16, be64(b, at + 8)? as usize),
                0 => (8, end - at),
                s => (8, s),
            };
            if size < head || at + size > end {
                return None;
            }
            if &b[at + 4..at + 8] == want {
                return Some((at + head, at + size));
            }
            at += size;
        }
        None
    }
    let (moov, moov_end) = boxes(b, 0, b.len(), b"moov")?;
    let (mvhd, _) = boxes(b, moov, moov_end, b"mvhd")?;
    let (scale, duration) = match b.get(mvhd)? {
        1 => (be32(b, mvhd + 20)?, be64(b, mvhd + 24)? as f64),
        _ => (be32(b, mvhd + 12)?, be32(b, mvhd + 16)? as f64),
    };
    (scale > 0).then(|| duration / scale as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_a_large_wav_reads_metadata_not_its_audio_payload() {
        use std::io::{Read, Seek, SeekFrom};
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

        // A virtual hour-long WAV: counting reads proves the duration probe never loads
        // its 635 MB payload, without allocating or writing that payload in the test.
        struct LargeWav { header: Vec<u8>, at: u64, length: u64, read: Arc<AtomicUsize> }
        impl Read for LargeWav {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let count = (self.length - self.at.min(self.length)).min(out.len() as u64) as usize;
                out[..count].fill(0);
                if self.at < self.header.len() as u64 {
                    let header = &self.header[self.at as usize..];
                    let copied = header.len().min(count);
                    out[..copied].copy_from_slice(&header[..copied]);
                }
                self.at += count as u64;
                self.read.fetch_add(count, Ordering::Relaxed);
                Ok(count)
            }
        }
        impl Seek for LargeWav {
            fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
                let at = match from {
                    SeekFrom::Start(at) => at as i128,
                    SeekFrom::Current(offset) => self.at as i128 + offset as i128,
                    SeekFrom::End(offset) => self.length as i128 + offset as i128,
                };
                self.at = u64::try_from(at).map_err(|_| std::io::ErrorKind::InvalidInput)?;
                Ok(self.at)
            }
        }
        impl MediaSource for LargeWav {
            fn is_seekable(&self) -> bool { true }
            fn byte_len(&self) -> Option<u64> { Some(self.length) }
        }
        let bytes = 176_400u32 * 3600;
        let mut header = b"RIFF".to_vec();
        header.extend((bytes + 36).to_le_bytes());
        header.extend(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x02\x00");
        header.extend(44_100u32.to_le_bytes());
        header.extend(176_400u32.to_le_bytes());
        header.extend(b"\x04\x00\x10\x00data");
        header.extend(bytes.to_le_bytes());
        let read = Arc::new(AtomicUsize::new(0));
        let source = LargeWav { length: bytes as u64 + header.len() as u64, header, at: 0, read: read.clone() };
        assert_eq!(source_seconds(Box::new(source), "wav"), Some(3600.0));
        assert!(read.load(Ordering::Relaxed) < 256 * 1024, "duration lookup read the payload");
    }

    /// `n` MPEG-1 layer III frames at 128 kbit/s, 44.1 kHz (417 bytes each, unpadded).
    fn mp3_frames(n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..n {
            let mut f = vec![0u8; 417];
            f[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
            out.extend(f);
        }
        out
    }

    #[test]
    fn mp3_frames_are_counted_past_a_tag() {
        let mut b = b"ID3\x04\x00\x00\x00\x00\x00\x0a".to_vec();
        b.extend([0u8; 10]);
        b.extend(mp3_frames(100));
        b.extend(b"TAG");
        b.extend([0u8; 125]);
        let s = seconds(&b).unwrap();
        assert!((s - 100.0 * 1152.0 / 44_100.0).abs() < 1e-9, "{s}");
    }

    #[test]
    fn an_mp3_xing_header_gives_the_frames() {
        let mut b = mp3_frames(3);
        // (In the first frame, after the side information of a stereo MPEG-1 frame.)
        b[36..40].copy_from_slice(b"Xing");
        b[40..44].copy_from_slice(&1u32.to_be_bytes());
        b[44..48].copy_from_slice(&10_000u32.to_be_bytes());
        let s = seconds(&b).unwrap();
        assert!((s - 10_000.0 * 1152.0 / 44_100.0).abs() < 1e-9, "{s}");
    }

    #[test]
    fn ogg_flac_wav_and_m4a_read_their_headers() {
        // Ogg Vorbis: an identification packet at 48 kHz, a last page at 96 000 samples.
        let mut ogg = b"OggS".to_vec();
        ogg.extend([0u8; 22]);
        ogg.push(1);
        ogg.push(30);
        ogg.extend(b"\x01vorbis");
        ogg.extend([0u8; 5]);
        ogg.extend(48_000u32.to_le_bytes());
        ogg.extend([0u8; 14]);
        ogg.extend(b"OggS\x00\x04");
        ogg.extend(96_000i64.to_le_bytes());
        ogg.extend([0u8; 20]);
        assert_eq!(seconds(&ogg), Some(2.0));

        // FLAC: 44.1 kHz, 441 000 samples.
        let mut flac = b"fLaC\x00\x00\x00\x22".to_vec();
        flac.extend([0u8; 10]);
        let packed: u64 = (44_100u64 << 44) | (1 << 41) | (15 << 36) | 441_000;
        flac.extend(packed.to_be_bytes());
        flac.extend([0u8; 16]);
        assert_eq!(seconds(&flac), Some(10.0));

        // WAV: 16-bit stereo 44.1 kHz, a second of data.
        let mut wav = b"RIFF\x00\x00\x00\x00WAVEfmt \x10\x00\x00\x00".to_vec();
        wav.extend([1, 0, 2, 0]);
        wav.extend(44_100u32.to_le_bytes());
        wav.extend(176_400u32.to_le_bytes());
        wav.extend([4, 0, 16, 0]);
        wav.extend(b"data");
        wav.extend(176_400u32.to_le_bytes());
        wav.extend(vec![0u8; 176_400]);
        assert_eq!(seconds(&wav), Some(1.0));

        // M4A: a movie header of 3 minutes at a time scale of 1000.
        let mut m4a = 16u32.to_be_bytes().to_vec();
        m4a.extend(b"ftypM4A \x00\x00\x00\x00");
        let mut mvhd = 108u32.to_be_bytes().to_vec();
        mvhd.extend(b"mvhd");
        mvhd.extend([0u8; 12]);
        mvhd.extend(1000u32.to_be_bytes());
        mvhd.extend(180_000u32.to_be_bytes());
        mvhd.extend(vec![0u8; 108 - mvhd.len()]);
        m4a.extend((8 + mvhd.len() as u32).to_be_bytes());
        m4a.extend(b"moov");
        m4a.extend(mvhd);
        assert_eq!(seconds(&m4a), Some(180.0));
    }

    #[test]
    fn what_is_not_music_has_no_length() {
        assert_eq!(seconds(b""), None);
        assert_eq!(seconds(&[0u8; 4096]), None);
        assert_eq!(seconds(b"OggS"), None);
    }
}
