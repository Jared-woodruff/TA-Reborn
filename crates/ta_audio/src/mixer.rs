//! The real-time mixer.
//!
//! Runs inside the audio device callback (or on the caller's thread in offline mode). It never
//! allocates, blocks on locks, or frees memory while mixing: sounds arrive as `Arc`s inside
//! [`Command`]s and leave in [`Event`]s, so their last reference is always dropped on the game
//! thread. All buffers are allocated up front.
//!
//! Signal flow per block: voices (cubic-interpolated, per-voice gain ramps and fades) are summed
//! into the effects or voice bus; the two music decks (crossfade gains) into the music bus; the
//! buses are summed with smoothed bus gains, scaled by the master gain and peak-limited.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use cpal::{FromSample, SizedSample};

use crate::decode::SoundData;
use crate::dsp::{GainRamp, Limiter, hermite};
use crate::stream::MusicStream;

/// Frames rendered per internal block (device buffers are split into blocks of this size).
pub(crate) const MAX_BLOCK: usize = 512;
/// Number of music decks (current track + the one fading out).
pub(crate) const DECKS: usize = 2;
/// Buses with their own gain. The master gain is separate.
pub(crate) const BUSES: usize = 3;

/// Mixer bus indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MixBus {
    Effects = 0,
    Voice = 1,
    Music = 2,
}

/// Instructions from the game thread.
pub(crate) enum Command {
    /// Starts a voice in `slot` (replacing whatever was there).
    Play {
        slot: u16,
        generation: u32,
        sound: Arc<SoundData>,
        bus: MixBus,
        gains: [f32; 2],
        /// Source frames advanced per output frame (sample-rate ratio × pitch).
        step: f64,
        looping: bool,
        /// Initial read position in source frames.
        start: f64,
        /// Fade-in length in output frames (0 = start at full level).
        fade_in: u32,
    },
    /// New target gains, reached by a 10 ms linear ramp.
    SetGains { slot: u16, generation: u32, gains: [f32; 2] },
    SetStep { slot: u16, generation: u32, step: f64 },
    /// Fades a voice out over `fade` frames, then ends it.
    Stop { slot: u16, generation: u32, fade: u32 },
    /// Target gain for a bus (smoothed over 10 ms).
    SetBusGain { bus: MixBus, gain: f32 },
    SetMasterGain(f32),
    /// Starts a music stream on a deck, fading in from silence over `fade_in` frames.
    DeckStart { deck: u8, stream: MusicStream, fade_in: u32, crossfade_from: Option<(u8, u64)> },
    /// Ramps a deck's gain towards `target` over `frames`; with `release` the stream is
    /// released once the ramp completes.
    DeckFade { deck: u8, stream_id: u64, target: f32, frames: u32, release: bool },
}

/// Notifications to the game thread. They carry the resources the audio thread is done with.
pub(crate) enum Event {
    VoiceEnded { slot: u16, generation: u32, sound: Arc<SoundData> },
    /// A deck let go of its stream: `natural` when the track played to its end.
    DeckEnded { deck: u8, stream: MusicStream, natural: bool },
}

/// Counters readable from any thread.
#[derive(Debug, Default)]
pub(crate) struct MixerStats {
    pub callbacks: AtomicU64,
    pub frames: AtomicU64,
    pub active_voices: AtomicU32,
    /// Music frames that were due but not yet decoded.
    pub underrun_frames: AtomicU64,
    /// Highest output sample magnitude since the last read (f32 bits).
    pub peak_bits: AtomicU32,
    /// Lowest limiter gain since the last read (f32 bits).
    pub limiter_bits: AtomicU32,
    /// The music's latest samples.
    pub scope: MusicScope,
}

/// How many of the music's latest samples [`MusicScope`] keeps (about 43 ms at 48 kHz).
pub const SCOPE: usize = 2048;

/// The music as it leaves the decks (before the music volume), its latest [`SCOPE`] samples in a
/// ring, mono: for the jukebox's spectrum. Written by the audio thread, read from any (a read
/// racing a write may mix two blocks, which a picture of the sound does not mind).
#[derive(Debug)]
pub(crate) struct MusicScope {
    samples: Box<[AtomicU32]>,
    /// Samples written so far (the next goes at this modulo [`SCOPE`]).
    written: AtomicU64,
}

impl Default for MusicScope {
    fn default() -> Self {
        MusicScope { samples: (0..SCOPE).map(|_| AtomicU32::new(0)).collect(), written: AtomicU64::new(0) }
    }
}

impl MusicScope {
    /// Adds interleaved stereo frames.
    pub(crate) fn write(&self, stereo: &[f32]) {
        let start = self.written.load(Ordering::Relaxed);
        let (frames, _) = stereo.as_chunks::<2>();
        for (i, [l, r]) in frames.iter().enumerate() {
            self.samples[(start as usize + i) % SCOPE].store(((l + r) * 0.5).to_bits(), Ordering::Relaxed);
        }
        self.written.store(start + frames.len() as u64, Ordering::Release);
    }

    /// The latest `out.len()` samples (at most [`SCOPE`]), oldest first.
    pub(crate) fn read(&self, out: &mut [f32]) {
        let end = self.written.load(Ordering::Acquire) as usize;
        let n = out.len().min(SCOPE);
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o = f32::from_bits(self.samples[(end + SCOPE - n + i) % SCOPE].load(Ordering::Relaxed));
        }
    }
}

impl MixerStats {
    pub(crate) fn new() -> MixerStats {
        let stats = MixerStats::default();
        stats.limiter_bits.store(1f32.to_bits(), Ordering::Relaxed);
        stats
    }

    /// Returns and resets the peak level.
    pub(crate) fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed))
    }

    /// Returns and resets the lowest limiter gain.
    pub(crate) fn take_limiter_gain(&self) -> f32 {
        f32::from_bits(self.limiter_bits.swap(1f32.to_bits(), Ordering::Relaxed))
    }
}

struct Voice {
    generation: u32,
    sound: Arc<SoundData>,
    bus: MixBus,
    pos: f64,
    step: f64,
    looping: bool,
    gains: [GainRamp; 2],
    /// Fade envelope in `0..=1` and its per-frame change.
    envelope: f32,
    envelope_step: f32,
    stopping: bool,
    /// Finished but its end event has not been delivered yet (event queue full).
    finished: bool,
}

struct Deck {
    stream: Option<MusicStream>,
    gain: f32,
    target: f32,
    /// Per-frame gain change of the current ramp.
    step: f32,
    release_at_target: bool,
    /// Waiting for the decoder to buffer enough before starting.
    priming: bool,
    /// Fade the outgoing deck only once this replacement has buffered audible samples.
    crossfade_from: Option<(u8, u64)>,
    fade_in: u32,
    /// A pending end: set when the stream should be handed back but the event queue was full.
    pending_end: Option<bool>,
}

impl Deck {
    fn new() -> Deck {
        Deck {
            stream: None,
            gain: 0.0,
            target: 0.0,
            step: 0.0,
            release_at_target: false,
            priming: false,
            crossfade_from: None,
            fade_in: 0,
            pending_end: None,
        }
    }
}

pub(crate) struct Mixer {
    sample_rate: u32,
    commands: rtrb::Consumer<Command>,
    events: rtrb::Producer<Event>,
    voices: Box<[Option<Voice>]>,
    decks: [Deck; DECKS],
    bus_gain: [GainRamp; BUSES],
    master: GainRamp,
    gain_ramp_frames: u32,
    /// Per-bus stereo accumulation buffers, `BUSES × 2 × MAX_BLOCK`.
    bus_buffers: Box<[f32]>,
    /// Final stereo block.
    mix: Box<[f32]>,
    deck_scratch: Box<[[f32; 2]]>,
    limiter: Limiter,
    /// Offline rendering: wait for music decoders instead of dropping out.
    blocking_music: bool,
    /// Frames of decoded music a deck waits for before it starts.
    prime_frames: usize,
    stats: Arc<MixerStats>,
}

impl Mixer {
    pub(crate) fn new(
        sample_rate: u32,
        slots: usize,
        commands: rtrb::Consumer<Command>,
        events: rtrb::Producer<Event>,
        stats: Arc<MixerStats>,
        blocking_music: bool,
    ) -> Mixer {
        Mixer {
            sample_rate,
            commands,
            events,
            voices: (0..slots).map(|_| None).collect(),
            decks: [Deck::new(), Deck::new()],
            bus_gain: std::array::from_fn(|_| GainRamp::new(1.0)),
            master: GainRamp::new(1.0),
            gain_ramp_frames: (sample_rate / 100).max(1),
            bus_buffers: vec![0.0; BUSES * 2 * MAX_BLOCK].into_boxed_slice(),
            mix: vec![0.0; 2 * MAX_BLOCK].into_boxed_slice(),
            deck_scratch: vec![[0.0; 2]; MAX_BLOCK].into_boxed_slice(),
            limiter: Limiter::new(sample_rate),
            blocking_music,
            prime_frames: (sample_rate as usize / 10).max(MAX_BLOCK),
            stats,
        }
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Apply the user's initial gains before the device can begin its first callback.
    pub(crate) fn initialize_gains(&mut self, master: f32, buses: [f32; BUSES]) {
        self.master = GainRamp::new(master);
        self.bus_gain = buses.map(GainRamp::new);
    }

    /// Fills an interleaved device buffer with `channels` channels. Stereo goes to the first two
    /// channels (extra channels stay silent); mono devices get the average of both sides.
    pub(crate) fn render<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T], channels: usize) {
        self.process_commands();
        let channels = channels.max(1);
        let frames = out.len() / channels;
        let mut done = 0;
        let mut peak = 0.0f32;
        let mut min_limiter = 1.0f32;
        while done < frames {
            let n = (frames - done).min(MAX_BLOCK);
            self.render_block(n);
            min_limiter = min_limiter.min(self.limiter.gain());
            let block = &self.mix[..2 * n];
            let dst = &mut out[done * channels..(done + n) * channels];
            for (frame, lr) in dst.chunks_exact_mut(channels).zip(block.as_chunks::<2>().0) {
                peak = peak.max(lr[0].abs()).max(lr[1].abs());
                if channels == 1 {
                    frame[0] = T::from_sample(0.5 * (lr[0] + lr[1]));
                } else {
                    frame[0] = T::from_sample(lr[0]);
                    frame[1] = T::from_sample(lr[1]);
                    for extra in &mut frame[2..] {
                        *extra = T::EQUILIBRIUM;
                    }
                }
            }
            done += n;
        }
        let stats = &self.stats;
        stats.callbacks.fetch_add(1, Ordering::Relaxed);
        stats.frames.fetch_add(frames as u64, Ordering::Relaxed);
        stats.peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
        stats.limiter_bits.fetch_min(min_limiter.to_bits(), Ordering::Relaxed);
        let active = self.voices.iter().filter(|v| v.as_ref().is_some_and(|v| !v.finished)).count();
        stats.active_voices.store(active as u32, Ordering::Relaxed);
    }

    fn process_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            match command {
                Command::Play { slot, generation, sound, bus, gains, step, looping, start, fade_in } => {
                    let Some(entry) = self.voices.get_mut(slot as usize) else {
                        // Cannot happen with a well-behaved game thread; hand the sound back.
                        let _ = self.events.push(Event::VoiceEnded { slot, generation, sound });
                        continue;
                    };
                    if let Some(old) = entry.take() {
                        let ended = Event::VoiceEnded { slot, generation: old.generation, sound: old.sound };
                        let _ = self.events.push(ended);
                    }
                    let frames = sound.frames() as f64;
                    let start = if looping && frames > 0.0 { start.rem_euclid(frames) } else { start.max(0.0) };
                    *entry = Some(Voice {
                        generation,
                        sound,
                        bus,
                        pos: start,
                        step: step.max(0.0),
                        looping,
                        gains: gains.map(GainRamp::new),
                        envelope: if fade_in > 0 { 0.0 } else { 1.0 },
                        envelope_step: if fade_in > 0 { 1.0 / fade_in as f32 } else { 0.0 },
                        stopping: false,
                        finished: false,
                    });
                }
                Command::SetGains { slot, generation, gains } => {
                    let frames = self.gain_ramp_frames;
                    if let Some(v) = self.voice_mut(slot, generation) {
                        for (ramp, target) in v.gains.iter_mut().zip(gains) { ramp.set(target, frames); }
                    }
                }
                Command::SetStep { slot, generation, step } => {
                    if let Some(v) = self.voice_mut(slot, generation) {
                        v.step = step.max(0.0);
                    }
                }
                Command::Stop { slot, generation, fade } => {
                    if let Some(v) = self.voice_mut(slot, generation) {
                        v.stopping = true;
                        v.envelope_step = -1.0 / fade.max(1) as f32;
                    }
                }
                Command::SetBusGain { bus, gain } => self.bus_gain[bus as usize].set(gain.max(0.0), self.gain_ramp_frames),
                Command::SetMasterGain(gain) => self.master.set(gain.max(0.0), self.gain_ramp_frames),
                Command::DeckStart { deck, stream, fade_in, crossfade_from } => {
                    let d = deck as usize % DECKS;
                    if let Some(old) = self.decks[d].stream.take() {
                        let _ = self.events.push(Event::DeckEnded { deck, stream: old, natural: false });
                    }
                    let deck_state = &mut self.decks[d];
                    deck_state.stream = Some(stream);
                    deck_state.target = 1.0;
                    deck_state.release_at_target = false;
                    deck_state.priming = true;
                    deck_state.crossfade_from = crossfade_from;
                    deck_state.fade_in = fade_in;
                    deck_state.pending_end = None;
                    if fade_in > 0 {
                        deck_state.gain = 0.0;
                        deck_state.step = 1.0 / fade_in as f32;
                    } else {
                        deck_state.gain = 1.0;
                        deck_state.step = 0.0;
                    }
                }
                Command::DeckFade { deck, stream_id, target, frames, release } => {
                    self.fade_deck(deck, stream_id, target, frames, release);
                }
            }
        }
    }

    fn voice_mut(&mut self, slot: u16, generation: u32) -> Option<&mut Voice> {
        self.voices
            .get_mut(slot as usize)?
            .as_mut()
            .filter(|v| v.generation == generation && !v.finished)
    }

    fn fade_deck(&mut self, deck: u8, stream_id: u64, target: f32, frames: u32, release: bool) {
        let d = &mut self.decks[deck as usize % DECKS];
        if !d.stream.as_ref().is_some_and(|s| s.id == stream_id) { return; }
        d.target = target.clamp(0.0, 1.0);
        d.step = (d.target - d.gain).abs() / frames.max(1) as f32;
        d.release_at_target = release;
        // Stopping a replacement that has not started must also stop the outgoing music
        // it kept alive. Take the link before following it, so stale links cannot recurse.
        if release && target == 0.0 && let Some((old_deck, old_id)) = d.crossfade_from.take() {
            self.fade_deck(old_deck, old_id, target, frames, release);
        }
    }

    fn begin_crossfades(&mut self) {
        for index in 0..DECKS {
            let d = &mut self.decks[index];
            let Some(stream) = &d.stream else { continue };
            if self.blocking_music && d.priming && d.crossfade_from.is_some() {
                // Offline mixes must arm the handover before consuming even a track
                // shorter than one block. Real-time callbacks never wait for decoding.
                let mut waited = 0;
                while stream.consumer.is_empty() && !stream.shared.finished.load(Ordering::Acquire) && waited < 10_000 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    waited += 1;
                }
            }
            let buffered = stream.consumer.slots();
            let ready = self.blocking_music || !d.priming || buffered >= self.prime_frames || stream.shared.finished.load(Ordering::Acquire);
            // An empty/failed replacement leaves the previous track audible while the
            // sequencer tries another.
            if ready && buffered > 0 && let Some((old_deck, old_id)) = d.crossfade_from.take() {
                let frames = d.fade_in;
                self.fade_deck(old_deck, old_id, 0.0, frames, true);
            }
        }
    }

    /// Renders `frames` (≤ [`MAX_BLOCK`]) stereo frames into `self.mix`.
    fn render_block(&mut self, frames: usize) {
        debug_assert!(frames <= MAX_BLOCK);
        let len = 2 * frames;
        self.begin_crossfades();
        for bus in 0..BUSES {
            self.bus_buffers[bus * 2 * MAX_BLOCK..bus * 2 * MAX_BLOCK + len].fill(0.0);
        }

        for (slot, entry) in self.voices.iter_mut().enumerate() {
            let Some(voice) = entry else { continue };
            if !voice.finished {
                let offset = voice.bus as usize * 2 * MAX_BLOCK;
                let out = &mut self.bus_buffers[offset..offset + len];
                voice.finished = render_voice(voice, out, frames);
            }
            if voice.finished {
                // Hand the sound back; if the queue is full, retry next block.
                let generation = voice.generation;
                let sound = voice.sound.clone();
                if self.events.push(Event::VoiceEnded { slot: slot as u16, generation, sound }).is_ok() {
                    *entry = None;
                }
            }
        }

        let music = &mut self.bus_buffers[MixBus::Music as usize * 2 * MAX_BLOCK..][..len];
        for (index, deck) in self.decks.iter_mut().enumerate() {
            render_deck(
                index as u8,
                deck,
                music,
                &mut self.deck_scratch[..frames],
                &mut self.events,
                self.blocking_music,
                self.prime_frames,
                &self.stats,
            );
        }
        self.stats.scope.write(music);

        // Sum the buses with smoothed gains, then master gain and limiter.
        for f in 0..frames {
            let mut frame = [0.0f32; 2];
            for b in 0..BUSES {
                let gain = self.bus_gain[b].next();
                let base = b * 2 * MAX_BLOCK + 2 * f;
                frame[0] += self.bus_buffers[base] * gain;
                frame[1] += self.bus_buffers[base + 1] * gain;
            }
            let master = self.master.next();
            frame[0] *= master;
            frame[1] *= master;
            if !(frame[0].is_finite() && frame[1].is_finite()) {
                frame = [0.0; 2];
            }
            self.limiter.process(&mut frame);
            self.mix[2 * f] = frame[0];
            self.mix[2 * f + 1] = frame[1];
        }
    }
}

/// Mixes one voice into `out` (stereo, `frames` long). Returns `true` when the voice is done.
fn render_voice(v: &mut Voice, out: &mut [f32], frames: usize) -> bool {
    let data = v.sound.samples();
    let channels = v.sound.channels() as usize;
    let len = v.sound.frames();
    if len == 0 {
        return true;
    }
    let looping = v.looping;
    let len_f = len as f64;
    let mut done = false;

    // Sample fetch with edge handling: silence outside a one-shot, wrap-around for loops.
    let fetch = |index: isize, channel: usize| -> f32 {
        let i = if looping {
            index.rem_euclid(len as isize) as usize
        } else if index < 0 || index as usize >= len {
            return 0.0;
        } else {
            index as usize
        };
        data[i * channels + channel]
    };

    for f in 0..frames {
        if !looping && v.pos >= len_f {
            done = true;
            break;
        }
        let i = v.pos as usize;
        let t = (v.pos - i as f64) as f32;
        let interior = i >= 1 && i + 2 < len;
        let sample = |channel: usize| -> f32 {
            if interior {
                let at = |k: usize| data[k * channels + channel];
                hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t)
            } else {
                let i = i as isize;
                hermite(fetch(i - 1, channel), fetch(i, channel), fetch(i + 1, channel), fetch(i + 2, channel), t)
            }
        };
        let (left, right) = if channels == 1 {
            let s = sample(0);
            (s, s)
        } else {
            (sample(0), sample(1))
        };
        let gains = [v.gains[0].next(), v.gains[1].next()];
        let env = v.envelope;
        out[2 * f] += left * gains[0] * env;
        out[2 * f + 1] += right * gains[1] * env;

        if v.envelope_step != 0.0 {
            v.envelope = (v.envelope + v.envelope_step).clamp(0.0, 1.0);
            if v.envelope >= 1.0 && v.envelope_step > 0.0 {
                v.envelope_step = 0.0;
            }
            if v.stopping && v.envelope <= 0.0 {
                done = true;
                break;
            }
        }
        v.pos += v.step;
        if looping && v.pos >= len_f {
            v.pos %= len_f;
        }
    }
    done
}

/// Mixes one deck into the music bus buffer.
#[allow(clippy::too_many_arguments)]
fn render_deck(
    index: u8,
    deck: &mut Deck,
    out: &mut [f32],
    scratch: &mut [[f32; 2]],
    events: &mut rtrb::Producer<Event>,
    blocking: bool,
    prime_frames: usize,
    stats: &MixerStats,
) {
    // Deliver an end that could not be sent earlier.
    if let Some(natural) = deck.pending_end {
        if let Some(stream) = deck.stream.take() {
            match events.push(Event::DeckEnded { deck: index, stream, natural }) {
                Ok(()) => deck.pending_end = None,
                Err(rtrb::PushError::Full(Event::DeckEnded { stream, .. })) => deck.stream = Some(stream),
                Err(_) => {}
            }
        } else {
            deck.pending_end = None;
        }
        return;
    }
    let Some(stream) = deck.stream.as_mut() else { return };
    let frames = scratch.len();
    let finished = stream.shared.finished.load(Ordering::Acquire);

    if deck.priming {
        let fading_away = deck.release_at_target && deck.target == 0.0;
        if !blocking && stream.consumer.slots() < prime_frames && !finished && !fading_away {
            return;
        }
        deck.priming = false;
        if fading_away {
            // Released before it was ever heard.
            deck.gain = 0.0;
        }
    }

    let (got, _) = stream.consumer.pop_partial_slice(scratch);
    let mut available = got.len();
    if blocking {
        // Offline: wait for the decoder (bounded, in case it died without finishing).
        let mut waited = 0;
        while available < frames && !stream.shared.finished.load(Ordering::Acquire) && waited < 10_000 {
            std::thread::sleep(std::time::Duration::from_millis(1));
            waited += 1;
            let (more, _) = stream.consumer.pop_partial_slice(&mut scratch[available..]);
            available += more.len();
        }
        if available < frames {
            let (more, _) = stream.consumer.pop_partial_slice(&mut scratch[available..]);
            available += more.len();
        }
    }
    let finished = stream.shared.finished.load(Ordering::Acquire);
    let ended_naturally = available < frames && finished && stream.consumer.is_empty();
    if available < frames && !ended_naturally {
        stats.underrun_frames.fetch_add((frames - available) as u64, Ordering::Relaxed);
    }
    stream.shared.played.fetch_add(available as u64, Ordering::Relaxed);

    // The gain ramp advances with time even through an underrun, so a fade-out always ends.
    let mut released = false;
    for f in 0..frames {
        if deck.gain != deck.target {
            if deck.gain < deck.target {
                deck.gain = (deck.gain + deck.step).min(deck.target);
            } else {
                deck.gain = (deck.gain - deck.step).max(deck.target);
            }
        }
        if f < available {
            out[2 * f] += scratch[f][0] * deck.gain;
            out[2 * f + 1] += scratch[f][1] * deck.gain;
        }
        if deck.release_at_target && deck.gain == 0.0 && deck.target == 0.0 {
            released = true;
            break;
        }
    }

    if released || ended_naturally {
        deck.pending_end = Some(ended_naturally && !released);
        let natural = ended_naturally && !released;
        if let Some(stream) = deck.stream.take() {
            match events.push(Event::DeckEnded { deck: index, stream, natural }) {
                Ok(()) => deck.pending_end = None,
                Err(rtrb::PushError::Full(Event::DeckEnded { stream, .. })) => deck.stream = Some(stream),
                Err(_) => {}
            }
        }
        deck.release_at_target = false;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use cpal::Sample;

    pub(crate) struct Harness {
        pub mixer: Mixer,
        pub commands: rtrb::Producer<Command>,
        pub events: rtrb::Consumer<Event>,
    }

    pub(crate) fn harness(rate: u32, slots: usize) -> Harness {
        let (commands, command_rx) = rtrb::RingBuffer::new(256);
        let (event_tx, events) = rtrb::RingBuffer::new(256);
        let mixer = Mixer::new(rate, slots, command_rx, event_tx, Arc::new(MixerStats::new()), true);
        Harness { mixer, commands, events }
    }

    impl Harness {
        pub(crate) fn send(&mut self, c: Command) {
            assert!(self.commands.push(c).is_ok());
        }

        pub(crate) fn render(&mut self, frames: usize) -> Vec<f32> {
            let mut out = vec![0.0f32; frames * 2];
            self.mixer.render(&mut out, 2);
            out
        }
    }

    fn tone(rate: u32, freq: f32, seconds: f32, amp: f32) -> Arc<SoundData> {
        let n = (rate as f32 * seconds) as usize;
        let samples = (0..n).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin()).collect();
        Arc::new(SoundData::mono(rate, samples))
    }

    fn play(sound: &Arc<SoundData>, slot: u16, gains: [f32; 2], out_rate: u32) -> Command {
        Command::Play {
            slot,
            generation: 1,
            sound: sound.clone(),
            bus: MixBus::Effects,
            gains,
            step: sound.sample_rate() as f64 / out_rate as f64,
            looping: false,
            start: 0.0,
            fade_in: 0,
        }
    }

    fn rms(x: impl Iterator<Item = f32>) -> f32 {
        let (s, n) = x.fold((0.0f64, 0usize), |(s, n), v| (s + (v as f64).powi(2), n + 1));
        (s / n.max(1) as f64).sqrt() as f32
    }

    #[test]
    fn resamples_ta_rate_to_device_rate() {
        let mut h = harness(48_000, 4);
        let sound = tone(11_025, 440.0, 0.5, 0.5);
        h.send(play(&sound, 0, [1.0, 0.0], 48_000));
        let out = h.render(48_000);
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        // 0.5 s of sound at 48 kHz, then silence.
        let active = left.iter().rposition(|s| s.abs() > 1e-3).unwrap();
        assert!((active as i64 - 24_000).abs() < 60, "sound ends at frame {active}");
        // Frequency preserved: count rising zero crossings in the first 0.4 s.
        let crossings = left[..19_200].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((crossings as i64 - 176).abs() <= 1, "{crossings} crossings");
        // Level preserved: RMS of a 0.5-amplitude sine.
        let r = rms(left[1000..20_000].iter().copied());
        assert!((r - 0.5 / 2f32.sqrt()).abs() < 0.01, "rms {r}");
        assert!(out.chunks(2).all(|f| f[1] == 0.0), "right gain 0");
        match h.events.pop() {
            Ok(Event::VoiceEnded { slot: 0, generation: 1, .. }) => {}
            _ => panic!("expected the voice to end"),
        }
    }

    #[test]
    fn stop_fades_out_without_clicks() {
        let mut h = harness(48_000, 4);
        let sound = Arc::new(SoundData::mono(48_000, vec![0.5; 48_000]));
        h.send(play(&sound, 1, [1.0, 1.0], 48_000));
        let a = h.render(256);
        assert!((a[510] - 0.5).abs() < 1e-6);
        h.send(Command::Stop { slot: 1, generation: 1, fade: 480 });
        let b = h.render(1024);
        let left: Vec<f32> = b.chunks(2).map(|f| f[0]).collect();
        assert!(left.windows(2).all(|w| w[1] <= w[0] + 1e-6), "monotonic fade");
        assert!(left[479].abs() < 0.01 && left[600] == 0.0);
        assert!(matches!(h.events.pop(), Ok(Event::VoiceEnded { slot: 1, .. })));
        // A stale command for the old generation is ignored.
        h.send(Command::SetGains { slot: 1, generation: 1, gains: [1.0, 1.0] });
        assert!(h.render(64).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn gains_ramp_and_buses_scale() {
        let mut h = harness(48_000, 4);
        let sound = Arc::new(SoundData::mono(48_000, vec![0.5; 96_000]));
        h.send(play(&sound, 0, [0.0, 0.0], 48_000));
        h.send(Command::SetGains { slot: 0, generation: 1, gains: [1.0, 1.0] });
        let out = h.render(512);
        // Linear ramp from 0 to 0.5 over 10 ms, independent of the block size.
        assert!(out[0] < 0.01 && (out[1022] - 0.5).abs() < 0.01 && (out[478] - 0.25).abs() < 0.01);
        h.send(Command::SetBusGain { bus: MixBus::Effects, gain: 0.5 });
        h.send(Command::SetMasterGain(0.5));
        h.render(512);
        let out = h.render(64);
        assert!((out[0] - 0.125).abs() < 1e-5, "{}", out[0]);
    }

    #[test]
    fn loops_wrap_and_limiter_bounds_output() {
        let mut h = harness(48_000, 32);
        let sound = Arc::new(SoundData::mono(48_000, vec![0.9; 100]));
        for slot in 0..32 {
            h.send(Command::Play {
                slot,
                generation: 7,
                sound: sound.clone(),
                bus: MixBus::Effects,
                gains: [1.0, 1.0],
                step: 1.0,
                looping: true,
                start: 0.0,
                fade_in: 0,
            });
        }
        let out = h.render(4800);
        assert!(out.iter().all(|s| s.abs() <= 0.97 + 1e-6), "limited");
        assert!(out[9000].abs() > 0.9, "still playing after 45 loops");
        assert!(h.events.pop().is_err(), "loops never end by themselves");
    }

    #[test]
    fn output_channel_mapping() {
        let mut h = harness(48_000, 1);
        let sound = Arc::new(SoundData::new(48_000, 2, [0.25, -0.5].repeat(1000)).unwrap());
        h.send(play(&sound, 0, [1.0, 1.0], 48_000));
        let mut quad = vec![0.0f32; 400];
        h.mixer.render(&mut quad, 4);
        assert_eq!(&quad[40..44], &[0.25, -0.5, 0.0, 0.0]);
        let mut mono = vec![0i16; 100];
        h.mixer.render(&mut mono, 1);
        assert_eq!(mono[50], i16::from_sample(-0.125f32));
    }

    #[test]
    fn volume_and_pan_ramps_do_not_depend_on_callback_size() {
        fn mix(chunks: &[usize]) -> Vec<f32> {
            let mut h = harness(48_000, 1);
            let sound = Arc::new(SoundData::mono(48_000, vec![0.5; 4000]));
            h.send(play(&sound, 0, [1.0, 0.0], 48_000));
            h.render(100);
            h.send(Command::SetGains { slot: 0, generation: 1, gains: [0.0, 1.0] });
            h.send(Command::SetBusGain { bus: MixBus::Effects, gain: 0.5 });
            h.send(Command::SetMasterGain(0.5));
            chunks.iter().flat_map(|&n| h.render(n)).collect()
        }
        let whole = mix(&[1024]);
        assert_eq!(whole, mix(&[1, 31, 64, 256, 7, 500, 165]));
        assert!(whole[0] > 0.49, "a one-sample callback must not snap to the target");
        assert_eq!(&whole[960..962], &[0.0, 0.125], "10 ms ramp completes on time");
    }

    pub(crate) fn stream(id: u64, value: f32, frames: usize) -> (MusicStream, rtrb::Producer<[f32; 2]>) {
        use std::sync::atomic::AtomicBool;
        let (mut producer, consumer) = rtrb::RingBuffer::new(16384);
        for _ in 0..frames { producer.push([value; 2]).unwrap(); }
        let shared = Arc::new(crate::stream::StreamShared {
            finished: AtomicBool::new(false), failed: AtomicBool::new(false), stop: AtomicBool::new(false),
            played: AtomicU64::new(0), total: AtomicU64::new(u64::MAX),
        });
        (MusicStream { id, consumer, shared }, producer)
    }

    #[test]
    fn crossfade_keeps_the_old_track_until_the_replacement_is_buffered() {
        let mut h = harness(48_000, 1);
        h.mixer.blocking_music = false;
        let (old, _old_producer) = stream(1, 0.2, 12000);
        h.send(Command::DeckStart { deck: 0, stream: old, fade_in: 0, crossfade_from: None });
        h.render(512);
        let (new, mut producer) = stream(2, 0.4, 0);
        h.send(Command::DeckStart { deck: 1, stream: new, fade_in: 480, crossfade_from: Some((0, 1)) });
        assert!(h.render(2048).iter().all(|&s| (s - 0.2).abs() < 1e-6));
        assert!(h.events.is_empty(), "old track survives the decoder delay");
        for _ in 0..8000 { producer.push([0.4; 2]).unwrap(); }
        let out = h.render(1024);
        assert!(out[..960].chunks(2).all(|s| (0.2..=0.40001).contains(&s[0])), "crossfade has no silent gap");
        assert!((out[2046] - 0.4).abs() < 1e-6);
        assert!(matches!(h.events.pop(), Ok(Event::DeckEnded { stream, natural: false, .. }) if stream.id == 1));
    }

    #[test]
    fn failed_replacement_preserves_music_and_stop_releases_a_waiting_crossfade() {
        for failed in [false, true] {
            let mut h = harness(48_000, 1);
            h.mixer.blocking_music = false;
            let (old, _old_producer) = stream(1, 0.2, 12000);
            h.send(Command::DeckStart { deck: 0, stream: old, fade_in: 0, crossfade_from: None });
            h.render(512);
            let (new, _producer) = stream(2, 0.4, 0);
            new.shared.finished.store(failed, Ordering::Release);
            new.shared.failed.store(failed, Ordering::Release);
            h.send(Command::DeckStart { deck: 1, stream: new, fade_in: 480, crossfade_from: Some((0, 1)) });
            assert!(h.render(512).iter().all(|&s| (s - 0.2).abs() < 1e-6));
            if failed {
                assert!(matches!(h.events.pop(), Ok(Event::DeckEnded { stream, .. }) if stream.id == 2));
                assert!(h.render(512).iter().all(|&s| (s - 0.2).abs() < 1e-6));
            } else {
                h.send(Command::DeckFade { deck: 1, stream_id: 2, target: 0.0, frames: 480, release: true });
                h.render(1024);
                assert!(h.render(512).iter().all(|&s| s == 0.0), "stop also fades the held outgoing deck");
            }
        }
    }

    #[test]
    fn offline_crossfade_arms_before_consuming_a_short_delayed_replacement() {
        let mut h = harness(48_000, 1);
        let (old, _producer) = stream(1, 0.2, 12000);
        old.shared.finished.store(true, Ordering::Release);
        h.send(Command::DeckStart { deck: 0, stream: old, fade_in: 0, crossfade_from: None });
        h.render(512);
        let (new, mut producer) = stream(2, 0.4, 0);
        let shared = new.shared.clone();
        h.send(Command::DeckStart { deck: 1, stream: new, fade_in: 480, crossfade_from: Some((0, 1)) });
        let decode = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            for _ in 0..32 { producer.push([0.4; 2]).unwrap(); }
            shared.finished.store(true, Ordering::Release);
        });
        h.render(1024);
        decode.join().unwrap();
        assert!(h.render(512).iter().all(|&s| s == 0.0), "outgoing deck must complete its handover");
    }
}
