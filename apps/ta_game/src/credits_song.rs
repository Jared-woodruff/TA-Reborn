//! The credits' song: "The War Reborn" by Hadal Static (© Hadal Static, used with permission in
//! TA Reborn; `assets/reborn/music/NOTICE.txt`), the music of TA Reborn's launch trailer, which
//! the credits' film (`credits_show.rs`) plays and is timed to. Its cue sheet,
//! `credits/song.toml` (written by `tools/credits/song.py` from the band's master), says where
//! every bar begins, where the song's moments are (the band coming in, the break, the drop after
//! it, the loudest bar, the flash, the dead stop) and how it is made longer for a film that needs
//! it: arrangements of it, shortest first, each the song's spans played in turn, joined on bar
//! lines where the music is as it would have been ([`ta_audio::Arrangement`]).
//!
//! [`Score`] is the song as one film plays it: an arrangement, its bars and moments on the
//! score's own clock (seconds from its first sample, which is the film's first picture).

use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;

/// The cue sheet as written.
#[derive(Clone, Debug, Deserialize)]
pub struct Sheet {
    /// The song in the game's files.
    pub path: String,
    /// Its length (seconds).
    pub length: f64,
    /// The level it plays at against its master (dB; the music volume on top).
    pub gain_db: f32,
    /// The crossfade before each join of an arrangement (seconds).
    pub crossfade: f64,
    /// Every bar's first beat (seconds).
    pub bars: Vec<f64>,
    pub moments: Moments,
    #[serde(rename = "arrangement")]
    pub arrangements: Vec<Arranged>,
}

/// The song's moments (seconds of the song).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Moments {
    /// The band comes in.
    pub band: f64,
    /// The break: near silence and a riser, before the drop.
    #[serde(rename = "break")]
    pub pause: f64,
    /// The drop after the break: the strongest hit.
    pub drop: f64,
    /// The loudest bar's first beat, and the next bar's.
    pub loudest: f64,
    pub after: f64,
    /// The flash: the finale comes back in.
    pub flash: f64,
    /// The song stops dead.
    pub stop: f64,
}

/// An arrangement as written: its bars, the jumps back it takes (bar indices) and its spans.
#[derive(Clone, Debug, Deserialize)]
pub struct Arranged {
    pub bars: usize,
    #[serde(default)]
    pub jumps: Vec<[usize; 2]>,
    pub spans: Vec<[f64; 2]>,
}

/// The cue sheet built into the game.
const SHEET: &str = include_str!("../credits/song.toml");

impl Sheet {
    pub fn parse(text: &str) -> Result<Sheet> {
        let sheet: Sheet = toml::from_str(text).context("the credits' song's cue sheet")?;
        anyhow::ensure!(sheet.bars.windows(2).all(|w| w[0] < w[1]), "its bars are not in order");
        anyhow::ensure!(!sheet.arrangements.is_empty(), "it has no arrangement");
        for a in &sheet.arrangements {
            anyhow::ensure!(!a.spans.is_empty() && a.spans.iter().all(|s| s[0] < s[1] && s[1] <= sheet.length + 1e-6), "an arrangement's spans are out of the song");
        }
        Ok(sheet)
    }

    /// The cue sheet: `TA_CREDITS_DIR`'s `song.toml` when there is one (no rebuild), else the
    /// game's own.
    pub fn load() -> Result<Sheet> {
        if let Some(dir) = std::env::var_os("TA_CREDITS_DIR")
            && let Ok(text) = std::fs::read_to_string(std::path::Path::new(&dir).join("song.toml"))
        {
            return Sheet::parse(&text);
        }
        Sheet::parse(SHEET)
    }

    /// The bars from one bar line to the next, on average (seconds).
    pub fn bar(&self) -> f64 {
        match (self.bars.first(), self.bars.last()) {
            (Some(a), Some(b)) if self.bars.len() > 1 => (b - a) / (self.bars.len() - 1) as f64,
            _ => 1.75,
        }
    }
}

/// How far before a span's start a bar line may be and still be its first (the joins are cut a
/// few milliseconds before the incoming bar's kick, which the tracked bar line may lead by more).
const JOIN_SLACK: f64 = 0.06;

/// The song as a film plays it: one of its arrangements, with its bars and moments on the
/// score's own clock.
#[derive(Clone, Debug)]
pub struct Score {
    pub path: String,
    pub arrangement: Arc<ta_audio::Arrangement>,
    /// Every bar's first beat, in order.
    pub bars: Vec<f64>,
    /// The band coming in (its first time); every drop after a break; the loudest bar, the bar
    /// after, the flash and the dead stop (their last time: the song's end).
    pub band: f64,
    pub drops: Vec<f64>,
    pub loudest: f64,
    pub after: f64,
    pub flash: f64,
    pub stop: f64,
    /// The score's length (seconds) and its average bar.
    pub length: f64,
    pub bar: f64,
    /// The bars it has in all, and the jumps back it takes (from bar, to bar: the song's).
    pub count: usize,
    pub jumps: Vec<[usize; 2]>,
}

impl Score {
    /// The score of the sheet's arrangement `which`.
    pub fn new(sheet: &Sheet, which: usize) -> Score {
        let arranged = &sheet.arrangements[which.min(sheet.arrangements.len() - 1)];
        let arrangement = ta_audio::Arrangement {
            spans: arranged.spans.iter().map(|s| (s[0], s[1])).collect(),
            crossfade: sheet.crossfade,
            gain: 10f32.powf(sheet.gain_db / 20.0),
        };
        // Each span's bar lines (those just before its start too: its first), on its clock. (The
        // sheet's last bar line is the song's end, after its stop.)
        let lines = &sheet.bars[..sheet.bars.len().saturating_sub(1)];
        let mut bars = Vec::new();
        let mut at = 0.0;
        for &(s, e) in &arrangement.spans {
            for &b in lines {
                if b >= s - JOIN_SLACK && b < e - JOIN_SLACK {
                    bars.push(at + (b - s).max(0.0));
                }
            }
            at += e - s;
        }
        let m = &sheet.moments;
        let all = |t: f64| arrangement.arranged_at(t);
        let last = |t: f64| all(t).last().copied().unwrap_or(t);
        Score {
            path: sheet.path.clone(),
            band: all(m.band).first().copied().unwrap_or(m.band),
            // (Each with its break: a span holding the drop but not the break before it holds no
            // drop of its own.)
            drops: all(m.drop).into_iter().filter(|&d| arrangement.source_at(d - (m.drop - m.pause)).is_some_and(|(_, s)| (s - m.pause).abs() < 1e-6)).collect(),
            loudest: last(m.loudest),
            after: last(m.after),
            flash: last(m.flash),
            stop: last(m.stop),
            length: arrangement.length(),
            bar: sheet.bar(),
            count: arranged.bars,
            jumps: arranged.jumps.clone(),
            bars,
            arrangement: Arc::new(arrangement),
        }
    }

    /// The bar whose line is nearest second `t`.
    pub fn bar_near(&self, t: f64) -> usize {
        let i = self.bars.partition_point(|&b| b < t);
        match (i.checked_sub(1), self.bars.get(i)) {
            (Some(p), Some(&n)) if t - self.bars[p] < n - t => p,
            (Some(p), None) => p,
            _ => i,
        }
    }

    /// Bar `i`'s line (past the last, a bar on from it for each).
    pub fn bar_time(&self, i: usize) -> f64 {
        match self.bars.get(i) {
            Some(&t) => t,
            None => self.bars.last().copied().unwrap_or(0.0) + (i + 1 - self.bars.len()) as f64 * self.bar,
        }
    }
}

/// The arrangements of the sheet, as scores, shortest first.
pub fn scores(sheet: &Sheet) -> Vec<Score> {
    (0..sheet.arrangements.len()).map(|i| Score::new(sheet, i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cue_sheet_reads_and_its_scores_hold_the_songs_moments() {
        let sheet = Sheet::parse(SHEET).expect("the built-in cue sheet");
        assert_eq!(sheet.path, "music/the_war_reborn.ogg");
        assert!((sheet.length - 141.96).abs() < 0.01);
        assert!((1.6..1.9).contains(&sheet.bar()), "{}", sheet.bar());
        let all = scores(&sheet);
        assert!(all.windows(2).all(|w| w[0].length < w[1].length), "shortest first");
        // The first is the song itself; the longest about one and a half more.
        assert!((all[0].length - sheet.length).abs() < 1e-6);
        assert!(all.last().unwrap().length > 2.4 * sheet.length, "{}", all.last().unwrap().length);
        for s in &all {
            // Bar lines in order, about a bar apart all through (none lost or doubled at a join).
            let gaps: Vec<f64> = s.bars.windows(2).map(|w| w[1] - w[0]).collect();
            assert!(gaps.iter().all(|&g| (1.55..1.95).contains(&g)), "{:.0} s: bar gaps {:?}", s.length, gaps.iter().filter(|g| !(1.55..1.95).contains(*g)).collect::<Vec<_>>());
            // The band once, at the start; a drop on every pass through the break; the ending last.
            assert!((s.band - sheet.moments.band).abs() < 1e-9);
            assert!(!s.drops.is_empty());
            assert_eq!(s.drops.len(), s.arrangement.spans.iter().filter(|&&(a, b)| a <= sheet.moments.pause && sheet.moments.drop < b).count());
            assert!(s.drops.iter().all(|&d| d < s.loudest));
            assert!(s.loudest < s.after && s.after < s.flash && s.flash < s.stop && s.stop < s.length);
            assert!((s.length - s.stop - (sheet.length - sheet.moments.stop)).abs() < 1e-6, "every arrangement ends as the song does");
            // A bar line at the drop (it hits a little before the beat tracked) and the loudest bar.
            for &d in &s.drops {
                assert!((s.bar_time(s.bar_near(d)) - d).abs() < 0.15, "a bar at the drop {d}");
            }
            assert!((s.bar_time(s.bar_near(s.loudest)) - s.loudest).abs() < 1e-6);
        }
    }

    /// The jumps of every arrangement are where its spans join: each span ends at the bar line it
    /// jumps from and the next begins at the one it lands on (within the alignment's few
    /// milliseconds).
    #[test]
    fn every_arrangement_joins_on_its_jumps_bar_lines() {
        let sheet = Sheet::parse(SHEET).unwrap();
        for a in &sheet.arrangements {
            assert_eq!(a.spans.len(), a.jumps.len() + 1);
            for (k, [from, to]) in a.jumps.iter().enumerate() {
                assert!((a.spans[k][1] - sheet.bars[*from]).abs() < 0.06, "{:?}: span {k} ends at {} for bar {from}", a.jumps, a.spans[k][1]);
                assert!((a.spans[k + 1][0] - sheet.bars[*to]).abs() < 0.06, "{:?}: span {} starts at {} for bar {to}", a.jumps, k + 1, a.spans[k + 1][0]);
            }
            let lines = &sheet.bars[..sheet.bars.len() - 1];
            let bars: usize = a.spans.iter().map(|s| lines.iter().filter(|&&b| b >= s[0] - JOIN_SLACK && b < s[1] - JOIN_SLACK).count()).sum();
            assert_eq!(bars, a.bars, "{:?}", a.jumps);
        }
    }

    /// The song the game ships decodes as the cue sheet has it: as long, its hits where it says.
    #[test]
    fn the_shipped_song_is_the_cue_sheets() {
        let sheet = Sheet::parse(SHEET).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn").join(&sheet.path);
        let bytes = std::fs::read(&path).expect("the song beside the game's own maps");
        let song = ta_audio::decode::decode_music(bytes, Some("ogg")).expect("it decodes");
        let seconds = song.frames() as f64 / song.sample_rate() as f64;
        assert!((seconds - sheet.length).abs() < 0.01, "{seconds} s against {}", sheet.length);
        // The band comes in: the level a second before against a second after.
        let level = |from: f64, to: f64| {
            let (a, b) = ((from * song.sample_rate() as f64) as usize * 2, (to * song.sample_rate() as f64) as usize * 2);
            let s = &song.samples()[a..b];
            (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        let m = &sheet.moments;
        // The band comes in out of the swell: the kick and the bass (under 120 Hz) with it.
        let rate = song.sample_rate() as f64;
        // (Four one-pole low-passes in turn: 24 dB an octave.)
        let alpha = (std::f64::consts::TAU * 120.0 / rate) as f32;
        let mut y = [0.0f32; 4];
        let low: Vec<f32> = song.samples()[..((m.band + 2.0) * rate) as usize * 2].chunks(2).map(|f| {
            let mut x = 0.5 * (f[0] + f[1]);
            for v in &mut y {
                *v += alpha * (x - *v);
                x = *v;
            }
            x
        }).collect();
        let bass = |from: f64, to: f64| {
            let s = &low[(from * rate) as usize..(to * rate) as usize];
            (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        assert!(bass(m.band, m.band + 2.0) > 5.0 * bass(m.band - 2.0, m.band), "the band comes in");
        assert!(level(m.stop + 0.15, m.stop + 0.6) < 0.05 * level(m.stop - 0.6, m.stop - 0.15), "the song stops dead");
    }

    /// The shipped song played as a score through the audio engine's own mixer, across a join of
    /// the film's arrangement: the frames are the track's own, as the arrangement has them (the
    /// crossfade before the join, the jump back after it), at its gain.
    #[test]
    fn the_engine_plays_the_song_across_a_join_as_arranged() {
        let sheet = Sheet::parse(SHEET).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn").join(&sheet.path);
        let bytes = std::fs::read(&path).expect("the song");
        let song = ta_audio::decode::decode_music(bytes.clone(), Some("ogg")).expect("it decodes");
        let rate = song.sample_rate();
        assert_eq!(rate, 48_000);
        // An arrangement with a jump back: the first join, a few seconds after 90 s.
        let score = scores(&sheet).into_iter().find(|s| s.arrangement.spans.len() > 1).expect("a jump");
        let arrangement = score.arrangement.clone();
        let mut source = ta_audio::MemorySource::new();
        source.insert(&sheet.path, bytes);
        let bank = std::sync::Arc::new(ta_audio::SoundBank::new(std::sync::Arc::new(source)));
        let settings = ta_audio::AudioSettings { master: 1.0, effects: 1.0, voice: 1.0, music: 1.0, ..Default::default() };
        let mut engine = ta_audio::AudioEngine::new_offline(settings, bank, rate);
        let join = arrangement.spans[0].1 - arrangement.spans[0].0;
        let from = join - 2.0;
        engine.music_mut().hush(0.0);
        assert!(engine.music_mut().play_score(&sheet.path, arrangement.clone(), from, 0.0));
        engine.update(0.0);
        let mut out = vec![0.0f32; rate as usize * 4 * 2];
        for chunk in out.chunks_mut(1600) {
            engine.render_offline(chunk);
            engine.update(chunk.len() as f32 / 2.0 / rate as f32);
        }
        // What the arrangement has there, from the track decoded whole.
        let at = |s: f64| (s * rate as f64).round() as usize;
        let (e0, s1, x) = (at(arrangement.spans[0].1), at(arrangement.spans[1].0), at(arrangement.crossfade));
        let pcm = song.samples();
        let expect = |a: usize| -> f32 {
            let l = |i: usize| pcm[i * 2];
            let v = if a < e0 - x {
                l(a)
            } else if a < e0 {
                let u = (a - (e0 - x)) as f32 / x as f32;
                l(a) * (1.0 - u) + l(s1 - (e0 - a)) * u
            } else {
                l(s1 + (a - e0))
            };
            v * arrangement.gain
        };
        let first = at(from);
        let mut worst = 0.0f32;
        for i in 0..out.len() / 2 {
            worst = worst.max((out[i * 2] - expect(first + i)).abs());
        }
        assert!(worst < 2e-4, "{worst} off the arrangement");
        assert!(out.iter().any(|v| v.abs() > 0.1), "and it plays");
    }
}
