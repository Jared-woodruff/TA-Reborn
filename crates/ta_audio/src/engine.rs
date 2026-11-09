//! The game-facing audio engine.
//!
//! ```no_run
//! use std::sync::Arc;
//! use ta_audio::{AudioEngine, AudioSettings, Mood, SoundBank, SoundCategories, SoundEvent};
//!
//! # fn main() -> anyhow::Result<()> {
//! let install = ta_vfs::install::discover(None)?;
//! let mut vfs = ta_vfs::Vfs::new();
//! vfs.mount_game_dir(&install.root, "base")?;
//! let vfs = Arc::new(vfs);
//!
//! let config = ta_config::Config::load();
//! let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
//! let mut audio = AudioEngine::new(AudioSettings::from(&config.audio), bank)?;
//! let categories = SoundCategories::load(vfs.as_ref())?;
//!
//! audio.music_mut().set_mood(Mood::Peace);
//! // Every frame:
//! audio.set_listener([1024.0, 0.0, 768.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0);
//! audio.play_at("xplolrg1", [1100.0, 0.0, 800.0], 1.0);
//! audio.play_unit_sound(categories.category("ARM_COM"), SoundEvent::Select, [1024.0, 0.0, 768.0], 1.0);
//! audio.update(1.0 / 60.0);
//! # Ok(()) }
//! ```

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::bank::{SoundBank, sound_path};
use crate::categories::{SoundCategory, SoundEvent};
use crate::mixer::{Command, DECKS, Event, MixBus, Mixer, MixerStats};
use crate::music::{Mood, MusicAction, MusicConfig, MusicPlayer};
use crate::output::{self, DeviceOutput, OutputInfo};
use crate::playlog::{PlayEvent, SoundKind};
use crate::rng::Rng;
use crate::spatial::{Listener, SpatialSettings};
use crate::stream::{self, StreamShared};
use crate::voices::{CommandSink, VoiceConfig, VoiceManager, VoiceStats};

/// Commands the game can queue between two audio callbacks.
const COMMAND_QUEUE: usize = 4096;
/// Events the audio thread can report between two updates.
const EVENT_QUEUE: usize = 2048;
/// Seconds between attempts to re-open a lost output device.
const REOPEN_INTERVAL: f64 = 2.0;
const VOLUME_BUSES: [Bus; 4] = [Bus::Master, Bus::Music, Bus::Effects, Bus::Voice];

fn volume_gain(volume: f32) -> f32 {
    if volume.is_finite() { volume.clamp(0.0, 2.0) } else { 1.0 }
}

/// Volume groups. Sounds play on [`Bus::Effects`] or [`Bus::Voice`]; the soundtrack uses
/// [`Bus::Music`]; [`Bus::Master`] scales everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Bus {
    Master,
    Music,
    Effects,
    Voice,
}

/// How important a sound is when voices run out: a new sound only takes the voice of a
/// lower-priority one, or of an equally important but quieter (more distant) one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Ambience that may be dropped first.
    Low,
    /// Positional effects.
    #[default]
    Normal,
    /// Interface sounds, unit responses.
    High,
    /// Never stolen (briefing narration, critical alerts).
    Critical,
}

/// Identifies a playing sound. Stale handles (the sound ended) are harmless: operations on
/// them do nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl SoundHandle {
    /// A handle that never refers to a sound (returned when nothing was played).
    pub const NONE: SoundHandle = SoundHandle { index: u32::MAX, generation: 0 };

    pub fn is_none(self) -> bool {
        self == SoundHandle::NONE
    }
}

impl Default for SoundHandle {
    fn default() -> Self {
        SoundHandle::NONE
    }
}

/// Everything about one play request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    /// Linear volume (1 = as recorded).
    pub volume: f32,
    /// Playback-rate multiplier (1 = original pitch).
    pub pitch: f32,
    pub bus: Bus,
    pub priority: Priority,
    /// World position, or `None` for non-positional (interface, voice) sounds.
    pub position: Option<[f32; 3]>,
    pub looping: bool,
    /// Apply the anti-spam merge rule (default on).
    pub merge: bool,
    /// What the sound is, for the log of what plays ([`AudioEngine::take_log`]).
    pub kind: SoundKind,
}

impl Default for PlayParams {
    fn default() -> Self {
        PlayParams {
            volume: 1.0,
            pitch: 1.0,
            bus: Bus::Effects,
            priority: Priority::Normal,
            position: None,
            looping: false,
            merge: true,
            kind: SoundKind::World,
        }
    }
}

impl PlayParams {
    /// Interface sound: non-positional, effects bus, high priority.
    pub fn ui(volume: f32) -> PlayParams {
        PlayParams { volume, priority: Priority::High, kind: SoundKind::Interface, ..Default::default() }
    }

    /// Unit response or narration: non-positional, voice bus, high priority.
    pub fn voice(volume: f32) -> PlayParams {
        PlayParams { volume, bus: Bus::Voice, priority: Priority::High, kind: SoundKind::Voice, ..Default::default() }
    }

    /// Positional effect.
    pub fn at(position: [f32; 3], volume: f32) -> PlayParams {
        PlayParams { volume, position: Some(position), ..Default::default() }
    }

    pub fn looping(self, looping: bool) -> PlayParams {
        PlayParams { looping, ..self }
    }

    pub fn pitch(self, pitch: f32) -> PlayParams {
        PlayParams { pitch, ..self }
    }

    pub fn priority(self, priority: Priority) -> PlayParams {
        PlayParams { priority, ..self }
    }

    pub fn bus(self, bus: Bus) -> PlayParams {
        PlayParams { bus, ..self }
    }

    pub fn merge(self, merge: bool) -> PlayParams {
        PlayParams { merge, ..self }
    }

    pub fn kind(self, kind: SoundKind) -> PlayParams {
        PlayParams { kind, ..self }
    }
}

/// Engine settings. `From<&ta_config::Audio>` takes the user's volumes and voice limit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Linear bus volumes in `0..=1`.
    pub master: f32,
    pub music: f32,
    pub effects: f32,
    pub voice: f32,
    /// Maximum simultaneously audible sounds.
    pub max_voices: u32,
    /// Identical sounds started within this many seconds of each other...
    pub merge_window: f32,
    /// ...and within this many world units are merged into one.
    pub merge_distance: f32,
    pub spatial: SpatialSettings,
    /// Device buffer size in frames (`None` = backend default, ~10 ms on WASAPI).
    pub buffer_frames: Option<u32>,
    /// How far ahead of playback music is decoded.
    pub music_buffer_seconds: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings::from(&ta_config::Audio::default())
    }
}

impl From<&ta_config::Audio> for AudioSettings {
    fn from(a: &ta_config::Audio) -> Self {
        AudioSettings {
            master: a.master,
            music: a.music,
            effects: a.effects,
            voice: a.voice,
            max_voices: a.max_voices,
            merge_window: 0.06,
            merge_distance: 96.0,
            spatial: SpatialSettings::default(),
            buffer_frames: None,
            music_buffer_seconds: 2.0,
        }
    }
}

impl AudioSettings {
    pub fn volume(&self, bus: Bus) -> f32 {
        match bus {
            Bus::Master => self.master,
            Bus::Music => self.music,
            Bus::Effects => self.effects,
            Bus::Voice => self.voice,
        }
    }

    fn volume_mut(&mut self, bus: Bus) -> &mut f32 {
        match bus {
            Bus::Master => &mut self.master,
            Bus::Music => &mut self.music,
            Bus::Effects => &mut self.effects,
            Bus::Voice => &mut self.voice,
        }
    }
}

/// A snapshot of engine activity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioStats {
    /// Sounds being tracked (audible plus silent loops).
    pub sounds: usize,
    /// Sounds holding a mixer voice.
    pub audible: usize,
    /// Voices the audio thread mixed in its last callback (includes fade-outs).
    pub mixer_voices: u32,
    pub cached_sounds: usize,
    pub voices: VoiceStats,
    pub callbacks: u64,
    pub frames_rendered: u64,
    /// Music frames that were due before the decoder delivered them.
    pub music_underrun_frames: u64,
    /// Highest output sample since the previous `stats()` call.
    pub peak: f32,
    /// Lowest limiter gain since the previous `stats()` call (1 = never engaged).
    pub limiter_gain: f32,
}

enum Output {
    Device(DeviceOutput),
    Offline(Mixer),
}

struct DecoderThread {
    shared: Arc<StreamShared>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy)]
struct PendingFade {
    stream: u64,
    frames: u32,
}

/// Everything that exists only while audio is actually produced.
struct Active {
    commands: rtrb::Producer<Command>,
    events: rtrb::Consumer<Event>,
    stats: Arc<MixerStats>,
    sample_rate: u32,
    output: Output,
    decoders: Vec<DecoderThread>,
    /// The music streams on the decks, by id: how far each has played.
    streams: Vec<(u64, Arc<StreamShared>)>,
    volumes: [f32; 4],
    pending_volumes: [bool; 4],
    /// At most one undelivered stop per deck, tagged with the stream generation.
    pending_fades: [Option<PendingFade>; DECKS],
}

impl Active {
    /// Opens the default device (`offline_rate: None`) or an offline mixer.
    fn open(settings: &AudioSettings, offline_rate: Option<u32>) -> Result<Active> {
        let slots = VoiceManager::slots_for(settings.max_voices as usize);
        let (commands, command_rx) = rtrb::RingBuffer::new(COMMAND_QUEUE);
        let (event_tx, events) = rtrb::RingBuffer::new(EVENT_QUEUE);
        let stats = Arc::new(MixerStats::new());
        let volumes = VOLUME_BUSES.map(|bus| volume_gain(settings.volume(bus)));
        let mixer_stats = stats.clone();
        let make_mixer = move |rate| {
            let mut mixer = Mixer::new(rate, slots, command_rx, event_tx, mixer_stats, offline_rate.is_some());
            mixer.initialize_gains(volumes[0], [volumes[2], volumes[3], volumes[1]]);
            mixer
        };
        let output = match offline_rate {
            Some(rate) => Output::Offline(make_mixer(rate.max(8000))),
            None => Output::Device(output::open(settings.buffer_frames, make_mixer)?),
        };
        let sample_rate = match &output {
            Output::Device(d) => d.info.sample_rate,
            Output::Offline(m) => m.sample_rate(),
        };
        Ok(Active { commands, events, stats, sample_rate, output, decoders: Vec::new(), streams: Vec::new(),
            volumes, pending_volumes: [false; 4], pending_fades: [None; DECKS] })
    }

    fn send_volume(&mut self, bus: Bus, volume: f32) {
        let index = VOLUME_BUSES.iter().position(|&b| b == bus).expect("known volume bus");
        let gain = volume_gain(volume);
        if self.volumes[index] != gain {
            self.volumes[index] = gain;
            self.pending_volumes[index] = true;
        }
        self.flush_volumes();
    }

    fn flush_volumes(&mut self) {
        for (index, bus) in VOLUME_BUSES.into_iter().enumerate() {
            if !self.pending_volumes[index] { continue; }
            let gain = self.volumes[index];
            let command = match bus {
                Bus::Master => Command::SetMasterGain(gain),
                Bus::Music => Command::SetBusGain { bus: MixBus::Music, gain },
                Bus::Effects => Command::SetBusGain { bus: MixBus::Effects, gain },
                Bus::Voice => Command::SetBusGain { bus: MixBus::Voice, gain },
            };
            if self.commands.push(command).is_err() { break; }
            self.pending_volumes[index] = false;
        }
    }

    fn stop_decoders(&mut self) {
        for d in &self.decoders {
            d.shared.stop.store(true, Ordering::Release);
        }
    }

    fn send_fade(&mut self, deck: u8, stream: u64, frames: u32) {
        self.pending_fades[deck as usize % DECKS] = Some(PendingFade { stream, frames });
        self.flush_fades();
    }

    fn flush_fades(&mut self) {
        for (deck, pending) in self.pending_fades.iter_mut().enumerate() {
            let Some(PendingFade { stream, frames }) = *pending else { continue };
            let command = Command::DeckFade { deck: deck as u8, stream_id: stream, target: 0.0, frames, release: true };
            if self.commands.push(command).is_err() { break; }
            *pending = None;
        }
    }
}

/// The audio engine: sound effects, voices and music on the default output device.
///
/// All methods are cheap and non-blocking (except the first play of a sound, which decodes
/// it) and are meant to be called from the game thread. Call [`update`](Self::update) once
/// per frame.
pub struct AudioEngine {
    settings: AudioSettings,
    bank: Arc<SoundBank>,
    listener: Listener,
    /// Logical voices. They outlive device changes: loops resume on a re-opened device and
    /// handles held by the game stay valid.
    voices: VoiceManager,
    music: MusicPlayer,
    backend: Option<Active>,
    /// Why the engine is silent, when it is.
    silent_reason: Option<String>,
    /// The device failed; re-opening is retried every `REOPEN_INTERVAL` seconds.
    device_lost: bool,
    clock: f64,
    next_reopen: f64,
    rng: Rng,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine")
            .field("output", &self.output_info())
            .field("silent_reason", &self.silent_reason)
            .finish_non_exhaustive()
    }
}

impl AudioEngine {
    /// Opens the default audio device. Without a usable device the engine starts silently
    /// and periodically retries, so connecting a headset later restores sound.
    /// The music configuration is read from `<config dir>/music.toml` when present.
    ///
    /// The `Result` is reserved for engine-internal failures; a missing device is not one.
    pub fn new(settings: AudioSettings, bank: Arc<SoundBank>) -> Result<AudioEngine> {
        let mut engine = AudioEngine::build(settings, bank, MusicConfig::load_user());
        match Active::open(&engine.settings, None) {
            Ok(active) => {
                if let Output::Device(d) = &active.output {
                    log::info!("audio output: {}", d.info);
                }
                engine.voices.reset_output(active.sample_rate);
                engine.backend = Some(active);
            }
            Err(e) => {
                log::warn!("audio disabled: {e}");
                engine.output_unavailable(e.to_string());
            }
        }
        Ok(engine)
    }

    /// An engine that plays nothing.
    pub fn new_silent(settings: AudioSettings, bank: Arc<SoundBank>) -> AudioEngine {
        let mut engine = AudioEngine::build(settings, bank, MusicConfig::default());
        engine.silent_reason = Some("audio disabled".into());
        engine
    }

    /// An engine that mixes into buffers supplied to [`render_offline`](Self::render_offline)
    /// instead of a device. Music decoding waits for the decoder threads, so rendering is
    /// deterministic apart from the shuffle order.
    pub fn new_offline(settings: AudioSettings, bank: Arc<SoundBank>, sample_rate: u32) -> AudioEngine {
        let mut engine = AudioEngine::build(settings, bank, MusicConfig::default());
        let active = Active::open(&engine.settings, Some(sample_rate)).expect("an offline mixer cannot fail");
        engine.voices.reset_output(active.sample_rate);
        engine.backend = Some(active);
        engine
    }

    fn build(mut settings: AudioSettings, bank: Arc<SoundBank>, music: MusicConfig) -> AudioEngine {
        for bus in VOLUME_BUSES { *settings.volume_mut(bus) = volume_gain(settings.volume(bus)); }
        let available = bank.source().list("music");
        let voices = VoiceManager::new(VoiceConfig {
            max_voices: settings.max_voices as usize,
            merge_window: settings.merge_window.max(0.0) as f64,
            merge_distance: settings.merge_distance.max(0.0),
            output_rate: 48_000, // replaced by the real rate once an output is open
        });
        AudioEngine {
            settings,
            listener: Listener::default(),
            voices,
            music: MusicPlayer::new(music, &available),
            bank,
            backend: None,
            silent_reason: None,
            device_lost: false,
            clock: 0.0,
            next_reopen: 0.0,
            rng: Rng::from_time(),
        }
    }

    /// Mixes the next `out.len() / 2` stereo frames (offline engines only; others get silence).
    pub fn render_offline(&mut self, out: &mut [f32]) {
        match &mut self.backend {
            Some(Active { output: Output::Offline(mixer), .. }) => mixer.render(out, 2),
            _ => out.fill(0.0),
        }
    }

    pub fn is_silent(&self) -> bool {
        self.backend.is_none()
    }

    /// Why the engine is silent (no device, device error...).
    pub fn silent_reason(&self) -> Option<&str> {
        self.silent_reason.as_deref()
    }

    /// The output device, when playing to one.
    pub fn output_info(&self) -> Option<&OutputInfo> {
        match &self.backend {
            Some(Active { output: Output::Device(d), .. }) => Some(&d.info),
            _ => None,
        }
    }

    /// Mixing rate (device or offline), `None` when silent.
    pub fn sample_rate(&self) -> Option<u32> {
        self.backend.as_ref().map(|a| a.sample_rate)
    }

    pub fn bank(&self) -> &Arc<SoundBank> {
        &self.bank
    }

    pub fn settings(&self) -> &AudioSettings {
        &self.settings
    }

    /// Places the listener at the camera: ground focus, screen-up and screen-right vectors and
    /// the visible ground width in world units (TA map pixels).
    pub fn set_listener(&mut self, position: [f32; 3], forward: [f32; 3], right: [f32; 3], view_width: f32) {
        self.listener = Listener::new(position, forward, right, view_width);
    }

    pub fn listener(&self) -> &Listener {
        &self.listener
    }

    /// Plays a sound by TA name (`"xplolrg1"`) or VFS path. Returns [`SoundHandle::NONE`] when
    /// nothing plays: the engine is silent, the sound is missing, it would be inaudible, or
    /// every voice is taken by more important sounds. A request merged by the anti-spam rule
    /// returns the handle of the voice it joined.
    pub fn play(&mut self, name: &str, params: PlayParams) -> SoundHandle {
        let Some(active) = self.backend.as_mut() else { return SoundHandle::NONE };
        let Some(sound) = self.bank.get(name) else { return SoundHandle::NONE };
        let path = sound_path(name);
        self.voices.play(&path, sound, params, &self.listener, &self.settings.spatial, &mut active.commands)
    }

    /// Interface sound (clicks, alerts): non-positional, never displaced by effects.
    pub fn play_ui(&mut self, name: &str, volume: f32) -> SoundHandle {
        self.play(name, PlayParams::ui(volume))
    }

    /// Spoken sound (unit responses, narration) on the voice bus.
    pub fn play_voice(&mut self, name: &str, volume: f32) -> SoundHandle {
        self.play(name, PlayParams::voice(volume))
    }

    /// Positional effect at a world position.
    pub fn play_at(&mut self, name: &str, position: [f32; 3], volume: f32) -> SoundHandle {
        self.play(name, PlayParams::at(position, volume))
    }

    /// Plays a unit event from its `sound.tdf` category, picking a random variant.
    /// Acknowledgements and alerts ([`SoundEvent::is_acknowledgement`]) play on the voice bus
    /// without position; everything else plays as a positional effect at `position`.
    pub fn play_unit_sound(
        &mut self,
        category: &SoundCategory,
        event: SoundEvent,
        position: [f32; 3],
        volume: f32,
    ) -> SoundHandle {
        let random = self.rng.next_u64() as u32;
        let Some(name) = category.pick(event, random) else { return SoundHandle::NONE };
        let name = name.to_string();
        if event.is_acknowledgement() {
            self.play(&name, PlayParams::voice(volume))
        } else {
            self.play(&name, PlayParams::at(position, volume))
        }
    }

    /// Stops a sound with a short fade.
    pub fn stop(&mut self, handle: SoundHandle) {
        let mut discard = Vec::new();
        let sink: &mut dyn CommandSink = match self.backend.as_mut() {
            Some(a) => &mut a.commands,
            None => &mut discard,
        };
        self.voices.stop(handle, sink);
    }

    /// Stops every sound effect and voice (music continues).
    pub fn stop_all(&mut self) {
        let mut discard = Vec::new();
        let sink: &mut dyn CommandSink = match self.backend.as_mut() {
            Some(a) => &mut a.commands,
            None => &mut discard,
        };
        self.voices.stop_all(sink);
    }

    /// Stops the sounds of one kind (the world's, at a cut in a film: narration, the interface
    /// and the wind go on).
    pub fn stop_kind(&mut self, kind: crate::playlog::SoundKind) {
        let mut discard = Vec::new();
        let sink: &mut dyn CommandSink = match self.backend.as_mut() {
            Some(a) => &mut a.commands,
            None => &mut discard,
        };
        self.voices.stop_kind(kind, sink);
    }

    /// Moves a positional sound (applied at the next [`update`](Self::update)).
    pub fn set_position(&mut self, handle: SoundHandle, position: [f32; 3]) {
        self.voices.set_position(handle, Some(position));
    }

    /// Changes a sound's volume (applied at the next [`update`](Self::update)).
    pub fn set_sound_volume(&mut self, handle: SoundHandle, volume: f32) {
        self.voices.set_volume(handle, volume);
    }

    pub fn set_pitch(&mut self, handle: SoundHandle, pitch: f32) {
        let mut discard = Vec::new();
        let sink: &mut dyn CommandSink = match self.backend.as_mut() {
            Some(a) => &mut a.commands,
            None => &mut discard,
        };
        self.voices.set_pitch(handle, pitch, sink);
    }

    /// Whether the sound is still tracked (audible, or a silent loop waiting to resume).
    pub fn is_playing(&self, handle: SoundHandle) -> bool {
        self.voices.is_playing(handle)
    }

    /// Whether the sound currently holds a mixer voice.
    pub fn is_audible(&self, handle: SoundHandle) -> bool {
        self.backend.is_some() && self.voices.is_audible(handle)
    }

    /// Keeps a record of what the mixer plays (or stops keeping it): each sound started on a
    /// voice and each change to it, for [`take_log`](Self::take_log). Nothing is recorded while
    /// the engine is silent: an offline engine ([`new_offline`](Self::new_offline)) rendered on
    /// the game's clock records what would play without a device.
    pub fn set_logging(&mut self, on: bool) {
        self.voices.set_logging(on);
    }

    /// What the mixer was told to play since the last call, in order (empty unless logging).
    pub fn take_log(&mut self) -> Vec<PlayEvent> {
        self.voices.take_log()
    }

    /// Sets a bus volume (linear, `0..=1`; values up to 2 boost).
    pub fn set_volume(&mut self, bus: Bus, volume: f32) {
        let volume = volume_gain(volume);
        *self.settings.volume_mut(bus) = volume;
        if let Some(a) = self.backend.as_mut() {
            a.send_volume(bus, volume);
        }
    }

    pub fn volume(&self, bus: Bus) -> f32 {
        self.settings.volume(bus)
    }

    /// A gain on the world's (placed) sounds only, from the next update: the interface's,
    /// voices' and the music keep theirs. See [`SpatialSettings::world_gain`].
    pub fn set_world_gain(&mut self, gain: f32) {
        self.settings.spatial.world_gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 1.0 };
    }

    /// Applies the volumes of a (changed) user configuration. A new `max_voices` takes effect
    /// the next time the engine is created.
    pub fn apply_config(&mut self, audio: &ta_config::Audio) {
        let volumes = [
            (Bus::Master, audio.master),
            (Bus::Music, audio.music),
            (Bus::Effects, audio.effects),
            (Bus::Voice, audio.voice),
        ];
        for (bus, v) in volumes {
            self.set_volume(bus, v);
        }
    }

    pub fn music(&self) -> &MusicPlayer {
        &self.music
    }

    pub fn music_mut(&mut self) -> &mut MusicPlayer {
        &mut self.music
    }

    /// How far into the track playing the music is, in seconds (`None` while none plays).
    pub fn music_position(&self) -> Option<f64> {
        let id = self.music.now_playing_stream()?;
        self.stream_position(id)
    }

    /// How far into a film's score the music is ([`MusicPlayer::play_score`]): its own seconds,
    /// as far as the mixer has played it (`None` while no score plays, before it starts, or when
    /// it could not be played).
    pub fn score_position(&self) -> Option<f64> {
        let id = self.music.score_stream()?;
        self.stream_position(id)
    }

    fn stream_position(&self, id: u64) -> Option<f64> {
        let active = self.backend.as_ref()?;
        let (_, shared) = active.streams.iter().find(|(s, _)| *s == id)?;
        if shared.failed.load(Ordering::Acquire) {
            return None;
        }
        Some(shared.played.load(Ordering::Relaxed) as f64 / active.sample_rate.max(1) as f64)
    }

    /// The music as it last left the decks (before its volume), mono, the latest `out.len()`
    /// samples (at most [`crate::mixer::SCOPE`]), oldest first: for a spectrum. Returns the
    /// sample rate, or none without an output.
    pub fn music_scope(&self, out: &mut [f32]) -> Option<u32> {
        let active = self.backend.as_ref()?;
        active.stats.scope.read(out);
        Some(active.sample_rate)
    }

    /// Shortcut for `music_mut().set_mood(mood)`.
    pub fn set_mood(&mut self, mood: Mood) {
        self.music.set_mood(mood);
    }

    /// Replaces the music configuration (playlists resolved against the VFS `music/` folder).
    pub fn set_music_config(&mut self, config: MusicConfig) {
        let available = self.bank.source().list("music");
        self.music.set_config(config, &available);
    }

    /// Advances the engine by `dt` seconds: processes finished sounds, re-spatialises sounds
    /// for the current listener, sequences music, and recovers from a lost device.
    pub fn update(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 1.0) as f64 } else { 0.0 };
        self.clock += dt;
        self.recover_output();

        // Without an output the music state machine is frozen: requested moods are remembered
        // and take effect once audio is available again.
        let Some(active) = self.backend.as_mut() else { return };
        active.flush_fades();
        active.flush_volumes();
        while let Ok(event) = active.events.pop() {
            match event {
                Event::VoiceEnded { slot, generation, sound } => {
                    self.voices.on_voice_ended(slot, generation);
                    drop(sound);
                }
                Event::DeckEnded { deck, stream, natural } => {
                    log::debug!("music deck {deck} released stream {} (natural end: {natural})", stream.id);
                    stream.shared.stop.store(true, Ordering::Release);
                    let failed = stream.shared.failed.load(Ordering::Acquire);
                    active.streams.retain(|(id, _)| *id != stream.id);
                    self.music.on_stream_ended(stream.id, natural, failed);
                }
            }
        }
        self.voices.update(dt, &self.listener, &self.settings.spatial, &mut active.commands);

        self.music.update(dt);
        for action in self.music.take_actions() {
            match action {
                MusicAction::Start { deck, stream: id, path, from, fade_in, crossfade_from, score } => {
                    let rate = active.sample_rate;
                    let buffer = (self.settings.music_buffer_seconds.clamp(0.25, 30.0) * rate as f32) as usize;
                    let spawned = match score {
                        Some(arrangement) => stream::spawn_arranged(id, path.clone(), arrangement, from, self.bank.source().clone(), rate, buffer),
                        None => stream::spawn_decoder(id, path.clone(), from, self.bank.source().clone(), rate, buffer),
                    };
                    match spawned {
                        Ok((music_stream, shared, thread)) => {
                            let fade_in = (fade_in.max(0.0) * rate as f32) as u32;
                            let start = Command::DeckStart { deck, stream: music_stream, fade_in, crossfade_from };
                            if active.commands.push(start).is_err() {
                                shared.stop.store(true, Ordering::Release);
                                self.music.on_stream_ended(id, false, false);
                            } else {
                                active.streams.push((id, shared.clone()));
                            }
                            active.decoders.push(DecoderThread { shared, thread: Some(thread) });
                            log::debug!("music: {path} on deck {deck}");
                        }
                        Err(e) => {
                            log::warn!("cannot start music decoder: {e}");
                            self.music.on_stream_ended(id, false, true);
                        }
                    }
                }
                MusicAction::FadeOut { deck, stream, seconds } => {
                    let frames = (seconds.max(0.01) * active.sample_rate as f32) as u32;
                    active.send_fade(deck, stream, frames);
                }
            }
        }
        // Reap decoder threads that have finished (their streams may still be playing).
        active.decoders.retain_mut(|d| {
            if d.thread.as_ref().is_some_and(|t| t.is_finished()) {
                if let Some(t) = d.thread.take() {
                    let _ = t.join();
                }
                false
            } else {
                true
            }
        });
    }

    /// Handles a fatal stream error (unplugged headset, driver reset): drops the stream and
    /// retries opening the default device every `REOPEN_INTERVAL` seconds. Loops resume on
    /// the new device, one-shots in flight are lost, and the music mood restarts.
    fn recover_output(&mut self) {
        self.recover_output_with(|settings| Active::open(settings, None));
    }

    fn output_unavailable(&mut self, reason: String) {
        self.silent_reason = Some(reason);
        self.device_lost = true;
        self.next_reopen = self.clock + REOPEN_INTERVAL;
    }

    fn recover_output_with(&mut self, open: impl FnOnce(&AudioSettings) -> Result<Active>) {
        let failed = matches!(
            &self.backend,
            Some(Active { output: Output::Device(d), .. }) if d.failed.load(Ordering::Acquire)
        );
        if failed {
            if let Some(mut old) = self.backend.take() {
                old.stop_decoders();
            }
            self.voices.reset_output(self.voices.output_rate());
            self.music.on_output_reset();
            self.device_lost = true;
            self.next_reopen = self.clock;
            log::warn!("audio output lost; trying to re-open it");
        }
        if !self.device_lost || self.clock < self.next_reopen {
            return;
        }
        self.next_reopen = self.clock + REOPEN_INTERVAL;
        match open(&self.settings) {
            Ok(active) => {
                if let Output::Device(d) = &active.output {
                    log::info!("audio output re-opened: {}", d.info);
                }
                self.voices.reset_output(active.sample_rate);
                self.backend = Some(active);
                self.silent_reason = None;
                self.device_lost = false;
            }
            Err(e) => {
                log::debug!("audio output still unavailable: {e}");
                self.silent_reason = Some(format!("audio output lost: {e}"));
            }
        }
    }

    pub fn stats(&self) -> AudioStats {
        let mut stats = AudioStats { cached_sounds: self.bank.cached_count(), limiter_gain: 1.0, ..Default::default() };
        (stats.sounds, stats.audible) = self.voices.counts();
        stats.voices = self.voices.stats();
        if let Some(a) = &self.backend {
            stats.mixer_voices = a.stats.active_voices.load(Ordering::Relaxed);
            stats.callbacks = a.stats.callbacks.load(Ordering::Relaxed);
            stats.frames_rendered = a.stats.frames.load(Ordering::Relaxed);
            stats.music_underrun_frames = a.stats.underrun_frames.load(Ordering::Relaxed);
            stats.peak = a.stats.take_peak();
            stats.limiter_gain = a.stats.take_limiter_gain();
        }
        stats
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        if let Some(a) = self.backend.as_mut() {
            a.stop_decoders();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SoundData;
    use crate::bank::MemorySource;

    const RATE: u32 = 48_000;

    fn engine() -> AudioEngine {
        let bank = Arc::new(SoundBank::new(Arc::new(MemorySource::new())));
        let tone = |freq: f32, secs: f32| {
            let n = (11_025.0 * secs) as usize;
            let samples = (0..n).map(|i| 0.5 * (std::f32::consts::TAU * freq * i as f32 / 11_025.0).sin()).collect();
            SoundData::mono(11_025, samples)
        };
        bank.insert("boom", tone(110.0, 1.0));
        bank.insert("laser", tone(880.0, 0.3));
        bank.insert("click", tone(2000.0, 0.05));
        let settings =
            AudioSettings { master: 1.0, effects: 1.0, voice: 1.0, music: 1.0, max_voices: 8, ..Default::default() };
        let mut e = AudioEngine::new_offline(settings, bank, RATE);
        e.set_listener([1000.0, 0.0, 1000.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0);
        e
    }

    fn render(e: &mut AudioEngine, seconds: f32) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * RATE as f32) as usize * 2];
        for chunk in out.chunks_mut(2 * 800) {
            e.render_offline(chunk);
            e.update(chunk.len() as f32 / 2.0 / RATE as f32);
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
    }

    fn channel(x: &[f32], c: usize) -> Vec<f32> {
        x.chunks(2).map(|f| f[c]).collect()
    }

    #[test]
    fn engine_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AudioEngine>();
    }

    #[test]
    fn positional_sounds_pan_and_attenuate() {
        let mut e = engine();
        e.play_at("boom", [1300.0, 0.0, 1000.0], 1.0);
        let right = render(&mut e, 0.5);
        let (l, r) = (rms(&channel(&right, 0)), rms(&channel(&right, 1)));
        assert!(r > 1.5 * l && l > 0.0, "l {l} r {r}");

        let mut e = engine();
        e.play_at("boom", [1000.0, 0.0, 1000.0], 1.0);
        let center = rms(&render(&mut e, 0.5));
        let mut e = engine();
        e.play_at("boom", [1000.0, 0.0, 1000.0 + 900.0], 1.0);
        let far = rms(&render(&mut e, 0.5));
        assert!(far < center * 0.15 && far > 0.0, "center {center} far {far}");
        let mut e = engine();
        assert!(e.play_at("boom", [9000.0, 0.0, 9000.0], 1.0).is_none(), "out of earshot");
    }

    #[test]
    fn spam_is_merged_in_the_mix() {
        let mut single = engine();
        single.play_at("laser", [1000.0, 0.0, 1000.0], 1.0);
        let one = rms(&render(&mut single, 0.3));
        let mut spam = engine();
        let handles: Vec<SoundHandle> =
            (0..50).map(|i| spam.play_at("laser", [1000.0 + i as f32, 0.0, 1000.0], 1.0)).collect();
        assert!(handles.windows(2).all(|w| w[0] == w[1]));
        let many = rms(&render(&mut spam, 0.3));
        assert!((many / one - 1.5).abs() < 0.05, "merged level {}", many / one);
        assert_eq!(spam.stats().voices.merged, 49);
    }

    #[test]
    fn voices_end_and_free_their_slots() {
        let mut e = engine();
        let h = e.play_ui("click", 1.0);
        assert!(e.is_playing(h));
        render(&mut e, 0.2);
        assert!(!e.is_playing(h));
        let s = e.stats();
        assert_eq!((s.sounds, s.audible, s.mixer_voices), (0, 0, 0));
        assert!(s.frames_rendered >= 9600 && s.peak > 0.0);
    }

    fn fill_command_queue(e: &mut AudioEngine) {
        let commands = &mut e.backend.as_mut().unwrap().commands;
        while commands.push(Command::SetMasterGain(1.0)).is_ok() {}
        assert!(e.backend.as_mut().unwrap().commands.push(Command::SetMasterGain(1.0)).is_err());
    }

    #[test]
    fn initial_mute_is_silent_from_the_first_sample() {
        for bus in [Bus::Master, Bus::Effects] {
            let bank = Arc::new(SoundBank::new(Arc::new(MemorySource::new())));
            bank.insert("constant", SoundData::mono(RATE, vec![0.5; 1000]));
            let mut settings = AudioSettings::default();
            *settings.volume_mut(bus) = 0.0;
            let mut e = AudioEngine::new_offline(settings, bank, RATE);
            e.play_ui("constant", 1.0);
            let mut out = [1.0; 1024];
            e.render_offline(&mut out);
            assert!(out.iter().all(|&s| s == 0.0), "initial {bus:?} mute leaked sound");
        }
    }

    #[test]
    fn saturated_volume_changes_retry_the_latest_target() {
        let mut e = engine();
        e.play("boom", PlayParams::ui(0.2).looping(true));
        assert!(rms(&render(&mut e, 0.02)) > 0.01);
        fill_command_queue(&mut e);
        e.set_volume(Bus::Master, 0.5);
        e.set_volume(Bus::Master, 0.0);
        assert_eq!(e.volume(Bus::Master), 0.0);
        render(&mut e, 0.05);
        assert_eq!(rms(&render(&mut e, 0.02)), 0.0, "mute survives a full command queue");
        e.set_volume(Bus::Master, 0.0);
        assert_eq!(e.backend.as_ref().unwrap().commands.slots(), COMMAND_QUEUE, "unchanged sliders queue nothing");
    }

    #[test]
    fn saturated_music_stop_retries_without_stopping_a_newer_stream() {
        for replace in [false, true] {
            let mut e = engine();
            let (old, _producer) = crate::mixer::tests::stream(11, 0.2, 12000);
            old.shared.finished.store(true, Ordering::Release);
            assert!(e.backend.as_mut().unwrap().commands.push(Command::DeckStart {
                deck: 0, stream: old, fade_in: 0, crossfade_from: None,
            }).is_ok());
            assert!(rms(&render(&mut e, 0.02)) > 0.1);
            fill_command_queue(&mut e);
            e.backend.as_mut().unwrap().send_fade(0, 11, 480);
            assert!(e.backend.as_ref().unwrap().pending_fades[0].is_some());
            // Drain without updating, then replace this deck before the stop can retry.
            e.render_offline(&mut [0.0; 1024]);
            if replace {
                let (new, _producer) = crate::mixer::tests::stream(12, 0.4, 12000);
                new.shared.finished.store(true, Ordering::Release);
                assert!(e.backend.as_mut().unwrap().commands.push(Command::DeckStart {
                    deck: 0, stream: new, fade_in: 0, crossfade_from: None,
                }).is_ok());
            }
            e.update(0.01);
            assert!(e.backend.as_ref().unwrap().pending_fades[0].is_none());
            render(&mut e, 0.03);
            let level = rms(&render(&mut e, 0.02));
            if replace { assert!((level - 0.4).abs() < 1e-6, "stale stop cannot mute replacement"); }
            else { assert_eq!(level, 0.0, "queued stop must finish"); }
        }
    }

    #[test]
    fn unavailable_startup_output_retries_until_available() {
        let mut e = engine();
        e.backend = None;
        e.output_unavailable("headset absent".into());
        e.recover_output_with(|_| panic!("retry must be delayed"));
        e.clock += REOPEN_INTERVAL;
        e.recover_output_with(|_| Err(crate::AudioError::NoDevice));
        assert!(e.is_silent() && e.device_lost);
        e.recover_output_with(|_| panic!("failed retry must be delayed again"));
        e.clock += REOPEN_INTERVAL;
        e.recover_output_with(|settings| Active::open(settings, Some(RATE)));
        assert!(!e.is_silent() && !e.device_lost && e.silent_reason().is_none());
        assert!(!e.play_ui("click", 1.0).is_none());
    }

    #[test]
    fn stopped_loops_cannot_return_after_device_recovery() {
        for all in [false, true] {
            let mut e = engine();
            let handle = e.play("boom", PlayParams::ui(0.2).looping(true));
            render(&mut e, 0.02);
            // The failed-device path parks surviving loops before attempting a re-open.
            e.backend = None;
            e.voices.reset_output(RATE);
            e.output_unavailable("headset disconnected".into());
            assert!(e.is_playing(handle));
            if all { e.stop_all(); } else { e.stop(handle); }
            assert!(!e.is_playing(handle));
            e.clock += REOPEN_INTERVAL;
            e.recover_output_with(|settings| Active::open(settings, Some(RATE)));
            e.update(0.01);
            assert_eq!(rms(&render(&mut e, 0.05)), 0.0, "stopped scene loops must not resume");
            assert_eq!(e.stats().sounds, 0);
        }
    }

    #[test]
    fn saturated_stops_silence_loops_and_recycle_slots() {
        for stop_all in [false, true] {
            let mut e = engine();
            e.set_logging(true);
            // More than the available slots over these rounds: any orphan eventually
            // prevents a full set of voices from starting again.
            for _ in 0..3 {
                let handles: Vec<_> = (0..8).map(|_| e.play("boom", PlayParams::ui(0.1).looping(true))).collect();
                assert!(handles.iter().all(|&h| e.is_audible(h)));
                assert!(rms(&render(&mut e, 0.05)) > 0.01);
                let starts: Vec<_> = e.take_log().into_iter().filter_map(|event| match event {
                    PlayEvent::Start(start) => Some(start.id),
                    _ => None,
                }).collect();
                assert_eq!(starts.len(), 8);
                fill_command_queue(&mut e);
                for _ in 0..2 {
                    if stop_all {
                        e.stop_all();
                    } else {
                        for &handle in &handles {
                            e.stop(handle);
                        }
                    }
                }
                assert!(handles.iter().all(|&h| !e.is_playing(h)));
                e.update(0.01);
                assert!(e.take_log().is_empty(), "failed stops are not logged as delivered");
                render(&mut e, 0.1); // drain the queue, retry, and finish the fades
                assert_eq!(rms(&render(&mut e, 0.05)), 0.0);
                let stats = e.stats();
                assert_eq!((stats.sounds, stats.audible, stats.mixer_voices), (0, 0, 0));
                let stops: Vec<_> = e.take_log().into_iter().map(|event| match event {
                    PlayEvent::Stop { id, .. } => id,
                    _ => panic!("only the retried stops should be logged"),
                }).collect();
                assert_eq!(stops, starts, "each stopped playing has exactly one stop");
            }
        }
    }

    #[test]
    fn saturated_virtualization_stops_the_old_voice_and_resumes_once() {
        let mut e = engine();
        e.set_logging(true);
        let handle = e.play("boom", PlayParams::at([1000.0, 0.0, 1000.0], 0.5).looping(true));
        render(&mut e, 0.05);
        let log = e.take_log();
        let [PlayEvent::Start(first)] = &log[..] else { panic!("one playing") };
        fill_command_queue(&mut e);
        e.set_position(handle, [9000.0, 0.0, 9000.0]);
        e.update(0.01);
        assert!(e.is_playing(handle) && !e.is_audible(handle));
        // Becoming audible while the stop is still blocked must not start a duplicate.
        e.set_position(handle, [1000.0, 0.0, 1000.0]);
        e.update(0.01);
        assert!(!e.is_audible(handle));
        assert!(e.take_log().is_empty());
        render(&mut e, 0.1);
        assert!(e.is_audible(handle));
        assert_eq!(e.stats().mixer_voices, 1);
        let log = e.take_log();
        assert!(matches!(&log[..], [PlayEvent::Stop { id, .. }, PlayEvent::Start(next)] if *id == first.id && next.id != first.id));
        assert_eq!(e.stats().voices.resumed, 1);

        // Also cover staying virtual until the queued stop has really silenced the mixer.
        fill_command_queue(&mut e);
        e.set_position(handle, [9000.0, 0.0, 9000.0]);
        e.update(0.01);
        render(&mut e, 0.1);
        assert_eq!(rms(&render(&mut e, 0.05)), 0.0);
        assert!(e.is_playing(handle) && !e.is_audible(handle));
        assert_eq!(e.stats().mixer_voices, 0);
        e.stop_all();
        e.set_position(handle, [1000.0, 0.0, 1000.0]);
        render(&mut e, 0.1);
        assert_eq!(e.stats().sounds, 0);
    }

    #[test]
    fn bus_volumes_scale_output() {
        let mut e = engine();
        e.play_voice("boom", 1.0);
        let full = rms(&render(&mut e, 0.25));
        let mut e = engine();
        e.set_volume(Bus::Voice, 0.5);
        render(&mut e, 0.02); // let the bus gain ramp settle
        e.play_voice("boom", 1.0);
        let half = rms(&render(&mut e, 0.25));
        assert!((half / full - 0.5).abs() < 0.005, "{}", half / full);
        let mut e = engine();
        e.set_volume(Bus::Master, 0.0);
        render(&mut e, 0.02);
        e.play_voice("boom", 1.0);
        assert_eq!(rms(&render(&mut e, 0.1)), 0.0);
        assert_eq!(e.volume(Bus::Master), 0.0);
    }

    /// A film's score through the mixer: its spans in turn at its gain on the music bus, and its
    /// position, the score's own seconds, as far as the mixer has played it.
    #[test]
    fn a_score_plays_its_arrangement_and_tells_its_position() {
        // Two seconds of a 48 kHz stereo WAV: 0.25 in the first, 0.5 in the second.
        let frames = 96_000u32;
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36 + frames * 4).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(frames * 4).to_le_bytes());
        for i in 0..frames {
            let v: i16 = if i < 48_000 { 8192 } else { 16384 };
            wav.extend_from_slice(&v.to_le_bytes());
            wav.extend_from_slice(&v.to_le_bytes());
        }
        let mut source = MemorySource::new();
        source.insert("music/song.wav", wav);
        let bank = Arc::new(SoundBank::new(Arc::new(source)));
        let settings = AudioSettings { master: 1.0, effects: 1.0, voice: 1.0, music: 1.0, max_voices: 8, ..Default::default() };
        let mut e = AudioEngine::new_offline(settings, bank, RATE);
        assert_eq!(e.score_position(), None);
        e.music_mut().hush(0.0);
        // The second second, then the first, at half the level.
        let arrangement = Arc::new(crate::music::Arrangement { spans: vec![(1.0, 2.0), (0.0, 1.0)], crossfade: 0.0, gain: 0.5 });
        assert!(e.music_mut().play_score("music/song.wav", arrangement, 0.0, 0.0));
        e.update(0.0);
        let out = render(&mut e, 1.5);
        let left = channel(&out, 0);
        assert!((left[24_000] - 0.25).abs() < 0.002, "the second second first, at half: {}", left[24_000]);
        assert!((left[60_000] - 0.125).abs() < 0.002, "then the first: {}", left[60_000]);
        let position = e.score_position().expect("the score plays");
        assert!((position - 1.5).abs() < 0.03, "{position}");
        assert_eq!(e.music_position(), Some(position));
        // The film over: the score fades out and is let go.
        e.music_mut().end_score(0.05);
        render(&mut e, 0.2);
        assert_eq!(e.score_position(), None);
    }

    #[test]
    fn silent_engine_is_a_no_op() {
        let bank = Arc::new(SoundBank::new(Arc::new(MemorySource::new())));
        bank.insert("boom", SoundData::mono(11_025, vec![0.5; 100]));
        let mut e = AudioEngine::new_silent(AudioSettings::default(), bank);
        e.clock = 100.0;
        e.recover_output_with(|_| panic!("an intentionally silent engine must stay silent"));
        assert!(e.is_silent());
        assert!(e.play_ui("boom", 1.0).is_none());
        e.set_mood(Mood::Battle);
        e.update(0.1);
        assert_eq!(e.music().mood(), Some(Mood::Battle));
        assert_eq!(e.music().now_playing(), None, "nothing pretends to play");
        let mut buf = [1.0f32; 8];
        e.render_offline(&mut buf);
        assert_eq!(buf, [0.0; 8]);
        assert_eq!(e.stats().sounds, 0);
    }
}
