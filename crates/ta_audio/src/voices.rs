//! Game-thread voice bookkeeping: which sounds are playing, where, how loud, and which of them
//! occupy one of the mixer's limited voices.
//!
//! * **Culling**: a one-shot that would be inaudible where it starts (-60 dB) is not played.
//! * **Anti-spam**: a request for the same sound on the same bus, within `merge_window` seconds
//!   of an existing voice and within `merge_distance` world units of it (or both
//!   non-positional), is merged into that voice: 20 lasers firing in one tick make one laser
//!   sound, a little louder (power sum, capped at +3.5 dB over the loudest request).
//! * **Voice limit**: at most `max_voices` voices are audible. Each voice has a score:
//!   priority class (Low 0, Normal 1, High 2 — UI and voices are High) plus its audibility
//!   (volume × distance gain), slightly reduced as a one-shot nears its end. A new sound that
//!   outscores the weakest playing voice steals its slot (the loser fades out in 10 ms);
//!   [`Priority::Critical`] voices are never stolen.
//! * **Virtual loops**: looping sounds that lose their voice, or drift out of earshot, keep
//!   running silently and resume at the right phase when they become audible again.
//! * **Log**: on request, every command that starts, changes or stops a voice is also recorded
//!   as a [`PlayEvent`], with the sound's path and kind.

use std::sync::Arc;

use crate::decode::SoundData;
use crate::engine::{Bus, PlayParams, Priority, SoundHandle};
use crate::mixer::{Command, MixBus};
use crate::playlog::{PlayEvent, PlayStart};
use crate::spatial::{Listener, SpatialSettings, pan_gains, spatialize};

/// Destination of mixer commands: the real-time queue, or a `Vec` in tests.
pub(crate) trait CommandSink {
    /// Queues a command; gives it back if the queue is full.
    fn send(&mut self, command: Command) -> Result<(), Command>;
}

impl CommandSink for rtrb::Producer<Command> {
    fn send(&mut self, command: Command) -> Result<(), Command> {
        self.push(command).map_err(|rtrb::PushError::Full(c)| c)
    }
}

impl CommandSink for Vec<Command> {
    fn send(&mut self, command: Command) -> Result<(), Command> {
        self.push(command);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VoiceConfig {
    pub max_voices: usize,
    pub merge_window: f64,
    pub merge_distance: f32,
    pub output_rate: u32,
}

/// Gain below which a sound counts as inaudible (-60 dB).
pub(crate) const CULL_GAIN: f32 = 0.001;
/// Fade applied when a voice is stopped or stolen.
const STOP_FADE_SECONDS: f64 = 0.010;
/// Fade-in for loops that become audible again.
const RESUME_FADE_SECONDS: f64 = 0.030;
/// Merged requests may get this much louder than the loudest single request.
const MERGE_CAP: f32 = 1.5;
/// Score margin a virtual loop needs over a playing voice to take over its slot.
const STEAL_HYSTERESIS: f32 = 0.1;
/// Smallest gain change worth a mixer command.
const GAIN_EPSILON: f32 = 0.002;

/// Counters for diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceStats {
    /// Requests folded into an existing voice by the anti-spam rule.
    pub merged: u64,
    /// Voices that lost their slot to a higher-scoring sound.
    pub stolen: u64,
    /// One-shots not started because they were inaudible.
    pub culled: u64,
    /// One-shots not started because every voice was busy with more important sounds.
    pub rejected: u64,
    /// Loops that went silent (out of range or out-scored) but keep running.
    pub virtualized: u64,
    /// Loops that became audible again.
    pub resumed: u64,
}

struct LogicalVoice {
    generation: u32,
    /// Normalised VFS path of the sound (the merge key).
    name: Box<str>,
    sound: Arc<SoundData>,
    params: PlayParams,
    /// Loudest single request and the power sum of all merged requests.
    peak_request: f32,
    energy: f32,
    start: f64,
    /// Seconds per pass through the sound at the current pitch.
    period: f64,
    slot: Option<u16>,
    slot_generation: u32,
    sent_gains: [f32; 2],
    /// Last pitch accepted by the mixer, so a full command queue cannot lose a change.
    sent_pitch: f32,
    spatial_gain: f32,
    pan: f32,
    /// The log's id of its playing on the mixer voice it holds (when logging).
    play_id: u64,
}

impl LogicalVoice {
    fn audibility(&self) -> f32 {
        (self.params.volume * self.spatial_gain).clamp(0.0, 1.0)
    }

    /// Its level before the pan law.
    fn level(&self) -> f32 {
        self.params.volume * self.spatial_gain
    }

    fn score(&self, clock: f64) -> f32 {
        let base = match self.params.priority {
            Priority::Low => 0.0,
            Priority::Normal => 1.0,
            Priority::High => 2.0,
            Priority::Critical => 1000.0,
        };
        let progress = if self.params.looping || self.period <= 0.0 {
            0.0
        } else {
            ((clock - self.start) / self.period).clamp(0.0, 1.0) as f32
        };
        base + self.audibility() * (1.0 - 0.25 * progress)
    }

    fn gains(&self) -> [f32; 2] {
        let [l, r] = pan_gains(self.pan, self.sound.channels());
        let g = self.level();
        [l * g, r * g]
    }

    fn update_spatial(&mut self, listener: &Listener, settings: &SpatialSettings) {
        (self.spatial_gain, self.pan) = match self.params.position {
            Some(p) => {
                let s = spatialize(listener, settings, p);
                (s.gain, s.pan)
            }
            None => (1.0, 0.0),
        };
    }

    fn mix_bus(&self) -> MixBus {
        match self.params.bus {
            Bus::Voice => MixBus::Voice,
            Bus::Music => MixBus::Music,
            Bus::Effects | Bus::Master => MixBus::Effects,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Free,
    Playing { voice: u32 },
    /// Still owned by the mixer: retry the stop when its command queue has room.
    PendingStop { play_id: u64 },
    /// Fading out after a stop/steal; free once the mixer reports the end.
    Releasing,
}

pub(crate) struct VoiceManager {
    config: VoiceConfig,
    voices: Vec<Option<LogicalVoice>>,
    generations: Vec<u32>,
    free_voices: Vec<u32>,
    /// Mixer slots: state and the generation of the voice last started there.
    slots: Vec<(SlotState, u32)>,
    pending_stops: usize,
    /// Voices currently occupying a slot as `Playing`.
    real: usize,
    clock: f64,
    stats: VoiceStats,
    candidates: Vec<(u32, f32)>,
    /// What the mixer was told since it was last taken, when logging.
    log: Option<Vec<PlayEvent>>,
    /// The last id given to a playing in the log.
    last_play_id: u64,
}

fn sanitize(x: f32, default: f32) -> f32 {
    if x.is_finite() { x.max(0.0) } else { default }
}

impl VoiceManager {
    /// Mixer slots needed for a voice limit: the limit plus headroom so stolen voices can fade
    /// out while their replacements already play.
    pub(crate) fn slots_for(max_voices: usize) -> usize {
        let max_voices = max_voices.clamp(1, 1024);
        max_voices + (max_voices / 2).max(8)
    }

    pub(crate) fn new(config: VoiceConfig) -> VoiceManager {
        let config = VoiceConfig { max_voices: config.max_voices.clamp(1, 1024), ..config };
        VoiceManager {
            config,
            voices: Vec::new(),
            generations: Vec::new(),
            free_voices: Vec::new(),
            slots: vec![(SlotState::Free, 0); Self::slots_for(config.max_voices)],
            pending_stops: 0,
            real: 0,
            clock: 0.0,
            stats: VoiceStats::default(),
            candidates: Vec::new(),
            log: None,
            last_play_id: 0,
        }
    }

    pub(crate) fn set_logging(&mut self, on: bool) {
        if on != self.log.is_some() {
            self.log = on.then(Vec::new);
        }
    }

    pub(crate) fn take_log(&mut self) -> Vec<PlayEvent> {
        self.log.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Mixer slots the manager may use (voice limit plus fade-out headroom).
    #[cfg(test)]
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn output_rate(&self) -> u32 {
        self.config.output_rate
    }

    pub(crate) fn stats(&self) -> VoiceStats {
        self.stats
    }

    /// (sounds tracked, sounds audible).
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.voices.iter().flatten().count(), self.real)
    }

    fn voice(&self, handle: SoundHandle) -> Option<&LogicalVoice> {
        self.voices.get(handle.index as usize)?.as_ref().filter(|v| v.generation == handle.generation)
    }

    fn voice_index(&self, handle: SoundHandle) -> Option<usize> {
        self.voice(handle).map(|_| handle.index as usize)
    }

    fn handle(&self, index: usize) -> SoundHandle {
        SoundHandle { index: index as u32, generation: self.generations[index] }
    }

    pub(crate) fn is_playing(&self, handle: SoundHandle) -> bool {
        self.voice(handle).is_some()
    }

    /// Whether a sound is audible (holds a mixer voice) rather than culled or virtual.
    pub(crate) fn is_audible(&self, handle: SoundHandle) -> bool {
        self.voice(handle).is_some_and(|v| v.slot.is_some())
    }

    /// Starts (or merges) a sound. `name` must be the normalised path, used as merge key.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn play(
        &mut self,
        name: &str,
        sound: Arc<SoundData>,
        params: PlayParams,
        listener: &Listener,
        spatial: &SpatialSettings,
        sink: &mut dyn CommandSink,
    ) -> SoundHandle {
        let mut params = params;
        params.volume = sanitize(params.volume, 1.0);
        params.pitch = sanitize(params.pitch, 1.0).clamp(0.05, 8.0);
        if sound.frames() == 0 {
            return SoundHandle::NONE;
        }
        let mut voice = LogicalVoice {
            generation: 0,
            name: name.into(),
            period: sound.duration() / params.pitch as f64,
            sound,
            params,
            peak_request: params.volume,
            energy: params.volume * params.volume,
            start: self.clock,
            slot: None,
            slot_generation: 0,
            sent_gains: [0.0; 2],
            sent_pitch: params.pitch,
            spatial_gain: 1.0,
            pan: 0.0,
            play_id: 0,
        };
        voice.update_spatial(listener, spatial);
        if !params.looping && voice.audibility() < CULL_GAIN {
            self.stats.culled += 1;
            return SoundHandle::NONE;
        }
        if params.merge && !params.looping && self.config.merge_window > 0.0 && let Some(index) = self.find_merge(&voice) {
            let v = self.voices[index].as_mut().expect("merge target exists");
            v.energy += params.volume * params.volume;
            v.peak_request = v.peak_request.max(params.volume);
            v.params.volume = v.energy.sqrt().min(v.peak_request * MERGE_CAP);
            self.stats.merged += 1;
            self.push_gains(index, sink);
            return self.handle(index);
        }
        let index = self.alloc(voice);
        if self.try_start(index, 0.0, false, sink) {
            return self.handle(index);
        }
        if params.looping {
            self.stats.virtualized += 1;
            self.handle(index)
        } else {
            self.stats.rejected += 1;
            self.free(index);
            SoundHandle::NONE
        }
    }

    fn find_merge(&self, new: &LogicalVoice) -> Option<usize> {
        let window = self.config.merge_window;
        let max_dist_sq = self.config.merge_distance * self.config.merge_distance;
        self.voices.iter().position(|v| {
            let Some(v) = v else { return false };
            if v.params.looping || !v.params.merge || v.params.bus != new.params.bus || self.clock - v.start > window {
                return false;
            }
            if *v.name != *new.name {
                return false;
            }
            match (v.params.position, new.params.position) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= max_dist_sq
                }
                _ => false,
            }
        })
    }

    fn alloc(&mut self, mut voice: LogicalVoice) -> usize {
        let index = match self.free_voices.pop() {
            Some(i) => i as usize,
            None => {
                self.voices.push(None);
                self.generations.push(0);
                self.voices.len() - 1
            }
        };
        voice.generation = self.generations[index];
        self.voices[index] = Some(voice);
        index
    }

    fn free(&mut self, index: usize) {
        if self.voices[index].take().is_some() {
            self.generations[index] = self.generations[index].wrapping_add(1);
            self.free_voices.push(index as u32);
        }
    }

    /// The playing voice with the lowest score (Critical voices are never candidates).
    fn weakest_real(&self) -> Option<(usize, f32)> {
        self.slots
            .iter()
            .filter_map(|(state, _)| match state {
                SlotState::Playing { voice } => Some(*voice as usize),
                _ => None,
            })
            .filter_map(|i| self.voices[i].as_ref().map(|v| (i, v)))
            .filter(|(_, v)| v.params.priority != Priority::Critical)
            .map(|(i, v)| (i, v.score(self.clock)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|(state, _)| *state == SlotState::Free)
    }

    /// Gives voice `index` a mixer slot, stealing one from a weaker voice if the limit is
    /// reached and the voice outscores it by `margin`. `resume` starts loops at their phase.
    fn try_start(&mut self, index: usize, margin: f32, resume: bool, sink: &mut dyn CommandSink) -> bool {
        // In particular, do not resume a virtual loop before its old slot has been told
        // to stop. Failed stops take priority over new playback.
        if !self.retry_stops(sink) {
            return false;
        }
        let Some(slot) = self.free_slot() else { return false };
        if self.real >= self.config.max_voices {
            let score = self.voices[index].as_ref().map_or(0.0, |v| v.score(self.clock));
            match self.weakest_real() {
                Some((weak, weak_score)) if score > weak_score + margin => {
                    self.release(weak, sink);
                    self.stats.stolen += 1;
                    if !self.voices[weak].as_ref().is_some_and(|v| v.params.looping) {
                        self.free(weak);
                    }
                }
                _ => return false,
            }
        }
        if self.pending_stops != 0 {
            return false;
        }
        let rate = self.config.output_rate as f64;
        let clock = self.clock;
        let slot_generation = self.slots[slot].1.wrapping_add(1);
        let v = self.voices[index].as_mut().expect("voice exists");
        let frames = v.sound.frames() as f64;
        let start = if resume && v.params.looping && v.period > 0.0 {
            ((clock - v.start) / v.period).fract() * frames
        } else {
            0.0
        };
        let gains = v.gains();
        let command = Command::Play {
            slot: slot as u16,
            generation: slot_generation,
            sound: v.sound.clone(),
            bus: v.mix_bus(),
            gains,
            step: v.sound.sample_rate() as f64 / rate * v.params.pitch as f64,
            looping: v.params.looping,
            start,
            fade_in: if resume { (RESUME_FADE_SECONDS * rate) as u32 } else { 0 },
        };
        if sink.send(command).is_err() {
            log::warn!("audio command queue full; dropping sound");
            return false;
        }
        v.slot = Some(slot as u16);
        v.slot_generation = slot_generation;
        v.sent_gains = gains;
        v.sent_pitch = v.params.pitch;
        if let Some(log) = self.log.as_mut() {
            self.last_play_id += 1;
            v.play_id = self.last_play_id;
            log.push(PlayEvent::Start(PlayStart {
                id: v.play_id,
                path: v.name.to_string(),
                sound: v.sound.clone(),
                kind: v.params.kind,
                bus: v.params.bus,
                gain: v.level(),
                pan: v.pan,
                gains,
                pitch: v.params.pitch,
                looping: v.params.looping,
                offset: start / v.sound.sample_rate() as f64,
                fade_in: if resume { RESUME_FADE_SECONDS as f32 } else { 0.0 },
                position: v.params.position,
            }));
        }
        self.slots[slot] = (SlotState::Playing { voice: index as u32 }, slot_generation);
        self.real += 1;
        true
    }

    /// Takes voice `index` off its mixer slot (fading out). The logical voice stays.
    fn release(&mut self, index: usize, sink: &mut dyn CommandSink) {
        let Some(v) = self.voices[index].as_mut() else { return };
        let Some(slot) = v.slot.take() else { return };
        let generation = v.slot_generation;
        // The logical voice may now be freed or virtualized, but the slot retains its
        // stop request until accepted, even if the logical table entry is reused.
        self.slots[slot as usize] = (SlotState::PendingStop { play_id: v.play_id }, generation);
        self.pending_stops += 1;
        self.real -= 1;
        self.send_stop(slot as usize, sink);
    }

    fn send_stop(&mut self, slot: usize, sink: &mut dyn CommandSink) -> bool {
        let (SlotState::PendingStop { play_id }, generation) = self.slots[slot] else { return true };
        let fade = (STOP_FADE_SECONDS * self.config.output_rate as f64) as u32;
        if sink.send(Command::Stop { slot: slot as u16, generation, fade }).is_err() {
            return false;
        }
        self.slots[slot].0 = SlotState::Releasing;
        self.pending_stops -= 1;
        if let Some(log) = self.log.as_mut() {
            log.push(PlayEvent::Stop { id: play_id, fade: STOP_FADE_SECONDS as f32 });
        }
        true
    }

    fn retry_stops(&mut self, sink: &mut dyn CommandSink) -> bool {
        if self.pending_stops != 0 {
            for slot in 0..self.slots.len() {
                if !self.send_stop(slot, sink) {
                    return false;
                }
            }
        }
        true
    }

    fn push_gains(&mut self, index: usize, sink: &mut dyn CommandSink) {
        let Some(v) = self.voices[index].as_mut() else { return };
        let Some(slot) = v.slot else { return };
        let gains = v.gains();
        if ((gains[0] - v.sent_gains[0]).abs() > GAIN_EPSILON || (gains[1] - v.sent_gains[1]).abs() > GAIN_EPSILON) && sink.send(Command::SetGains { slot, generation: v.slot_generation, gains }).is_ok() {
            v.sent_gains = gains;
            if let Some(log) = self.log.as_mut() {
                log.push(PlayEvent::Gains { id: v.play_id, gain: v.level(), pan: v.pan, gains });
            }
        }
    }

    fn push_pitch(&mut self, index: usize, sink: &mut dyn CommandSink) {
        let Some(v) = self.voices[index].as_mut() else { return };
        let Some(slot) = v.slot else { return };
        let pitch = v.params.pitch;
        if pitch == v.sent_pitch { return; }
        let step = v.sound.sample_rate() as f64 / self.config.output_rate as f64 * pitch as f64;
        if sink.send(Command::SetStep { slot, generation: v.slot_generation, step }).is_ok() {
            v.sent_pitch = pitch;
            if let Some(log) = self.log.as_mut() {
                log.push(PlayEvent::Pitch { id: v.play_id, pitch });
            }
        }
    }

    /// The mixer finished a voice (end of a one-shot, or a completed fade-out).
    pub(crate) fn on_voice_ended(&mut self, slot: u16, generation: u32) {
        let Some(&(state, slot_generation)) = self.slots.get(slot as usize) else { return };
        if slot_generation != generation {
            return;
        }
        if matches!(state, SlotState::PendingStop { .. }) {
            self.pending_stops -= 1;
        }
        self.slots[slot as usize] = (SlotState::Free, slot_generation);
        if let SlotState::Playing { voice } = state {
            self.real -= 1;
            let index = voice as usize;
            if self.voices[index].as_ref().is_some_and(|v| v.slot == Some(slot)) {
                self.free(index);
            }
        }
    }

    /// Advances time, re-spatialises every sound for the current listener, parks inaudible
    /// loops and brings back loops that became audible.
    pub(crate) fn update(
        &mut self,
        dt: f64,
        listener: &Listener,
        spatial: &SpatialSettings,
        sink: &mut dyn CommandSink,
    ) {
        self.clock += dt.max(0.0);
        self.retry_stops(sink);
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.clear();
        for index in 0..self.voices.len() {
            let clock = self.clock;
            let Some(v) = self.voices[index].as_mut() else { continue };
            v.update_spatial(listener, spatial);
            let audible = v.audibility() >= CULL_GAIN;
            let score = v.score(clock);
            if v.slot.is_some() {
                if v.params.looping && !audible {
                    self.release(index, sink);
                    self.stats.virtualized += 1;
                } else {
                    self.push_gains(index, sink);
                    self.push_pitch(index, sink);
                }
            } else if audible {
                candidates.push((index as u32, score));
            }
        }
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        for &(index, _) in &candidates {
            if !self.try_start(index as usize, STEAL_HYSTERESIS, true, sink) {
                break;
            }
            self.stats.resumed += 1;
        }
        self.candidates = candidates;
    }

    pub(crate) fn stop(&mut self, handle: SoundHandle, sink: &mut dyn CommandSink) {
        if let Some(index) = self.voice_index(handle) {
            self.release(index, sink);
            self.free(index);
        }
    }

    pub(crate) fn stop_all(&mut self, sink: &mut dyn CommandSink) {
        for index in 0..self.voices.len() {
            if self.voices[index].is_some() {
                self.release(index, sink);
                self.free(index);
            }
        }
    }

    /// Stops the sounds of `kind` only.
    pub(crate) fn stop_kind(&mut self, kind: crate::playlog::SoundKind, sink: &mut dyn CommandSink) {
        for index in 0..self.voices.len() {
            if self.voices[index].as_ref().is_some_and(|v| v.params.kind == kind) {
                self.release(index, sink);
                self.free(index);
            }
        }
    }

    pub(crate) fn set_position(&mut self, handle: SoundHandle, position: Option<[f32; 3]>) {
        if let Some(index) = self.voice_index(handle) {
            self.voices[index].as_mut().expect("valid").params.position = position;
        }
    }

    pub(crate) fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        if let Some(index) = self.voice_index(handle) {
            let v = self.voices[index].as_mut().expect("valid");
            let volume = sanitize(volume, v.params.volume);
            v.params.volume = volume;
            v.peak_request = volume;
            v.energy = volume * volume;
        }
    }

    pub(crate) fn set_pitch(&mut self, handle: SoundHandle, pitch: f32, sink: &mut dyn CommandSink) {
        if let Some(index) = self.voice_index(handle) {
            let v = self.voices[index].as_mut().expect("valid");
            let pitch = sanitize(pitch, v.params.pitch).clamp(0.05, 8.0);
            // Keep the loop phase continuous for virtual voices.
            let phase = (self.clock - v.start) / v.period.max(1e-9);
            v.params.pitch = pitch;
            v.period = v.sound.duration() / pitch as f64;
            v.start = self.clock - phase * v.period;
            self.push_pitch(index, sink);
        }
    }

    /// Forgets every mixer slot (after the output device was re-opened with a fresh mixer).
    /// One-shots end; loops become virtual and resume on the next update.
    pub(crate) fn reset_output(&mut self, output_rate: u32) {
        self.config.output_rate = output_rate;
        for slot in &mut self.slots {
            slot.0 = SlotState::Free;
        }
        self.real = 0;
        self.pending_stops = 0;
        for index in 0..self.voices.len() {
            let Some(v) = self.voices[index].as_mut() else { continue };
            v.slot = None;
            if !v.params.looping {
                self.free(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn manager(max_voices: usize) -> VoiceManager {
        VoiceManager::new(VoiceConfig { max_voices, merge_window: 0.06, merge_distance: 96.0, output_rate: RATE })
    }

    fn sound(seconds: f32) -> Arc<SoundData> {
        Arc::new(SoundData::mono(11_025, vec![0.5; (11_025.0 * seconds) as usize]))
    }

    fn listener() -> Listener {
        Listener::new([1000.0, 0.0, 1000.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0)
    }

    fn plays(commands: &[Command]) -> usize {
        commands.iter().filter(|c| matches!(c, Command::Play { .. })).count()
    }

    fn stops(commands: &[Command]) -> usize {
        commands.iter().filter(|c| matches!(c, Command::Stop { .. })).count()
    }

    fn play(m: &mut VoiceManager, name: &str, p: PlayParams, cmds: &mut Vec<Command>) -> SoundHandle {
        m.play(name, sound(1.0), p, &listener(), &SpatialSettings::default(), cmds)
    }

    #[test]
    fn merges_identical_sounds_started_together_nearby() {
        let mut m = manager(32);
        let mut cmds = Vec::new();
        let a = play(&mut m, "sounds/lasrfir3.wav", PlayParams::at([1000.0, 0.0, 1000.0], 1.0), &mut cmds);
        for i in 0..19 {
            let p = PlayParams::at([1000.0 + i as f32 * 3.0, 0.0, 1000.0], 1.0);
            let h = play(&mut m, "sounds/lasrfir3.wav", p, &mut cmds);
            assert_eq!(h, a, "request {i} merged");
        }
        assert_eq!(plays(&cmds), 1);
        assert_eq!(m.stats().merged, 19);
        // Louder than one, capped.
        assert!((m.voice(a).unwrap().params.volume - 1.5).abs() < 1e-6);

        // Far away (> merge distance), different sound, or different bus: separate voices.
        let far = play(&mut m, "sounds/lasrfir3.wav", PlayParams::at([1400.0, 0.0, 1000.0], 1.0), &mut cmds);
        let other = play(&mut m, "sounds/lasrhit2.wav", PlayParams::at([1000.0, 0.0, 1000.0], 1.0), &mut cmds);
        let ui = play(&mut m, "sounds/lasrfir3.wav", PlayParams::ui(1.0), &mut cmds);
        assert!(far != a && other != a && ui != a);
        assert_eq!(plays(&cmds), 4);

        // After the window has passed, the same request starts a new voice.
        m.update(0.07, &listener(), &SpatialSettings::default(), &mut cmds);
        let later = play(&mut m, "sounds/lasrfir3.wav", PlayParams::at([1000.0, 0.0, 1000.0], 1.0), &mut cmds);
        assert!(later != a);
        assert_eq!(plays(&cmds), 5);
    }

    #[test]
    fn merging_can_be_disabled_per_request() {
        let mut m = manager(8);
        let mut cmds = Vec::new();
        let p = PlayParams { merge: false, ..PlayParams::ui(1.0) };
        let a = play(&mut m, "sounds/beep6.wav", p, &mut cmds);
        let b = play(&mut m, "sounds/beep6.wav", p, &mut cmds);
        assert_ne!(a, b);
        assert_eq!(plays(&cmds), 2);
    }

    #[test]
    fn culls_inaudible_one_shots() {
        let mut m = manager(8);
        let mut cmds = Vec::new();
        let h = play(&mut m, "sounds/xplolrg1.wav", PlayParams::at([5000.0, 0.0, 5000.0], 1.0), &mut cmds);
        assert!(h.is_none());
        assert!(cmds.is_empty());
        assert_eq!(m.stats().culled, 1);
        assert!(play(&mut m, "sounds/xplolrg1.wav", PlayParams::ui(0.0), &mut cmds).is_none());
    }

    #[test]
    fn voice_limit_prefers_ui_then_near_then_far() {
        let mut m = manager(4);
        let mut cmds = Vec::new();
        let s = SpatialSettings::default();
        let l = listener();
        // Fill the budget with far effects (at 1.4 half-widths: about -21 dB).
        let far: Vec<SoundHandle> = (0..4)
            .map(|i| {
                let p = PlayParams { merge: false, ..PlayParams::at([1000.0 + 900.0, 0.0, 1000.0 + i as f32], 1.0) };
                m.play(&format!("sounds/far{i}.wav"), sound(1.0), p, &l, &s, &mut cmds)
            })
            .collect();
        assert!(far.iter().all(|h| m.is_audible(*h)));
        assert_eq!(m.counts(), (4, 4));

        // A nearby explosion steals from a far one.
        let near = m.play("sounds/near.wav", sound(1.0), PlayParams::at([1000.0, 0.0, 1000.0], 1.0), &l, &s, &mut cmds);
        assert!(m.is_audible(near));
        assert_eq!(m.stats().stolen, 1);
        assert_eq!(stops(&cmds), 1);
        assert_eq!(far.iter().filter(|h| m.is_playing(**h)).count(), 3, "stolen one-shot is gone");

        // An even farther effect cannot displace anything.
        let p = PlayParams::at([1950.0, 0.0, 1000.0], 1.0);
        let rejected = m.play("sounds/far9.wav", sound(1.0), p, &l, &s, &mut cmds);
        assert!(rejected.is_none());
        assert_eq!(m.stats().rejected, 1);

        // UI sounds beat any effect, even the nearby one.
        let ui: Vec<SoundHandle> = (0..4)
            .map(|i| m.play(&format!("sounds/ui{i}.wav"), sound(1.0), PlayParams::ui(0.3), &l, &s, &mut cmds))
            .collect();
        assert!(ui.iter().all(|h| m.is_audible(*h)));
        assert!(!m.is_playing(near), "near effect stolen last");
        assert_eq!(m.counts().1, 4);
        assert!(m.slot_count() >= 4 + 8);
    }

    #[test]
    fn critical_sounds_are_never_stolen() {
        let mut m = manager(1);
        let mut cmds = Vec::new();
        let critical = PlayParams { priority: Priority::Critical, ..PlayParams::voice(0.1) };
        let brief = play(&mut m, "sounds/brief.wav", critical, &mut cmds);
        let ui = play(&mut m, "sounds/click.wav", PlayParams::ui(1.0), &mut cmds);
        assert!(m.is_audible(brief));
        assert!(ui.is_none());
    }

    #[test]
    fn slots_are_recycled_after_end_events() {
        let mut m = manager(2);
        let mut cmds = Vec::new();
        let a = play(&mut m, "sounds/a.wav", PlayParams::ui(1.0), &mut cmds);
        let Command::Play { slot, generation, .. } = cmds[0] else { panic!() };
        m.on_voice_ended(slot, generation);
        assert!(!m.is_playing(a));
        assert_eq!(m.counts(), (0, 0));
        // Stale end events are ignored.
        let b = play(&mut m, "sounds/b.wav", PlayParams::ui(1.0), &mut cmds);
        m.on_voice_ended(slot, generation);
        assert!(m.is_playing(b));
        // Handles of ended voices do not alias new voices in the same table entry.
        assert_eq!(a.index, b.index);
        assert_ne!(a, b);
    }

    #[test]
    fn a_natural_end_cancels_a_pending_stop_before_slot_reuse() {
        let mut m = manager(2);
        let mut commands = Vec::new();
        m.set_logging(true);
        let shot = play(&mut m, "sounds/shot.wav", PlayParams::ui(1.0), &mut commands);
        let Command::Play { slot, generation, .. } = commands[0] else { panic!() };
        m.take_log();
        let (mut sink, mut queue) = rtrb::RingBuffer::new(1);
        assert!(sink.send(Command::SetMasterGain(1.0)).is_ok());
        m.stop(shot, &mut sink);
        assert_eq!(m.pending_stops, 1);
        // The one-shot reaches its end before the game thread can send Stop.
        m.on_voice_ended(slot, generation);
        m.on_voice_ended(slot, generation); // duplicate notifications are harmless
        assert_eq!(m.pending_stops, 0);
        assert_eq!(m.counts(), (0, 0));
        queue.pop().unwrap();
        m.update(0.1, &listener(), &SpatialSettings::default(), &mut sink);
        assert!(queue.pop().is_err(), "no stale stop is sent");
        assert!(m.take_log().is_empty());

        commands.clear();
        let next = play(&mut m, "sounds/next.wav", PlayParams::ui(1.0), &mut commands);
        assert!(matches!(commands[0], Command::Play { slot: new_slot, generation: new_generation, .. } if new_slot == slot && new_generation != generation));
        m.on_voice_ended(slot, generation);
        assert!(m.is_audible(next));
    }

    #[test]
    fn output_reset_discards_pending_stops_and_only_resumes_live_loops() {
        let mut m = manager(2);
        let mut commands = Vec::new();
        let stopped = play(&mut m, "sounds/stopped.wav", PlayParams::ui(1.0).looping(true), &mut commands);
        let parked = play(&mut m, "sounds/parked.wav", PlayParams::at(listener().position, 1.0).looping(true), &mut commands);
        let (mut sink, _queue) = rtrb::RingBuffer::new(1);
        assert!(sink.send(Command::SetMasterGain(1.0)).is_ok());
        m.stop(stopped, &mut sink);
        let mut far = listener();
        far.position = [9000.0, 0.0, 9000.0];
        m.update(0.5, &far, &SpatialSettings::default(), &mut sink);
        assert_eq!(m.pending_stops, 2);

        m.reset_output(RATE);
        assert_eq!(m.pending_stops, 0);
        commands.clear();
        m.update(0.5, &listener(), &SpatialSettings::default(), &mut commands);
        assert_eq!((plays(&commands), stops(&commands)), (1, 0));
        assert!(!m.is_playing(stopped));
        assert!(m.is_audible(parked));
        assert_eq!(m.counts(), (1, 1));
    }

    #[test]
    fn a_stolen_loop_retains_its_stop_while_the_command_queue_is_full() {
        let mut m = manager(1);
        let mut commands = Vec::new();
        m.set_logging(true);
        let old = play(&mut m, "sounds/old.wav", PlayParams::ui(0.1).looping(true), &mut commands);
        let Command::Play { slot, generation, .. } = commands[0] else { panic!() };
        let log = m.take_log();
        let [PlayEvent::Start(first)] = &log[..] else { panic!() };
        let (mut sink, mut queue) = rtrb::RingBuffer::new(1);
        assert!(sink.send(Command::SetMasterGain(1.0)).is_ok());
        let next = m.play("sounds/next.wav", sound(1.0), PlayParams::ui(1.0).looping(true), &listener(), &SpatialSettings::default(), &mut sink);
        assert!(m.is_playing(old) && !m.is_audible(old));
        assert!(m.is_playing(next) && !m.is_audible(next));
        assert_eq!((m.stats().stolen, m.pending_stops), (1, 1));
        m.stop(old, &mut sink); // the pending stop outlives its logical voice
        assert_eq!(m.pending_stops, 1);
        queue.pop().unwrap();
        m.update(0.1, &listener(), &SpatialSettings::default(), &mut sink);
        assert!(matches!(queue.pop().unwrap(), Command::Stop { slot: s, generation: g, .. } if s == slot && g == generation));
        assert!(!m.is_audible(next), "the stop took the queue's only available entry");
        assert!(matches!(&m.take_log()[..], [PlayEvent::Stop { id, .. }] if *id == first.id));
        m.on_voice_ended(slot, generation);
        m.update(0.1, &listener(), &SpatialSettings::default(), &mut sink);
        assert!(matches!(queue.pop().unwrap(), Command::Play { .. }));
        assert!(m.is_audible(next));
        assert_eq!((m.counts(), m.pending_stops), ((1, 1), 0));
    }

    #[test]
    fn loops_go_virtual_out_of_range_and_resume_in_phase() {
        let mut m = manager(4);
        let mut cmds = Vec::new();
        let s = SpatialSettings::default();
        let mut l = listener();
        let p = PlayParams::at([1000.0, 0.0, 1000.0], 1.0).looping(true);
        let hum = m.play("sounds/hum.wav", sound(2.0), p, &l, &s, &mut cmds);
        assert!(m.is_audible(hum));
        // Scroll far away: the loop is parked.
        l.position = [9000.0, 0.0, 9000.0];
        m.update(0.5, &l, &s, &mut cmds);
        assert!(m.is_playing(hum) && !m.is_audible(hum));
        assert_eq!(stops(&cmds), 1);
        // Scroll back 1 s later: it resumes 1.5 s into its 2 s period, with a fade-in.
        l.position = [1000.0, 0.0, 1000.0];
        cmds.clear();
        m.update(1.0, &l, &s, &mut cmds);
        assert!(m.is_audible(hum));
        match &cmds[..] {
            [Command::Play { start, fade_in, looping: true, .. }] => {
                assert!((start - 1.5 * 11_025.0).abs() < 1.0, "start {start}");
                assert!(*fade_in > 0);
            }
            _ => panic!("expected one resume"),
        }
        assert_eq!(m.stats().resumed, 1);
    }

    #[test]
    fn output_reset_keeps_loops_and_ends_one_shots() {
        let mut m = manager(4);
        let mut cmds = Vec::new();
        let s = SpatialSettings::default();
        let l = listener();
        let shot = m.play("sounds/shot.wav", sound(1.0), PlayParams::ui(1.0), &l, &s, &mut cmds);
        let hum = m.play("sounds/hum.wav", sound(1.0), PlayParams::ui(1.0).looping(true), &l, &s, &mut cmds);
        m.reset_output(44_100);
        assert!(!m.is_playing(shot));
        assert!(m.is_playing(hum) && !m.is_audible(hum));
        assert_eq!(m.counts(), (1, 0));
        // On the new output the loop resumes at the new rate.
        cmds.clear();
        m.update(0.1, &l, &s, &mut cmds);
        assert!(m.is_audible(hum));
        match &cmds[..] {
            [Command::Play { step, slot: 0, .. }] => assert!((step - 11_025.0 / 44_100.0).abs() < 1e-9),
            _ => panic!("expected the loop to restart in slot 0"),
        }
        // The stale handle of the ended one-shot does not alias the next sound.
        let next = m.play("sounds/new.wav", sound(1.0), PlayParams::ui(1.0), &l, &s, &mut cmds);
        assert_ne!(next, shot);
    }

    #[test]
    fn the_log_follows_the_mixer_commands() {
        use crate::playlog::SoundKind;
        let mut m = manager(2);
        let mut cmds = Vec::new();
        let s = SpatialSettings::default();
        let mut l = listener();
        // Nothing is kept until logging is asked for.
        let click = play(&mut m, "sounds/before.wav", PlayParams::ui(1.0), &mut cmds);
        assert!(m.take_log().is_empty());
        cmds.clear();
        m.set_logging(true);
        let boom = m.play("sounds/boom.wav", sound(1.0), PlayParams::at([1300.0, 0.0, 1000.0], 0.5), &l, &s, &mut cmds);
        // Merged into it (louder); a far one past the voice limit is not played, nor logged.
        m.play("sounds/boom.wav", sound(1.0), PlayParams::at([1310.0, 0.0, 1000.0], 0.5), &l, &s, &mut cmds);
        assert!(m.play("sounds/far.wav", sound(1.0), PlayParams::at([1000.0, 0.0, 1900.0], 0.5), &l, &s, &mut cmds).is_none());
        // The camera moves; the click's pitch changes; the boom stops.
        l.position = [1600.0, 0.0, 1000.0];
        m.update(0.016, &l, &s, &mut cmds);
        m.set_pitch(click, 2.0, &mut cmds);
        m.stop(boom, &mut cmds);

        let log = m.take_log();
        assert_eq!(log.len(), cmds.len(), "one record per command");
        let mut boom_id = None;
        for (event, command) in log.iter().zip(&cmds) {
            match (event, command) {
                (PlayEvent::Start(p), Command::Play { gains, looping, .. }) => {
                    assert_eq!((p.gains, p.looping), (*gains, *looping));
                    assert_eq!((p.path.as_str(), p.kind), ("sounds/boom.wav", SoundKind::World));
                    let heard = spatialize(&listener(), &s, [1300.0, 0.0, 1000.0]);
                    assert!((p.gain - 0.5 * heard.gain).abs() < 1e-6 && (p.pan - heard.pan).abs() < 1e-6);
                    assert_eq!(p.position, Some([1300.0, 0.0, 1000.0]));
                    boom_id = Some(p.id);
                }
                (PlayEvent::Gains { id, gains: a, .. }, Command::SetGains { gains: b, .. }) => {
                    assert_eq!((Some(*id), a), (boom_id, b));
                }
                (PlayEvent::Pitch { pitch, .. }, Command::SetStep { step, .. }) => {
                    assert_eq!(*pitch, 2.0);
                    assert!((step - 2.0 * 11_025.0 / RATE as f64).abs() < 1e-9);
                }
                (PlayEvent::Stop { id, fade }, Command::Stop { .. }) => {
                    assert_eq!((Some(*id), *fade), (boom_id, STOP_FADE_SECONDS as f32));
                }
                _ => panic!("the log and the commands differ"),
            }
        }
        // Started, merged (louder), moved (to the left now), its pitch, stopped.
        assert!(matches!(log[..], [PlayEvent::Start(_), PlayEvent::Gains { .. }, PlayEvent::Gains { pan, .. }, PlayEvent::Pitch { .. }, PlayEvent::Stop { .. }] if pan < 0.0));
        assert!(m.take_log().is_empty(), "taken");
        // The same pitch again (the wind's, every frame): nothing new to record.
        let commands = cmds.len();
        m.set_pitch(click, 2.0, &mut cmds);
        assert_eq!(cmds.len(), commands, "unchanged pitch needs no mixer command");
        assert!(m.take_log().is_empty());
    }

    #[test]
    fn pitch_changes_retry_after_the_command_queue_drains() {
        let mut m = manager(2);
        let mut commands = Vec::new();
        m.set_logging(true);
        let h = play(&mut m, "sounds/engine.wav", PlayParams::ui(1.0).looping(true), &mut commands);
        let Command::Play { slot, generation, .. } = commands[0] else { panic!() };
        let (mut sink, mut queue) = rtrb::RingBuffer::new(1);
        assert!(sink.send(Command::SetMasterGain(1.0)).is_ok());
        m.take_log();
        m.set_pitch(h, 1.5, &mut sink);
        m.set_pitch(h, 2.0, &mut sink);
        assert_eq!(m.voice(h).unwrap().sent_pitch, 1.0);
        assert!(m.take_log().is_empty(), "a dropped command was not heard");
        queue.pop().unwrap();
        m.update(0.01, &listener(), &SpatialSettings::default(), &mut sink);
        assert!(matches!(queue.pop().unwrap(), Command::SetStep { slot: s, generation: g, step }
            if s == slot && g == generation && (step - 2.0 * 11_025.0 / RATE as f64).abs() < 1e-9));
        assert_eq!(m.voice(h).unwrap().sent_pitch, 2.0);
        assert!(matches!(&m.take_log()[..], [PlayEvent::Pitch { pitch: 2.0, .. }]));
        m.update(0.01, &listener(), &SpatialSettings::default(), &mut sink);
        m.set_pitch(h, 2.0, &mut sink);
        assert!(queue.pop().is_err(), "the delivered change is not sent twice");
    }

    #[test]
    fn pending_pitch_changes_do_not_escape_a_stopped_voice() {
        let mut m = manager(1);
        let mut commands = Vec::new();
        let h = play(&mut m, "sounds/old.wav", PlayParams::ui(1.0).looping(true), &mut commands);
        let Command::Play { slot, generation, .. } = commands[0] else { panic!() };
        let (mut sink, mut queue) = rtrb::RingBuffer::new(1);
        assert!(sink.send(Command::SetMasterGain(1.0)).is_ok());
        m.set_pitch(h, 2.0, &mut sink);
        m.stop(h, &mut sink);
        queue.pop().unwrap();
        m.update(0.01, &listener(), &SpatialSettings::default(), &mut sink);
        assert!(matches!(queue.pop().unwrap(), Command::Stop { .. }));
        m.on_voice_ended(slot, generation);
        commands.clear();
        let next = play(&mut m, "sounds/new.wav", PlayParams::ui(1.0).looping(true), &mut commands);
        assert!(matches!(commands[0], Command::Play { step, .. } if (step - 11_025.0 / RATE as f64).abs() < 1e-9));
        assert_ne!(next, h);
        m.update(0.01, &listener(), &SpatialSettings::default(), &mut sink);
        assert!(queue.pop().is_err(), "the replacement never inherits the stopped voice's pitch");
    }

    #[test]
    fn listener_movement_updates_gains() {
        let mut m = manager(4);
        let mut cmds = Vec::new();
        let s = SpatialSettings::default();
        let mut l = listener();
        let h = m.play("sounds/x.wav", sound(3.0), PlayParams::at([1300.0, 0.0, 1000.0], 1.0), &l, &s, &mut cmds);
        let Command::Play { gains: before, .. } = cmds[0] else { panic!() };
        assert!(before[1] > before[0], "source to the right");
        l.position = [1600.0, 0.0, 1000.0];
        m.update(0.016, &l, &s, &mut cmds);
        match cmds.last() {
            Some(Command::SetGains { gains, .. }) => assert!(gains[0] > gains[1], "now to the left"),
            _ => panic!("expected new gains"),
        }
        cmds.clear();
        m.update(0.016, &l, &s, &mut cmds);
        assert!(cmds.is_empty(), "no redundant updates");
        m.stop(h, &mut cmds);
        assert!(matches!(cmds[..], [Command::Stop { .. }]));
        assert!(!m.is_playing(h));
    }
}
