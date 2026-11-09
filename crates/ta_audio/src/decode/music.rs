//! Streaming decoding of compressed music (MP3, Ogg Vorbis) through symphonia.
//!
//! [`MusicDecoder`] yields stereo frames packet by packet so a background thread can feed the
//! mixer without holding a whole track in memory as PCM (TA's 18 tracks are 35 minutes of
//! 44.1 kHz stereo, ~740 MB as `f32`). [`decode_music`] decodes a whole file for tools/tests.

use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::TimeBase;

use super::SoundData;
use crate::AudioError;

/// Incremental decoder for a music file, backed by memory or a seekable file on disk.
pub struct MusicDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    time_base: Option<TimeBase>,
    total_frames: Option<u64>,
    buffer: Option<SampleBuffer<f32>>,
    /// Corrupt packets skipped so far.
    skipped_packets: u32,
}

impl std::fmt::Debug for MusicDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MusicDecoder")
            .field("sample_rate", &self.sample_rate)
            .field("total_frames", &self.total_frames)
            .finish_non_exhaustive()
    }
}

fn music_error(e: SymphoniaError) -> AudioError {
    AudioError::Music(e.to_string())
}

impl MusicDecoder {
    /// Opens a file. `extension` (e.g. `"mp3"`) speeds up format detection but is optional.
    pub fn open(bytes: Vec<u8>, extension: Option<&str>) -> Result<MusicDecoder, AudioError> {
        Self::from_source(Box::new(Cursor::new(bytes)), extension)
    }

    /// Streams a player's music from disk without reading the entire compressed file into
    /// memory first. Large WAV/FLAC collections use the same bounded decoder buffer as MP3s.
    pub fn open_file(path: &std::path::Path) -> Result<MusicDecoder, AudioError> {
        let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        Self::from_source(Box::new(std::fs::File::open(path)?), extension.as_deref())
    }

    fn from_source(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<MusicDecoder, AudioError> {
        crate::guarded(|| Self::probe(source, extension)).unwrap_or_else(|| Err(AudioError::Music("the file is damaged".into())))
    }

    fn probe(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<MusicDecoder, AudioError> {
        let stream = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions { enable_gapless: true, ..Default::default() },
                &MetadataOptions::default(),
            )
            .map_err(music_error)?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| AudioError::Music("no audio track".into()))?;
        let params = track.codec_params.clone();
        let track_id = track.id;
        let sample_rate = params.sample_rate.ok_or_else(|| AudioError::Music("unknown sample rate".into()))?;
        // (A rate of 0 would never move the resampler on: it would fill memory.)
        if !(1_000..=768_000).contains(&sample_rate) {
            return Err(AudioError::Music(format!("unsupported sample rate: {sample_rate} Hz")));
        }
        let decoder =
            symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(music_error)?;
        Ok(MusicDecoder {
            format,
            decoder,
            track_id,
            sample_rate,
            time_base: params.time_base,
            total_frames: params.n_frames,
            buffer: None,
            skipped_packets: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Track length in frames, when the container states or implies it (for CBR MP3 it is
    /// estimated from the bit rate and file size).
    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    pub fn skipped_packets(&self) -> u32 {
        self.skipped_packets
    }

    /// Goes to `seconds` into the track (as near as the file allows: the packet there), and
    /// returns where it went.
    pub fn seek(&mut self, seconds: f64) -> Result<f64, AudioError> {
        let to = SeekTo::Time { time: seconds.max(0.0).into(), track_id: Some(self.track_id) };
        let seeked = crate::guarded(|| self.format.seek(SeekMode::Coarse, to))
            .unwrap_or_else(|| Err(SymphoniaError::DecodeError("damaged")))
            .map_err(music_error)?;
        self.decoder.reset();
        Ok(match self.time_base {
            Some(base) => {
                let time = base.calc_time(seeked.actual_ts);
                time.seconds as f64 + time.frac
            }
            None => seeked.actual_ts as f64 / self.sample_rate.max(1) as f64,
        })
    }

    /// Decodes the next packet and appends its frames to `out` as stereo (mono is duplicated,
    /// extra channels are dropped). Returns `Ok(false)` at the end of the stream. Corrupt
    /// packets are skipped, as players conventionally do.
    pub fn next_frames(&mut self, out: &mut Vec<[f32; 2]>) -> Result<bool, AudioError> {
        crate::guarded(|| self.decode_next(out)).unwrap_or_else(|| Err(AudioError::Music("the file is damaged".into())))
    }

    fn decode_next(&mut self, out: &mut Vec<[f32; 2]>) -> Result<bool, AudioError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                // A chained stream (new logical Ogg stream): treat as the end of this track.
                Err(SymphoniaError::ResetRequired) => return Ok(false),
                Err(e) => return Err(music_error(e)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(msg)) => {
                    self.skipped_packets += 1;
                    log::debug!("skipping corrupt music packet: {msg}");
                    continue;
                }
                Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(e) => return Err(music_error(e)),
            };
            if decoded.frames() == 0 {
                continue;
            }
            let spec = *decoded.spec();
            let channels = spec.channels.count().max(1);
            let needed = decoded.capacity() * channels;
            if self.buffer.as_ref().is_none_or(|b| b.capacity() < needed) {
                self.buffer = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
            }
            let buffer = self.buffer.as_mut().expect("buffer was just created");
            buffer.copy_interleaved_ref(decoded);
            let samples = buffer.samples();
            if channels == 1 {
                out.extend(samples.iter().map(|&s| [s, s]));
            } else {
                out.extend(samples.chunks_exact(channels).map(|f| [f[0], f[1]]));
            }
            return Ok(true);
        }
    }
}

/// Decodes a whole music file to stereo PCM at its native sample rate.
/// Tools are limited to 256 MiB of decoded PCM. Longer tracks can still be played
/// incrementally with [`MusicDecoder`], without retaining the whole song.
pub fn decode_music(bytes: Vec<u8>, extension: Option<&str>) -> Result<SoundData, AudioError> {
    const MAX_FRAMES: usize = (256 << 20) / std::mem::size_of::<[f32; 2]>();
    let mut decoder = MusicDecoder::open(bytes, extension)?;
    let too_long = || AudioError::Music("whole-track decode exceeds 256 MiB PCM limit; use streaming decoding".into());
    if decoder.total_frames().is_some_and(|n| n > MAX_FRAMES as u64) {
        return Err(too_long());
    }
    // Container frame counts are untrusted estimates, not allocation requests.
    let mut frames = Vec::new();
    let mut packet = Vec::new();
    while decoder.next_frames(&mut packet)? {
        if packet.len() > MAX_FRAMES - frames.len() {
            return Err(too_long());
        }
        frames.extend_from_slice(&packet);
        packet.clear();
    }
    let samples = frames.into_flattened();
    SoundData::new(decoder.sample_rate(), 2, samples)
        .map_err(|e| AudioError::Music(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac_fixture(frames: u64) -> Vec<u8> {
        let mut bytes = b"fLaC\x80\0\0\x22".to_vec();
        bytes.extend_from_slice(&16u16.to_be_bytes()); // minimum block size
        bytes.extend_from_slice(&16u16.to_be_bytes()); // maximum block size
        bytes.extend_from_slice(&[0; 6]); // unknown compressed frame sizes
        let packed = (44_100u64 << 44) | (1 << 41) | (15 << 36) | frames;
        bytes.extend_from_slice(&packed.to_be_bytes()); // stereo, 16 bits, frame count
        bytes.extend_from_slice(&[0; 16]); // no MD5
        // One valid 16-frame silent stereo block (including FLAC CRC8/CRC16).
        bytes.extend_from_slice(&[255, 248, 105, 24, 0, 15, 146, 0, 0, 0, 0, 0, 0, 172, 6]);
        bytes
    }

    #[test]
    fn flac_frame_count_cannot_request_an_unbounded_allocation() {
        let bytes = flac_fixture((1 << 36) - 1);
        assert_eq!(bytes.len(), 57);
        assert_eq!(MusicDecoder::open(bytes.clone(), Some("flac")).unwrap().total_frames(), Some((1 << 36) - 1));
        let error = decode_music(bytes, Some("flac")).unwrap_err();
        assert!(error.to_string().contains("256 MiB PCM limit"), "{error}");
        // Unknown lengths are allowed and decoded incrementally to EOF.
        assert_eq!(decode_music(flac_fixture(0), Some("flac")).unwrap().frames(), 16);
    }

    /// A WAV that says it plays at 0 Hz: refused, never a crash (symphonia panics on it, and a
    /// resampler at 0 Hz never moves on, filling memory).
    #[test]
    fn a_file_at_zero_hertz_is_refused_not_a_crash() {
        let data = [0u8; 64];
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&2u16.to_le_bytes()); // stereo
        wav.extend_from_slice(&0u32.to_le_bytes()); // 0 Hz
        wav.extend_from_slice(&0u32.to_le_bytes()); // bytes a second
        wav.extend_from_slice(&4u16.to_le_bytes()); // block
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        assert!(MusicDecoder::open(wav, Some("wav")).is_err());
    }

    #[test]
    fn file_backed_music_decodes_the_same_frames_as_memory() {
        let path = std::env::temp_dir().join(format!("ta-music-file-test-{}.flac", std::process::id()));
        let bytes = flac_fixture(16);
        std::fs::write(&path, &bytes).unwrap();
        let mut file = MusicDecoder::open_file(&path).unwrap();
        let mut memory = MusicDecoder::open(bytes, Some("flac")).unwrap();
        assert_eq!(file.sample_rate(), memory.sample_rate());
        assert_eq!(file.total_frames(), memory.total_frames());
        let (mut a, mut b) = (Vec::new(), Vec::new());
        while file.next_frames(&mut a).unwrap() {}
        while memory.next_frames(&mut b).unwrap() {}
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}
