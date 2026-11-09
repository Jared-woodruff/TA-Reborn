//! `TA_SOUND_LOG=<file.jsonl>`: every sound the game plays, written down for films. A capture
//! (`--screenshot` on `TA_CAPTURE_FPS`'s fixed clock) saves pictures only; with this, each sound
//! it starts is kept on the capture's clock, as the camera of that moment hears it, so a film's
//! sound can be mixed again from the game's files in step with its frames
//! (`scratch/fan/sfx/mix.py`).
//!
//! Logging, the game's audio is mixed offline on the frames' clock, never on the speakers: each
//! frame mixes as much as it moves the game on, so sounds end, give up their voices and fade out
//! on the capture's clock as they would in play, with or without a sound card, muted or not. No
//! music plays.
//!
//! One JSON object a line, numbers in seconds and linear gains:
//!
//! * `{"ev":"log","version":1,"rate":48000,"fps":60,"master":0.8,"music":0.6,"effects":0.9,
//!   "voice":1,"max_voices":64}` first: the capture's frames a second (`null` off its fixed
//!   clock, `t` then being the frames' own seconds) and the player's volumes, which the gains
//!   below leave out.
//! * `{"t":2.016667,"frame":121,"ev":"play","id":17,"kind":"world","path":"sounds/xplolrg1.wav",
//!   "gain":0.83,"pan":-0.21,"gains":[0.66,0.5],"pitch":1,"loop":false,"at":0,"fade":0,
//!   "bus":"effects","pos":[1234.5,40,567]}`: a sound starts `t` seconds into the capture (frame
//!   `frame`, at `frame / fps`) on a mixer voice, which `id` names until it stops or plays out.
//!   `kind` is world (weapons, explosions, units at work, thunder), ambience (the wind), voice
//!   (units answering), alert, interface, narration or movie; `path` is the sound in the game's
//!   files; `gain` its level (the volume asked for times the distance's attenuation), `pan`
//!   where it is from -1 (left) to 1 (right), `gains` the left and right channels' factors (the
//!   pan law applied to `gain`, as the mixer plays it); `pitch` its playback rate; `at` the
//!   seconds into it it starts from and `fade` the seconds it fades in over (a loop resumed);
//!   `pos` where it is in the world (placed sounds). A sound the game makes itself (the wind)
//!   is in none of its files: `file` names a WAV of it beside the log.
//! * `{"t":..,"frame":..,"ev":"gains","id":17,"gain":..,"pan":..,"gains":[..,..]}`: new gains,
//!   reached over the mixer's next block (about 10 ms): the camera moved, or another request
//!   merged into the sound.
//! * `{"t":..,"frame":..,"ev":"pitch","id":17,"pitch":0.9}`: a new playback rate.
//! * `{"t":..,"frame":..,"ev":"stop","id":17,"fade":0.01}`: stopped, fading out (or its voice
//!   taken by a more important sound, or a loop gone out of earshot). One-shots that play out
//!   just end.
//! * A film's score (the credits' song, `credits_song.rs`; the logging engine plays no music
//!   itself): each span of it a `play` of the kind `music` (`bus` music) when it comes, `at` the
//!   seconds into the track it starts from and `fade` the crossfade it comes in over, and a
//!   `stop` over the crossfade before the next span, as the game's score stream plays them; at
//!   their exact times, `frame` then fractional on a capture's fixed clock (`frame / fps` is `t`),
//!   so `scratch/fan/sfx/mix.py --with music` joins the spans to the sample. Ids from 2^40.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use ta_audio::{AudioEngine, AudioSettings, Bus, PlayEvent, SoundData, SoundKind};

/// The offline mix's rate.
pub const RATE: u32 = 48_000;
/// Frames mixed at a time.
const CHUNK: u64 = 4096;

pub struct SoundLog {
    out: std::io::BufWriter<std::fs::File>,
    /// The folder sounds the game makes itself are written to, and its name beside the log.
    made_dir: PathBuf,
    made_name: String,
    /// Sounds' paths, and the file beside the log holding each one the game's files don't.
    made: HashMap<String, Option<String>>,
    /// The player's volumes, for the first line.
    settings: AudioSettings,
    started: bool,
    /// `TA_CAPTURE_FPS`.
    fps: Option<f64>,
    /// The frame the app is on, and whether the game moves on the capture's fixed clock.
    frame: u32,
    fixed: bool,
    /// Seconds of frames so far, off that clock.
    clock: f64,
    /// Frames of audio mixed so far, and the buffer they are mixed into (and dropped).
    mixed: u64,
    buffer: Vec<f32>,
}

impl SoundLog {
    /// The log `TA_SOUND_LOG` asks for, its file created.
    pub fn from_env(settings: &AudioSettings) -> Option<SoundLog> {
        let path = PathBuf::from(std::env::var_os("TA_SOUND_LOG").filter(|p| !p.is_empty())?);
        match SoundLog::create(&path, settings) {
            Ok(log) => {
                log::info!("sound log: {}", path.display());
                Some(log)
            }
            Err(e) => {
                log::warn!("sound log {}: {e}", path.display());
                None
            }
        }
    }

    fn create(path: &Path, settings: &AudioSettings) -> std::io::Result<SoundLog> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let stem = path.file_stem().map_or_else(|| "sound".to_string(), |s| s.to_string_lossy().into_owned());
        let made_name = format!("{stem}.sounds");
        Ok(SoundLog {
            out: std::io::BufWriter::new(std::fs::File::create(path)?),
            made_dir: path.with_file_name(&made_name),
            made_name,
            made: HashMap::new(),
            settings: settings.clone(),
            started: false,
            fps: std::env::var("TA_CAPTURE_FPS").ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|f| *f > 0.0),
            frame: 0,
            fixed: false,
            clock: 0.0,
            mixed: 0,
            buffer: Vec::new(),
        })
    }

    pub fn set_frame(&mut self, frame: u32, fixed: bool) {
        self.frame = frame;
        self.fixed = fixed;
    }

    /// The capture's frames a second, while the game moves on its fixed clock.
    fn fixed_fps(&self) -> Option<f64> {
        self.fps.filter(|_| self.fixed)
    }

    /// The log's time now, and that of the next frame (what starts before it is written now).
    pub fn now(&self) -> (f64, f64) {
        match self.fixed_fps() {
            Some(fps) => (self.frame as f64 / fps, (self.frame as f64 + 1.0) / fps),
            None => (self.clock, self.clock),
        }
    }

    /// A line's start at the exact time `t`: on a capture's fixed clock its frame is fractional
    /// (`frame / fps` is `t`), so a film's music joins to the sample.
    fn head_at(&mut self, t: f64) -> String {
        if !self.started {
            self.started = true;
            self.header();
        }
        let frame = match self.fixed_fps() {
            Some(fps) => num(t * fps, 6),
            None => self.frame.to_string(),
        };
        format!(r#"{{"t":{},"frame":{frame},"ev":"#, num(t, 6))
    }

    /// A span of a film's score starting at `t`: `path` from `at` seconds in, fading in over
    /// `fade` seconds, at `gain`, as music.
    fn music_play(&mut self, t: f64, id: u64, path: &str, at: f64, fade: f64, gain: f32) {
        let g = num(gain as f64, 5);
        let line = format!(
            r#"{}"play","id":{id},"kind":"music","path":{},"gain":{g},"pan":0,"gains":[{g},{g}],"pitch":1,"loop":false,"at":{},"fade":{},"bus":"music"}}"#,
            self.head_at(t),
            json_str(path),
            num(at, 6),
            num(fade, 6),
        );
        self.write(&line);
    }

    /// The span `id` fading out from `t` over `fade` seconds.
    fn music_stop(&mut self, t: f64, id: u64, fade: f64) {
        let line = format!(r#"{}"stop","id":{id},"fade":{}}}"#, self.head_at(t), num(fade, 6));
        self.write(&line);
    }

    fn write(&mut self, line: &str) {
        if let Err(e) = writeln!(self.out, "{line}") {
            log::warn!("sound log: {e}");
        }
    }

    /// Once a frame, after its sounds have started: writes what the mixer was told since the
    /// last frame at this frame's time, then mixes on to the next frame's.
    pub fn record(&mut self, engine: &mut AudioEngine, dt: f32) {
        let now = match self.fixed_fps() {
            Some(fps) => self.frame as f64 / fps,
            None => self.clock,
        };
        if !self.started {
            self.started = true;
            self.header();
        }
        for event in engine.take_log() {
            let line = self.line(&event, now, engine);
            if let Err(e) = writeln!(self.out, "{line}") {
                log::warn!("sound log: {e}");
            }
        }
        let next = match self.fixed_fps() {
            Some(fps) => (self.frame as f64 + 1.0) / fps,
            None => self.clock + dt.max(0.0) as f64,
        };
        self.clock = next;
        let until = (next * RATE as f64).round() as u64;
        while self.mixed < until {
            let n = (until - self.mixed).min(CHUNK) as usize;
            self.buffer.resize(n * 2, 0.0);
            engine.render_offline(&mut self.buffer[..n * 2]);
            self.mixed += n as u64;
        }
        let _ = self.out.flush();
    }

    fn header(&mut self) {
        let s = &self.settings;
        let fps = self.fixed_fps().map_or_else(|| "null".to_string(), |f| num(f, 6));
        let line = format!(
            r#"{{"ev":"log","version":1,"rate":{RATE},"fps":{fps},"master":{},"music":{},"effects":{},"voice":{},"max_voices":{}}}"#,
            num(s.master as f64, 3),
            num(s.music as f64, 3),
            num(s.effects as f64, 3),
            num(s.voice as f64, 3),
            s.max_voices,
        );
        if let Err(e) = writeln!(self.out, "{line}") {
            log::warn!("sound log: {e}");
        }
    }

    fn line(&mut self, event: &PlayEvent, now: f64, engine: &AudioEngine) -> String {
        let mut s = format!(r#"{{"t":{},"frame":{},"ev":"#, num(now, 6), self.frame);
        let gains = |g: [f32; 2]| format!("[{},{}]", num(g[0] as f64, 5), num(g[1] as f64, 5));
        match event {
            PlayEvent::Start(p) => {
                let bus = match p.bus {
                    Bus::Voice => "voice",
                    Bus::Music => "music",
                    Bus::Effects | Bus::Master => "effects",
                };
                let _ = write!(
                    s,
                    r#""play","id":{},"kind":"{}","path":{},"gain":{},"pan":{},"gains":{},"pitch":{},"loop":{},"at":{},"fade":{},"bus":"{bus}""#,
                    p.id,
                    p.kind.name(),
                    json_str(&p.path),
                    num(p.gain as f64, 5),
                    num(p.pan as f64, 5),
                    gains(p.gains),
                    num(p.pitch as f64, 5),
                    p.looping,
                    num(p.offset, 6),
                    num(p.fade_in as f64, 4),
                );
                if let Some([x, y, z]) = p.position {
                    let _ = write!(s, r#","pos":[{},{},{}]"#, num(x as f64, 1), num(y as f64, 1), num(z as f64, 1));
                }
                if let Some(file) = self.made_file(&p.path, &p.sound, p.kind, engine) {
                    let _ = write!(s, r#","file":{}"#, json_str(&file));
                }
            }
            PlayEvent::Gains { id, gain, pan, gains: g } => {
                let _ = write!(s, r#""gains","id":{id},"gain":{},"pan":{},"gains":{}"#, num(*gain as f64, 5), num(*pan as f64, 5), gains(*g));
            }
            PlayEvent::Pitch { id, pitch } => {
                let _ = write!(s, r#""pitch","id":{id},"pitch":{}"#, num(*pitch as f64, 5));
            }
            PlayEvent::Stop { id, fade } => {
                let _ = write!(s, r#""stop","id":{id},"fade":{}"#, num(*fade as f64, 4));
            }
        }
        s.push('}');
        s
    }

    /// For a sound in none of the game's files (one it makes itself, the wind), a WAV of it
    /// beside the log (once), named relative to the log's folder. Movies' soundtracks are left
    /// out (TA's own, and long).
    fn made_file(&mut self, path: &str, sound: &SoundData, kind: SoundKind, engine: &AudioEngine) -> Option<String> {
        if let Some(file) = self.made.get(path) {
            return file.clone();
        }
        let file = (kind != SoundKind::Movie && engine.bank().source().read(path).is_none()).then(|| {
            let name: String = path.rsplit('/').next().unwrap_or(path).chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
            let name = if name.to_ascii_lowercase().ends_with(".wav") { name } else { format!("{name}.wav") };
            (self.made_dir.join(&name), format!("{}/{name}", self.made_name))
        });
        let file = file.and_then(|(on_disk, named)| match std::fs::create_dir_all(&self.made_dir).and_then(|_| write_wav(&on_disk, sound)) {
            Ok(()) => Some(named),
            Err(e) => {
                log::warn!("sound log: {}: {e}", on_disk.display());
                None
            }
        });
        self.made.insert(path.to_string(), file.clone());
        file
    }
}

/// A film's score in the log (the logging engine plays no music): each span of its arrangement
/// written down as a sound of the kind "music" as it comes, the track from `at` seconds in, at
/// its exact time, fading in over the crossfade before its join and out over the one before the
/// next, as the game's own score stream plays it (`ta_audio`'s `stream::Arranged`), so
/// `scratch/fan/sfx/mix.py --with music` mixes the film's music to the sample.
pub struct ScoreLog {
    path: String,
    arrangement: std::sync::Arc<ta_audio::music::Arrangement>,
    /// The log's time of the score's second 0, and the second it started from.
    origin: f64,
    from: f64,
    fade_in: f64,
    /// The next span to write; the one playing (its id, when it starts fading out and over how
    /// long); ids given so far.
    next: usize,
    playing: Option<(u64, f64, f64)>,
    ids: u64,
}

/// Ids of the score's spans, apart from the mixer's.
const SCORE_IDS: u64 = 1 << 40;

impl ScoreLog {
    /// The score `path` as `arrangement` has it, from its second `from`, starting now.
    pub fn new(log: &SoundLog, path: &str, arrangement: std::sync::Arc<ta_audio::music::Arrangement>, from: f64, fade_in: f32) -> ScoreLog {
        let (now, _) = log.now();
        let from = from.clamp(0.0, arrangement.length());
        let next = arrangement.source_at(from).map_or(arrangement.spans.len(), |(k, _)| k);
        ScoreLog { path: path.to_string(), arrangement, origin: now - from, from, fade_in: fade_in.max(0.0) as f64, next, playing: None, ids: 0 }
    }

    /// Where span `k` starts in the score, and the crossfade before the join after it (as the
    /// stream clamps it: no longer than the span, nor reaching before the next one's start).
    fn span(&self, k: usize) -> (f64, f64) {
        let spans = &self.arrangement.spans;
        let start: f64 = spans[..k].iter().map(|&(a, b)| (b - a).max(0.0)).sum();
        let fade = spans.get(k + 1).map_or(0.0, |next| self.arrangement.crossfade.min(spans[k].1 - spans[k].0).min(next.0).max(0.0));
        (start, fade)
    }

    /// Writes what starts or stops before the log's next frame.
    pub fn write_due(&mut self, log: &mut SoundLog) {
        let (now, until) = log.now();
        loop {
            if let Some((id, stop, fade)) = self.playing
                && stop < until.max(now + 1e-9)
            {
                log.music_stop(stop, id, fade);
                self.playing = None;
            }
            if self.playing.is_some() || self.next >= self.arrangement.spans.len() {
                return;
            }
            let k = self.next;
            let (s, e) = self.arrangement.spans[k];
            let (start, fade_out) = self.span(k);
            let first = self.ids == 0;
            // (The first from where the score started; the others from the crossfade before their
            // join.)
            let (t, at, fade) = if first {
                (self.origin + self.from, s + (self.from - start), self.fade_in)
            } else {
                let lead = if k > 0 { self.span(k - 1).1 } else { 0.0 };
                (self.origin + start - lead, s - lead, lead)
            };
            if t >= until.max(now + 1e-9) {
                return;
            }
            let id = SCORE_IDS + self.ids;
            self.ids += 1;
            log.music_play(t, id, &self.path, at, fade, self.arrangement.gain);
            self.next += 1;
            // (The last plays to the track's end.)
            if k + 1 < self.arrangement.spans.len() {
                self.playing = Some((id, self.origin + start + (e - s) - fade_out, fade_out));
            }
        }
    }

    /// The score stopped now, fading out over `fade` seconds.
    pub fn stop(&mut self, log: &mut SoundLog, fade: f32) {
        let (now, _) = log.now();
        if let Some((id, stop, _)) = self.playing.take()
            && stop > now
        {
            log.music_stop(now, id, fade.max(0.0) as f64);
        } else if self.next >= self.arrangement.spans.len() && self.ids > 0 {
            // (The last span, playing to the end: stopped now.)
            log.music_stop(now, SCORE_IDS + self.ids - 1, fade.max(0.0) as f64);
        }
        self.next = self.arrangement.spans.len();
    }
}

/// A number for the log: at most `places` decimals, without trailing zeros (0 for what is not
/// finite).
fn num(x: f64, places: usize) -> String {
    if !x.is_finite() {
        return "0".into();
    }
    let s = format!("{x:.places$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s.as_str() };
    if s == "-0" { "0".into() } else { s.into() }
}

/// A JSON string.
fn json_str(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2);
    s.push('"');
    for c in text.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// A sound as a WAV file of 32-bit floats.
fn write_wav(path: &Path, sound: &SoundData) -> std::io::Result<()> {
    let channels = sound.channels();
    let rate = sound.sample_rate();
    let data: Vec<u8> = sound.samples().iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut b = Vec::with_capacity(data.len() + 44);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 4).to_le_bytes());
    b.extend_from_slice(&(channels * 4).to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    std::fs::write(path, b)
}

/// The music configuration while logging: none.
pub fn no_music() -> ta_audio::MusicConfig {
    let none = ta_audio::music::Playlists { peace: Vec::new(), battle: Vec::new(), victory: Vec::new(), defeat: Vec::new(), briefing: Vec::new() };
    ta_audio::MusicConfig { playlists: none, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn numbers_and_strings_for_json() {
        assert_eq!(num(2.0166666666, 6), "2.016667");
        assert_eq!(num(1.0, 5), "1");
        assert_eq!(num(-0.0000001, 5), "0");
        assert_eq!(num(f64::NAN, 3), "0");
        assert_eq!(num(-0.25, 5), "-0.25");
        assert_eq!(json_str("sounds/a \"b\"\\c\n"), r#""sounds/a \"b\"\\c\u000a""#);
    }

    /// A capture's frames: each sound at its frame's time, the mix kept up with the clock, the
    /// wind (made by the game) beside the log.
    #[test]
    fn a_capture_logs_its_sounds_on_its_clock() {
        let dir = std::env::temp_dir().join(format!("ta-sound-log-{}", std::process::id()));
        let path = dir.join("shot.jsonl");
        let settings = AudioSettings::default();
        let mut log = SoundLog::create(&path, &settings).expect("log");
        log.fps = Some(60.0);
        let mut source = ta_audio::MemorySource::new();
        // TA's sounds: 8-bit mono at 11 025 Hz.
        let mut wav = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x11\x2b\0\0\x11\x2b\0\0\x01\0\x08\0data".to_vec();
        let boom: Vec<u8> = (0..11_025).map(|i| (128.0 + 100.0 * (i as f32 * 0.05).sin()) as u8).collect();
        wav.extend_from_slice(&(boom.len() as u32).to_le_bytes());
        wav.extend_from_slice(&boom);
        source.insert("sounds/boom.wav", wav);
        let bank = Arc::new(ta_audio::SoundBank::new(Arc::new(source)));
        let mut engine = AudioEngine::new_offline(settings, bank, RATE);
        engine.set_logging(true);
        engine.set_listener([1000.0, 0.0, 1000.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0);
        engine.bank().insert("__wind", SoundData::new(22_050, 2, vec![0.25; 4410]).expect("stereo"));
        let wind = engine.play("__wind", ta_audio::PlayParams::ui(0.5).looping(true).kind(SoundKind::Ambience));
        for frame in 0..90u32 {
            log.set_frame(frame, true);
            if frame == 30 {
                engine.play_at("boom", [1200.0, 0.0, 1000.0], 1.0);
            }
            if frame == 80 {
                engine.stop(wind);
            }
            engine.update(1.0 / 60.0);
            log.record(&mut engine, 1.0 / 60.0);
        }
        assert_eq!(log.mixed, 90 * 800, "mixed up to the clock");
        drop(log);
        let text = std::fs::read_to_string(&path).expect("written");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with(r#"{"ev":"log","version":1,"rate":48000,"fps":60,"#), "{}", lines[0]);
        assert!(lines[1].starts_with(r#"{"t":0,"frame":0,"ev":"play","id":1,"kind":"ambience","path":"sounds/__wind.wav","gain":0.5,"pan":0,"gains":[0.5,0.5],"pitch":1,"loop":true,"#), "{}", lines[1]);
        assert!(lines[1].ends_with(r#""file":"shot.sounds/__wind.wav"}"#), "{}", lines[1]);
        let boom = lines.iter().find(|l| l.contains("boom")).expect("the boom");
        assert!(boom.starts_with(r#"{"t":0.5,"frame":30,"ev":"play","id":2,"kind":"world","path":"sounds/boom.wav","#), "{boom}");
        assert!(boom.contains(r#""pos":[1200,0,1000]"#) && !boom.contains("file"), "{boom}");
        assert!(lines.last().is_some_and(|l| l.starts_with(r#"{"t":1.333333,"frame":80,"ev":"stop","id":1,"fade":0.01}"#)), "{text}");
        let wind = std::fs::read(dir.join("shot.sounds/__wind.wav")).expect("the wind beside the log");
        assert_eq!(wind.len(), 44 + 4410 * 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A film's score in the log: each span written as music as it comes, at its exact time (a
    /// fractional frame), from the crossfade before its join, its stop over the crossfade before
    /// the next; started in its middle, from there; stopped early, there.
    #[test]
    fn a_score_is_logged_span_by_span_at_its_exact_times() {
        let dir = std::env::temp_dir().join(format!("ta-score-log-{}", std::process::id()));
        let path = dir.join("film.jsonl");
        let settings = AudioSettings::default();
        let mut log = SoundLog::create(&path, &settings).expect("log");
        log.fps = Some(30.0);
        let mut engine = AudioEngine::new_offline(settings, Arc::new(ta_audio::SoundBank::new(Arc::new(ta_audio::MemorySource::new()))), RATE);
        // Spans [10, 12) and [5, 9), a 12 ms crossfade, from the score's second 1.5 at frame 30.
        let arrangement = Arc::new(ta_audio::Arrangement { spans: vec![(10.0, 12.0), (5.0, 9.0)], crossfade: 0.012, gain: 0.5 });
        let mut score = None;
        for frame in 0..200u32 {
            log.set_frame(frame, true);
            if frame == 30 {
                score = Some(ScoreLog::new(&log, "music/song.ogg", arrangement.clone(), 1.5, 0.35));
            }
            if frame == 150 {
                score.as_mut().unwrap().stop(&mut log, 0.8);
            }
            if let Some(score) = score.as_mut() {
                score.write_due(&mut log);
            }
            log.record(&mut engine, 1.0 / 30.0);
        }
        drop(log);
        let text = std::fs::read_to_string(&path).expect("written");
        let music: Vec<&str> = text.lines().filter(|l| l.contains(r#""kind":"music""#) || l.contains(r#""ev":"stop""#)).collect();
        // The first span from its second 11.5, at frame 30, fading in as asked.
        assert!(music[0].starts_with(r#"{"t":1,"frame":30,"ev":"play","id":1099511627776,"kind":"music","path":"music/song.ogg","gain":0.5,"pan":0,"gains":[0.5,0.5],"pitch":1,"loop":false,"at":11.5,"fade":0.35,"#), "{}", music[0]);
        // Its stop and the next span's start, together, the crossfade before the join (half a
        // second on: frame 44.64).
        assert_eq!(music[1], r#"{"t":1.488,"frame":44.64,"ev":"stop","id":1099511627776,"fade":0.012}"#);
        assert!(music[2].starts_with(r#"{"t":1.488,"frame":44.64,"ev":"play","id":1099511627777,"kind":"music","path":"music/song.ogg","#) && music[2].contains(r#""at":4.988,"fade":0.012,"#), "{}", music[2]);
        // Stopped early: the last span stopped then.
        assert_eq!(music[3], r#"{"t":5,"frame":150,"ev":"stop","id":1099511627777,"fade":0.8}"#);
        assert_eq!(music.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
