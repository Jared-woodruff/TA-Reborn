//! Game audio: maps simulation events and UI actions onto the audio engine.

use std::sync::Arc;

use ta_audio::{AudioEngine, AudioSettings, Mood, PlayParams, SoundBank, SoundCategories, SoundEvent, SoundKind};
use ta_render::Camera;
use ta_sim::{Sim, SimEvent, UnitId};
use ta_vfs::Vfs;

use crate::sound_log::SoundLog;

mod units;

pub struct GameAudio {
    engine: AudioEngine,
    categories: SoundCategories,
    /// TA's interface sounds (`gamedata/allsound.tdf`).
    interface: ta_audio::UiSounds,
    /// Recent combat activity driving the music mood (decays over time).
    intensity: f32,
    /// The music playing: none (the intro's movies play alone, or the jukebox stopped it), the
    /// menus', or a battle's; and the screen's (what the jukebox starts again).
    mood: Option<Mood>,
    screen_mood: Option<Mood>,
    movie: ta_audio::SoundHandle,
    narration: ta_audio::SoundHandle,
    /// The narration is a briefing's: the music falls silent under it (a tutorial's lines only
    /// dip it).
    briefing: bool,
    /// The user's music level, independent of the temporary narration mix.
    music_volume: f32,
    music_duck: f32,
    /// The user's effects level (once the settings are known), and how far the battle's own
    /// sounds are stepped back now and are to be (the film behind the menu: under the music).
    effects_volume: Option<f32>,
    effects_duck: f32,
    pub scene_level: f32,
    /// The user's master level; whether the game goes silent in the background, whether it is
    /// there now, and how far it has faded out (1: not at all).
    master_volume: f32,
    mute_in_background: bool,
    background: bool,
    background_duck: f32,
    /// The ambient wind loop and its current volume.
    wind: ta_audio::SoundHandle,
    wind_level: f32,
    /// TA's +sing: acknowledgements go up and down a tune, a note each; the next note.
    pub sing: bool,
    note: usize,
    /// Units answer when selected and given orders (the settings' UNIT REPLIES).
    unit_replies: bool,
    /// `TA_SOUND_LOG`: what plays written down for films (the engine then mixes offline).
    log: Option<SoundLog>,
    /// A film's score written down in the sound log, span by span as each comes (the logging
    /// engine plays no music).
    score_log: Option<crate::sound_log::ScoreLog>,
    /// Whose music plays (the settings' SOUNDTRACK), whether the battle's player commands one of
    /// Forged Alliance's factions (the soundtrack by side), the playlists as TA's music has them
    /// (the player's own `music.toml`, or the game's), and Forged Alliance's soundtrack as
    /// installed (none: TA's plays whatever the choice).
    soundtrack: ta_config::Soundtrack,
    forged_alliance_side: bool,
    ta_music: ta_audio::MusicConfig,
    fa_tracks: Vec<FaTrack>,
    /// The menus' theme (MENU THEME, `menu_theme`): its sound for each of TA's interface events,
    /// played while the menus are shown (`in_menus`), and its music, the menus' playlist. None:
    /// TA's.
    menu_sounds: std::collections::BTreeMap<String, String>,
    menu_music: Option<String>,
    in_menus: bool,
    /// The units followed for the sounds they make as they go about the battle (`units`).
    units: units::UnitSounds,
}

/// Ticks between two announcements of a strategic missile's launch (a salvo is one).
const LAUNCH_ANNOUNCED_EVERY: u64 = 150;

/// The sides of Supreme Commander's factions (their mod, made from the player's own copy): the
/// soundtrack by side plays Forged Alliance's music when the player commands one.
pub const FORGED_ALLIANCE_SIDES: [&str; 4] = ["UEF", "CYBRAN", "AEON", "SERAPHIM"];

/// The list of Forged Alliance's soundtrack as the music mod has it (`ta_supcom::soundtrack::LIST`).
const FA_LIST: &str = "music/supcom/soundtrack.toml";

/// A track of Forged Alliance's soundtrack: its file, its part (`menu`, `building`, `battle`,
/// `briefing`, `film`) and its number in it.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct FaTrack {
    pub file: String,
    pub part: String,
    pub number: usize,
}

/// Forged Alliance's soundtrack as installed, read from its list in `source` (the game's files):
/// none where the music mod is not on.
pub fn fa_tracks(source: &dyn ta_audio::SoundSource) -> Vec<FaTrack> {
    #[derive(serde::Deserialize)]
    struct List {
        #[serde(default)]
        track: Vec<FaTrack>,
    }
    let Some(bytes) = source.read(FA_LIST) else { return Vec::new() };
    match toml::from_str::<List>(&String::from_utf8_lossy(&bytes)) {
        Ok(list) => list.track.into_iter().map(|t| FaTrack { file: ta_vfs::normalize(&t.file), ..t }).collect(),
        Err(e) => {
            log::warn!("{FA_LIST}: {e}");
            Vec::new()
        }
    }
}

/// Forged Alliance's menu theme as the music's mod has it: the menus' theme's own music is the
/// same, and is not listed twice.
pub const FA_MENU_THEME: &str = "music/supcom/menu.flac";

/// The menus' playlist (`list`, the soundtrack's for them) with the menus' theme's music
/// (`theme`) first: one track alone, played over and over, was too little. As it is where it has
/// that music already, or Forged Alliance's menu theme (the same music, from the music's mod).
pub fn with_theme_music(list: &mut Vec<ta_audio::music::TrackRef>, theme: Option<&str>) {
    use ta_audio::music::TrackRef;
    let Some(theme) = theme else { return };
    let same = |track: &TrackRef| matches!(track, TrackRef::Path(path) if path.eq_ignore_ascii_case(theme) || path.eq_ignore_ascii_case(FA_MENU_THEME));
    if !list.iter().any(same) {
        list.insert(0, TrackRef::Path(theme.to_string()));
    }
}

/// The playlists `soundtrack` asks for: TA's (`ta`), Forged Alliance's in their place (its
/// building themes at peace, its battle themes in battle, its menu's theme and briefings' music in
/// the menus; TA's victory and defeat kept), or both together. By side: Forged Alliance's where
/// the player commands one of its factions (`fa_side`). TA's alone without Forged Alliance's
/// tracks.
pub fn soundtrack_music(ta: &ta_audio::MusicConfig, fa: &[FaTrack], soundtrack: ta_config::Soundtrack, fa_side: bool) -> ta_audio::MusicConfig {
    use ta_audio::music::TrackRef;
    use ta_config::Soundtrack as S;
    let (with_fa, with_ta) = match soundtrack {
        _ if fa.is_empty() => (false, true),
        S::TotalAnnihilation => (false, true),
        S::ForgedAlliance => (true, false),
        S::Both => (true, true),
        S::BySide => (fa_side, !fa_side),
    };
    if !with_fa {
        return ta.clone();
    }
    let part = |parts: &[&str]| -> Vec<TrackRef> { fa.iter().filter(|t| parts.contains(&t.part.as_str())).map(|t| TrackRef::Path(t.file.clone())).collect() };
    let mut config = ta.clone();
    let lists = &mut config.playlists;
    for (list, fa_list) in [(&mut lists.peace, part(&["building"])), (&mut lists.battle, part(&["battle"])), (&mut lists.briefing, part(&["menu", "briefing"]))] {
        if fa_list.is_empty() {
            continue;
        }
        if !with_ta {
            list.clear();
        }
        list.extend(fa_list);
    }
    config
}

/// The tune singing units follow (Beethoven's Ode to Joy), in semitones.
const TUNE: [i32; 15] = [4, 4, 5, 7, 7, 5, 4, 2, 0, 0, 2, 4, 4, 2, 2];

/// A unit answering the player (selected, given an order, arrived, or refusing one): what UNIT
/// REPLIES turns off. Alerts (a unit attacked), a unit done in its factory, the self-destruct
/// count and its cancelling still speak: they tell the player something they need to know.
fn is_reply(event: SoundEvent) -> bool {
    matches!(event, SoundEvent::Select | SoundEvent::Ok | SoundEvent::Arrived | SoundEvent::Cant)
}

/// Eight seconds of stereo wind: noise through two resonant band-passes (a low rush and a
/// higher whistle), with slow gusts; the tail is cross-faded into the head so it loops
/// without a seam.
/// How wide a view a unit's own ears hear as (world units): full volume within a quarter of
/// it, fading to nothing at its width.
const EAR_WIDTH: f32 = 1000.0;

fn synth_wind() -> Option<ta_audio::SoundData> {
    const RATE: u32 = 22_050;
    let frames = RATE as usize * 8;
    let fade = RATE as usize / 2;
    let mut rng = 0x9e37_79b9u32;
    let mut noise = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        (rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    // State-variable filters per channel: (low, band) for the rush and the whistle.
    let mut chans = [[0.0f32; 4]; 2];
    let total = frames + fade;
    let mut out = vec![0.0f32; total * 2];
    for i in 0..total {
        let t = i as f32 / RATE as f32;
        // Gusts: a few slow, incommensurate swells (periods divide the loop length).
        let gust = 0.55 + 0.25 * (t * std::f32::consts::TAU / 8.0).sin() + 0.2 * (t * std::f32::consts::TAU / 2.0 + 1.3).sin().powi(2);
        for (c, st) in chans.iter_mut().enumerate() {
            let x = noise();
            let sweep = 1.0 + 0.3 * (t * std::f32::consts::TAU / 4.0 + c as f32 * 2.0).sin();
            // Chamberlin state-variable filter: f = 2 sin(pi fc / rate).
            let (f1, q1) = (2.0 * (std::f32::consts::PI * 180.0 * sweep / RATE as f32).sin(), 0.35);
            st[0] += f1 * st[1];
            let hp = x - st[0] - q1 * st[1];
            st[1] += f1 * hp;
            let (f2, q2) = (2.0 * (std::f32::consts::PI * 900.0 * sweep / RATE as f32).sin(), 0.12);
            st[2] += f2 * st[3];
            let hp2 = x - st[2] - q2 * st[3];
            st[3] += f2 * hp2;
            out[i * 2 + c] = (st[1] * 0.9 + st[3] * 0.12 * gust) * gust * 0.5;
        }
    }
    // Cross-fade the extra half second over the start.
    for i in 0..fade {
        let k = i as f32 / fade as f32;
        for c in 0..2 {
            let head = out[i * 2 + c];
            let tail = out[(frames + i) * 2 + c];
            out[i * 2 + c] = head * k + tail * (1.0 - k);
        }
    }
    out.truncate(frames * 2);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    for v in &mut out {
        *v *= 0.9 / peak;
    }
    ta_audio::SoundData::new(RATE, 2, out).ok()
}

impl GameAudio {
    pub fn new(vfs: Arc<Vfs>, config: &ta_config::Audio) -> GameAudio {
        let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
        let settings = AudioSettings::from(config);
        // A film's sound log: everything mixed offline on the frames' clock and written down,
        // the speakers silent, no music (see `sound_log`).
        if let Some(log) = SoundLog::from_env(&settings) {
            let mut engine = AudioEngine::new_offline(settings, bank, crate::sound_log::RATE);
            engine.set_logging(true);
            engine.set_music_config(crate::sound_log::no_music());
            let mut audio = Self::with_engine(engine, vfs);
            audio.log = Some(log);
            audio.unit_replies = config.unit_replies;
            return audio;
        }
        let engine = match AudioEngine::new(settings.clone(), bank.clone()) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("audio disabled: {e}");
                AudioEngine::new_silent(settings, bank)
            }
        };
        let mut audio = Self::with_engine(engine, vfs);
        audio.effects_volume = Some(config.effects);
        audio.mute_in_background = config.mute_in_background;
        audio.unit_replies = config.unit_replies;
        let music = audio.engine.music_mut();
        music.set_shuffle(config.music_shuffle);
        music.set_repeat(config.music_repeat);
        audio.soundtrack = config.soundtrack;
        audio.apply_soundtrack();
        audio
    }

    /// A replay's film (`film.rs`): its sound mixed offline at `rate` as the pictures are made
    /// ([`GameAudio::mix`]), with the battle's sounds and the music as chosen, the player's own
    /// balance between them at full volume; the speakers' engine plays on meanwhile, set aside.
    pub fn film(vfs: Arc<Vfs>, config: &ta_config::Audio, rate: u32, sounds: bool, music: bool) -> GameAudio {
        let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
        let (effects, voice) = if sounds { (config.effects, config.voice) } else { (0.0, 0.0) };
        let config = ta_config::Audio { master: 0.9, effects, voice, music: if music { config.music } else { 0.0 }, ..config.clone() };
        let mut engine = AudioEngine::new_offline(AudioSettings::from(&config), bank, rate);
        if !music {
            engine.set_music_config(crate::sound_log::no_music());
        }
        let mut audio = Self::with_engine(engine, vfs);
        // (Its music the soundtrack chosen, as the speakers' is.)
        if music {
            audio.soundtrack = config.soundtrack;
            audio.apply_soundtrack();
        }
        audio
    }

    /// The next `out.len() / 2` stereo frames of a film's sound ([`GameAudio::film`]).
    pub fn mix(&mut self, out: &mut [f32]) {
        self.engine.render_offline(out);
    }

    fn with_engine(engine: AudioEngine, vfs: Arc<Vfs>) -> GameAudio {
        let categories = SoundCategories::load(vfs.as_ref()).unwrap_or_else(|e| {
            log::warn!("sound categories unavailable: {e}");
            SoundCategories::default()
        });
        let mut engine = engine;
        // (Their sound sets, named wherever a sound's file name goes.)
        engine.set_sound_sets(categories.sets().clone());
        let interface = ta_audio::UiSounds::load(vfs.as_ref()).unwrap_or_else(|e| {
            log::warn!("interface sounds unavailable: {e}");
            ta_audio::UiSounds::default()
        });
        let music_volume = engine.volume(ta_audio::Bus::Music);
        let master_volume = engine.volume(ta_audio::Bus::Master);
        let mut audio = GameAudio {
            engine,
            categories,
            interface,
            intensity: 0.0,
            mood: None,
            screen_mood: None,
            movie: ta_audio::SoundHandle::NONE,
            narration: ta_audio::SoundHandle::NONE,
            briefing: false,
            music_volume,
            music_duck: 1.0,
            effects_volume: None,
            effects_duck: 1.0,
            scene_level: 1.0,
            master_volume,
            mute_in_background: false,
            background: false,
            background_duck: 1.0,
            wind: ta_audio::SoundHandle::NONE,
            wind_level: 0.0,
            sing: false,
            note: 0,
            unit_replies: true,
            log: None,
            score_log: None,
            soundtrack: ta_config::Soundtrack::TotalAnnihilation,
            forged_alliance_side: false,
            ta_music: ta_audio::MusicConfig::default(),
            fa_tracks: Vec::new(),
            menu_sounds: std::collections::BTreeMap::new(),
            menu_music: None,
            in_menus: false,
            units: units::UnitSounds::default(),
        };
        audio.ta_music = audio.engine.music().config().clone();
        audio.fa_tracks = fa_tracks(vfs.as_ref());
        if let Some(wind) = synth_wind() {
            audio.engine.bank().insert("__wind", wind);
            audio.wind = audio.engine.play("__wind", PlayParams::ui(0.0).looping(true).priority(ta_audio::Priority::Low).bus(ta_audio::Bus::Effects).kind(SoundKind::Ambience));
        }
        // (No music yet: the screen shown first picks it, see `set_mood`.)
        audio
    }

    fn unit_category<'a>(&'a self, sim: &Sim, id: UnitId) -> Option<&'a ta_audio::SoundCategory> {
        let u = sim.unit(id)?;
        self.categories.try_category(&sim.unit_type(u).def.sound_category)
    }

    /// Voice acknowledgement for the first unit of a selection/order (none of a unit's replies when
    /// the player turned them off: [`is_reply`]).
    pub fn unit_voice(&mut self, sim: &Sim, id: UnitId, event: SoundEvent) {
        if !self.unit_replies && is_reply(event) {
            return;
        }
        let pos = sim.unit(id).map(|u| u.pos.to_f32()).unwrap_or_default();
        // (The unit the player sent somewhere, to be heard arriving.)
        if event == SoundEvent::Ok {
            self.ordered(sim, id);
        }
        if let Some(cat) = self.unit_category(sim, id).cloned() {
            let voice = self.engine.play_unit_sound(&cat, event, pos, 1.0);
            if self.sing && event.is_acknowledgement() && !voice.is_none() {
                let semitones = TUNE[self.note % TUNE.len()] - 2;
                self.note += 1;
                self.engine.set_pitch(voice, 2f32.powf(semitones as f32 / 12.0));
            }
        }
    }

    /// The chat line's music codes (TA's `+cdstart`, `+cdstop`...).
    pub fn music_request(&mut self, request: crate::chat::MusicRequest) {
        use crate::chat::MusicRequest as R;
        // (A track chosen while the music is stopped: the screen's music goes on after it, not
        // the menus'.)
        if matches!(request, R::Random | R::Track(_) | R::Previous) {
            self.resume_mood();
        }
        match request {
            R::Start => {
                self.mood = None;
                self.resume_mood();
            }
            R::Stop => {
                // (Until the next +cdstart or a change of screen.)
                self.mood = None;
                self.engine.music_mut().stop();
            }
            R::Next => self.engine.music_mut().skip(),
            R::Random => {
                let music = self.engine.music();
                let mut tracks: Vec<String> = Mood::ALL.iter().flat_map(|&m| music.playlist(m).to_vec()).collect();
                tracks.sort();
                tracks.dedup();
                let pick = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize);
                if let Some(track) = tracks.get(pick % tracks.len().max(1)).cloned() {
                    self.engine.music_mut().play_track(&track);
                }
            }
            R::Track(track) => {
                self.engine.music_mut().play_track(&track);
            }
            R::Narration(path) => self.play_briefing(&path),
            R::Previous => {
                self.engine.music_mut().previous();
            }
        }
    }

    /// A world sound that is not part of the battle (thunder): it doesn't stir up the music.
    pub fn play_ambient(&mut self, name: &str, pos: glam::Vec3, volume: f32) {
        self.engine.play_at(name, pos.to_array(), volume);
    }

    /// An effect's sound: TA's where it is, or a unit's own for it (a wreck crashing, sinking,
    /// a building coming down) where its sound category gives one.
    pub fn play_effect(&mut self, sim: &Sim, sound: crate::effects::EffectSound) {
        if let Some((type_id, event)) = sound.unit
            && self.type_has(sim, type_id, event)
        {
            self.type_sound(sim, type_id, &[event], sound.pos.to_array(), sound.volume);
            return;
        }
        self.play_ambient(sound.name, sound.pos, sound.volume);
    }

    /// Whether unit type `type_id`'s sound category gives `event`.
    fn type_has(&self, sim: &Sim, type_id: usize, event: SoundEvent) -> bool {
        type_id < sim.types.types.len() && self.categories.try_category(&sim.types.get(type_id).def.sound_category).is_some_and(|c| c.get(event).is_some())
    }

    /// The local player's side's alert for `event` (its alerts category's: Forged Alliance's
    /// announcer), played over the battle: false where the side has none (TA's own).
    fn side_alert(&mut self, sim: &Sim, player: u8, event: SoundEvent) -> bool {
        let Some(side) = sim.players.get(player as usize).map(|p| p.side.clone()) else { return false };
        let named = sim.data.sides.iter().find(|s| s.name.eq_ignore_ascii_case(&side)).map(|s| s.alerts.clone());
        let Some(alerts) = self.categories.alerts(&side, named.as_deref()).cloned() else { return false };
        let Some(name) = self.engine.pick(&alerts, event) else { return false };
        self.engine.play(&name, PlayParams::voice(1.0).priority(ta_audio::Priority::Critical).kind(SoundKind::Alert));
        true
    }

    /// `viewer`: whose eyes the view uses (none: everything is seen); the world's sounds of
    /// what they cannot see are not heard.
    pub fn on_event(&mut self, e: &SimEvent, sim: &Sim, local_player: u8, viewer: Option<u8>) {
        let seen = |pos: ta_sim::FxVec3| viewer.is_none_or(|player| sim.in_sight(player, pos));
        let unit_seen = |id: ta_sim::UnitId| sim.unit(id).is_some_and(|u| seen(u.pos));
        match e {
            SimEvent::WeaponFired { pos, .. } | SimEvent::UnitExploded { pos, .. } if !seen(*pos) => {}
            // (A missile of its own side's going off out of sight is heard: see `nuclear::strategic`.)
            SimEvent::Explosion { pos, weapon, player, .. }
                if !seen(*pos) && !(crate::game::nuclear::strategic(&sim.weapon_types.types[*weapon]) && viewer.zip(*player).is_some_and(|(v, o)| sim.allied(v, o))) => {}
            SimEvent::NanoSpray { builder, .. } | SimEvent::Reclaiming { builder, .. } if !unit_seen(*builder) => {}
            SimEvent::UnitLoaded { transport, .. } | SimEvent::UnitUnloaded { transport, .. } if !unit_seen(*transport) => {}
            SimEvent::WeaponFired { unit, weapon, pos, burst_start, .. } => {
                let w = &sim.weapon_types.types[*weapon].def;
                // (Under the water, its sound for there where it has one; with `soundtrigger`,
                // once a burst.)
                let under = pos.y < sim.world.sea_level && !w.sound_start_water.is_empty();
                let name = if under { &w.sound_start_water } else { &w.sound_start };
                if !name.is_empty() && (*burst_start || !w.sound_trigger) {
                    self.engine.play_at(name, pos.to_f32(), 0.8);
                }
                self.weapon_loop(sim, *unit, *weapon, pos.to_f32());
                // A strategic missile's launch, announced by the player's side where it has an
                // announcer (at most one every few seconds: a salvo is one).
                if crate::game::nuclear::strategic(&sim.weapon_types.types[*weapon]) && self.units.announced.is_none_or(|at| sim.tick >= at + LAUNCH_ANNOUNCED_EVERY) && self.side_alert(sim, local_player, SoundEvent::NuclearLaunch) {
                    self.units.announced = Some(sim.tick);
                }
                self.intensity += 0.15;
            }
            SimEvent::Explosion { weapon, pos, water, death_of, .. } => {
                // (A unit's death blast: its own death sound in the blast's place, where its sound
                // category gives one.)
                match death_of.filter(|&t| self.type_has(sim, t, SoundEvent::Death)) {
                    Some(t) => {
                        self.type_sound(sim, t, &[SoundEvent::Death], pos.to_f32(), 1.0);
                    }
                    None => {
                        let w = &sim.weapon_types.types[*weapon].def;
                        let name = if *water && !w.sound_water.is_empty() { &w.sound_water } else { &w.sound_hit };
                        if !name.is_empty() {
                            self.engine.play_at(name, pos.to_f32(), 1.0);
                        }
                    }
                }
                self.intensity += 0.3;
            }
            SimEvent::UnitExploded { pos, big, type_id, .. } => {
                if self.type_has(sim, *type_id, SoundEvent::Death) {
                    self.type_sound(sim, *type_id, &[SoundEvent::Death], pos.to_f32(), 1.0);
                } else {
                    self.engine.play_at(if *big { "xplolrg1" } else { "xplomed2" }, pos.to_f32(), 1.0);
                }
                self.intensity += 1.0;
            }
            SimEvent::UnitCompleted { id, type_id } => {
                // Done, where it stands (seen): its `donebuilt`.
                if let Some(pos) = sim.unit(*id).map(|u| u.pos).filter(|p| seen(*p)) {
                    self.type_sound(sim, *type_id, &[SoundEvent::DoneBuilt], pos.to_f32(), 1.0);
                }
                if sim.unit(*id).is_some_and(|u| u.owner == local_player) {
                    // Factory-built units announce themselves; buildings chime.
                    if let Some(cat) = self.unit_category(sim, *id).cloned() {
                        let pos = sim.unit(*id).map(|u| u.pos.to_f32()).unwrap_or_default();
                        self.engine.play_unit_sound(&cat, SoundEvent::UnitComplete, pos, 0.9);
                    }
                }
            }
            // TA's own sounds for the work (`sound.tdf`): building, repairing ("repair1"),
            // reclaiming ("working": "reclaim1").
            SimEvent::NanoSpray { builder, target } => {
                let reclaiming = sim.unit(*builder).is_some_and(|u| matches!(u.orders.front(), Some(ta_sim::Order::ReclaimUnit(_))));
                // (Work on one being taken apart mends it.)
                let repairing = !reclaiming && sim.unit(*target).is_some_and(|t| t.is_complete() || sim.being_taken_apart(t));
                self.nano_sound(sim, *builder, if reclaiming { SoundEvent::Working } else if repairing { SoundEvent::Repair } else { SoundEvent::Build });
            }
            SimEvent::Reclaiming { builder, .. } => self.nano_sound(sim, *builder, SoundEvent::Working),
            SimEvent::OrderFailed { unit, .. } => {
                if sim.unit(*unit).is_some_and(|u| u.owner == local_player) {
                    self.unit_voice(sim, *unit, SoundEvent::Cant);
                }
            }
            SimEvent::CloakChanged { id, cloaked } => {
                if sim.unit(*id).is_some_and(|u| u.owner == local_player) {
                    self.unit_voice(sim, *id, if *cloaked { SoundEvent::Cloak } else { SoundEvent::Uncloak });
                }
            }
            SimEvent::UnitCaptured { id, to, .. } => {
                // The capturing commander's voice.
                if *to == local_player {
                    let commander = sim.units().find(|u| u.owner == local_player && sim.unit_type(u).def.is_commander).map(|u| u.id);
                    if let Some(cat) = commander.and_then(|c| self.unit_category(sim, c)).cloned() {
                        let pos = sim.unit(*id).map(|u| u.pos.to_f32()).unwrap_or_default();
                        self.engine.play_unit_sound(&cat, SoundEvent::Capture, pos, 1.0);
                    }
                }
            }
            SimEvent::UnitLoaded { transport, .. } | SimEvent::UnitUnloaded { transport, .. } => {
                if let Some(t) = sim.unit(*transport) {
                    let name = if sim.unit_type(t).is_plane() { "loadair" } else { "loadwtr1" };
                    self.engine.play_at(name, t.pos.to_f32(), 0.8);
                }
            }
            SimEvent::UnderAttack { player, unit, .. }
                // A global alert (not placed in the world), in the attacked unit's voice (one of
                // its variants), or the side's announcer for its commander.
                if *player == local_player => {
                    let commander = sim.unit(*unit).is_some_and(|u| sim.unit_type(u).def.is_commander);
                    if !(commander && self.side_alert(sim, local_player, SoundEvent::CommanderAttacked)) {
                        let cat = self.unit_category(sim, *unit).cloned();
                        let name = cat.and_then(|c| self.engine.pick(&c, SoundEvent::UnderAttack)).unwrap_or_else(|| "warning1".to_string());
                        self.engine.play(&name, PlayParams::ui(0.9).kind(SoundKind::Alert));
                    }
                }
            // A script's sound (COB `play-sound`), where its unit is.
            SimEvent::ScriptSound { unit, sound } => {
                if let Some(u) = sim.unit(*unit).filter(|u| seen(u.pos))
                    && let Some(name) = sim.unit_type(u).script.as_ref().and_then(|s| s.script.sounds.get(usize::try_from(*sound).ok()?).cloned())
                {
                    self.engine.play_at(&name, u.pos.to_f32(), 1.0);
                }
            }
            _ => {}
        }
    }

    /// Per-frame update: listener follows the camera; music follows the fighting.
    /// The ambient wind: louder with the map's wind and higher up (zoomed out or looking
    /// at the horizon), eased so it never jumps.
    pub fn set_wind(&mut self, wind: f32, camera: &Camera, dt: f32) {
        // A scene change or output-device recovery releases the old handle. Recreate the
        // cached loop quietly and let the normal wind ramp bring it back.
        if !self.engine.is_playing(self.wind) {
            self.wind_level = 0.0;
            self.wind = self.engine.play("__wind", PlayParams::ui(0.0).looping(true).priority(ta_audio::Priority::Low).bus(ta_audio::Bus::Effects).kind(SoundKind::Ambience));
        }
        let strength = (wind / 25.0).clamp(0.0, 1.0);
        let height = (1.0 / camera.zoom.max(0.1)).clamp(0.5, 4.0) / 4.0 + (1.0 - camera.pitch / 1.52).clamp(0.0, 1.0) * 0.3;
        let target = (0.03 + 0.17 * strength) * (0.4 + height);
        self.wind_level += (target - self.wind_level) * (dt * 0.8).min(1.0);
        // (The wind is the world's too: it steps back with the battle behind the menu.)
        self.engine.set_sound_volume(self.wind, self.wind_level * self.effects_duck);
        self.engine.set_pitch(self.wind, 0.85 + 0.3 * strength);
    }

    /// `ear`: the eye and the way it looks, when the player is in a unit (Perspective): the
    /// listener is there, hearing as far round as a battle view would (from the camera's focus
    /// a view only a few dozen units wide heard nothing, not even the unit's own guns).
    /// Place the listener before dispatching this frame's sounds, so a camera cut does
    /// not cull or pan them from the previous camera position.
    pub fn set_listener(&mut self, camera: &Camera, ear: Option<(glam::Vec3, glam::Vec3)>) {
        match ear {
            Some((eye, look)) => {
                let fwd = glam::Vec3::new(look.x, 0.0, look.z).normalize_or(glam::Vec3::NEG_Z);
                let right = fwd.cross(glam::Vec3::Y);
                self.engine.set_listener(eye.to_array(), fwd.to_array(), right.to_array(), EAR_WIDTH);
            }
            None => {
                // Classic rendering stays north-up even after the perspective camera turned.
                let yaw = if camera.mode == ta_render::CameraMode::Classic { 0.0 } else { camera.yaw };
                let (sy, cy) = yaw.sin_cos();
                let right = [cy, 0.0, sy];
                let forward = [sy, 0.0, -cy];
                let f = camera.focus;
                let view_width = camera.viewport.x / camera.zoom.max(0.05);
                self.engine.set_listener([f.x, f.y, f.z], forward, right, view_width);
            }
        }
    }

    pub fn update(&mut self, camera: &Camera, ear: Option<(glam::Vec3, glam::Vec3)>, dt: f32) {
        self.set_listener(camera, ear);
        self.intensity = (self.intensity * (1.0 - dt * 0.08).max(0.0)).min(60.0);
        // In battle the music follows the fighting. The menus' music (the battle behind the
        // title screen does not change it), a won or lost battle's (the fighting that goes on
        // does not bring the battle music back) and silence stay as they are.
        let want = match self.mood {
            Some(Mood::Peace | Mood::Battle) if self.intensity > 8.0 => Some(Mood::Battle),
            Some(Mood::Peace | Mood::Battle) if self.intensity < 2.0 => Some(Mood::Peace),
            other => other,
        };
        if want != self.mood {
            self.mood = want;
            if let Some(mood) = want {
                self.engine.set_mood(mood);
            }
        }
        self.update_idle(dt);
    }

    /// Loading screens still service music sequencing and device recovery, without
    /// emitting sounds or advancing combat activity from the previous battlefield.
    pub fn update_idle(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        // Silent in the background (when the player asked for it): out quickly, back gently.
        let target = if self.mute_in_background && self.background { 0.0 } else { 1.0 };
        if self.background_duck != target {
            let seconds = if target < self.background_duck { 0.15 } else { 0.4 };
            self.background_duck += (target - self.background_duck) * (1.0 - (-dt / seconds).exp());
            if (self.background_duck - target).abs() < 0.002 {
                self.background_duck = target;
            }
            self.engine.set_volume(ta_audio::Bus::Master, self.master_volume * self.background_duck);
        }
        // The music under a voice: silent under a briefing's, lower under the tutorial's lines.
        let speaking = self.speaking();
        let target = match (speaking, self.briefing) {
            (false, _) => 1.0,
            (true, true) => 0.0,
            (true, false) => 0.28,
        };
        let seconds = if speaking { 0.12 } else { 0.8 };
        self.music_duck += (target - self.music_duck) * (1.0 - (-dt / seconds).exp());
        if (self.music_duck - target).abs() < 0.002 {
            self.music_duck = target;
        }
        self.engine.set_volume(ta_audio::Bus::Music, self.music_volume * self.music_duck);
        // The battle's sounds stepped back (or forward again) over a moment.
        if self.effects_volume.is_some() {
            let before = self.effects_duck;
            self.effects_duck += (self.scene_level - self.effects_duck) * (1.0 - (-dt / 0.6).exp());
            if (self.effects_duck - self.scene_level).abs() < 0.002 {
                self.effects_duck = self.scene_level;
            }
            if self.effects_duck != before {
                self.engine.set_world_gain(self.effects_duck);
            }
        }
        self.engine.update(dt);
        if let Some(log) = self.log.as_mut() {
            if let Some(score) = self.score_log.as_mut() {
                score.write_due(log);
            }
            log.record(&mut self.engine, dt);
        }
    }

    /// The music steps aside for a film's score (the credits'): what plays (the score too) fades
    /// out over `seconds`, and nothing else plays until [`GameAudio::end_score`].
    pub fn hush_music(&mut self, seconds: f32) {
        if let (Some(log), Some(mut score)) = (self.log.as_mut(), self.score_log.take()) {
            score.stop(log, seconds);
        }
        self.engine.music_mut().hush(seconds);
    }

    /// Whether there is no sound at all (no device): nothing plays, nothing to follow.
    pub fn is_silent(&self) -> bool {
        self.engine.is_silent()
    }

    /// Plays `path` (a VFS path) as the film's score, as `arrangement` has it, from its own second
    /// `from`, fading in over `fade_in`. False when it cannot (no such file). Logging sounds, its
    /// spans are written down instead, each as the music it is, at the capture's time.
    pub fn play_score(&mut self, path: &str, arrangement: std::sync::Arc<ta_audio::music::Arrangement>, from: f64, fade_in: f32) -> bool {
        if let Some(log) = self.log.as_mut() {
            if let Some(mut old) = self.score_log.take() {
                old.stop(log, fade_in.max(0.02));
            }
            self.score_log = Some(crate::sound_log::ScoreLog::new(log, path, arrangement, from, fade_in));
            return true;
        }
        self.engine.music_mut().play_score(path, arrangement, from, fade_in)
    }

    /// How far into the film's score the music is (its own seconds, as far as it has been
    /// heard), while it plays.
    pub fn score_position(&self) -> Option<f64> {
        self.engine.score_position()
    }

    /// The film is over: its score fades out over `seconds` and the screen's music comes back.
    pub fn end_score(&mut self, seconds: f32) {
        if let (Some(log), Some(mut score)) = (self.log.as_mut(), self.score_log.take()) {
            score.stop(log, seconds);
        }
        self.engine.music_mut().end_score(seconds);
    }

    /// A cut in the menu's film: the last scene's sounds stop (in the world, its units' voices
    /// and alerts); the interface, a narration, the wind and the music go on.
    pub fn cut_scene(&mut self) {
        for kind in [SoundKind::World, SoundKind::Voice, SoundKind::Alert] {
            self.engine.stop_kind(kind);
        }
        self.units.forget();
        self.intensity = 0.0;
    }

    /// Leave a battlefield or narration behind while the soundtrack continues.
    pub fn reset_scene(&mut self) {
        self.engine.stop_all();
        self.units.forget();
        self.movie = ta_audio::SoundHandle::NONE;
        self.narration = ta_audio::SoundHandle::NONE;
        self.wind = ta_audio::SoundHandle::NONE;
        self.wind_level = 0.0;
        self.intensity = 0.0;
        self.note = 0;
    }

    pub fn begin_intro(&mut self) {
        self.reset_scene();
        self.mood = None;
        self.engine.music_mut().stop_with_fade(0.15);
        self.update_idle(0.0);
    }

    /// The frame the app is on, and whether the game moves on a capture's fixed clock: the sound
    /// log's time for the sounds it starts (`TA_SOUND_LOG`).
    pub fn set_frame(&mut self, frame: u32, fixed_clock: bool) {
        if let Some(log) = self.log.as_mut() {
            log.set_frame(frame, fixed_clock);
        }
    }

    /// Plays a decoded movie soundtrack; returns immediately.
    pub fn play_movie_audio(&mut self, name: &str, sound: ta_audio::SoundData) {
        let key = format!("__movie_{name}");
        self.engine.bank().insert(&key, sound);
        self.stop_movie_audio();
        self.movie = self.engine.play(&key, PlayParams::ui(1.0).priority(ta_audio::Priority::Critical).kind(SoundKind::Movie));
    }

    pub fn stop_movie_audio(&mut self) {
        if !self.movie.is_none() {
            self.engine.stop(self.movie);
            self.movie = ta_audio::SoundHandle::NONE;
        }
    }

    /// Plays a briefing's narration (VFS path) on the voice bus, replacing any playing one: the
    /// music falls silent under it, and so does the menu's film (`briefing_speaking`).
    pub fn play_briefing(&mut self, path: &str) {
        self.play_voice(path, true);
    }

    /// Plays a spoken line (VFS path: the tutorial's) on the voice bus, replacing any playing
    /// one: the music only dips under it.
    pub fn play_narration(&mut self, path: &str) {
        self.play_voice(path, false);
    }

    fn play_voice(&mut self, path: &str, briefing: bool) {
        self.stop_narration();
        self.narration = self.engine.play(path, PlayParams::voice(1.0).priority(ta_audio::Priority::Critical).kind(SoundKind::Narration));
        self.briefing = briefing;
    }

    /// Whether a voice is heard now (playing, the voice level not off).
    fn speaking(&self) -> bool {
        self.engine.is_playing(self.narration) && self.engine.volume(ta_audio::Bus::Voice) > 0.0
    }

    /// Whether a briefing's voice is heard now: everything else steps aside for it.
    pub fn briefing_speaking(&self) -> bool {
        self.briefing && self.speaking()
    }

    pub fn stop_narration(&mut self) {
        if !self.narration.is_none() {
            self.engine.stop(self.narration);
            self.narration = ta_audio::SoundHandle::NONE;
        }
    }

    /// Whether the game is in the background (another window in front): it goes silent there
    /// when the settings say so.
    pub fn set_background(&mut self, background: bool) {
        self.background = background;
    }

    pub fn set_mood(&mut self, mood: Mood) {
        self.mood = Some(mood);
        self.screen_mood = Some(mood);
        self.engine.set_mood(mood);
    }

    /// Whether the battle's player commands one of Forged Alliance's factions: with the
    /// soundtrack by side, its music plays from then on (TA's otherwise), the track playing
    /// changed at once where it is not one of the new playlist's.
    pub fn set_side(&mut self, forged_alliance: bool) {
        if std::mem::replace(&mut self.forged_alliance_side, forged_alliance) != forged_alliance && self.soundtrack == ta_config::Soundtrack::BySide {
            self.apply_soundtrack();
        }
    }

    /// The playlists as SOUNDTRACK and the side say (none while logging sounds: a film's
    /// music is its own), the track playing changed at once where the new playlist of its mood
    /// does not have it.
    fn apply_soundtrack(&mut self) {
        if self.log.is_some() {
            return;
        }
        let mut config = soundtrack_music(&self.ta_music, &self.fa_tracks, self.soundtrack, self.forged_alliance_side);
        // (The player's shuffle as it is now: the playlists change, not how they play.)
        config.shuffle = self.engine.music().shuffle();
        with_theme_music(&mut config.playlists.briefing, self.menu_music.as_deref());
        self.engine.set_music_config(config);
        let music = self.engine.music();
        let stale = match (music.mood(), music.now_playing()) {
            (Some(mood), Some(track)) => !music.playlist(mood).iter().any(|t| t == track),
            _ => false,
        };
        if stale && music.playing() {
            self.engine.music_mut().skip();
        }
    }

    /// The menus' theme's sounds and music (MENU THEME): its sound for each of TA's interface
    /// events (`ta_ui::sound::UiSound::event`), played while the menus are shown
    /// ([`GameAudio::set_in_menus`]), and its music, first in the menus' playlist (before the
    /// soundtrack's own for them). None of either: TA's own.
    pub fn set_menu_theme(&mut self, sounds: std::collections::BTreeMap<String, String>, music: Option<String>) {
        self.menu_sounds = sounds;
        if std::mem::replace(&mut self.menu_music, music) != self.menu_music {
            self.apply_soundtrack();
        }
    }

    /// Whether the menus are shown (or a battle in their theme's look): their theme's interface
    /// sounds play then.
    pub fn set_in_menus(&mut self, in_menus: bool) {
        self.in_menus = in_menus;
    }

    /// Forged Alliance's soundtrack is installed (its music mod on).
    pub fn has_fa_soundtrack(&self) -> bool {
        !self.fa_tracks.is_empty()
    }

    /// The screen's music again, after the jukebox stopped it (a battle's at peace).
    fn resume_mood(&mut self) {
        if self.mood.is_none() {
            let mood = match self.screen_mood {
                Some(Mood::Battle) | None => Mood::Peace,
                Some(mood) => mood,
            };
            self.mood = Some(mood);
            self.engine.set_mood(mood);
        }
    }

    pub fn apply_config(&mut self, config: &ta_config::Audio) {
        if std::mem::replace(&mut self.soundtrack, config.soundtrack) != config.soundtrack {
            self.apply_soundtrack();
        }
        self.music_volume = if config.music.is_finite() { config.music.clamp(0.0, 2.0) } else { 1.0 };
        self.effects_volume = Some(config.effects);
        self.master_volume = if config.master.is_finite() { config.master.clamp(0.0, 2.0) } else { 1.0 };
        self.mute_in_background = config.mute_in_background;
        self.unit_replies = config.unit_replies;
        let audio = ta_config::Audio { master: self.master_volume * self.background_duck, music: self.music_volume * self.music_duck, ..config.clone() };
        self.engine.apply_config(&audio);
        let music = self.engine.music_mut();
        music.set_shuffle(config.music_shuffle);
        music.set_repeat(config.music_repeat);
    }

    /// The jukebox: the track playing again from `seconds` in.
    pub fn seek_music(&mut self, seconds: f64) {
        self.engine.music_mut().seek(seconds);
    }

    /// How loud the music is in each of `out.len()` bands of pitch (0..1), from its latest
    /// sound. False without a sound device (`out` then silent).
    pub fn music_spectrum(&self, out: &mut [f32]) -> bool {
        let mut samples = [0.0f32; ta_audio::spectrum::SAMPLES];
        match self.engine.music_scope(&mut samples) {
            Some(rate) => {
                ta_audio::spectrum::bands(&samples, rate, out);
                true
            }
            None => {
                out.fill(0.0);
                false
            }
        }
    }

    /// The player's own music, and how it plays with TA's (none while logging sounds).
    pub fn set_user_music(&mut self, tracks: &[std::path::PathBuf], mode: ta_config::UserMusic) {
        if self.log.is_none() {
            self.engine.music_mut().set_user_tracks(tracks, mode);
        }
    }

    /// The track playing (a VFS path, or `file:` and a path), and whether music is on.
    pub fn now_playing(&self) -> (Option<String>, bool) {
        let music = self.engine.music();
        (music.now_playing().map(str::to_owned), music.playing())
    }

    /// How far into the track playing the music is, in seconds.
    pub fn music_position(&self) -> Option<f64> {
        self.engine.music_position()
    }

    /// Where TA's music files are read from (the game's files).
    pub fn music_source(&self) -> Arc<dyn ta_audio::SoundSource> {
        self.engine.bank().source().clone()
    }

    /// TA's music files: the battle themes, the peace themes, the briefing's, victory, defeat (as
    /// TA's playlists name them, whatever SOUNDTRACK plays); then Forged Alliance's soundtrack
    /// where it is installed, every track of it (its films' too).
    pub fn ta_tracks(&self) -> Vec<String> {
        use ta_audio::music::Mood;
        let mut tracks: Vec<String> = Vec::new();
        for mood in [Mood::Battle, Mood::Peace, Mood::Briefing, Mood::Victory, Mood::Defeat] {
            let mut list: Vec<String> = self.engine.music().resolve(&self.ta_music, mood).into_iter().filter(|p| !p.starts_with("file:") && !p.starts_with("music/supcom/") && !tracks.contains(p)).collect();
            list.sort_by_key(|p| p.rsplit('/').next().and_then(|f| f.split('.').next()).and_then(|n| n.parse::<u32>().ok()).unwrap_or(u32::MAX));
            tracks.extend(list);
        }
        tracks.extend(self.fa_tracks.iter().map(|t| t.file.clone()));
        tracks
    }

    /// A builder's sound for its work (its category's build sound when it has none of its
    /// own for it).
    fn nano_sound(&mut self, sim: &Sim, builder: ta_sim::UnitId, work: SoundEvent) {
        if let Some(cat) = self.unit_category(sim, builder).cloned() {
            let event = if cat.get(work).is_some() { work } else { SoundEvent::Build };
            let pos = sim.unit(builder).map(|u| u.pos.to_f32()).unwrap_or_default();
            self.engine.play_unit_sound(&cat, event, pos, 0.7);
        }
    }

    /// The interface sound TA plays for an event of `gamedata/allsound.tdf` ("BIGBUTTON").
    pub fn play_interface(&mut self, event: &str) {
        // (In the menus, their theme's sound for it where it has one.)
        let themed = self.in_menus.then(|| self.menu_sounds.get(event).cloned()).flatten();
        if let Some(name) = themed.or_else(|| self.interface.get(event).map(str::to_owned)) {
            self.engine.play_ui(&name, 0.8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fa(parts: &[(&str, usize)]) -> Vec<FaTrack> {
        parts.iter().map(|(part, n)| FaTrack { file: format!("music/supcom/{part}_{n}.flac"), part: part.to_string(), number: *n }).collect()
    }

    /// SOUNDTRACK: TA's playlists as they are; Forged Alliance's in their place (its building
    /// themes at peace, its battle themes in battle, its menu's and briefings' music in the menus;
    /// TA's victory and defeat kept); both together; by side, Forged Alliance's for one of its
    /// factions; and TA's whatever the choice where none of Forged Alliance's is installed.
    #[test]
    fn the_soundtrack_setting_makes_the_playlists() {
        use ta_audio::music::TrackRef;
        use ta_config::Soundtrack as S;
        let ta = ta_audio::MusicConfig::default();
        let fa = fa(&[("menu", 1), ("building", 1), ("building", 2), ("battle", 1), ("briefing", 1), ("film", 1)]);
        let path = |p: &str| TrackRef::Path(p.into());
        let only = soundtrack_music(&ta, &fa, S::ForgedAlliance, false);
        assert_eq!(only.playlists.peace, [path("music/supcom/building_1.flac"), path("music/supcom/building_2.flac")]);
        assert_eq!(only.playlists.battle, [path("music/supcom/battle_1.flac")]);
        assert_eq!(only.playlists.briefing, [path("music/supcom/menu_1.flac"), path("music/supcom/briefing_1.flac")]);
        assert_eq!((&only.playlists.victory, &only.playlists.defeat), (&ta.playlists.victory, &ta.playlists.defeat), "TA's victory and defeat kept");
        let both = soundtrack_music(&ta, &fa, S::Both, false);
        assert_eq!(both.playlists.peace.len(), ta.playlists.peace.len() + 2);
        assert!(both.playlists.peace.starts_with(&ta.playlists.peace));
        assert_eq!(soundtrack_music(&ta, &fa, S::TotalAnnihilation, true).playlists, ta.playlists);
        assert_eq!(soundtrack_music(&ta, &fa, S::BySide, false).playlists, ta.playlists, "a side of TA's");
        assert_eq!(soundtrack_music(&ta, &fa, S::BySide, true).playlists, only.playlists, "one of Forged Alliance's factions");
        assert_eq!(soundtrack_music(&ta, &[], S::ForgedAlliance, true).playlists, ta.playlists, "nothing installed: TA's");
    }

    /// The menus' theme's music first in the menus' list, the soundtrack's own for them after it;
    /// not twice where Forged Alliance's menu theme, the same music, is there already.
    #[test]
    fn the_menus_themes_music_joins_their_list() {
        use ta_audio::music::TrackRef;
        let path = |p: &str| TrackRef::Path(p.into());
        let theme = "music/supcom-menus/menu.flac";
        let mut ta = vec![TrackRef::Number(17)];
        with_theme_music(&mut ta, Some(theme));
        assert_eq!(ta, [path(theme), TrackRef::Number(17)], "TA's soundtrack: the theme's first, TA's after it");
        let mut fa = vec![path(FA_MENU_THEME), path("music/supcom/briefing_1.flac")];
        with_theme_music(&mut fa, Some(theme));
        assert_eq!(fa, [path(FA_MENU_THEME), path("music/supcom/briefing_1.flac")], "the same music not twice");
        let mut none = vec![TrackRef::Number(17)];
        with_theme_music(&mut none, None);
        assert_eq!(none, [TrackRef::Number(17)]);
    }

    /// The music mod's list read from the game's files: each track's file (as the files name
    /// it), part and number; none without it.
    #[test]
    fn the_installed_soundtrack_is_read_from_its_list() {
        let mut source = ta_audio::MemorySource::new();
        assert!(fa_tracks(&source).is_empty());
        source.insert(FA_LIST, b"[[track]]\nfile = \"music/supcom/Menu.flac\"\npart = \"menu\"\nnumber = 1\nseconds = 80.9\n\n[[track]]\nfile = \"music/supcom/battle_2.flac\"\npart = \"battle\"\nnumber = 2\n".to_vec());
        let tracks = fa_tracks(&source);
        assert_eq!(tracks, fa(&[("menu", 1), ("battle", 2)]).into_iter().enumerate().map(|(i, t)| if i == 0 { FaTrack { file: "music/supcom/menu.flac".into(), ..t } } else { t }).collect::<Vec<_>>());
        source.insert(FA_LIST, b"not toml [[".to_vec());
        assert!(fa_tracks(&source).is_empty(), "a list that does not read: none");
    }

    /// Game files of TA's music (a track of each default playlist: battle 2, peace 11, the
    /// briefing's 17, victory 0, defeat 1) and Forged Alliance's soundtrack (a building theme, a
    /// battle theme, the menu's, and its list), in a folder of the test's own: the folder, and
    /// the files mounted.
    fn music_files(name: &str) -> (std::path::PathBuf, Arc<Vfs>) {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ta-audio-{name}-{}-{nanos}", std::process::id()));
        for file in ["music/2.ogg", "music/11.ogg", "music/17.ogg", "music/0.ogg", "music/1.ogg", "music/supcom/building_1.flac", "music/supcom/battle_1.flac", "music/supcom/menu.flac"] {
            let at = dir.join(file);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, b"").unwrap();
        }
        std::fs::write(dir.join(FA_LIST), "[[track]]
file = \"music/supcom/building_1.flac\"
part = \"building\"
number = 1

[[track]]
file = \"music/supcom/battle_1.flac\"
part = \"battle\"
number = 1

[[track]]
file = \"music/supcom/menu.flac\"
part = \"menu\"
number = 1
").unwrap();
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&dir, "music").unwrap();
        (dir, Arc::new(vfs))
    }

    fn audio_of(vfs: Arc<Vfs>) -> GameAudio {
        let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
        GameAudio::with_engine(AudioEngine::new_offline(AudioSettings::default(), bank, 1000), vfs)
    }

    /// The soundtrack chosen rebuilds the playlists, never how they play: the player's shuffle
    /// off stays off, as the soundtrack is applied and as the side changes it.
    #[test]
    fn the_soundtrack_keeps_the_players_shuffle() {
        let (dir, vfs) = music_files("shuffle");
        let mut audio = audio_of(vfs);
        audio.engine.music_mut().set_shuffle(false);
        audio.soundtrack = ta_config::Soundtrack::BySide;
        audio.apply_soundtrack();
        assert!(!audio.engine.music().shuffle());
        audio.set_side(true);
        assert_eq!(audio.engine.music().playlist(Mood::Peace), ["music/supcom/building_1.flac"], "Forged Alliance's, for one of its factions");
        assert!(!audio.engine.music().shuffle(), "still off");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The jukebox lists TA's music whatever plays: with Forged Alliance's alone, TA's battle,
    /// peace and briefing themes are there all the same, before Forged Alliance's.
    #[test]
    fn the_jukebox_lists_tas_music_whatever_plays() {
        let (dir, vfs) = music_files("jukebox");
        let mut audio = audio_of(vfs);
        audio.soundtrack = ta_config::Soundtrack::ForgedAlliance;
        audio.apply_soundtrack();
        assert!(audio.engine.music().playlist(Mood::Battle).iter().all(|p| p.starts_with("music/supcom/")), "Forged Alliance's plays");
        let tracks = audio.ta_tracks();
        assert_eq!(&tracks[..5], ["music/2.ogg", "music/11.ogg", "music/17.ogg", "music/0.ogg", "music/1.ogg"], "{tracks:?}");
        assert!(tracks.contains(&"music/supcom/menu.flac".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A replay's film plays the soundtrack chosen (its music on); with its music off, none.
    #[test]
    fn a_films_music_is_the_soundtrack_chosen() {
        let (dir, vfs) = music_files("film");
        let config = ta_config::Audio { soundtrack: ta_config::Soundtrack::ForgedAlliance, ..Default::default() };
        let film = GameAudio::film(vfs.clone(), &config, 1000, false, true);
        assert_eq!(film.engine.music().playlist(Mood::Peace), ["music/supcom/building_1.flac"]);
        let silent = GameAudio::film(vfs, &config, 1000, false, false);
        assert!(silent.engine.music().playlist(Mood::Peace).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn offline_audio() -> GameAudio {
        let vfs = Arc::new(Vfs::new());
        let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
        bank.insert("test-tone", ta_audio::SoundData::new(1000, 1, vec![0.1; 10_000]).unwrap());
        let engine = AudioEngine::new_offline(AudioSettings::default(), bank, 1000);
        GameAudio::with_engine(engine, vfs)
    }

    /// UNIT REPLIES off silences what units say back to the player, never what warns them.
    #[test]
    fn replies_are_the_units_answers_not_their_warnings() {
        for event in [SoundEvent::Select, SoundEvent::Ok, SoundEvent::Arrived, SoundEvent::Cant] {
            assert!(is_reply(event), "{event:?}");
        }
        for event in [SoundEvent::UnderAttack, SoundEvent::UnitComplete, SoundEvent::Count(3), SoundEvent::CancelDestruct, SoundEvent::Build] {
            assert!(!is_reply(event), "{event:?}");
        }
        let mut audio = offline_audio();
        assert!(audio.unit_replies, "on by default");
        audio.apply_config(&ta_config::Audio { unit_replies: false, ..Default::default() });
        assert!(!audio.unit_replies);
    }

    #[test]
    fn the_jukebox_stopped_then_started_plays_the_screens_music_again() {
        use crate::chat::MusicRequest;
        let mut audio = offline_audio();
        // In a battle: its music, not the menus'.
        audio.set_mood(Mood::Peace);
        audio.music_request(MusicRequest::Stop);
        assert_eq!(audio.mood, None);
        audio.music_request(MusicRequest::Start);
        assert_eq!(audio.mood, Some(Mood::Peace));
        // A track chosen while it is stopped: the screen's music goes on after it.
        audio.music_request(MusicRequest::Stop);
        audio.music_request(MusicRequest::Previous);
        assert_eq!(audio.mood, Some(Mood::Peace), "the battle's, and it follows the fighting again");
        // On the menus, theirs.
        audio.set_mood(Mood::Briefing);
        audio.music_request(MusicRequest::Stop);
        audio.music_request(MusicRequest::Start);
        assert_eq!(audio.mood, Some(Mood::Briefing));
    }

    #[test]
    fn changing_scenes_clears_old_sounds_and_combat_but_keeps_music() {
        let mut audio = offline_audio();
        let camera = Camera::new(glam::Vec2::new(800.0, 600.0));
        audio.set_mood(Mood::Briefing);
        audio.play_narration("test-tone");
        let narration = audio.narration;
        let effect = audio.engine.play_ui("test-tone", 0.5);
        audio.intensity = 50.0;
        audio.reset_scene();
        assert!(!audio.engine.is_playing(narration));
        assert!(!audio.engine.is_playing(effect));
        assert_eq!(audio.intensity, 0.0);
        assert_eq!(audio.engine.music().mood(), Some(Mood::Briefing));
        // Wind comes back on the next battlefield, even after its old handle was released.
        audio.set_wind(10.0, &camera, 0.1);
        assert!(audio.engine.is_playing(audio.wind));
        audio.set_mood(Mood::Peace);
        audio.update(&camera, None, 0.1);
        assert_eq!(audio.engine.music().mood(), Some(Mood::Peace));
    }

    #[test]
    fn narration_lowers_music_then_restores_the_current_slider_level() {
        let mut audio = offline_audio();
        audio.play_narration("test-tone");
        audio.update_idle(0.5);
        assert!(audio.engine.volume(ta_audio::Bus::Music) < 0.2);
        // Changing the slider during narration retains the quieter mix.
        let mut config = ta_config::Audio { music: 0.4, ..Default::default() };
        audio.apply_config(&config);
        assert!(audio.engine.volume(ta_audio::Bus::Music) < 0.15);
        audio.stop_narration();
        audio.update_idle(4.0);
        assert!((audio.engine.volume(ta_audio::Bus::Music) - 0.4).abs() < 0.003);
        // Muted speech should not suppress the soundtrack, nor may ducking unmute music.
        audio.play_narration("test-tone");
        config.voice = 0.0;
        audio.apply_config(&config);
        audio.update_idle(4.0);
        assert!((audio.engine.volume(ta_audio::Bus::Music) - 0.4).abs() < 0.001);
        config.music = 0.0;
        audio.apply_config(&config);
        audio.stop_narration();
        audio.update_idle(4.0);
        assert_eq!(audio.engine.volume(ta_audio::Bus::Music), 0.0);
    }

    #[test]
    fn a_briefing_silences_the_music_and_a_tutorial_line_only_lowers_it() {
        let mut audio = offline_audio();
        audio.apply_config(&ta_config::Audio { music: 0.5, ..Default::default() });
        audio.play_briefing("test-tone");
        assert!(audio.briefing_speaking());
        audio.update_idle(0.5);
        audio.update_idle(0.5);
        assert_eq!(audio.engine.volume(ta_audio::Bus::Music), 0.0, "silent under the briefing");
        // Back to the slider's level once it has been said.
        audio.stop_narration();
        assert!(!audio.briefing_speaking());
        audio.update_idle(6.0);
        assert!((audio.engine.volume(ta_audio::Bus::Music) - 0.5).abs() < 0.003);
        // The tutorial's lines: the music lower, not gone, and nothing else steps aside.
        audio.play_narration("test-tone");
        assert!(!audio.briefing_speaking());
        audio.update_idle(1.0);
        let level = audio.engine.volume(ta_audio::Bus::Music);
        assert!(level > 0.1 && level < 0.2, "{level}");
        // (A briefing after a tutorial line, and the other way about: each its own way.)
        audio.play_briefing("test-tone");
        audio.update_idle(1.0);
        assert_eq!(audio.engine.volume(ta_audio::Bus::Music), 0.0);
        audio.play_narration("test-tone");
        audio.update_idle(2.0);
        assert!(audio.engine.volume(ta_audio::Bus::Music) > 0.1);
        // With the voice off, a briefing does not silence anything.
        audio.apply_config(&ta_config::Audio { music: 0.5, voice: 0.0, ..Default::default() });
        audio.play_briefing("test-tone");
        assert!(!audio.briefing_speaking());
        audio.update_idle(6.0);
        assert!((audio.engine.volume(ta_audio::Bus::Music) - 0.5).abs() < 0.003);
    }

    #[test]
    fn replaying_the_intro_clears_background_audio_and_music() {
        let mut audio = offline_audio();
        audio.set_mood(Mood::Briefing);
        audio.play_narration("test-tone");
        let narration = audio.narration;
        audio.begin_intro();
        audio.update_idle(0.2);
        assert_eq!(audio.engine.music().mood(), None);
        assert!(!audio.engine.is_playing(narration));
        assert!(audio.wind.is_none());
    }

    #[test]
    fn camera_cut_places_new_sounds_before_culling_them() {
        let mut audio = offline_audio();
        let mut camera = Camera::new(glam::Vec2::new(800.0, 600.0));
        camera.focus = glam::Vec3::new(10_000.0, 0.0, 10_000.0);
        audio.set_listener(&camera, None);
        let sound = audio.engine.play_at("test-tone", camera.focus.to_array(), 1.0);
        assert!(!sound.is_none(), "the new camera hears a sound that the old listener would cull");
    }

    #[test]
    fn classic_sound_panning_ignores_the_previous_perspective_rotation() {
        let mut audio = offline_audio();
        let mut camera = Camera::new(glam::Vec2::new(1280.0, 720.0));
        camera.focus = glam::Vec3::new(1000.0, 50.0, 1000.0);
        camera.yaw = std::f32::consts::PI;
        let settings = ta_audio::SpatialSettings::default();
        let east = (camera.focus + glam::Vec3::X * 200.0).to_array();
        audio.set_listener(&camera, None);
        assert!(ta_audio::spatialize(audio.engine.listener(), &settings, east).pan < 0.0,
            "east is left in the rotated perspective camera");
        camera.mode = ta_render::CameraMode::Classic;
        audio.set_listener(&camera, None);
        assert!(ta_audio::spatialize(audio.engine.listener(), &settings, east).pan > 0.0,
            "east is always right in the classic picture");
        let north = (camera.focus - glam::Vec3::Z * 200.0).to_array();
        assert_eq!(ta_audio::spatialize(audio.engine.listener(), &settings, north).pan, 0.0);
        assert_eq!(audio.engine.listener().position, camera.focus.to_array());
        // The retained yaw still takes effect when returning to the perspective camera.
        camera.mode = ta_render::CameraMode::Perspective;
        audio.set_listener(&camera, None);
        assert!(ta_audio::spatialize(audio.engine.listener(), &settings, east).pan < 0.0);
    }

    #[test]
    fn music_waits_for_the_intro_and_the_menus_keep_theirs() {
        let vfs = Arc::new(Vfs::new());
        let settings = AudioSettings::from(&ta_config::Audio::default());
        let engine = AudioEngine::new_silent(settings, Arc::new(SoundBank::from_vfs(vfs.clone())));
        let mut audio = GameAudio::with_engine(engine, vfs);
        let camera = Camera::new(glam::Vec2::new(800.0, 600.0));
        let mood = |audio: &GameAudio| audio.engine.music().mood();
        // The intro's movies play alone.
        audio.update(&camera, None, 0.1);
        assert_eq!(mood(&audio), None);
        // The title screen's music stays however the battle behind it goes.
        audio.set_mood(Mood::Briefing);
        audio.intensity = 20.0;
        audio.update(&camera, None, 0.1);
        assert_eq!(mood(&audio), Some(Mood::Briefing));
        // In battle the music follows the fighting.
        audio.set_mood(Mood::Peace);
        audio.update(&camera, None, 0.1);
        assert_eq!(mood(&audio), Some(Mood::Battle));
        audio.intensity = 0.0;
        audio.update(&camera, None, 0.1);
        assert_eq!(mood(&audio), Some(Mood::Peace));
    }

    #[test]
    fn riding_along_hears_from_inside_the_unit() {
        let vfs = Arc::new(Vfs::new());
        let settings = AudioSettings::from(&ta_config::Audio::default());
        let engine = AudioEngine::new_silent(settings, Arc::new(SoundBank::from_vfs(vfs.clone())));
        let mut audio = GameAudio::with_engine(engine, vfs);
        // The map's camera elsewhere; the eye in a unit, looking along +z.
        let camera = Camera::new(glam::Vec2::new(1920.0, 1080.0));
        let eye = glam::Vec3::new(3000.0, 40.0, 2500.0);
        audio.update(&camera, Some((eye, glam::Vec3::new(0.0, -0.2, 1.0))), 0.1);
        let s = ta_audio::SpatialSettings::default();
        let heard = |p: glam::Vec3| ta_audio::spatialize(audio.engine.listener(), &s, p.to_array());
        // Its own guns at full volume; a fight 300 units off still heard, one across the map not.
        assert!(heard(eye + glam::Vec3::new(0.0, -10.0, 8.0)).gain > 0.95);
        assert!(heard(eye + glam::Vec3::new(0.0, 0.0, 300.0)).gain > 0.3);
        assert_eq!(heard(eye + glam::Vec3::new(0.0, 0.0, 3000.0)).gain, 0.0);
        // Looking along +z, -x is on the right.
        assert!(heard(eye + glam::Vec3::new(-150.0, 0.0, 0.0)).pan > 0.1);
        assert!(heard(eye + glam::Vec3::new(150.0, 0.0, 0.0)).pan < -0.1);
    }

    /// A battle for the unit sounds' tests: a tank whose category gives its moves and death (a
    /// sound set), a builder, a radar at work, a commander of a side with an announcer, a gun of a
    /// loop, a burst gun of one sound a burst, a nuke; the sounds in a sound add-on, their files
    /// in the bank. The audio logs what it plays.
    fn sounding() -> (std::path::PathBuf, GameAudio, Sim) {
        use ta_sim::{Fx, FxVec3, PlayerSetup, UnitTypes};
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ta-unit-sounds-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("gamedata")).unwrap();
        std::fs::write(dir.join("gamedata/sound_test.tdf"), "[MOVER]\n{\nstartmove=go;\nstopmove=halt;\nmoveloop=engine;\ndeath=BOOMSET;\ncrash=wreck;\nok1=yes;\narrived1=there;\n}\n\
            [WORKER]\n{\nbuildloop=drill;\nbuildstop=drilled;\n}\n[DISH]\n{\nactiveloop=hum;\nopen=unfold;\nclose=fold;\ndonebuilt=done;\n}\n\
            [ALERTS_UEF]\n{\ncommanderattacked=help;\nnuclearlaunch=launch;\n}\n[STARTER]\n{\nstartmove=go;\nstopmove=halt;\n}\n[FARMOVER]\n{\nmoveloop=FARSET;\n}\n\
            [SOUNDSETS]\n{\n[BOOMSET]\n{\nsound1=boom;\nvolume=0.5;\n}\n[FARSET]\n{\nsound1=engine;\nradius=3;\n}\n}\n").unwrap();
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&dir, "sounds").unwrap();
        let vfs = Arc::new(vfs);
        let bank = Arc::new(SoundBank::from_vfs(vfs.clone()));
        for name in ["go", "halt", "engine", "boom", "wreck", "yes", "there", "drill", "drilled", "hum", "unfold", "fold", "done", "help", "launch", "whirr", "spun", "fire", "bubble", "zap", "xplomed2", "xplosml2"] {
            bank.insert(name, ta_audio::SoundData::new(1000, 1, vec![0.2; 3000]).unwrap());
        }
        let mut engine = AudioEngine::new_offline(AudioSettings::default(), bank, 1000);
        engine.set_logging(true);
        let audio = GameAudio::with_engine(engine, vfs);
        let mut data = ta_data::GameData::load(&Vfs::new());
        for text in [
            "[UNITINFO] { UnitName=TANK; CanMove=1; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; SoundCategory=MOVER; }",
            "[UNITINFO] { UnitName=RADAR; FootprintX=2; FootprintZ=2; SoundCategory=DISH; OnOffable=1; }",
            "[UNITINFO] { UnitName=BOSS; Commander=1; CanMove=1; MaxVelocity=1; FootprintX=2; FootprintZ=2; SoundCategory=MOVER; }",
            "[UNITINFO] { UnitName=STARTER; CanMove=1; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; SoundCategory=STARTER; }",
            "[UNITINFO] { UnitName=FARTANK; CanMove=1; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; SoundCategory=FARMOVER; }",
        ] {
            data.units.push(ta_data::UnitDef::from_tdf(&ta_formats::tdf::parse(text).sections[0]).unwrap());
        }
        data.weapons.push(ta_data::WeaponDef { name: "SPINNER".into(), sound_start: "fire".into(), sound_loop: "whirr".into(), sound_stop: "spun".into(), sound_start_water: "bubble".into(), reload_time: 0.1, ..Default::default() });
        data.weapons.push(ta_data::WeaponDef { name: "EMG".into(), sound_start: "fire".into(), sound_trigger: true, ..Default::default() });
        data.weapons.push(ta_data::WeaponDef { name: "NUKE".into(), stockpile: true, targetable: true, area_of_effect: 512.0, ..Default::default() });
        data.sides.push(ta_data::SideDef { name: "UEF".into(), ..Default::default() });
        let data = Arc::new(data);
        let mut types = UnitTypes::compile(&data, &|_| None);
        // (The radar's script plays its sound 0.)
        let script = ta_formats::cob::CobScript { sounds: vec!["zap".into()], ..Default::default() };
        types.types[1].script = Some(Arc::new(ta_sim::cob::CobProgram::new(script, &[])));
        let map = ta_formats::tnt::TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![0; 4096], features: vec![ta_formats::tnt::FEATURE_NONE; 4096], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let player = PlayerSetup { side: "UEF".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false };
        let sim = Sim::new(Arc::new(types), data, &map, &ta_data::MapInfo::default(), 0, &[player], 7);
        (dir, audio, sim)
    }

    /// The files started since the last look, in order.
    fn started(audio: &mut GameAudio) -> Vec<String> {
        audio.engine.take_log().into_iter().filter_map(|e| match e {
            ta_audio::PlayEvent::Start(s) => Some(s.path.trim_start_matches("sounds/").trim_end_matches(".wav").to_string()),
            _ => None,
        }).filter(|p| p != "__wind").collect()
    }

    fn step(sim: &mut Sim, audio: &mut GameAudio, ticks: usize, commands: &[ta_sim::sim::Command]) {
        for i in 0..ticks {
            sim.step(if i == 0 { commands } else { &[] });
            for e in sim.take_events() {
                audio.on_event(&e, sim, 0, None);
            }
            audio.update_units(sim, None, 0, false);
            audio.update_idle(1.0 / 30.0);
        }
    }

    /// A unit heard setting off (`startmove`), on its way (`moveloop`, a loop following it),
    /// coming to a stop (`stopmove`, the loop stopped), arriving where the player sent it
    /// (`arrived`, after its `ok`), dying (`death`, a set, in the blast's place); a building
    /// opening (`open`), at work (`activeloop`), closing (`close`).
    #[test]
    fn units_are_heard_as_they_go_about_the_battle() {
        use ta_sim::{FxVec3, Order};
        let (dir, mut audio, mut sim) = sounding();
        let tank = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 0, 100), 0, true).unwrap();
        step(&mut sim, &mut audio, 2, &[]);
        assert!(started(&mut audio).is_empty(), "nothing on the first look");
        audio.unit_voice(&sim, tank, SoundEvent::Ok);
        let go = ta_sim::sim::Command::Order { player: 0, units: vec![tank], order: Order::Move(FxVec3::from_ints(300, 0, 100)), queue: false };
        step(&mut sim, &mut audio, 10, &[go]);
        let heard = started(&mut audio);
        assert_eq!(heard.iter().take(3).map(String::as_str).collect::<Vec<_>>(), ["yes", "go", "engine"], "{heard:?}");
        assert_eq!(audio.units.playing()[0], 1, "the loop on");
        step(&mut sim, &mut audio, 240, &[]);
        let heard = started(&mut audio);
        assert!(heard.contains(&"halt".to_string()) && heard.contains(&"there".to_string()), "{heard:?}");
        assert_eq!(audio.units.playing()[0], 0, "the loop off");
        // Its death: its own sound (the set's file), not the blast's.
        audio.on_event(&SimEvent::Explosion { weapon: 0, pos: FxVec3::from_ints(300, 0, 100), water: false, when: ta_sim::Fx::ZERO, player: Some(0), death_of: Some(0) }, &sim, 0, None);
        audio.on_event(&SimEvent::UnitExploded { id: tank, pos: FxVec3::from_ints(700, 0, 100), big: false, type_id: 0 }, &sim, 0, None);
        assert_eq!(started(&mut audio), ["boom", "boom"]);
        audio.on_event(&SimEvent::UnitExploded { id: tank, pos: FxVec3::from_ints(300, 0, 100), big: false, type_id: 1 }, &sim, 0, None);
        assert_eq!(started(&mut audio), ["xplomed2"], "TA's blast for a unit without one");
        // A wreck crashing: its own; a building coming down without one: TA's.
        audio.play_effect(&sim, crate::effects::EffectSound::of_unit(0, SoundEvent::Crash, "xplomed2", glam::Vec3::new(100.0, 0.0, 100.0), 0.8));
        audio.play_effect(&sim, crate::effects::EffectSound::of_unit(1, SoundEvent::Collapse, "xplosml2", glam::Vec3::new(100.0, 0.0, 100.0), 0.8));
        assert_eq!(started(&mut audio), ["wreck", "xplosml2"]);
        // The radar: done where it stands, open, at work, closed.
        let radar = sim.spawn_unit(1, 0, FxVec3::from_ints(200, 0, 300), 0, false).unwrap();
        step(&mut sim, &mut audio, 1, &[]);
        audio.on_event(&SimEvent::UnitCompleted { id: radar, type_id: 1 }, &sim, 0, None);
        assert_eq!(started(&mut audio), ["done"]);
        let dish = sim.spawn_unit(1, 0, FxVec3::from_ints(250, 0, 300), 0, true).unwrap();
        let switch = |on| ta_sim::sim::Command::SetActive { player: 0, units: vec![dish], on };
        step(&mut sim, &mut audio, 2, &[switch(false)]);
        started(&mut audio);
        step(&mut sim, &mut audio, 3, &[switch(true)]);
        assert_eq!(started(&mut audio), ["unfold", "hum"], "opened, then at work");
        assert_eq!(audio.units.playing()[2], 1);
        step(&mut sim, &mut audio, 3, &[switch(false)]);
        assert_eq!(started(&mut audio), ["fold"]);
        assert_eq!(audio.units.playing()[2], 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A weapon's loop on from its first shot while the shots keep coming, its `soundstop` once
    /// they stop; a burst's sound once a burst (`soundtrigger`); a shot from under the water its
    /// own; the announcer for the commander and a launch (once for a salvo); a script's sound.
    #[test]
    fn weapons_announcers_and_scripts_are_heard() {
        use ta_sim::{Fx, FxVec3};
        let (dir, mut audio, mut sim) = sounding();
        let tank = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 0, 100), 0, true).unwrap();
        let boss = sim.spawn_unit(2, 0, FxVec3::from_ints(150, 0, 100), 0, true).unwrap();
        let radar = sim.spawn_unit(1, 0, FxVec3::from_ints(200, 0, 100), 0, true).unwrap();
        step(&mut sim, &mut audio, 2, &[]);
        started(&mut audio);
        let fired = |weapon, y, burst_start| SimEvent::WeaponFired { unit: tank, weapon, pos: FxVec3::from_ints(100, y, 100), target: FxVec3::from_ints(400, 0, 100), burst_start };
        audio.on_event(&fired(0, 10, true), &sim, 0, None);
        assert_eq!(started(&mut audio), ["fire", "whirr"]);
        for _ in 0..4 {
            step(&mut sim, &mut audio, 3, &[]);
            audio.on_event(&fired(0, 10, false), &sim, 0, None);
        }
        assert_eq!(started(&mut audio), ["fire"; 4], "the loop goes on, unstarted");
        assert_eq!(audio.units.playing()[3], 1);
        step(&mut sim, &mut audio, 30, &[]);
        assert_eq!(started(&mut audio), ["spun"], "stopped as the shots stopped");
        assert_eq!(audio.units.playing()[3], 0);
        audio.on_event(&fired(0, -20, true), &sim, 0, None);
        assert_eq!(started(&mut audio)[0], "bubble", "from under the water");
        audio.on_event(&fired(1, 10, false), &sim, 0, None);
        assert!(started(&mut audio).is_empty(), "a burst's sound not again");
        audio.on_event(&fired(1, 10, true), &sim, 0, None);
        assert_eq!(started(&mut audio), ["fire"]);
        // The announcer: the commander attacked (a unit not: its own, TA's), launches.
        audio.on_event(&SimEvent::UnderAttack { player: 0, unit: boss, pos: FxVec3::ZERO }, &sim, 0, None);
        audio.on_event(&SimEvent::UnderAttack { player: 0, unit: radar, pos: FxVec3::ZERO }, &sim, 0, None);
        assert_eq!(started(&mut audio), ["help"], "the radar has no underattack and TA's warning is not in the bank");
        audio.on_event(&fired(2, 10, true), &sim, 0, None);
        audio.on_event(&fired(2, 10, true), &sim, 0, None);
        assert_eq!(started(&mut audio), ["launch"], "a salvo announced once");
        // The radar's script plays its sound.
        audio.on_event(&SimEvent::ScriptSound { unit: radar, sound: 0 }, &sim, 0, None);
        audio.on_event(&SimEvent::ScriptSound { unit: radar, sound: 9 }, &sim, 0, None);
        assert_eq!(started(&mut audio), ["zap"]);
        // A shield's hook, and the scene's end stopping every loop.
        audio.on_event(&fired(0, 10, true), &sim, 0, None);
        audio.reset_scene();
        assert_eq!(audio.units.playing(), [0; 4]);
        audio.shield_changed(&sim, radar, true);
        let _ = (Fx::ZERO, std::fs::remove_dir_all(dir));
    }

    /// Of many units on the move, the nearest alone are heard looping; none while the battle is
    /// paused.
    #[test]
    fn only_the_nearest_units_loop() {
        use ta_sim::{FxVec3, Order};
        let (dir, mut audio, mut sim) = sounding();
        let tanks: Vec<_> = (0..14).map(|i| sim.spawn_unit(0, 0, FxVec3::from_ints(100 + 40 * i, 0, 100 + 40 * (i % 3)), 0, true).unwrap()).collect();
        step(&mut sim, &mut audio, 2, &[]);
        let go = ta_sim::sim::Command::Order { player: 0, units: tanks, order: Order::Move(FxVec3::from_ints(600, 0, 800)), queue: false };
        step(&mut sim, &mut audio, 20, &[go]);
        assert_eq!(audio.units.playing()[0], 8, "the eight nearest");
        // (Silent while the battle stands still, back as it goes on.)
        audio.update_units(&sim, None, 0, true);
        assert_eq!(audio.units.playing(), [0; 4]);
        audio.update_units(&sim, None, 0, false);
        assert_eq!(audio.units.playing()[0], 8);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Units that only set off and stop (`startmove`, no `moveloop`) take no loop's place: ten
    /// of them nearer leave a farther tank's loop on.
    #[test]
    fn a_unit_without_a_loop_takes_no_loops_place() {
        use ta_sim::{FxVec3, Order};
        let (dir, mut audio, mut sim) = sounding();
        let mut movers: Vec<_> = (0..10).map(|i| sim.spawn_unit(3, 0, FxVec3::from_ints(100 + 20 * i, 0, 100), 0, true).unwrap()).collect();
        movers.push(sim.spawn_unit(0, 0, FxVec3::from_ints(400, 0, 300), 0, true).unwrap());
        step(&mut sim, &mut audio, 2, &[]);
        let go = ta_sim::sim::Command::Order { player: 0, units: movers, order: Order::Move(FxVec3::from_ints(600, 0, 700)), queue: false };
        step(&mut sim, &mut audio, 10, &[go]);
        assert_eq!(audio.units.playing()[0], 1, "the tank's loop on, the ten without one taking no place");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A unit whose sound reaches farther (a sound set's `radius`) is listened to farther than an
    /// ordinary sound: its loop heard where a plain tank's is not.
    #[test]
    fn a_far_reaching_sound_is_listened_to_farther() {
        use ta_sim::{FxVec3, Order};
        let (dir, mut audio, mut sim) = sounding();
        // (A narrow view: an ordinary sound heard to 440 px round the listener.)
        audio.engine.set_listener([0.0; 3], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 400.0);
        let far = sim.spawn_unit(4, 0, FxVec3::from_ints(600, 0, 300), 0, true).unwrap();
        let plain = sim.spawn_unit(0, 0, FxVec3::from_ints(600, 0, 340), 0, true).unwrap();
        step(&mut sim, &mut audio, 2, &[]);
        let go = ta_sim::sim::Command::Order { player: 0, units: vec![far, plain], order: Order::Move(FxVec3::from_ints(700, 0, 400)), queue: false };
        step(&mut sim, &mut audio, 10, &[go]);
        assert_eq!(audio.units.playing()[0], 1, "the far-reaching one alone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wind_loop_is_clean() {
        let wind = super::synth_wind().expect("wind");
        let s = wind.samples();
        assert!(s.iter().all(|v| v.is_finite()), "no NaN or infinity");
        let rms = (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        // The loop point: the last frame runs into the first without a jump.
        let n = s.len();
        let seam = ((s[n - 2] - s[0]).abs()).max((s[n - 1] - s[1]).abs());
        let typical = s.windows(4).step_by(997).map(|w| (w[2] - w[0]).abs()).fold(0.0f32, f32::max);
        println!("wind: {} frames, rms {rms:.3}, peak {peak:.3}, seam step {seam:.4}, largest step {typical:.4}", wind.frames());
        assert!(rms > 0.05 && peak <= 0.91, "audible and never clipping");
        assert!(seam <= typical * 1.5 + 0.01, "no click at the loop point");
    }
}
