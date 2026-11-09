//! Music: mood-driven playlists over TA's CD tracks, with crossfades.
//!
//! TA's own music menu (`guis/music.gui`) let players tag each CD track as *Building*,
//! *Battle*, *Victory*, *Defeat* or *Unused*, and switched between the building and battle
//! sets as fighting started and stopped. [`Mood`] mirrors those classes and adds *Briefing* for
//! menus and mission briefings.
//!
//! The GOG release ships the tracks as `music/0.mp3` .. `music/17.mp3`; 9 and 10 are
//! byte-identical copies of 0 and 1. The default assignment below was derived by measuring the
//! tracks (spectral centroid, onset density, loudness range): 2–8 are bright and busy
//! (centroid 1.75–2.1 kHz, 0.55–0.9 onsets/s), 11–16 darker and calmer, 17 a steady low
//! drone, and the two short duplicated tracks behave like stingers: 0 bright with a wide
//! dynamic swell (victory), 1 dark and bass-heavy (defeat). Players can override everything
//! with a small TOML file (see [`MusicConfig`]).
//!
//! [`MusicPlayer`] is a pure state machine: it decides *what* should play and emits actions
//! that [`crate::AudioEngine`] carries out (decoder threads, mixer decks).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::rng::Rng;
use crate::{AudioError, Result};

/// What the music should express.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    /// Building up, no fighting (TA's "Building" tracks).
    Peace,
    /// Combat nearby.
    Battle,
    /// The player won. Plays once, then silence.
    Victory,
    /// The player lost. Plays once, then silence.
    Defeat,
    /// Menus and mission briefings.
    Briefing,
}

impl Mood {
    pub const ALL: [Mood; 5] = [Mood::Peace, Mood::Battle, Mood::Victory, Mood::Defeat, Mood::Briefing];

    /// Whether the playlist continues with another track when one ends.
    pub fn loops(self) -> bool {
        !matches!(self, Mood::Victory | Mood::Defeat)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A track in a playlist: a number (`music/<n>.mp3`, or `.ogg`) or an explicit VFS path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrackRef {
    Number(u32),
    Path(String),
}

/// Track lists per mood. Moods missing from a TOML file keep their defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlists {
    pub peace: Vec<TrackRef>,
    pub battle: Vec<TrackRef>,
    pub victory: Vec<TrackRef>,
    pub defeat: Vec<TrackRef>,
    pub briefing: Vec<TrackRef>,
}

impl Default for Playlists {
    fn default() -> Self {
        let n = |v: &[u32]| v.iter().map(|&i| TrackRef::Number(i)).collect();
        Playlists {
            peace: n(&[11, 12, 13, 14, 15, 16]),
            battle: n(&[2, 3, 4, 5, 6, 7, 8]),
            victory: n(&[0]),
            defeat: n(&[1]),
            briefing: n(&[17]),
        }
    }
}

impl Playlists {
    pub fn get(&self, mood: Mood) -> &[TrackRef] {
        match mood {
            Mood::Peace => &self.peace,
            Mood::Battle => &self.battle,
            Mood::Victory => &self.victory,
            Mood::Defeat => &self.defeat,
            Mood::Briefing => &self.briefing,
        }
    }
}

/// Music behaviour, loadable from TOML:
///
/// ```toml
/// crossfade = 3.0      # seconds, when the mood changes
/// fade_in = 1.5        # seconds, when music starts from silence
/// gap = 2.0            # seconds of silence between tracks of the same mood
/// shuffle = true       # false plays each list in order
/// calm_delay = 8.0     # seconds without battle before returning to peace music
/// battle_delay = 0.0   # seconds of battle before switching to battle music
///
/// [playlists]
/// peace = [11, 12, 13, 14, 15, 16]
/// battle = [2, 3, 4, 5, 6, 7, 8]
/// victory = [0]
/// defeat = [1]
/// briefing = [17, "music/custom.ogg"]
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicConfig {
    pub crossfade: f32,
    pub fade_in: f32,
    pub gap: f32,
    pub shuffle: bool,
    pub calm_delay: f32,
    pub battle_delay: f32,
    pub playlists: Playlists,
}

impl Default for MusicConfig {
    fn default() -> Self {
        MusicConfig {
            crossfade: 3.0,
            fade_in: 1.5,
            gap: 2.0,
            shuffle: true,
            calm_delay: 8.0,
            battle_delay: 0.0,
            playlists: Playlists::default(),
        }
    }
}

impl MusicConfig {
    /// File name looked for in the user config directory.
    pub const FILE_NAME: &'static str = "music.toml";

    pub fn from_toml(text: &str) -> Result<MusicConfig> {
        let config: MusicConfig = toml::from_str(text).map_err(|e| AudioError::Config(e.to_string()))?;
        for (name, value) in config.durations() {
            if !value.is_finite() || value < 0.0 {
                return Err(AudioError::Config(format!("{name} must be a finite, non-negative number of seconds")));
            }
        }
        Ok(config)
    }

    fn durations(&self) -> [(&'static str, f32); 5] {
        [("crossfade", self.crossfade), ("fade_in", self.fade_in), ("gap", self.gap),
            ("calm_delay", self.calm_delay), ("battle_delay", self.battle_delay)]
    }

    /// Public callers can construct a config directly instead of parsing TOML.
    fn sanitize(&mut self) {
        let defaults = Self::default();
        for (value, (name, fallback)) in [&mut self.crossfade, &mut self.fade_in, &mut self.gap,
            &mut self.calm_delay, &mut self.battle_delay].into_iter().zip(defaults.durations()) {
            if !value.is_finite() || *value < 0.0 {
                log::warn!("invalid music {name}; using {fallback} seconds");
                *value = fallback;
            }
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn load(path: &Path) -> Result<MusicConfig> {
        MusicConfig::from_toml(&std::fs::read_to_string(path)?)
    }

    /// `<user config dir>/music.toml` if present and valid, else the defaults (logged).
    pub fn load_user() -> MusicConfig {
        let path = ta_config::config_dir().join(Self::FILE_NAME);
        if !path.is_file() {
            return MusicConfig::default();
        }
        match MusicConfig::load(&path) {
            Ok(c) => {
                log::info!("music config from {}", path.display());
                c
            }
            Err(e) => {
                log::warn!("{}: {e}; using default music settings", path.display());
                MusicConfig::default()
            }
        }
    }
}

/// A track played as a film's score ([`MusicPlayer::play_score`]): spans of it played in turn,
/// each join a short crossfade, so a song can be made longer on its bar lines (the credits' film
/// plays one twice its length). Seconds of the track; the score's own seconds ("arranged") run
/// from 0 to [`Arrangement::length`].
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    /// The spans of the track played in turn, `[from, to)` seconds.
    pub spans: Vec<(f64, f64)>,
    /// Seconds before each join over which the outgoing span fades out as the incoming one
    /// (from as far before its start) fades in, linearly: the join itself then plays the
    /// incoming span alone.
    pub crossfade: f64,
    /// The level it plays at (linear; the music volume on top).
    pub gain: f32,
}

impl Arrangement {
    /// The whole track as it is.
    pub fn whole(seconds: f64) -> Arrangement {
        Arrangement { spans: vec![(0.0, seconds.max(0.0))], crossfade: 0.0, gain: 1.0 }
    }

    /// The score's length (seconds).
    pub fn length(&self) -> f64 {
        self.spans.iter().map(|&(a, b)| (b - a).max(0.0)).sum()
    }

    /// Where the score's second `t` is in the track: the span and the track's second.
    pub fn source_at(&self, t: f64) -> Option<(usize, f64)> {
        let mut at = 0.0;
        for (k, &(a, b)) in self.spans.iter().enumerate() {
            let len = (b - a).max(0.0);
            if t < at + len {
                return Some((k, a + (t - at).max(0.0)));
            }
            at += len;
        }
        None
    }

    /// Every second of the score at which the track's second `s` plays (once a span holding it).
    pub fn arranged_at(&self, s: f64) -> Vec<f64> {
        let mut out = Vec::new();
        let mut at = 0.0;
        for &(a, b) in &self.spans {
            if (a..b).contains(&s) {
                out.push(at + s - a);
            }
            at += (b - a).max(0.0);
        }
        out
    }
}

/// Instructions for the engine.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MusicAction {
    /// Decode `path` from `from` seconds in and play it on `deck`, fading in over `fade_in`
    /// seconds; a score plays the arrangement's spans of it (`from` its own seconds).
    Start { deck: u8, stream: u64, path: String, from: f64, fade_in: f32, crossfade_from: Option<(u8, u64)>, score: Option<Arc<Arrangement>> },
    /// Fade `stream` out over `seconds` and release it.
    FadeOut { deck: u8, stream: u64, seconds: f32 },
}

#[derive(Clone, Debug)]
struct Playing {
    deck: u8,
    stream: u64,
    path: String,
    mood: Mood,
    /// A film's score: it plays once, and nothing follows it.
    score: bool,
}

/// A score asked for, started at the next update.
#[derive(Clone, Debug)]
struct ScoreRequest {
    path: String,
    arrangement: Arc<Arrangement>,
    from: f64,
    fade_in: f32,
}

/// How quickly a track sought in crossfades from where it was (seconds).
const SEEK_FADE: f32 = 0.12;

/// Chooses and sequences music tracks. Owned by [`crate::AudioEngine`]; see
/// [`crate::AudioEngine::music`].
#[derive(Debug)]
pub struct MusicPlayer {
    config: MusicConfig,
    /// Music files that exist (VFS paths).
    available: Vec<String>,
    /// Resolved VFS paths per mood.
    playlists: [Vec<String>; 5],
    /// A track requested with [`MusicPlayer::play_track`], started at the next update; how far
    /// in it starts (a seek's), and how quickly it fades in when it is not the usual crossfade.
    forced: Option<String>,
    forced_from: f64,
    forced_fade: Option<f32>,
    /// The track that ended, to be played again ([`MusicPlayer::set_repeat`]).
    again: Option<String>,
    repeat: bool,
    desired: Option<Mood>,
    /// Clock time when `desired` last changed.
    requested_at: f64,
    current: Option<Playing>,
    /// Streams fading out: (deck, stream).
    fading: Vec<(u8, u64)>,
    clock: f64,
    /// When the next track may start after a natural end (the gap).
    next_start_at: Option<f64>,
    /// The one-shot mood (victory/defeat) has played.
    finished_one_shot: bool,
    last_index: [Option<usize>; 5],
    bad_tracks: HashSet<String>,
    next_stream: u64,
    rng: Rng,
    actions: Vec<MusicAction>,
    /// The player's own tracks (`file:` paths) and how they play with TA's.
    user_tracks: Vec<String>,
    user_mode: ta_config::UserMusic,
    /// The tracks started, newest last (for "previous").
    history: Vec<String>,
    /// The music given over to a film's score ([`MusicPlayer::hush`], [`MusicPlayer::play_score`]):
    /// no mood plays, the score asked for starts at the next update.
    scoring: bool,
    score_request: Option<ScoreRequest>,
}

impl MusicPlayer {
    /// A player whose playlists are resolved against the files in the music directory.
    pub fn new(config: MusicConfig, available: &[String]) -> MusicPlayer {
        let mut player = MusicPlayer {
            config: MusicConfig::default(),
            available: Vec::new(),
            playlists: Default::default(),
            forced: None,
            forced_from: 0.0,
            forced_fade: None,
            again: None,
            repeat: false,
            desired: None,
            requested_at: 0.0,
            current: None,
            fading: Vec::new(),
            clock: 0.0,
            next_start_at: None,
            finished_one_shot: false,
            last_index: [None; 5],
            bad_tracks: HashSet::new(),
            next_stream: 1,
            rng: Rng::from_time(),
            actions: Vec::new(),
            user_tracks: Vec::new(),
            user_mode: ta_config::UserMusic::Mixed,
            history: Vec::new(),
            scoring: false,
            score_request: None,
        };
        player.set_config(config, available);
        player
    }

    #[cfg(test)]
    pub(crate) fn with_seed(mut self, seed: u64) -> MusicPlayer {
        self.rng = Rng::new(seed);
        self
    }

    /// Replaces the configuration; takes effect with the next track.
    pub fn set_config(&mut self, mut config: MusicConfig, available: &[String]) {
        config.sanitize();
        for mood in Mood::ALL {
            let list: Vec<String> =
                config.playlists.get(mood).iter().filter_map(|t| resolve_track(t, available)).collect();
            if list.is_empty() && !config.playlists.get(mood).is_empty() {
                log::warn!("no music files found for the {mood:?} playlist");
            }
            self.playlists[mood.index()] = list;
        }
        // The player's own music in the peace, battle and menu playlists, mixed in or in TA's
        // place (victory and defeat keep TA's).
        if self.user_mode != ta_config::UserMusic::Off && !self.user_tracks.is_empty() {
            for mood in [Mood::Peace, Mood::Battle, Mood::Briefing] {
                let list = &mut self.playlists[mood.index()];
                if self.user_mode == ta_config::UserMusic::Only {
                    list.clear();
                }
                list.extend(self.user_tracks.iter().cloned());
            }
        }
        self.last_index = [None; 5];
        self.config = config;
        self.available = available.to_vec();
    }

    pub fn config(&self) -> &MusicConfig {
        &self.config
    }

    /// The player's own tracks (files) and how they play with TA's; takes effect with the next
    /// track.
    pub fn set_user_tracks(&mut self, tracks: &[std::path::PathBuf], mode: ta_config::UserMusic) {
        self.user_tracks = tracks.iter().map(|p| format!("file:{}", p.display())).collect();
        self.user_mode = mode;
        let (config, available) = (self.config.clone(), self.available.clone());
        self.set_config(config, &available);
    }

    /// The track before the one playing, again. False when there was none.
    pub fn previous(&mut self) -> bool {
        if self.history.len() < 2 {
            return false;
        }
        self.history.pop();
        let Some(path) = self.history.pop() else { return false };
        if self.desired.is_none() {
            self.set_mood(Mood::Briefing);
        }
        self.forced = Some(path);
        true
    }

    /// Whether music is playing (not stopped).
    pub fn playing(&self) -> bool {
        self.desired.is_some()
    }

    /// Gives the music over to a film's score: what plays (a score too) fades out over `seconds`,
    /// and no mood plays until [`MusicPlayer::end_score`] (the moods asked for meanwhile are kept
    /// for then). The jukebox choosing a track takes the music back.
    pub fn hush(&mut self, seconds: f32) {
        self.scoring = true;
        self.score_request = None;
        self.forced = None;
        self.again = None;
        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { self.config.crossfade };
        if let Some(current) = self.current.take() {
            self.fade_out(current.deck, current.stream, seconds);
        }
        self.next_start_at = None;
    }

    /// Plays `track` (a VFS path) as a film's score, as `arrangement` has it, from its own second
    /// `from`, fading in over `fade_in` (whatever plays fades out as quickly); once, nothing after
    /// it. False when no such music file exists.
    pub fn play_score(&mut self, track: &str, arrangement: Arc<Arrangement>, from: f64, fade_in: f32) -> bool {
        let Some(path) = resolve_track(&TrackRef::Path(track.to_string()), &self.available) else {
            log::warn!("score '{track}' not found");
            return false;
        };
        self.scoring = true;
        self.forced = None;
        self.again = None;
        let from = if from.is_finite() { from.max(0.0) } else { 0.0 };
        let fade_in = if fade_in.is_finite() { fade_in.max(0.0) } else { 0.0 };
        self.score_request = Some(ScoreRequest { path, arrangement, from, fade_in });
        true
    }

    /// The film is over: its score fades out over `seconds` and the moods' music comes back (as
    /// from silence, fading in).
    pub fn end_score(&mut self, seconds: f32) {
        if !self.scoring {
            return;
        }
        self.scoring = false;
        self.score_request = None;
        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { self.config.crossfade };
        if let Some(current) = self.current.take() {
            self.fade_out(current.deck, current.stream, seconds);
        }
        self.next_start_at = None;
        self.finished_one_shot = false;
    }

    /// Whether the music is given over to a film's score ([`MusicPlayer::hush`]).
    pub fn scoring(&self) -> bool {
        self.scoring
    }

    /// The stream of the score playing (or starting), if one is.
    pub(crate) fn score_stream(&self) -> Option<u64> {
        self.current.as_ref().filter(|c| c.score).map(|c| c.stream)
    }

    /// Whether a score was asked for and has not started yet.
    pub fn score_pending(&self) -> bool {
        self.score_request.is_some()
    }

    /// Plays the track playing again from `seconds` in (a quick crossfade). False when none
    /// plays, or a film's score does (it is the film's clock).
    pub fn seek(&mut self, seconds: f64) -> bool {
        if self.current.as_ref().is_some_and(|c| c.score) {
            return false;
        }
        let Some(path) = self.current.as_ref().map(|c| c.path.clone()) else { return false };
        self.forced = Some(path);
        self.forced_from = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self.forced_fade = Some(SEEK_FADE);
        true
    }

    /// Each track, once it has played to its end, plays again (until another is chosen or the
    /// mood changes).
    pub fn set_repeat(&mut self, repeat: bool) {
        self.repeat = repeat;
        if !repeat {
            self.again = None;
        }
    }

    pub fn repeat(&self) -> bool {
        self.repeat
    }

    /// The next track is chosen at random (else the playlist's next); takes effect with it.
    pub fn set_shuffle(&mut self, shuffle: bool) {
        self.config.shuffle = shuffle;
    }

    pub fn shuffle(&self) -> bool {
        self.config.shuffle
    }

    /// Resolved VFS paths of a mood's playlist.
    pub fn playlist(&self, mood: Mood) -> &[String] {
        &self.playlists[mood.index()]
    }

    /// Requests a mood. Battle music starts after `battle_delay`, peace music returns only
    /// after `calm_delay` without battle; other changes crossfade at the next update.
    pub fn set_mood(&mut self, mood: Mood) {
        if self.desired != Some(mood) {
            self.desired = Some(mood);
            self.requested_at = self.clock;
            self.finished_one_shot = false;
            self.again = None;
            // A new scene/mood must not inherit the previous playlist's silence.
            self.next_start_at = None;
        }
    }

    /// The requested mood (`None` when stopped).
    pub fn mood(&self) -> Option<Mood> {
        self.desired
    }

    /// Fades the music out and stays silent until the next [`set_mood`](Self::set_mood).
    pub fn stop(&mut self) {
        self.stop_with_fade(self.config.crossfade);
    }

    /// Stops every music deck over `seconds`, including a track already fading out.
    /// Useful when handing the soundtrack over to a movie or another audio source.
    pub fn stop_with_fade(&mut self, seconds: f32) {
        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { self.config.crossfade };
        self.desired = None;
        self.forced = None;
        self.again = None;
        // (A film's score too: the player stopped the music.)
        self.scoring = false;
        self.score_request = None;
        if let Some(current) = self.current.take() {
            self.fading.push((current.deck, current.stream));
        }
        for &(deck, stream) in &self.fading {
            self.actions.push(MusicAction::FadeOut { deck, stream, seconds });
        }
        self.next_start_at = None;
    }

    /// Crossfades to another track of the current mood.
    pub fn skip(&mut self) {
        if let Some(mood) = self.desired {
            // Queue a replacement even during a playlist gap or after a one-shot ends.
            self.forced = self.pick(mood);
        }
    }

    /// Plays a specific track now, crossfading from the current one; afterwards the current
    /// mood's playlist continues (music starts in [`Mood::Briefing`] if it was stopped).
    /// `track` is a track number (`"7"` → `music/7.mp3`) or a VFS path. Returns `false` if no
    /// such music file exists.
    pub fn play_track(&mut self, track: &str) -> bool {
        let reference = match track.trim().parse::<u32>() {
            Ok(n) => TrackRef::Number(n),
            Err(_) => TrackRef::Path(track.to_string()),
        };
        match resolve_track(&reference, &self.available) {
            Some(path) => {
                if self.desired.is_none() {
                    self.set_mood(Mood::Briefing);
                }
                self.forced = Some(path);
                true
            }
            None => {
                log::warn!("music track '{track}' not found");
                false
            }
        }
    }

    /// Path of the track currently playing (or starting).
    pub fn now_playing(&self) -> Option<&str> {
        self.current.as_ref().map(|c| c.path.as_str())
    }

    /// The stream of the track currently playing (or starting).
    pub(crate) fn now_playing_stream(&self) -> Option<u64> {
        self.current.as_ref().map(|c| c.stream)
    }

    /// Mood of the track currently playing.
    pub fn playing_mood(&self) -> Option<Mood> {
        self.current.as_ref().map(|c| c.mood)
    }

    pub(crate) fn take_actions(&mut self) -> Vec<MusicAction> {
        std::mem::take(&mut self.actions)
    }

    pub(crate) fn update(&mut self, dt: f64) {
        self.clock += dt.max(0.0);
        if self.scoring {
            if let Some(score) = self.score_request.take() {
                // The score in place of whatever plays (a score before it too), as quickly as it
                // fades in.
                let previous = self.current.take();
                if let Some(p) = &previous {
                    self.fade_out(p.deck, p.stream, score.fade_in.max(0.02));
                }
                self.next_start_at = None;
                self.start_score(score, previous.map(|p| p.deck));
                return;
            }
            if self.forced.is_none() {
                // (Nothing more: the score plays out, then silence.)
                return;
            }
            // The jukebox chose a track: the music is the player's again.
            self.scoring = false;
        }
        let Some(want) = self.desired else {
            if let Some(current) = self.current.take() {
                self.fade_out(current.deck, current.stream, self.config.crossfade);
            }
            self.next_start_at = None;
            return;
        };
        if let Some(path) = self.forced.take() {
            let previous = self.current.take();
            if let Some(p) = &previous {
                self.fading.push((p.deck, p.stream));
            }
            let fade = if previous.is_some() { self.config.crossfade } else { self.config.fade_in };
            let fade = self.forced_fade.take().unwrap_or(fade);
            let from = std::mem::take(&mut self.forced_from);
            self.next_start_at = None;
            self.finished_one_shot = false;
            self.again = None;
            self.start_path_from(path, want, from, fade, previous.map(|p| p.deck));
            return;
        }
        match self.current.clone() {
            None => {
                if self.finished_one_shot {
                    return;
                }
                let fade = match self.next_start_at {
                    Some(t) if self.clock < t => return,
                    // Continuing a playlist after a gap: no fade, tracks have their own intros.
                    Some(_) => 0.0,
                    None => self.config.fade_in,
                };
                self.next_start_at = None;
                match self.again.take() {
                    Some(path) => self.start_path(path, want, fade, None),
                    None => self.start_track(want, fade, None),
                }
            }
            Some(current) if current.mood != want => {
                let delay = match (current.mood, want) {
                    (_, Mood::Battle) => self.config.battle_delay,
                    (Mood::Battle, Mood::Peace) => self.config.calm_delay,
                    _ => 0.0,
                };
                if self.clock - self.requested_at >= delay as f64 {
                    // A shared song still suits the requested mood. In particular, the
                    // player's own playlist should not restart whenever fighting begins.
                    if current.mood.loops() && want.loops() && self.playlist(want).contains(&current.path) {
                        self.last_index[want.index()] = self.playlist(want).iter().position(|p| p == &current.path);
                        self.current.as_mut().unwrap().mood = want;
                        return;
                    }
                    self.current = None;
                    if let Some(path) = self.pick(want) {
                        self.fading.push((current.deck, current.stream));
                        self.start_path(path, want, self.config.crossfade, Some(current.deck));
                    } else {
                        self.fade_out(current.deck, current.stream, self.config.crossfade);
                    }
                }
            }
            Some(_) => {}
        }
    }

    /// A deck let go of a stream: `natural` if it played to the end, `failed` if it could not
    /// be decoded.
    pub(crate) fn on_stream_ended(&mut self, stream: u64, natural: bool, failed: bool) {
        self.fading.retain(|&(_, s)| s != stream);
        let Some(current) = self.current.clone().filter(|c| c.stream == stream) else { return };
        self.current = None;
        if current.score {
            // (A score plays once: silence after it, until the film is over.)
            if failed {
                log::warn!("score {} could not be played", current.path);
            }
            return;
        }
        if failed {
            log::warn!("music track {} failed; skipping it from now on", current.path);
            self.bad_tracks.insert(current.path);
            self.next_start_at = Some(self.clock);
        } else if self.desired != Some(current.mood) {
            // The callback may report the old track's end after a new mood was requested.
            // Its one-shot completion or gap must not suppress the replacement mood.
            self.next_start_at = None;
        } else if natural {
            if current.mood.loops() {
                self.next_start_at = Some(self.clock + self.config.gap as f64);
                if self.repeat {
                    self.again = Some(current.path);
                }
            } else {
                self.finished_one_shot = true;
            }
        } else {
            // Released unexpectedly (e.g. replaced); start over at the next update.
            self.next_start_at = Some(self.clock);
        }
    }

    /// The output was re-created: every stream is gone. Restart the mood on the next update.
    pub(crate) fn on_output_reset(&mut self) {
        self.current = None;
        self.fading.clear();
        self.next_start_at = None;
    }

    fn fade_out(&mut self, deck: u8, stream: u64, seconds: f32) {
        self.fading.push((deck, stream));
        self.actions.push(MusicAction::FadeOut { deck, stream, seconds });
    }

    fn start_track(&mut self, mood: Mood, fade_in: f32, avoid_deck: Option<u8>) {
        if let Some(path) = self.pick(mood) {
            self.start_path(path, mood, fade_in, avoid_deck);
        }
    }

    fn start_path(&mut self, path: String, mood: Mood, fade_in: f32, avoid_deck: Option<u8>) {
        self.start_path_from(path, mood, 0.0, fade_in, avoid_deck);
    }

    fn start_path_from(&mut self, path: String, mood: Mood, from: f64, fade_in: f32, avoid_deck: Option<u8>) {
        self.start_on_deck(path, mood, from, fade_in, avoid_deck, None);
    }

    fn start_score(&mut self, score: ScoreRequest, avoid_deck: Option<u8>) {
        // (Its mood is the one asked for meanwhile, for when the film is over.)
        let mood = self.desired.unwrap_or(Mood::Briefing);
        self.start_on_deck(score.path, mood, score.from, score.fade_in, avoid_deck, Some(score.arrangement));
    }

    fn start_on_deck(&mut self, path: String, mood: Mood, from: f64, fade_in: f32, avoid_deck: Option<u8>, score: Option<Arc<Arrangement>>) {
        // Use a deck that is not playing; a deck still fading is taken over (the mixer releases
        // the old stream when a new one starts on it).
        let busy: Vec<u8> = self.fading.iter().map(|&(d, _)| d).chain(avoid_deck).collect();
        let deck = (0..crate::mixer::DECKS as u8).find(|d| !busy.contains(d)).unwrap_or_else(|| {
            let d = avoid_deck.map_or(0, |a| (a + 1) % crate::mixer::DECKS as u8);
            self.fading.retain(|&(fd, _)| fd != d);
            d
        });
        let stream = self.next_stream;
        self.next_stream += 1;
        self.current = Some(Playing { deck, stream, path: path.clone(), mood, score: score.is_some() });
        if score.is_none()
            && let Some(index) = self.playlist(mood).iter().position(|p| p == &path)
        {
            self.last_index[mood.index()] = Some(index);
        }
        // (A track sought in, or played again, is still the one track in the history.)
        if self.history.last() != Some(&path) {
            self.history.push(path.clone());
        }
        if self.history.len() > 50 {
            self.history.remove(0);
        }
        // The mixer keeps the outgoing deck audible until this decoder is primed. A
        // failed replacement can leave that deck playing while the next track is tried.
        let crossfade_from = self.fading.last().copied();
        self.actions.push(MusicAction::Start { deck, stream, path, from, fade_in, crossfade_from, score });
    }

    fn pick(&mut self, mood: Mood) -> Option<String> {
        let list: Vec<(usize, &String)> =
            self.playlists[mood.index()].iter().enumerate().filter(|(_, p)| !self.bad_tracks.contains(*p)).collect();
        if list.is_empty() {
            return None;
        }
        let last = self.last_index[mood.index()];
        let choice = if self.config.shuffle {
            let options: Vec<&(usize, &String)> =
                list.iter().filter(|(i, _)| list.len() == 1 || Some(*i) != last).collect();
            *options[self.rng.below(options.len())]
        } else {
            let after = last.map_or(0, |l| l + 1);
            *list.iter().find(|(i, _)| *i >= after).unwrap_or(&list[0])
        };
        self.last_index[mood.index()] = Some(choice.0);
        Some(choice.1.clone())
    }
}

/// Resolves a playlist entry against the available files: a numbered track as GOG's copy
/// names it (`music/7.mp3`), else as another names it (`music/07.ogg`, `music/Track 07.ogg`).
fn resolve_track(track: &TrackRef, available: &[String]) -> Option<String> {
    match track {
        TrackRef::Number(n) => ["mp3", "ogg"]
            .iter()
            .map(|ext| format!("music/{n}.{ext}"))
            .find(|p| available.iter().any(|a| a == p))
            .or_else(|| available.iter().find(|a| track_number(a) == Some(*n)).cloned()),
        // The player's own music: a file on disk.
        TrackRef::Path(p) if p.starts_with("file:") => std::path::Path::new(&p[5..]).is_file().then(|| p.clone()),
        TrackRef::Path(p) => {
            let p = ta_vfs::normalize(p);
            available.contains(&p).then_some(p)
        }
    }
}

/// The number a file of TA's music folder stands for, however a copy names its tracks: a number,
/// perhaps after "track" (`music/7.mp3`, `music/07.ogg`, `music/track07.mp3`, `music/Track 07.ogg`,
/// `music/track_07.wav`); none for any other file.
fn track_number(path: &str) -> Option<u32> {
    let name = path.strip_prefix("music/").filter(|rest| !rest.contains('/'))?;
    let (stem, ext) = name.rsplit_once('.')?;
    if !["mp3", "ogg", "wav", "flac"].iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        return None;
    }
    let stem = stem.to_ascii_lowercase();
    let digits = stem.trim_start_matches("track").trim_start_matches([' ', '_', '-', '.']);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_tracks_are_found_however_a_copy_names_them() {
        let gog: Vec<String> = (0..18).map(|i| format!("music/{i}.mp3")).collect();
        assert_eq!(resolve_track(&TrackRef::Number(7), &gog).as_deref(), Some("music/7.mp3"));
        for names in [["music/07.ogg", "music/11.ogg"], ["music/Track 07.ogg", "music/Track 11.ogg"], ["music/track07.mp3", "music/track11.mp3"], ["music/TRACK_07.WAV", "music/TRACK_11.WAV"]] {
            let available: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            assert_eq!(resolve_track(&TrackRef::Number(7), &available).as_deref(), Some(names[0]), "{names:?}");
            assert_eq!(resolve_track(&TrackRef::Number(11), &available).as_deref(), Some(names[1]), "{names:?}");
            assert_eq!(resolve_track(&TrackRef::Number(3), &available), None);
        }
        // Not a track: any other name, a file in a folder of its own, another kind of file.
        for other in ["music/custom.ogg", "music/7 remix.mp3", "music/live/07.ogg", "music/07.txt", "sounds/07.wav"] {
            assert_eq!(track_number(other), None, "{other}");
        }
    }

    fn files() -> Vec<String> {
        let mut v: Vec<String> = (0..18).map(|i| format!("music/{i}.mp3")).collect();
        v.push("music/custom.ogg".into());
        v
    }

    fn player() -> MusicPlayer {
        MusicPlayer::new(MusicConfig::default(), &files()).with_seed(7)
    }

    fn starts(actions: &[MusicAction]) -> Vec<(u8, u64, String, f32)> {
        actions
            .iter()
            .filter_map(|a| match a {
                MusicAction::Start { deck, stream, path, fade_in, .. } => Some((*deck, *stream, path.clone(), *fade_in)),
                _ => None,
            })
            .collect()
    }

    /// The jukebox: a seek plays the same track again from further in, quickly, and leaves the
    /// history alone; repeat plays a track that ended again, until the mood changes.
    #[test]
    fn seeking_and_repeating_play_the_same_track() {
        let mut p = player();
        p.set_mood(Mood::Peace);
        p.update(0.016);
        let first = starts(&p.take_actions())[0].clone();
        assert!(p.seek(83.5));
        p.update(0.016);
        let actions = p.take_actions();
        let sought: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                MusicAction::Start { path, from, fade_in, crossfade_from, .. } => Some((path.clone(), *from, *fade_in, *crossfade_from)),
                _ => None,
            })
            .collect();
        assert_eq!(sought, vec![(first.2.clone(), 83.5, SEEK_FADE, Some((first.0, first.1)))]);
        assert_eq!(p.history, vec![first.2.clone()]);
        // Repeat: the track that played to its end comes round again after the gap.
        p.set_repeat(true);
        let stream = p.now_playing_stream().unwrap();
        p.on_stream_ended(stream, true, false);
        p.update(f64::from(MusicConfig::default().gap) + 0.1);
        assert_eq!(starts(&p.take_actions()).iter().map(|s| s.2.clone()).collect::<Vec<_>>(), vec![first.2.clone()]);
        // Not across a change of mood.
        let stream = p.now_playing_stream().unwrap();
        p.set_mood(Mood::Battle);
        p.on_stream_ended(stream, true, false);
        p.update(10.0);
        assert!(starts(&p.take_actions()).iter().all(|s| s.2 != first.2), "the battle music, not the peace track");
        // Nothing to seek in while silent.
        p.stop();
        p.update(0.1);
        assert!(!p.seek(10.0));
    }

    #[test]
    fn default_playlists_cover_the_ta_tracks() {
        let p = player();
        assert_eq!(p.playlist(Mood::Battle).len(), 7);
        assert_eq!(p.playlist(Mood::Peace).len(), 6);
        assert_eq!(p.playlist(Mood::Victory), &["music/0.mp3".to_string()]);
        assert_eq!(p.playlist(Mood::Defeat), &["music/1.mp3".to_string()]);
        assert_eq!(p.playlist(Mood::Briefing), &["music/17.mp3".to_string()]);
        // 9 and 10 duplicate 0 and 1 and are not used by default.
        let all: Vec<&String> = Mood::ALL.iter().flat_map(|m| p.playlist(*m)).collect();
        assert!(!all.iter().any(|t| *t == "music/9.mp3" || *t == "music/10.mp3"));
        assert_eq!(all.len(), 16);
    }

    #[test]
    fn toml_overrides_and_keeps_defaults() {
        let text = "crossfade = 5.0\nshuffle = false\n[playlists]\nbattle = [3, \"music/custom.ogg\", 99]\n";
        let cfg = MusicConfig::from_toml(text).unwrap();
        assert_eq!(cfg.crossfade, 5.0);
        assert!(!cfg.shuffle);
        assert_eq!(cfg.gap, MusicConfig::default().gap);
        assert_eq!(cfg.playlists.peace, Playlists::default().peace);
        let p = MusicPlayer::new(cfg.clone(), &files());
        assert_eq!(p.playlist(Mood::Battle), &["music/3.mp3".to_string(), "music/custom.ogg".to_string()]);
        assert!(MusicConfig::from_toml("crossfade = \"soon\"").is_err());
        assert_eq!(MusicConfig::from_toml(&cfg.to_toml()).unwrap(), cfg);
        // .ogg is found when there is no .mp3.
        let p = MusicPlayer::new(MusicConfig::default(), &["music/2.ogg".to_string()]);
        assert_eq!(p.playlist(Mood::Battle), &["music/2.ogg".to_string()]);
    }

    #[test]
    fn mood_changes_crossfade_with_hysteresis() {
        let mut p = player();
        p.set_mood(Mood::Peace);
        p.update(0.016);
        let a = p.take_actions();
        let s = starts(&a);
        assert_eq!(s.len(), 1);
        let (deck0, stream0, path0, fade0) = s[0].clone();
        assert!(p.playlist(Mood::Peace).contains(&path0));
        assert_eq!(fade0, 1.5);

        // Battle starts immediately with a crossfade on the other deck.
        p.set_mood(Mood::Battle);
        p.update(0.016);
        let a = p.take_actions();
        assert!(a.iter().any(|a| matches!(a, MusicAction::Start { crossfade_from: Some(pair), .. }
            if *pair == (deck0, stream0))), "outgoing track waits for replacement to be primed");
        let s = starts(&a);
        assert_eq!(s.len(), 1);
        assert_ne!(s[0].0, deck0);
        assert!(p.playlist(Mood::Battle).contains(&s[0].2));
        assert_eq!(p.playing_mood(), Some(Mood::Battle));

        // A brief lull does not bring back peace music...
        p.set_mood(Mood::Peace);
        for _ in 0..60 {
            p.update(0.1);
        }
        assert!(p.take_actions().is_empty());
        p.set_mood(Mood::Battle);
        p.set_mood(Mood::Peace);
        // ... but eight quiet seconds do.
        for _ in 0..85 {
            p.update(0.1);
        }
        let s = starts(&p.take_actions());
        assert_eq!(s.len(), 1);
        assert!(p.playlist(Mood::Peace).contains(&s[0].2));
    }

    #[test]
    fn playlist_advances_after_a_gap_and_one_shots_stop() {
        let mut p = player();
        p.set_mood(Mood::Battle);
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        p.on_stream_ended(first.1, true, false);
        p.update(1.0);
        assert!(p.take_actions().is_empty(), "gap");
        p.update(1.5);
        let second = starts(&p.take_actions())[0].clone();
        assert_ne!(second.2, first.2, "shuffle avoids an immediate repeat");
        assert_eq!(second.3, 0.0, "no fade after a gap");

        p.set_mood(Mood::Victory);
        p.update(0.0);
        let v = starts(&p.take_actions())[0].clone();
        assert_eq!(v.2, "music/0.mp3");
        p.on_stream_ended(v.1, true, false);
        for _ in 0..100 {
            p.update(0.1);
        }
        assert!(p.take_actions().is_empty(), "victory plays once");
        assert_eq!(p.now_playing(), None);
    }

    #[test]
    fn failed_tracks_are_skipped() {
        let cfg = MusicConfig { shuffle: false, ..MusicConfig::default() };
        let mut p = MusicPlayer::new(cfg, &files());
        p.set_mood(Mood::Battle);
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        assert_eq!(first.2, "music/2.mp3");
        p.on_stream_ended(first.1, false, true);
        p.update(0.0);
        let next = starts(&p.take_actions())[0].clone();
        assert_eq!(next.2, "music/3.mp3");
        assert!(!p.pick_all(Mood::Battle, 20).contains(&"music/2.mp3".to_string()));
    }

    #[test]
    fn explicit_tracks_crossfade_then_the_playlist_continues() {
        let mut p = player();
        assert!(!p.play_track("42"));
        assert!(p.play_track("7"));
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        assert_eq!((first.2.as_str(), first.3), ("music/7.mp3", 1.5));
        assert_eq!(p.mood(), Some(Mood::Briefing));
        assert!(p.play_track("music/custom.ogg"));
        p.update(0.0);
        let a = p.take_actions();
        assert!(a.iter().any(|a| matches!(a, MusicAction::Start { crossfade_from: Some(pair), .. }
            if *pair == (first.0, first.1))));
        let second = starts(&a)[0].clone();
        assert_eq!(second.2, "music/custom.ogg");
        p.on_stream_ended(second.1, true, false);
        p.update(5.0);
        assert_eq!(starts(&p.take_actions())[0].2, "music/17.mp3", "back to the briefing playlist");
    }

    #[test]
    fn stop_fades_out() {
        let mut p = player();
        p.set_mood(Mood::Briefing);
        p.update(0.0);
        let s = starts(&p.take_actions())[0].clone();
        p.stop();
        p.update(0.0);
        assert_eq!(p.take_actions(), vec![MusicAction::FadeOut { deck: s.0, stream: s.1, seconds: 3.0 }]);
        p.update(10.0);
        assert!(p.take_actions().is_empty());
    }

    #[test]
    fn music_durations_reject_nonfinite_and_negative_values() {
        for (name, _) in MusicConfig::default().durations() {
            for value in ["nan", "inf", "-1.0"] {
                let error = MusicConfig::from_toml(&format!("{name} = {value}")).unwrap_err();
                assert!(error.to_string().contains(name), "{error}");
            }
        }
        let cfg = MusicConfig { crossfade: f32::INFINITY, calm_delay: f32::NAN, gap: 0.5,
            fade_in: -1.0, ..Default::default() };
        let p = MusicPlayer::new(cfg, &files());
        assert_eq!(p.config.crossfade, 3.0);
        assert_eq!(p.config.calm_delay, 8.0);
        assert_eq!(p.config.fade_in, 1.5);
        assert_eq!(p.config.gap, 0.5, "valid values survive sanitization");
    }

    #[test]
    fn missing_track_does_not_resume_stopped_music() {
        let mut p = player();
        assert!(!p.play_track("missing.ogg"));
        p.update(1.0);
        assert!(!p.playing());
        assert!(p.take_actions().is_empty());
    }

    #[test]
    fn next_skips_the_gap_and_continues_after_a_manually_selected_track() {
        let cfg = MusicConfig { shuffle: false, ..Default::default() };
        let mut p = MusicPlayer::new(cfg, &files());
        p.set_mood(Mood::Battle);
        assert!(p.play_track("5"));
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        p.on_stream_ended(first.1, true, false);
        p.update(0.0);
        assert!(p.take_actions().is_empty());
        p.skip();
        p.update(0.0);
        assert_eq!(starts(&p.take_actions())[0].2, "music/6.mp3");
    }

    #[test]
    fn old_track_completion_does_not_block_a_new_mood() {
        for (old, new) in [(Mood::Victory, Mood::Peace), (Mood::Peace, Mood::Victory)] {
            let mut p = player();
            p.set_mood(old);
            p.update(0.0);
            let first = starts(&p.take_actions())[0].clone();
            // The game requests a scene change before draining the audio callback's event.
            p.set_mood(new);
            p.on_stream_ended(first.1, true, false);
            p.update(0.0);
            assert_eq!(starts(&p.take_actions()).len(), 1);
            assert_eq!(p.playing_mood(), Some(new));
        }
        let mut p = player();
        p.set_mood(Mood::Peace);
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        p.on_stream_ended(first.1, true, false);
        p.set_mood(Mood::Victory);
        p.update(0.0);
        assert_eq!(p.playing_mood(), Some(Mood::Victory), "new mood does not wait out an old gap");
    }

    #[test]
    fn shared_playlist_song_survives_battle_and_calm_transitions() {
        let mut cfg = MusicConfig::default();
        cfg.playlists.peace = vec![TrackRef::Number(11)];
        cfg.playlists.battle = vec![TrackRef::Number(11)];
        let mut p = MusicPlayer::new(cfg, &files());
        p.set_mood(Mood::Peace);
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        p.set_mood(Mood::Battle);
        p.update(0.0);
        assert!(p.take_actions().is_empty());
        assert_eq!(p.playing_mood(), Some(Mood::Battle));
        p.set_mood(Mood::Peace);
        p.update(4.0);
        assert_eq!(p.playing_mood(), Some(Mood::Battle), "calm hysteresis remains");
        p.update(4.0);
        assert!(p.take_actions().is_empty());
        assert_eq!(p.playing_mood(), Some(Mood::Peace));
        assert_eq!(p.now_playing_stream(), Some(first.1), "song position is preserved");
    }

    #[test]
    fn stop_with_short_fade_releases_both_crossfade_decks() {
        let mut p = player();
        p.set_mood(Mood::Peace);
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        p.set_mood(Mood::Battle);
        p.update(0.0);
        let second = starts(&p.take_actions())[0].clone();
        p.stop_with_fade(0.15);
        p.update(0.0);
        let actions = p.take_actions();
        for (deck, stream, _, _) in [first, second] {
            assert!(actions.contains(&MusicAction::FadeOut { deck, stream, seconds: 0.15 }));
        }
        assert_eq!(actions.len(), 2);
        assert!(!p.playing());
    }

    #[test]
    fn failed_replacement_keeps_the_outgoing_deck_for_the_retry() {
        let mut p = player();
        p.set_mood(Mood::Peace);
        p.update(0.0);
        let first = starts(&p.take_actions())[0].clone();
        p.set_mood(Mood::Battle);
        p.update(0.0);
        let failed = starts(&p.take_actions())[0].clone();
        p.on_stream_ended(failed.1, false, true);
        p.update(0.0);
        let actions = p.take_actions();
        assert!(actions.iter().any(|a| matches!(a, MusicAction::Start { crossfade_from: Some(pair), .. }
            if *pair == (first.0, first.1))));
        assert_ne!(starts(&actions)[0].2, failed.2);
    }

    /// A film's score: the menu's music steps aside, the score plays once as arranged, nothing
    /// after it; the film over, the menu's music comes back. The jukebox choosing a track takes
    /// the music back from it.
    #[test]
    fn a_score_plays_once_in_the_moods_place_then_gives_the_music_back() {
        let arrangement = Arc::new(Arrangement { spans: vec![(0.0, 10.0), (5.0, 20.0)], crossfade: 0.01, gain: 0.7 });
        assert!((arrangement.length() - 25.0).abs() < 1e-9);
        assert_eq!(arrangement.source_at(12.0), Some((1, 7.0)));
        assert_eq!(arrangement.arranged_at(6.0), vec![6.0, 11.0]);
        let mut p = player();
        p.set_mood(Mood::Briefing);
        p.update(0.0);
        let menu = starts(&p.take_actions())[0].clone();
        p.hush(1.0);
        p.update(0.016);
        assert_eq!(p.take_actions(), vec![MusicAction::FadeOut { deck: menu.0, stream: menu.1, seconds: 1.0 }]);
        p.update(10.0);
        assert!(p.take_actions().is_empty(), "silence while the film comes");
        assert!(!p.play_score("music/missing.ogg", arrangement.clone(), 0.0, 0.0));
        assert!(p.play_score("music/custom.ogg", arrangement.clone(), 3.5, 0.0));
        p.update(0.016);
        let actions = p.take_actions();
        let score: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                MusicAction::Start { path, from, fade_in, score, stream, .. } => Some((path.clone(), *from, *fade_in, score.clone(), *stream)),
                _ => None,
            })
            .collect();
        assert_eq!(score.len(), 1);
        assert_eq!((score[0].0.as_str(), score[0].1, score[0].2), ("music/custom.ogg", 3.5, 0.0));
        assert_eq!(score[0].3.as_deref(), Some(&*arrangement));
        assert_eq!(p.score_stream(), Some(score[0].4));
        assert!(!p.seek(1.0), "the score is the film's clock");
        // It plays out: silence, however long.
        p.on_stream_ended(score[0].4, true, false);
        p.update(30.0);
        assert!(p.take_actions().is_empty());
        assert_eq!(p.score_stream(), None);
        // The film over: the menu's music again, fading in.
        p.end_score(0.8);
        p.update(0.016);
        let back = starts(&p.take_actions());
        assert_eq!(back.len(), 1);
        assert!(p.playlist(Mood::Briefing).contains(&back[0].2));
        assert_eq!(back[0].3, MusicConfig::default().fade_in);
        // Left while it plays: it fades out as the music comes back.
        p.hush(0.5);
        assert!(p.play_score("music/custom.ogg", arrangement.clone(), 0.0, 0.0));
        p.update(0.016);
        let actions = p.take_actions();
        let stream = starts(&actions).last().unwrap().1;
        p.end_score(0.8);
        p.update(0.016);
        let actions = p.take_actions();
        assert!(actions.iter().any(|a| matches!(a, MusicAction::FadeOut { stream: s, seconds, .. } if *s == stream && (*seconds - 0.8).abs() < 1e-6)));
        // The jukebox's track takes the music back.
        p.hush(0.5);
        assert!(p.play_score("music/custom.ogg", arrangement, 0.0, 0.0));
        p.update(0.016);
        p.take_actions();
        assert!(p.play_track("7"));
        p.update(0.016);
        assert_eq!(starts(&p.take_actions())[0].2, "music/7.mp3");
        assert!(!p.scoring());
    }

    impl MusicPlayer {
        fn pick_all(&mut self, mood: Mood, n: usize) -> Vec<String> {
            (0..n).filter_map(|_| self.pick(mood)).collect()
        }
    }
}
