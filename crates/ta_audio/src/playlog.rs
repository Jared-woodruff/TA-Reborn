//! A record of what the mixer plays, for films: each sound started on a mixer voice, at the
//! gains, pan and pitch it plays at for the listener of that moment, and each later change to it
//! (the listener moved, another request merged into it, it stopped). The game takes it every
//! frame ([`crate::AudioEngine::take_log`]) and writes it down on its frames' clock
//! (`TA_SOUND_LOG`), so a film's soundtrack can be mixed again from the game's files in step with
//! its pictures.

use std::sync::Arc;

use crate::decode::SoundData;
use crate::engine::Bus;

/// What a sound is, so that a soundtrack mixed from the log can leave some out. It changes
/// nothing about how the sound plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoundKind {
    /// At a place in the world: weapons, explosions, units at work, thunder.
    #[default]
    World,
    /// Over the whole view: the wind.
    Ambience,
    /// Units answering (selected, ordered, finished) on the voice bus.
    Voice,
    /// An alert (a unit under attack).
    Alert,
    /// The interface's clicks.
    Interface,
    /// A briefing's narration.
    Narration,
    /// A movie's soundtrack.
    Movie,
}

impl SoundKind {
    /// The kind's name in a log (`"world"`, `"ambience"`...).
    pub fn name(self) -> &'static str {
        match self {
            SoundKind::World => "world",
            SoundKind::Ambience => "ambience",
            SoundKind::Voice => "voice",
            SoundKind::Alert => "alert",
            SoundKind::Interface => "interface",
            SoundKind::Narration => "narration",
            SoundKind::Movie => "movie",
        }
    }
}

/// A sound starting on a mixer voice.
#[derive(Clone, Debug)]
pub struct PlayStart {
    /// Names this playing in the changes that follow (a loop parked out of earshot and resumed
    /// starts again under a new id).
    pub id: u64,
    /// The sound's path in the game's files (`sounds/xplolrg1.wav`).
    pub path: String,
    /// The sound as decoded (sounds the game makes itself are in no file: the wind).
    pub sound: Arc<SoundData>,
    pub kind: SoundKind,
    pub bus: Bus,
    /// Its level before the pan law: the volume asked for times the distance's attenuation.
    pub gain: f32,
    /// Where it is between the speakers, -1 (left) to 1 (right).
    pub pan: f32,
    /// What the mixer multiplies its left and right channels by (`gain` through the pan law).
    pub gains: [f32; 2],
    /// Playback rate (1 = as recorded).
    pub pitch: f32,
    pub looping: bool,
    /// Seconds into the sound it starts at (a loop resumed in phase).
    pub offset: f64,
    /// Seconds it fades in over (a loop resumed).
    pub fade_in: f32,
    /// Where it is in the world, when it is placed.
    pub position: Option<[f32; 3]>,
}

/// One change to what the mixer plays.
#[derive(Clone, Debug)]
pub enum PlayEvent {
    Start(PlayStart),
    /// New gains, reached by a ramp over the mixer's next block (about 10 ms).
    Gains { id: u64, gain: f32, pan: f32, gains: [f32; 2] },
    /// A new playback rate.
    Pitch { id: u64, pitch: f32 },
    /// Stopped (or its voice taken by a more important sound, or a loop parked out of
    /// earshot), fading out over `fade` seconds.
    Stop { id: u64, fade: f32 },
}
