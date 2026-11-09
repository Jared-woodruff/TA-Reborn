//! Music streaming: a decoder thread per playing track fills a lock-free ring buffer of
//! device-rate stereo frames that a mixer deck drains.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::bank::SoundSource;
use crate::decode::MusicDecoder;
use crate::dsp::StereoResampler;

/// State shared between a decoder thread, the mixer deck playing it and the game thread.
#[derive(Debug)]
pub(crate) struct StreamShared {
    /// The decoder will produce no more frames (end of file, error or stop).
    pub finished: AtomicBool,
    /// Decoding failed before the end of the track.
    pub failed: AtomicBool,
    /// Asks the decoder thread to exit.
    pub stop: AtomicBool,
    /// Frames consumed by the mixer.
    pub played: AtomicU64,
    /// Track length in output frames, `u64::MAX` while unknown.
    pub total: AtomicU64,
}

impl StreamShared {
    fn new() -> StreamShared {
        StreamShared {
            finished: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            played: AtomicU64::new(0),
            total: AtomicU64::new(u64::MAX),
        }
    }
}

/// The consuming end of a music stream, owned by a mixer deck while it plays.
pub(crate) struct MusicStream {
    pub id: u64,
    pub consumer: rtrb::Consumer<[f32; 2]>,
    pub shared: Arc<StreamShared>,
}

/// Marks the stream finished however the decoder thread exits (including a panic).
struct FinishGuard(Arc<StreamShared>);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.failed.store(true, Ordering::Release);
        }
        self.0.finished.store(true, Ordering::Release);
    }
}

/// Starts decoding `path` on a new thread, from `from` seconds in. `buffer_frames` sets how far
/// ahead of playback the decoder may run.
pub(crate) fn spawn_decoder(
    id: u64,
    path: String,
    from: f64,
    source: Arc<dyn SoundSource>,
    output_rate: u32,
    buffer_frames: usize,
) -> std::io::Result<(MusicStream, Arc<StreamShared>, JoinHandle<()>)> {
    let (producer, consumer) = rtrb::RingBuffer::new(buffer_frames.max(4096));
    let shared = Arc::new(StreamShared::new());
    let thread_shared = shared.clone();
    let handle = std::thread::Builder::new()
        .name(format!("ta-music-{id}"))
        .spawn(move || decode_thread(path, from, source, output_rate, producer, thread_shared))?;
    Ok((MusicStream { id, consumer, shared: shared.clone() }, shared, handle))
}

fn decode_thread(
    path: String,
    from: f64,
    source: Arc<dyn SoundSource>,
    output_rate: u32,
    mut producer: rtrb::Producer<[f32; 2]>,
    shared: Arc<StreamShared>,
) {
    let _guard = FinishGuard(shared.clone());
    let fail = |msg: String| {
        log::warn!("music {path}: {msg}");
        shared.failed.store(true, Ordering::Release);
    };
    if shared.stop.load(Ordering::Acquire) {
        return;
    }
    // Custom songs can be multi-gigabyte WAV/FLAC files: open them as a seekable stream.
    // Archive-backed TA/mod music still comes from the VFS as compressed bytes.
    let decoder = match path.strip_prefix("file:") {
        Some(file) => MusicDecoder::open_file(std::path::Path::new(file)),
        None => {
            let Some(bytes) = source.read(&path) else {
                fail("file not found".into());
                return;
            };
            let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            MusicDecoder::open(bytes, extension.as_deref())
        }
    };
    let mut decoder = match decoder {
        Ok(d) => d,
        Err(e) => {
            fail(e.to_string());
            return;
        }
    };
    if let Some(n) = decoder.total_frames() {
        let out = (n as u128 * output_rate as u128 / decoder.sample_rate().max(1) as u128) as u64;
        shared.total.store(out, Ordering::Relaxed);
    }
    // Further in (the jukebox's seek): played counts from there. A file that cannot seek plays
    // from its start.
    if from > 0.0 {
        match decoder.seek(from) {
            Ok(at) => shared.played.store((at.max(0.0) * output_rate as f64) as u64, Ordering::Relaxed),
            Err(e) => log::warn!("music {path}: cannot seek to {from:.1} s: {e}"),
        }
    }
    let mut resampler = StereoResampler::new(decoder.sample_rate(), output_rate);
    let mut decoded = Vec::with_capacity(4096);
    let mut resampled = Vec::with_capacity(4096);
    let mut produced_frames = 0u64;
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        decoded.clear();
        resampled.clear();
        let more = match decoder.next_frames(&mut decoded) {
            Ok(more) => more,
            Err(e) => {
                fail(e.to_string());
                false
            }
        };
        resampler.process(&decoded, &mut resampled);
        if !more {
            resampler.finish(&mut resampled);
        }
        produced_frames += resampled.len() as u64;
        if !push_all(&mut producer, &resampled, &shared) {
            return;
        }
        if !more {
            shared.total.store(produced_frames, Ordering::Relaxed);
            // A valid container with no decodable audio is still an unusable song. Skip it
            // instead of cycling through it forever with only the playlist gap audible.
            if produced_frames == 0 && !shared.failed.load(Ordering::Acquire) {
                fail("track contains no audio frames".into());
            }
            return;
        }
    }
}

/// Starts playing `path` as a film's score on a new thread: the spans of `arrangement` in turn,
/// from its own second `from` (see [`crate::music::Arrangement`]). `played` and `total` count
/// the score's frames, so its position is the score's own seconds.
pub(crate) fn spawn_arranged(
    id: u64,
    path: String,
    arrangement: Arc<crate::music::Arrangement>,
    from: f64,
    source: Arc<dyn SoundSource>,
    output_rate: u32,
    buffer_frames: usize,
) -> std::io::Result<(MusicStream, Arc<StreamShared>, JoinHandle<()>)> {
    let (producer, consumer) = rtrb::RingBuffer::new(buffer_frames.max(4096));
    let shared = Arc::new(StreamShared::new());
    let thread_shared = shared.clone();
    let handle = std::thread::Builder::new()
        .name(format!("ta-score-{id}"))
        .spawn(move || arranged_thread(path, arrangement, from, source, output_rate, producer, thread_shared))?;
    Ok((MusicStream { id, consumer, shared: shared.clone() }, shared, handle))
}

/// A track as decoded so far, kept (a score goes back to what it has played), 16-bit stereo at
/// its own rate.
struct Kept {
    decoder: Option<MusicDecoder>,
    frames: Vec<[i16; 2]>,
    chunk: Vec<[f32; 2]>,
}

impl Kept {
    /// Decodes until frame `i` is kept, or the track ends (false then, or on a failure).
    fn reach(&mut self, i: usize) -> Result<bool, crate::AudioError> {
        while self.frames.len() <= i {
            let Some(decoder) = self.decoder.as_mut() else { return Ok(false) };
            self.chunk.clear();
            let more = decoder.next_frames(&mut self.chunk)?;
            let q = |s: f32| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            self.frames.extend(self.chunk.iter().map(|&[l, r]| [q(l), q(r)]));
            if !more {
                self.decoder = None;
            }
        }
        Ok(true)
    }

    fn get(&self, i: usize) -> [f32; 2] {
        self.frames.get(i).map_or([0.0; 2], |&[l, r]| [l as f32 / 32767.0, r as f32 / 32767.0])
    }
}

/// Which frames of the track each frame of a score plays: the spans in frames, where each starts
/// in the score, and the crossfade before each join.
#[derive(Debug)]
pub(crate) struct Arranged {
    spans: Vec<(usize, usize)>,
    starts: Vec<usize>,
    /// Frames of crossfade before the join after each span (0 for the last).
    fades: Vec<usize>,
    total: usize,
}

impl Arranged {
    pub(crate) fn new(arrangement: &crate::music::Arrangement, rate: u32) -> Arranged {
        let at = |s: f64| (s.max(0.0) * rate as f64).round() as usize;
        let spans: Vec<(usize, usize)> = arrangement.spans.iter().map(|&(a, b)| (at(a), at(b).max(at(a)))).collect();
        let mut starts = Vec::with_capacity(spans.len());
        let mut total = 0;
        for &(a, b) in &spans {
            starts.push(total);
            total += b - a;
        }
        let x = at(arrangement.crossfade);
        // (Never longer than the span it ends, nor reaching before the track's start.)
        let fades = (0..spans.len()).map(|k| spans.get(k + 1).map_or(0, |next| x.min(spans[k].1 - spans[k].0).min(next.0))).collect();
        Arranged { spans, starts, fades, total }
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }

    /// The track's frames the score's frame `a` plays and their weights (one, or two in a
    /// crossfade); `k` is a hint, the span it was in last (kept up to date).
    pub(crate) fn taps(&self, a: usize, k: &mut usize) -> [(usize, f32); 2] {
        while *k + 1 < self.spans.len() && a >= self.starts[*k + 1] {
            *k += 1;
        }
        while *k > 0 && a < self.starts[*k] {
            *k -= 1;
        }
        let (s, e) = self.spans[*k];
        let o = a - self.starts[*k];
        let len = e - s;
        let x = self.fades[*k];
        if x > 0 && o + x >= len {
            // Before the join: this span fading out, the next one's frames before its start
            // fading in.
            let u = (o + x - len) as f32 / x as f32;
            let next = self.spans[*k + 1].0 - (len - o);
            [(s + o, 1.0 - u), (next, u)]
        } else {
            [(s + o, 1.0), (0, 0.0)]
        }
    }
}

fn arranged_thread(
    path: String,
    arrangement: Arc<crate::music::Arrangement>,
    from: f64,
    source: Arc<dyn SoundSource>,
    output_rate: u32,
    mut producer: rtrb::Producer<[f32; 2]>,
    shared: Arc<StreamShared>,
) {
    let _guard = FinishGuard(shared.clone());
    let fail = |msg: String| {
        log::warn!("score {path}: {msg}");
        shared.failed.store(true, Ordering::Release);
    };
    if shared.stop.load(Ordering::Acquire) {
        return;
    }
    let decoder = match path.strip_prefix("file:") {
        Some(file) => MusicDecoder::open_file(std::path::Path::new(file)),
        None => match source.read(&path) {
            Some(bytes) => MusicDecoder::open(bytes, path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref()),
            None => {
                fail("file not found".into());
                return;
            }
        },
    };
    let decoder = match decoder {
        Ok(d) => d,
        Err(e) => {
            fail(e.to_string());
            return;
        }
    };
    let rate = decoder.sample_rate();
    let plan = Arranged::new(&arrangement, rate);
    let to_output = |frames: usize| (frames as u128 * output_rate as u128 / rate.max(1) as u128) as u64;
    shared.total.store(to_output(plan.total()), Ordering::Relaxed);
    let mut at = ((from.max(0.0) * rate as f64).round() as usize).min(plan.total());
    shared.played.store(to_output(at), Ordering::Relaxed);
    let mut kept = Kept { decoder: Some(decoder), frames: Vec::new(), chunk: Vec::with_capacity(4096) };
    let mut resampler = StereoResampler::new(rate, output_rate);
    let gain = if arrangement.gain.is_finite() { arrangement.gain.max(0.0) } else { 1.0 };
    let mut block = Vec::with_capacity(4096);
    let mut resampled = Vec::with_capacity(4096);
    let mut k = 0;
    while at < plan.total() {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        block.clear();
        resampled.clear();
        let end = (at + 4096).min(plan.total());
        for a in at..end {
            let taps = plan.taps(a, &mut k);
            let need = taps.iter().filter(|t| t.1 > 0.0).map(|t| t.0).max().unwrap_or(0);
            if let Err(e) = kept.reach(need) {
                fail(e.to_string());
                return;
            }
            let mut frame = [0.0f32; 2];
            for (i, w) in taps {
                if w > 0.0 {
                    let s = kept.get(i);
                    frame[0] += s[0] * w;
                    frame[1] += s[1] * w;
                }
            }
            block.push([frame[0] * gain, frame[1] * gain]);
        }
        at = end;
        resampler.process(&block, &mut resampled);
        if at >= plan.total() {
            resampler.finish(&mut resampled);
        }
        if !push_all(&mut producer, &resampled, &shared) {
            return;
        }
    }
    if kept.frames.is_empty() {
        fail("track contains no audio frames".into());
    }
}

/// Pushes every frame, sleeping while the ring buffer is full. Returns `false` if the stream
/// was stopped or its consumer dropped.
fn push_all(producer: &mut rtrb::Producer<[f32; 2]>, mut frames: &[[f32; 2]], shared: &StreamShared) -> bool {
    loop {
        let (_, rest) = producer.push_partial_slice(frames);
        frames = rest;
        if frames.is_empty() {
            return true;
        }
        if shared.stop.load(Ordering::Acquire) || producer.is_abandoned() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bank::MemorySource;

    fn wav(frames: usize) -> Vec<u8> {
        let data_bytes = frames as u32 * 4;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&192_000u32.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for _ in 0..frames {
            bytes.extend_from_slice(&8192i16.to_le_bytes());
            bytes.extend_from_slice(&(-8192i16).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn stream_finishes_with_exact_output_length_and_stereo_samples() {
        let mut source = MemorySource::new();
        source.insert("music/test.wav", wav(120));
        let (mut stream, shared, thread) = spawn_decoder(1, "music/test.wav".into(), 0.0, Arc::new(source), 48_000, 4096).unwrap();
        thread.join().unwrap();
        assert!(shared.finished.load(Ordering::Acquire));
        assert!(!shared.failed.load(Ordering::Acquire));
        assert_eq!(shared.total.load(Ordering::Relaxed), 120);
        for _ in 0..120 {
            assert_eq!(stream.consumer.pop().unwrap(), [0.25, -0.25]);
        }
        assert!(stream.consumer.is_empty());
    }

    /// The jukebox's seek: the stream starts that far in, and counts its playing from there.
    #[test]
    fn a_stream_can_start_further_in() {
        let mut source = MemorySource::new();
        source.insert("music/long.wav", wav(96_000));
        let (mut stream, shared, thread) = spawn_decoder(5, "music/long.wav".into(), 1.5, Arc::new(source), 48_000, 128_000).unwrap();
        thread.join().unwrap();
        assert!(!shared.failed.load(Ordering::Acquire));
        let played = shared.played.load(Ordering::Relaxed);
        let mut left = 0u64;
        while stream.consumer.pop().is_ok() {
            left += 1;
        }
        // (A coarse seek lands at a packet: near enough.)
        assert!(played.abs_diff(72_000) < 4_800, "{played}");
        assert!((played + left).abs_diff(96_000) < 4_800, "{played} + {left}");
    }

    /// A score's spans in turn, each join a linear crossfade over the frames before it: the
    /// outgoing span's last frames fading out as the incoming span's frames before its start fade
    /// in, the join itself the incoming span's.
    #[test]
    fn a_score_plays_its_spans_with_crossfades_before_the_joins() {
        use crate::music::Arrangement;
        // 1000 frames a second: spans [2, 4) s and [1, 3) s, a 10-frame crossfade.
        let arrangement = Arrangement { spans: vec![(2.0, 4.0), (1.0, 3.0)], crossfade: 0.010, gain: 1.0 };
        let plan = Arranged::new(&arrangement, 1000);
        assert_eq!(plan.total(), 4000);
        let mut k = 0;
        assert_eq!(plan.taps(0, &mut k), [(2000, 1.0), (0, 0.0)]);
        assert_eq!(plan.taps(1989, &mut k), [(3989, 1.0), (0, 0.0)]);
        // The crossfade: frames 1990..2000 of the score mix 3990..4000 out and 990..1000 in.
        let [(out, wo), (inc, wi)] = plan.taps(1990, &mut k);
        assert_eq!((out, inc), (3990, 990));
        assert!((wo - 1.0).abs() < 1e-6 && wi.abs() < 1e-6);
        let [(out, wo), (inc, wi)] = plan.taps(1995, &mut k);
        assert_eq!((out, inc), (3995, 995));
        assert!((wo - 0.5).abs() < 1e-6 && (wi - 0.5).abs() < 1e-6);
        // The join: the incoming span alone, from its start.
        assert_eq!(plan.taps(2000, &mut k), [(1000, 1.0), (0, 0.0)]);
        assert_eq!(plan.taps(3999, &mut k), [(2999, 1.0), (0, 0.0)]);
        // (Going back finds its span again.)
        assert_eq!(plan.taps(5, &mut k), [(2005, 1.0), (0, 0.0)]);
        // A crossfade never longer than its span, nor reaching before the track's start.
        // (Spans of 4 and 998 frames, the second from frame 2: two frames of crossfade.)
        let short = Arranged::new(&Arrangement { spans: vec![(0.0, 0.004), (0.002, 1.0)], crossfade: 0.010, gain: 1.0 }, 1000);
        let mut k = 0;
        assert_eq!(short.taps(1, &mut k), [(1, 1.0), (0, 0.0)]);
        assert_eq!(short.taps(3, &mut k), [(3, 0.5), (1, 0.5)]);
        assert_eq!(short.taps(4, &mut k), [(2, 1.0), (0, 0.0)]);
    }

    /// The score's stream: its frames as the arrangement has them, the gain on them, counted from
    /// where it started in its own seconds.
    #[test]
    fn a_score_stream_counts_its_own_seconds() {
        use crate::music::Arrangement;
        // A ramp: frame i holds i / 100000 (16-bit kept: within 1/32767).
        let frames = 48_000 * 3;
        let mut bytes = b"RIFF".to_vec();
        let data = frames as u32 * 4;
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&192_000u32.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 / frames as f32) * 30000.0) as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
            bytes.extend_from_slice(&(-v).to_le_bytes());
        }
        let mut source = MemorySource::new();
        source.insert("music/song.wav", bytes);
        let arrangement = Arc::new(Arrangement { spans: vec![(0.0, 2.0), (1.0, 3.0)], crossfade: 0.0, gain: 0.5 });
        // From the score's second 1.5: half a second of the first span, then the second's two.
        let (mut stream, shared, thread) = spawn_arranged(9, "music/song.wav".into(), arrangement, 1.5, Arc::new(source), 48_000, 400_000).unwrap();
        thread.join().unwrap();
        assert!(!shared.failed.load(Ordering::Acquire));
        assert_eq!(shared.total.load(Ordering::Relaxed), 4 * 48_000);
        assert_eq!(shared.played.load(Ordering::Relaxed), 72_000, "1.5 s in");
        let mut out = Vec::new();
        while let Ok(f) = stream.consumer.pop() {
            out.push(f);
        }
        assert_eq!(out.len(), 2 * 48_000 + 24_000);
        let expect = |src: usize| 0.5 * (((src as f32 / frames as f32) * 30000.0) as i16) as f32 / 32767.0;
        for (i, src) in [(0usize, 72_000usize), (23_999, 95_999), (24_000, 48_000), (24_000 + 96_000 - 1, 143_999)] {
            assert!((out[i][0] - expect(src)).abs() < 2e-4, "frame {i}: {} against {}", out[i][0], expect(src));
            assert!((out[i][1] + expect(src)).abs() < 2e-4);
        }
    }

    #[test]
    fn empty_music_is_a_failure_instead_of_an_endless_silent_playlist() {
        let mut source = MemorySource::new();
        source.insert("music/empty.wav", wav(0));
        let (stream, shared, thread) = spawn_decoder(2, "music/empty.wav".into(), 0.0, Arc::new(source), 48_000, 4096).unwrap();
        thread.join().unwrap();
        assert!(shared.finished.load(Ordering::Acquire));
        assert!(shared.failed.load(Ordering::Acquire));
        assert!(stream.consumer.is_empty());
    }

    #[test]
    fn missing_and_corrupt_music_finish_as_failures_without_samples() {
        let mut source = MemorySource::new();
        source.insert("music/corrupt.wav", b"not a wave file".to_vec());
        let source = Arc::new(source);
        for path in ["music/missing.wav", "music/corrupt.wav"] {
            let (stream, shared, thread) = spawn_decoder(3, path.into(), 0.0, source.clone(), 48_000, 4096).unwrap();
            thread.join().unwrap();
            assert!(shared.finished.load(Ordering::Acquire));
            assert!(shared.failed.load(Ordering::Acquire));
            assert!(stream.consumer.is_empty());
        }
    }

    #[test]
    fn full_music_buffer_can_be_cancelled_or_abandoned_without_a_consumer_drain() {
        for abandon in [false, true] {
            let (mut producer, consumer) = rtrb::RingBuffer::new(1);
            producer.push([0.25, -0.25]).unwrap();
            let shared = Arc::new(StreamShared::new());
            let mut consumer = Some(consumer);
            if abandon { drop(consumer.take()); }
            else { shared.stop.store(true, Ordering::Release); }
            let (done, result) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                done.send(push_all(&mut producer, &[[0.5, -0.5]], &shared)).unwrap();
            });
            assert!(!result.recv_timeout(Duration::from_secs(5)).expect("decoder must not wait for a stopped output to drain"));
            worker.join().unwrap();
            if let Some(mut consumer) = consumer {
                assert_eq!(consumer.pop().unwrap(), [0.25, -0.25]);
                assert!(consumer.is_empty());
            }
        }
    }

    #[test]
    fn panicking_decoder_source_marks_the_stream_failed_and_finished() {
        struct FailedSource;
        impl SoundSource for FailedSource {
            fn read(&self, _: &str) -> Option<Vec<u8>> { panic!("synthetic decoder source failure") }
            fn list(&self, _: &str) -> Vec<String> { Vec::new() }
        }
        let (stream, shared, thread) = spawn_decoder(4, "music/panic.wav".into(), 0.0, Arc::new(FailedSource), 48_000, 4096).unwrap();
        assert!(thread.join().is_err());
        assert!(shared.finished.load(Ordering::Acquire));
        assert!(shared.failed.load(Ordering::Acquire));
        assert!(stream.consumer.is_empty());
    }
}
