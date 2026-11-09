//! The launcher's sounds: TA's own interface sounds from the player's install (the buttons,
//! the tabs, the big Play button, as TA's menus had them; `gamedata/allsound.tdf`), and its
//! music: TA's calmer tracks (the briefing's drone, the building music; or the player's own,
//! as the settings have it), quietly, at a share of the music volume. The music fades in as
//! the launcher opens, out while the game plays or the launcher is minimised, and back after.
//! The install's archives are opened on a thread of their own, so the launcher shows at once
//! and the sounds join a moment later; with no install, or the settings off, it stays silent.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use ta_audio::{AudioEngine, AudioSettings, Mood, MusicConfig, SoundBank, UiSounds};
use ta_vfs::Vfs;

/// Seconds the music takes to fade in or out.
const FADE: f32 = 1.6;

/// The launcher's music: TA's calmer tracks, the briefing's drone and the building music,
/// shuffled, coming and going slowly.
fn playlist() -> MusicConfig {
    let mut config = MusicConfig { crossfade: 4.0, fade_in: 4.0, gap: 4.0, shuffle: true, ..Default::default() };
    config.playlists.briefing = [17, 11, 12, 13, 14, 15, 16].into_iter().map(ta_audio::music::TrackRef::Number).collect();
    config
}

/// How loud the music is now (a share of what it is set to), eased toward where it should be.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MusicFade {
    pub level: f32,
}

impl MusicFade {
    /// `dt` seconds on toward `target`, over [`FADE`] seconds for the whole way.
    pub fn step(&mut self, target: f32, dt: f32) {
        let step = (dt.max(0.0) / FADE).min(1.0);
        self.level += (target - self.level).clamp(-step, step);
    }
}

/// The install's archives, opened, and its interface sounds.
type Opened = Option<(Arc<Vfs>, UiSounds)>;

pub struct Sounds {
    /// The install's archives being opened, and its interface sounds read.
    loading: Option<JoinHandle<Opened>>,
    playing: Option<(AudioEngine, UiSounds)>,
    /// `--film`: the sounds are named for the film's sound log, never played.
    filming: bool,
    named: Option<UiSounds>,
    last: Instant,
    /// The volumes last applied.
    applied: Option<ta_config::Audio>,
    /// The music: how loud it is now, whether a playlist is asked for, a track skip asked for.
    music: MusicFade,
    music_on: bool,
    skip: bool,
    /// The track playing (a VFS path, or `file:` and a path), and whether the install has
    /// music at all.
    playing_track: Option<String>,
    has_music: bool,
}

impl Sounds {
    /// Opens the install at `root` (when there is one) for its sounds; `filming`: names them
    /// for the film instead of playing them.
    pub fn start(root: Option<PathBuf>, filming: bool) -> Sounds {
        let loading = root.map(|root| {
            std::thread::spawn(move || {
                let mut vfs = Vfs::new();
                if let Err(e) = vfs.mount_game_dir(&root, "base") {
                    log::warn!("launcher sounds: {e}");
                    return None;
                }
                let ui = UiSounds::load(&vfs).ok()?;
                Some((Arc::new(vfs), ui))
            })
        });
        Sounds { loading, playing: None, filming, named: None, last: Instant::now(), applied: None, music: MusicFade::default(), music_on: false, skip: false, playing_track: None, has_music: false }
    }

    /// The install's sounds are known (or there is no install to know them from).
    pub fn ready(&self) -> bool {
        self.loading.is_none()
    }

    /// `--film`: the sound the interface asked for this frame, by its path in the game's files.
    pub fn take_named(&self, ctx: &egui::Context) -> Option<String> {
        let sound = ta_ui::sound::take(ctx)?;
        self.named.as_ref()?.get(sound.event()).map(ta_audio::sound_path)
    }

    /// Before drawing widgets: finish loading and let them request sounds for this frame.
    pub fn begin_frame(&mut self, ctx: &egui::Context, audio: &ta_config::Audio) {
        if self.loading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.filming {
            if self.loading.as_ref().is_some_and(|l| l.is_finished()) {
                self.named = self.loading.take().and_then(|l| l.join().ok()).flatten().map(|(_, ui)| ui);
            }
            ta_ui::sound::enable(ctx, audio.launcher_sounds && self.named.is_some());
            return;
        }
        if self.loading.as_ref().is_some_and(|l| l.is_finished())
            && let Some(Ok(Some((vfs, ui)))) = self.loading.take().map(|l| l.join())
        {
            let bank = Arc::new(SoundBank::from_vfs(vfs));
            // (Silent to begin with: the music fades in, see `update`.)
            let quiet = ta_config::Audio { music: 0.0, ..audio.clone() };
            match AudioEngine::new(AudioSettings::from(&quiet), bank) {
                Ok(mut engine) => {
                    engine.set_music_config(playlist());
                    engine.music_mut().set_user_tracks(&ta_config::music_files(audio), audio.user_music);
                    self.has_music = !engine.music().playlist(Mood::Briefing).is_empty();
                    self.playing = Some((engine, ui));
                }
                Err(e) => log::warn!("launcher sounds: {e}"),
            }
        }
        let on = audio.launcher_sounds && self.playing.is_some();
        ta_ui::sound::enable(ctx, on);
    }

    /// The track playing, if any (a VFS path, or `file:` and a path).
    pub fn now_playing(&self) -> Option<&str> {
        self.playing_track.as_deref()
    }

    /// The install has music the launcher can play.
    pub fn has_music(&self) -> bool {
        self.has_music
    }

    /// Captures (`--show music`): as if `track` played (they are silent).
    pub fn preview_music(&mut self, track: &str) {
        self.has_music = true;
        self.playing_track = Some(track.to_string());
    }

    /// On to the next track.
    pub fn skip_track(&mut self) {
        self.skip = true;
    }

    /// After drawing widgets: this frame's interface sound, played at once.
    pub fn update(&mut self, ctx: &egui::Context, audio: &ta_config::Audio) {
        if self.filming {
            if !audio.launcher_sounds {
                let _ = ta_ui::sound::take(ctx);
            }
            return;
        }
        let sound = ta_ui::sound::take(ctx);
        let Some((engine, ui)) = self.playing.as_mut() else { return };
        if let Some(sound) = sound
            && audio.launcher_sounds
            && let Some(name) = ui.get(sound.event())
        {
            engine.play_ui(name, 0.8);
        }
    }

    /// Every frame, from `logic` (which runs minimised too, when nothing is drawn): the music
    /// follows the settings, keeps `quiet` (the game playing, the launcher minimised) fading in
    /// and out, and goes on to its next track.
    pub fn music(&mut self, ctx: &egui::Context, audio: &ta_config::Audio, quiet: bool) {
        if self.filming {
            return;
        }
        let dt = self.last.elapsed().as_secs_f32();
        self.last = Instant::now();
        let Some((engine, _)) = self.playing.as_mut() else { return };
        // The player's own music turned on or off, or its folder changed: its tracks again.
        if self.applied.as_ref().is_some_and(|applied| applied.user_music != audio.user_music || applied.music_folder != audio.music_folder) {
            engine.music_mut().set_user_tracks(&ta_config::music_files(audio), audio.user_music);
            self.has_music = !engine.music().playlist(Mood::Briefing).is_empty();
        }
        // The music: faded toward its share of the music volume, or out (and stopped once
        // silent); a playlist asked for as it comes back.
        let target = if audio.launcher_music && !quiet && self.has_music { audio.launcher_music_volume.clamp(0.0, 1.0) } else { 0.0 };
        let before = self.music.level;
        self.music.step(target, dt);
        if target > 0.0 && !self.music_on {
            engine.set_mood(Mood::Briefing);
            self.music_on = true;
        } else if target <= 0.0 && self.music.level <= 0.0 && self.music_on {
            engine.music_mut().stop_with_fade(0.0);
            self.music_on = false;
        }
        if std::mem::take(&mut self.skip) && self.music_on {
            engine.music_mut().skip();
        }
        if self.applied.as_ref() != Some(audio) || before != self.music.level {
            engine.apply_config(&ta_config::Audio { music: audio.music * self.music.level, ..audio.clone() });
            self.applied = Some(audio.clone());
        }
        if self.music.level != target {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if self.music_on {
            // (A track ending starts the next, though nothing else asks for a frame.)
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        engine.update(dt);
        self.playing_track = engine.music().now_playing().map(str::to_string);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filming_respects_sound_settings_and_discards_a_muted_click() {
        let ctx = egui::Context::default();
        let mut sounds = Sounds::start(None, true);
        sounds.named = Some(UiSounds::parse(b"[SMLBUTTON] { sound=click; }"));
        let mut audio = ta_config::Audio::default();
        sounds.begin_frame(&ctx, &audio);
        ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::Button);
        sounds.update(&ctx, &audio);
        assert_eq!(sounds.take_named(&ctx).as_deref(), Some("sounds/click.wav"));

        ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::Button);
        audio.launcher_sounds = false;
        sounds.update(&ctx, &audio);
        assert_eq!(sounds.take_named(&ctx), None);
        sounds.begin_frame(&ctx, &audio);
        ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::Button);
        assert_eq!(sounds.take_named(&ctx), None);
    }

    #[test]
    fn the_music_fades_in_and_out_over_its_time_and_never_past() {
        let mut fade = MusicFade::default();
        for _ in 0..10 {
            fade.step(0.45, 0.1);
        }
        assert!((fade.level - 0.45).abs() < 1e-6, "in by a second: {}", fade.level);
        fade.step(0.0, 0.8);
        assert!((fade.level - 0.0).abs() < 1e-6, "out over its time: {}", fade.level);
        fade.step(0.0, 1.0);
        assert_eq!(fade.level, 0.0);
        // (A long frame never overshoots.)
        fade.step(1.0, 10.0);
        assert_eq!(fade.level, 1.0);
    }

    #[test]
    fn the_launcher_plays_the_calm_tracks() {
        let tracks = playlist().playlists.briefing;
        assert_eq!(tracks.first(), Some(&ta_audio::music::TrackRef::Number(17)));
        assert!(!tracks.iter().any(|t| matches!(t, ta_audio::music::TrackRef::Number(2..=8))), "no battle music: {tracks:?}");
    }

    /// With an install: the launcher's playlist plays one of TA's calm tracks, and is heard
    /// (mixed offline: no speakers needed).
    #[test]
    fn the_calm_tracks_play_from_the_install() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("skipping: no TA install");
            return;
        };
        let mut vfs = Vfs::new();
        if vfs.mount_game_dir(&install.root, "base").is_err() {
            return;
        }
        let bank = Arc::new(SoundBank::from_vfs(Arc::new(vfs)));
        let mut engine = AudioEngine::new_offline(AudioSettings::default(), bank, 48_000);
        engine.set_music_config(playlist());
        engine.set_mood(Mood::Briefing);
        let mut out = vec![0.0f32; 48_000 * 2 / 10];
        let mut loudest = 0.0f32;
        for _ in 0..60 {
            engine.update(0.1);
            engine.render_offline(&mut out);
            loudest = out.iter().fold(loudest, |m, v| m.max(v.abs()));
        }
        let track = engine.music().now_playing().unwrap_or_default().to_string();
        let calm: Vec<String> = [17, 11, 12, 13, 14, 15, 16].iter().map(|n| format!("music/{n}.mp3")).collect();
        assert!(calm.contains(&track), "a calm track: {track}");
        assert!(loudest > 0.001, "and heard: {loudest}");
    }
}
