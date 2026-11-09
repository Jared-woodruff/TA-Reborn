//! Game audio for TA Reborn.
//!
//! # What Total Annihilation ships
//!
//! * **Sound effects**: 461 files under `sounds/` (plus 4 installer sounds). All but two are
//!   RIFF WAVE, mono, unsigned 8-bit PCM at 11 025 Hz (`cdoggy.wav` uses 22 254 Hz). The two
//!   oddities are `sing.wav`, a Humongous Entertainment `DIGI` container (8-bit PCM, rate in its
//!   `HSHD` header), and `honk.wav`, headerless 8-bit PCM. Neither is referenced by any data
//!   file, but both decode. No ADPCM is used by the shipped data; the WAV decoder still handles
//!   IMA and Microsoft ADPCM, G.711 and float PCM for mods.
//! * **Briefings**: `camps/briefs/*.wav` narration, mostly 16-bit mono 22 050 Hz (four are
//!   16-bit stereo 44 100 Hz). Mission `.ota` files name them with `narration=`.
//! * **Unit sounds**: each unit's FBI names a `SoundCategory=` section of
//!   `gamedata/sound.tdf` (120 sections) holding `select1`, `ok1`, `arrived1`, `cant1`,
//!   `underattack`, `build`, `repair`, `working`, `activate`, `deactivate`, `unitcomplete`,
//!   `cloak`, `uncloak`, `capture`, `count0`..`count5` (self-destruct countdown) and
//!   `canceldestruct`. See [`categories`].
//! * **Interface sounds**: `gamedata/allsound.tdf` maps UI events to `sound=` names.
//! * **Weapons**: `soundstart` (fire), `soundhit` (impact), `soundwater` (water impact) and
//!   `soundtrigger=1` (repeat the fire sound for every shot of a burst).
//! * **Music**: `music/0.mp3` .. `music/17.mp3` in the install folder, MPEG-1 layer III,
//!   44.1 kHz stereo, 320 kbit/s (tracks 9 and 10 are byte-identical copies of 0 and 1).
//!   TA's own music menu (`guis/music.gui`) classified tracks as
//!   Building / Battle / Victory / Defeat, which [`music::Mood`] mirrors.
//!
//! # Architecture
//!
//! ```text
//!  game thread                                   audio thread (cpal callback)
//!  ───────────                                   ────────────────────────────
//!  AudioEngine ── Command queue (rtrb, SPSC) ──▶ Mixer: voices, 2 music decks,
//!    ├ SoundBank (lazy decode + cache)             bus gains, limiter
//!    ├ VoiceManager (anti-spam, priorities)  ◀── Event queue (ended voices/decks)
//!    └ MusicPlayer (moods, playlists)
//!          └ decoder threads (symphonia) ── PCM ring buffers ──▶ decks
//! ```
//!
//! The mixer never allocates, locks or frees on the audio thread. Everything the mixer does can
//! also be run synchronously ([`AudioEngine::new_offline`] + [`AudioEngine::render_offline`]),
//! which is how the engine is tested without a sound card. When no device is available,
//! [`AudioEngine::new`] returns a silent engine instead of failing.
//!
//! On request the engine also records what it tells the mixer ([`playlog`]: each sound started,
//! with its path, kind, gains, pan and pitch, and each change to it), from which the game writes
//! a film's sound log (`TA_SOUND_LOG`).

mod bank;
pub mod categories;
pub mod decode;
mod dsp;
mod engine;
pub mod length;
mod mixer;
pub mod music;
mod output;
pub mod playlog;
mod rng;
mod spatial;
pub mod spectrum;
mod stream;
pub mod tags;
mod voices;

pub use bank::{MemorySource, SoundBank, SoundSource, sound_path};
pub use categories::{SoundCategories, SoundCategory, SoundEvent, UiSounds, WeaponSounds};
pub use decode::{DecodeError, SoundData};
pub use engine::{AudioEngine, AudioSettings, AudioStats, Bus, PlayParams, Priority, SoundHandle};
pub use music::{Arrangement, Mood, MusicConfig, MusicPlayer};
pub use output::OutputInfo;
pub use playlog::{PlayEvent, PlayStart, SoundKind};
pub use spatial::{Listener, Spatial, SpatialSettings, distance_gain, pan_gains, spatialize};
pub use voices::VoiceStats;

thread_local! {
    /// How many [`guarded`] calls this thread is inside.
    static GUARDED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Runs `read`, a reader of music files from anywhere (the player's own, a mod's), with its
/// panics caught: `None` then. (Symphonia panics on some damaged files, a WAV of 0 Hz, an M4A
/// whose timescale is 0: such a file cannot be read, nothing more.) The game's and the
/// launcher's panic hooks only log a panic [`guarded_panic`] says is one of these.
pub fn guarded<T>(read: impl FnOnce() -> T) -> Option<T> {
    GUARDED.with(|g| g.set(g.get() + 1));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(read));
    GUARDED.with(|g| g.set(g.get().saturating_sub(1)));
    result.ok()
}

/// Whether the panic under way is inside [`guarded`] (caught, a file that cannot be read): a
/// panic hook logs it and says nothing more.
pub fn guarded_panic() -> bool {
    GUARDED.with(|g| g.get() > 0)
}

#[cfg(test)]
mod guard_tests {
    #[test]
    fn a_guarded_panic_is_caught_and_told_apart() {
        assert!(!super::guarded_panic());
        assert_eq!(super::guarded(super::guarded_panic), Some(true));
        assert_eq!(super::guarded(|| -> u32 { panic!("a damaged file") }), None);
        assert!(!super::guarded_panic(), "and over once it is caught");
    }
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("no audio output device is available")]
    NoDevice,
    #[error("audio device error: {0}")]
    Device(String),
    #[error("sound not found: {0}")]
    NotFound(String),
    #[error("{path}: {source}")]
    Decode { path: String, source: DecodeError },
    #[error("music decoding failed: {0}")]
    Music(String),
    #[error("invalid music configuration: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AudioError>;
