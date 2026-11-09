//! Social media clips from a replay: 30 or 60 seconds of the battle, vertical (9:16, for phone
//! feeds) or horizontal (16:9), the stretch chosen by the game.
//!
//! The replay's scan (`replay_scan.rs`) keeps a log of what went on second by second and where
//! ([`ActionLog`]: shots, blasts and losses, and the places they gathered in, [`Spot`]). From it
//! [`candidates`] scores every stretch of the clip's length for how much goes on and how it
//! builds to a climax (the busiest moment late in it, clear of its end tag; a nuclear strike or
//! a Commander's fall above all), and starts the best few where no blast is going off in view
//! (never mid-explosion). The replay's EXPORT CLIP dialog (`clip_dialog.rs`) offers them, each
//! with a picture taken at its climax and what happens in it, the best chosen.
//!
//! [`plan`] directs the camera for the shape: it follows the fight that makes the climax from
//! before it starts to after, cutting to another only when its own goes quiet, and looks ahead
//! (the path is smoothed both ways in time, so the camera is on its way before the fighting
//! moves). The vertical frame is closer, framed by its width, the action held in the middle of
//! the part of the picture social apps leave clear.
//!
//! The clip is made as a film (`film.rs`), with titles of its own ([`ClipLook`]): the map and
//! who fought over its first seconds, the scoreboard's tug of war, the big moments named as they
//! come, and an end tag with the game's emblem; all laid out inside the apps' safe areas
//! ([`Shape::safe`]). Its sound is lifted a little and kept under -1 dBFS ([`Limiter`]), and it
//! goes to `Videos\TA Reborn\Clips` ([`clip_path`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use glam::Vec2;
use ta_sim::{Sim, SimEvent};
use ta_ui::i18n::{n, t, tf};

use crate::highlights::{Moment, MomentKind};
use crate::theme::*;

/// Ticks a second.
const SECOND: u64 = 30;
/// A clip's pictures a second, and its bits a pixel a picture (about 7.5 Mbit/s at 1080 × 1920:
/// what the film's High quality takes; a minute is some 57 MB, within what social sites accept).
pub const FPS: u32 = 30;
const BITS_PER_PIXEL: f64 = 0.12;
/// Seconds: the opening title over the clip's start, the end tag over its end.
const TITLE: f32 = 3.2;
pub const END_TAG: f32 = 3.2;
/// A big moment named in the clip stays up so long (seconds); at most so many are named.
const CALLOUT: f32 = 2.8;
const CALLOUTS: usize = 2;
/// The most stretches offered.
pub const CANDIDATES: usize = 4;

// ---- The shape -------------------------------------------------------------------------------

/// A clip's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Shape {
    /// 9:16, 1080 × 1920: phone feeds (TikTok, Reels, Shorts).
    Vertical,
    /// 16:9, 1920 × 1080.
    Horizontal,
}

/// Where a picture's edges are covered by a social app's own controls (shares of its width and
/// height): titles and the action keep inside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Safe {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Safe {
    /// The safe part of a picture `screen`.
    pub fn rect(self, screen: Rect) -> Rect {
        Rect::from_min_max(
            pos2(screen.left() + screen.width() * self.left, screen.top() + screen.height() * self.top),
            pos2(screen.right() - screen.width() * self.right, screen.bottom() - screen.height() * self.bottom),
        )
    }

    /// The part for centred titles: as wide on either side of the middle as the narrower side
    /// allows.
    pub fn centred(self, screen: Rect) -> Rect {
        let side = self.left.max(self.right) * screen.width();
        let r = self.rect(screen);
        Rect::from_x_y_ranges((screen.left() + side)..=(screen.right() - side), r.y_range())
    }
}

impl Shape {
    /// The clip's pictures (pixels).
    pub fn size(self) -> (u32, u32) {
        match self {
            Shape::Vertical => (1080, 1920),
            Shape::Horizontal => (1920, 1080),
        }
    }

    /// The parts of the picture the apps leave clear. Vertical: the feeds' tabs across the top
    /// (13%), the caption, the music and the account's name over the bottom (25%), the like,
    /// comment and share buttons down the right (14%) — the strictest of TikTok's, Reels' and
    /// Shorts' together. Horizontal: a player's title across the top, its controls and progress
    /// bar along the bottom, a margin each side.
    pub fn safe(self) -> Safe {
        match self {
            Shape::Vertical => Safe { left: 0.065, right: 0.14, top: 0.13, bottom: 0.25 },
            Shape::Horizontal => Safe { left: 0.05, right: 0.05, top: 0.065, bottom: 0.13 },
        }
    }

    /// How far the camera looks down (radians above the horizon): lower than the battle's view,
    /// so the ground has depth and the units their sides.
    pub fn pitch(self) -> f32 {
        match self {
            Shape::Vertical => 0.78,
            Shape::Horizontal => 0.74,
        }
    }

    /// World units across the picture's narrow side (the vertical's width, the horizontal's
    /// height) for action spreading `spread`: the vertical closer. (Never far out: on a phone a
    /// unit seen from far is a speck; part of a big fight close shows better than all of it.)
    pub fn span(self, spread: f32) -> f32 {
        match self {
            Shape::Vertical => (spread * 1.25).clamp(380.0, 800.0),
            Shape::Horizontal => (spread * 1.8).clamp(480.0, 1000.0),
        }
    }

    /// Whether `at` (world x and z) is in the picture of a camera looking at `focus` where the
    /// action spreads `spread` (near enough: within some way of the middle).
    pub fn in_view(self, focus: Vec2, spread: f32, at: Vec2) -> bool {
        at.distance(focus) < self.span(spread) * 0.65
    }

    /// The camera's zoom (pixels a world unit at its focus, in the clip's picture) for action
    /// spreading `spread`.
    pub fn zoom(self, spread: f32) -> f32 {
        let (w, h) = self.size();
        w.min(h) as f32 / self.span(spread)
    }

    /// The word in a clip's file name.
    pub fn word(self) -> &'static str {
        match self {
            Shape::Vertical => n("vertical"),
            Shape::Horizontal => n("horizontal"),
        }
    }
}

/// A clip as asked for: its length (seconds) and shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipSpec {
    pub seconds: u32,
    pub shape: Shape,
}

impl Default for ClipSpec {
    fn default() -> ClipSpec {
        ClipSpec { seconds: 30, shape: Shape::Vertical }
    }
}

impl ClipSpec {
    /// `30v`, `60h` (the capture switch `TA_CLIP`): the length in seconds and v or h.
    pub fn parse(text: &str) -> Option<ClipSpec> {
        let text = text.trim().to_ascii_lowercase();
        let (seconds, shape) = text.split_at(text.len().checked_sub(1)?);
        let shape = match shape {
            "v" => Shape::Vertical,
            "h" => Shape::Horizontal,
            _ => return None,
        };
        let seconds: u32 = seconds.parse().ok()?;
        (5..=600).contains(&seconds).then_some(ClipSpec { seconds, shape })
    }

    /// The pictures as the writer takes them.
    pub fn pictures(self) -> ta_mp4::Pictures {
        let (width, height) = self.shape.size();
        ta_mp4::Pictures { width, height, fps: FPS, bitrate: bitrate(width, height) }
    }

    /// About how large the clip will be (bytes), its sound at `sound_bits` a second.
    pub fn bytes(self, sound_bits: Option<u32>) -> f64 {
        let (w, h) = self.shape.size();
        (bitrate(w, h) as f64 + sound_bits.unwrap_or(0) as f64) / 8.0 * self.seconds as f64
    }
}

fn bitrate(width: u32, height: u32) -> u32 {
    (width as f64 * height as f64 * FPS as f64 * BITS_PER_PIXEL).round() as u32
}

// ---- What went on, second by second ------------------------------------------------------------

/// What goes on counts so much where it goes on: a shot (0.4 where it was fired from, 0.6 where
/// it was aimed), a blast (times its size), a loss (a metal of its worth); a nuclear strike
/// and a Commander's fall more again.
const SHOT: f32 = 1.0;
const BLAST: f32 = 2.5;
const LOSS: f32 = 1.0 / 40.0;
const NUKE: f32 = 150.0;
const COMMANDER: f32 = 125.0;
/// A blast's size: its weapon's area of effect over this (pixels), within these bounds; those
/// this big at least are kept, tick and place (no clip starts while one is going off in view).
const BLAST_AREA: f32 = 48.0;
const BLAST_SIZES: (f32, f32) = (0.4, 12.0);
const BIG_BLAST: f32 = 0.75;
/// The places a second's action gathered in: cells this wide (world units) gathered with those
/// within `REACH` cells of the busiest (two fights further apart stay two places, rather than one
/// with its middle on the empty ground between); at most `SPOTS` a second, each of `MIN_SPOT` at
/// least.
const CELL: f32 = 256.0;
const REACH: i32 = 1;
const SPOTS: usize = 6;
const MIN_SPOT: f64 = 0.5;

/// A place where the action gathered in a second: its middle (world x and z), how much went on
/// there, and how far it spread (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: Vec2,
    pub weight: f32,
    pub spread: f32,
}

/// A second of the battle: how much went on (all of it weighed), the shots, the blasts' sizes
/// summed, the units lost and their worth, whether a nuclear strike landed or a Commander fell,
/// and where it went on (the busiest place first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beat {
    pub weight: f32,
    pub shots: u32,
    pub blasts: f32,
    pub lost: u32,
    pub worth: f32,
    pub nuke: bool,
    pub commander: bool,
    pub spots: Vec<Spot>,
}

/// A blast big enough to see: when, where, how big.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub tick: u64,
    pub at: Vec2,
    pub size: f32,
}

/// The battle second by second (`beats[s]`: ticks `30 s` to `30 s + 29`), and its big blasts in
/// order.
#[derive(Clone, Debug, Default)]
pub struct ActionLog {
    pub beats: Vec<Beat>,
    pub blasts: Vec<Blast>,
}

/// A cell's sums: weight, and weighted x, z and x² + z² (in f64: the spread is their difference).
#[derive(Clone, Copy, Default)]
struct Cell {
    w: f64,
    x: f64,
    z: f64,
    rr: f64,
}

impl Cell {
    fn add(&mut self, at: Vec2, w: f32) {
        let (x, z, w) = (at.x as f64, at.y as f64, w as f64);
        self.w += w;
        self.x += w * x;
        self.z += w * z;
        self.rr += w * (x * x + z * z);
    }

    fn merge(&mut self, o: &Cell) {
        self.w += o.w;
        self.x += o.x;
        self.z += o.z;
        self.rr += o.rr;
    }

    fn spot(&self) -> Spot {
        let w = self.w.max(1e-9);
        let (x, z) = (self.x / w, self.z / w);
        let variance = (self.rr / w - x * x - z * z).max(0.0);
        Spot { at: Vec2::new(x as f32, z as f32), weight: self.w as f32, spread: (2.0 * variance.sqrt() as f32).clamp(160.0, 1400.0) }
    }
}

/// Keeps the [`ActionLog`] as a battle is played through (the replay's scan).
#[derive(Default)]
pub struct ActionRecorder {
    log: ActionLog,
    beat: Beat,
    cells: BTreeMap<(i32, i32), Cell>,
}

impl ActionRecorder {
    /// Notes a simulation event (of the tick `sim` is at).
    pub fn note(&mut self, e: &SimEvent, sim: &Sim) {
        let tick = sim.tick;
        self.advance(tick);
        let at = |p: &ta_sim::FxVec3| {
            let p = p.to_f32();
            Vec2::new(p[0], p[2])
        };
        match e {
            SimEvent::WeaponFired { pos, target, .. } => {
                self.beat.shots += 1;
                self.add(at(pos), SHOT * 0.4);
                self.add(at(target), SHOT * 0.6);
            }
            SimEvent::Explosion { weapon, pos, .. } => {
                let w = sim.weapon_types.get(*weapon);
                let nuke = w.def.stockpile && !w.def.interceptor;
                let size = (w.def.area_of_effect as f32 / BLAST_AREA).clamp(BLAST_SIZES.0, BLAST_SIZES.1);
                self.blast(tick, at(pos), size, if nuke { NUKE } else { 0.0 });
                self.beat.nuke |= nuke;
            }
            SimEvent::UnitExploded { pos, big, .. } => self.blast(tick, at(pos), if *big { 2.5 } else { 1.0 }, 0.0),
            SimEvent::UnitDestroyed { type_id, pos, removed: false, .. } => {
                let ty = sim.types.get(*type_id);
                if ty.def.is_feature {
                    return;
                }
                let worth = ty.cost_metal.to_f32() + ty.cost_energy.to_f32() / 60.0;
                self.beat.lost += 1;
                self.beat.worth += worth;
                self.beat.commander |= ty.def.is_commander;
                self.add(at(pos), worth * LOSS + if ty.def.is_commander { COMMANDER } else { 0.0 });
            }
            _ => {}
        }
    }

    fn blast(&mut self, tick: u64, at: Vec2, size: f32, extra: f32) {
        self.beat.blasts += size;
        self.add(at, BLAST * size + extra);
        if size >= BIG_BLAST || extra > 0.0 {
            self.log.blasts.push(Blast { tick, at, size });
        }
    }

    fn add(&mut self, at: Vec2, w: f32) {
        if !at.is_finite() {
            return;
        }
        self.beat.weight += w;
        self.cells.entry(((at.x / CELL).floor() as i32, (at.y / CELL).floor() as i32)).or_default().add(at, w);
    }

    /// The seconds before `tick`'s summed up.
    fn advance(&mut self, tick: u64) {
        let beat = (tick / SECOND) as usize;
        while self.log.beats.len() < beat {
            self.close();
        }
    }

    fn close(&mut self) {
        let mut beat = std::mem::take(&mut self.beat);
        beat.spots = spots(std::mem::take(&mut self.cells));
        self.log.beats.push(beat);
    }

    /// The log of a battle `end` ticks long.
    pub fn finish(mut self, end: u64) -> ActionLog {
        let last = (end / SECOND) as usize + 1;
        while self.log.beats.len() < last {
            self.close();
        }
        self.log
    }
}

/// The places a second's action gathered in: the busiest cell with those round it, then the
/// busiest of the rest...
fn spots(cells: BTreeMap<(i32, i32), Cell>) -> Vec<Spot> {
    let cells: Vec<((i32, i32), Cell)> = cells.into_iter().collect();
    let mut taken = vec![false; cells.len()];
    let mut out = Vec::new();
    while out.len() < SPOTS {
        let Some(i) = (0..cells.len()).filter(|&i| !taken[i]).max_by(|&a, &b| cells[a].1.w.total_cmp(&cells[b].1.w)) else { break };
        if cells[i].1.w < MIN_SPOT {
            break;
        }
        let (cx, cz) = cells[i].0;
        let mut sum = Cell::default();
        for (j, ((x, z), cell)) in cells.iter().enumerate() {
            if !taken[j] && (x - cx).abs() <= REACH && (z - cz).abs() <= REACH {
                taken[j] = true;
                sum.merge(cell);
            }
        }
        out.push(sum.spot());
    }
    out
}

// ---- The stretches offered ---------------------------------------------------------------------

/// Worth lost (metal) that makes a stretch heavy fighting (as the highlights have it).
const HEAVY: f32 = 1500.0;

/// A stretch offered for a clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Its ticks (from, to), and its climax's.
    pub from: u64,
    pub to: u64,
    pub climax: u64,
    /// Where the climax is (world x and z), and how far its action spreads.
    pub focus: Vec2,
    pub spread: f32,
    pub score: f32,
    /// Units lost in it and their worth; shots fired.
    pub lost: u32,
    pub worth: f32,
    pub shots: u32,
    /// What happens in it: a title (`n`), its most notable kind.
    pub title: &'static str,
    pub kind: MomentKind,
    /// How much goes on, second by second (0 to 1 of its busiest).
    pub curve: Vec<f32>,
}

impl Candidate {
    /// "14 units lost · 3,200 metal" (or the shots fired, where nothing was lost).
    pub fn detail(&self) -> String {
        let metal = ta_ui::i18n::grouped((self.worth.min(1.0e9) as u64 / 10) * 10);
        match self.lost {
            0 => tf("{shots} shots fired", &[("shots", &ta_ui::i18n::grouped(self.shots as u64))]),
            1 => tf("1 unit lost · {metal} metal", &[("metal", &metal)]),
            units => tf("{units} units lost · {metal} metal", &[("units", &units), ("metal", &metal)]),
        }
    }

    /// "12:20 – 12:50".
    pub fn span_text(&self) -> String {
        format!("{} – {}", clock(self.from), clock(self.to))
    }
}

/// "12:40" of battle time (ticks); "1:02:05" past the hour.
pub fn clock(ticks: u64) -> String {
    let s = ticks / SECOND;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// How busy each second is, against how busy the battle gets (1: busy; above it, growing slowly),
/// counting for more where it is all in one place.
fn intensity(beats: &[Beat]) -> Vec<f32> {
    let mut busy: Vec<f32> = beats.iter().map(|b| b.weight).filter(|&w| w > 0.0).collect();
    busy.sort_by(f32::total_cmp);
    let busy = busy.get(busy.len() * 9 / 10).copied().unwrap_or(1.0).max(4.0);
    beats
        .iter()
        .map(|b| {
            if b.weight <= 0.0 {
                return 0.0;
            }
            let i = b.weight / busy;
            let i = if i <= 1.0 { i } else { 1.0 + i.ln() }.min(6.0);
            let focus = b.spots.first().map_or(0.0, |s| (s.weight / b.weight).min(1.0));
            i * (0.4 + 0.6 * focus)
        })
        .collect()
}

/// The seconds into a stretch `len` long where its climax may come: from three tenths of the way
/// in to some seconds before the end tag (an aftermath to see before it darkens the picture).
fn climax_room(len: usize) -> (usize, usize) {
    let lo = (len as f32 * 0.3) as usize;
    let hi = len.saturating_sub(END_TAG.ceil() as usize + 2).max(lo).min(len.saturating_sub(1));
    (lo.min(hi), hi)
}

/// A stretch's score: how much goes on (the later the more it counts), how high its climax
/// comes and whether it comes where a climax should (between a half and most of the way, clear
/// of the end tag), whether the action builds to it, a nuclear strike or a Commander's fall in
/// it, a start where nothing goes on. Also the climax's second (into the stretch).
fn score(fi: &[f32], smooth: &[f32], beats: &[Beat], s: usize, len: usize) -> (f32, usize) {
    let (mut body, mut weights) = (0.0, 0.0);
    for u in 0..len {
        let x = (u as f32 + 0.5) / len as f32;
        let w = if x < 0.8 { 0.5 + 0.75 * x / 0.8 } else { 1.25 - 0.35 * (x - 0.8) / 0.2 };
        body += fi[s + u] * w;
        weights += w;
    }
    let body = body / weights.max(1e-6);
    let (lo, hi) = climax_room(len);
    let (mut climax, mut peak) = (lo, f32::MIN);
    for u in lo..=hi {
        if smooth[s + u] > peak {
            (climax, peak) = (u, smooth[s + u]);
        }
    }
    let x = climax as f32 / len as f32;
    let placed = if x < 0.55 { 0.5 + 0.5 * (x - 0.3).max(0.0) / 0.25 } else if x <= 0.85 { 1.0 } else { (1.0 - 0.3 * (x - 0.85) / 0.15).max(0.4) };
    let mean = |a: usize, b: usize| if b > a { fi[s + a..s + b].iter().sum::<f32>() / (b - a) as f32 } else { 0.0 };
    let build = 0.25 * (mean(climax / 2, climax) - mean(0, climax / 2)).clamp(-1.0, 1.0);
    let window = &beats[s + lo..=s + hi];
    // (Nothing in a battle shows better: they make the climax of any stretch they are in.)
    let special = if window.iter().any(|b| b.nuke) { 2.0 } else { 0.0 } + if window.iter().any(|b| b.commander) { 1.5 } else { 0.0 };
    let dead_start = if mean(0, 3.min(len)) < 0.08 { 0.2 } else { 0.0 };
    // How much of the stretch's action goes on round its climax (one fight followed as it builds
    // shows better than a camera sent round the map).
    let coherence = beats[s + climax].spots.first().map_or(0.0, |anchor| {
        let (mut near, mut all) = (0.0, 0.0);
        for spot in beats[s..s + len].iter().flat_map(|b| &b.spots) {
            all += spot.weight;
            if spot.at.distance(anchor.at) < COHERENT {
                near += spot.weight;
            }
        }
        if all > 0.0 { near / all } else { 0.0 }
    });
    (body * (0.4 + 0.6 * coherence) + 0.9 * peak.max(0.0) * placed + build + special - dead_start, climax)
}

/// The action within this of a stretch's climax (world units) counts as the same fight's.
const COHERENT: f32 = 1000.0;

/// The best stretches of `spec`'s length in a replay `end` ticks long (the best first, at most
/// [`CANDIDATES`], none overlapping another much or sharing its climax), each started where no big
/// blast is going off in view. `marks`: the scan's big moments (the biggest battle's is named so).
pub fn candidates(log: &ActionLog, marks: &[Moment], end: u64, spec: ClipSpec) -> Vec<Candidate> {
    let total = log.beats.len().min((end / SECOND) as usize);
    if total < 5 {
        return Vec::new();
    }
    let beats = &log.beats[..total];
    let len = (spec.seconds as usize).min(total);
    let fi = intensity(beats);
    let smooth: Vec<f32> = (0..total).map(|t| (fi[t.saturating_sub(1)] + 2.0 * fi[t] + fi[(t + 1).min(total - 1)]) / 4.0).collect();
    let starts: Vec<(usize, f32, usize)> = (0..=total - len).map(|s| {
        let (score, climax) = score(&fi, &smooth, beats, s, len);
        (s, score, climax)
    }).collect();
    let mut order: Vec<&(usize, f32, usize)> = starts.iter().collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let best = order.first().map_or(0.0, |s| s.1);
    let mut chosen: Vec<(usize, usize)> = Vec::new();
    for &&(s, score, climax) in &order {
        if chosen.len() == CANDIDATES || score < 0.3 * best || score <= 0.0 {
            break;
        }
        let at = s + climax;
        let overlaps = chosen.iter().any(|&(c, cc)| {
            let shared = (s + len).min(c + len).saturating_sub(s.max(c));
            shared as f32 > 0.35 * len as f32 || at.abs_diff(cc) < 8
        });
        // (Something has to go on: a few shots at least.)
        let action: f32 = beats[s..s + len].iter().map(|b| b.weight).sum();
        if !overlaps && action >= 30.0 {
            chosen.push((s, at));
        }
    }
    chosen
        .into_iter()
        .map(|(s, at)| {
            let climax_spot = beats[at].spots.first().copied();
            let focus = climax_spot.map_or_else(|| log_focus(beats, s, len), |c| c.at);
            let spread = climax_spot.map_or(400.0, |c| c.spread);
            let plan = plan(log, (s * SECOND as usize) as u64, ((s + len) * SECOND as usize) as u64, (at * SECOND as usize) as u64 + SECOND / 2, focus);
            let ideal = s as u64 * SECOND;
            // (Moved no further than keeps the climax where a climax goes: three tenths of the way
            // in at least, a second and a half before the end tag at most.)
            let climax = at as u64 * SECOND + SECOND / 2;
            let earliest = climax.saturating_sub(((len as f32 - END_TAG - 1.5).max(1.0) * SECOND as f32) as u64);
            let latest = climax.saturating_sub((len as f32 * 0.3 * SECOND as f32) as u64).min(end.saturating_sub(len as u64 * SECOND));
            let from = clean_start(log, &plan, ideal, earliest, latest.max(earliest), spec.shape);
            let to = (from + len as u64 * SECOND).min(end);
            describe(beats, marks, &plan, spec.shape, Candidate {
                from,
                to,
                climax: at as u64 * SECOND + SECOND / 2,
                focus,
                spread,
                score: starts[s].1,
                lost: 0,
                worth: 0.0,
                shots: 0,
                title: n("SKIRMISH"),
                kind: MomentKind::Battle,
                curve: Vec::new(),
            })
        })
        .collect()
}

/// Where a stretch's action mostly was (its busiest place's middle, weighed over its seconds),
/// for one whose climax had no place.
fn log_focus(beats: &[Beat], s: usize, len: usize) -> Vec2 {
    let (sum, weight) = beats[s..s + len].iter().filter_map(|b| b.spots.first()).fold((Vec2::ZERO, 0.0), |(p, w), spot| (p + spot.at * spot.weight, w + spot.weight));
    if weight > 0.0 { sum / weight } else { Vec2::ZERO }
}

/// What happens in `c`'s stretch: its losses and shots (all over the battle), its title (from the
/// big moments its camera, `plan`, sees), its curve.
fn describe(beats: &[Beat], marks: &[Moment], plan: &CameraPlan, shape: Shape, mut c: Candidate) -> Candidate {
    let (a, b) = ((c.from / SECOND) as usize, ((c.to / SECOND) as usize).min(beats.len()));
    let seconds = &beats[a.min(b)..b];
    c.lost = seconds.iter().map(|s| s.lost).sum();
    c.worth = seconds.iter().map(|s| s.worth).sum();
    c.shots = seconds.iter().map(|s| s.shots).sum();
    let shown: Vec<&Moment> = marks.iter().filter(|m| (c.from..c.to).contains(&crate::film::peak(m)) && seen(m, plan, shape)).collect();
    let biggest = shown.iter().any(|m| m.biggest);
    (c.title, c.kind) = if shown.iter().any(|m| m.kind == MomentKind::Nuke) {
        (n("NUCLEAR STRIKE"), MomentKind::Nuke)
    } else if shown.iter().any(|m| m.kind == MomentKind::Commander) {
        (n("COMMANDER DOWN"), MomentKind::Commander)
    } else if biggest && c.worth >= HEAVY {
        (n("THE BIGGEST BATTLE"), MomentKind::Battle)
    } else if c.worth >= HEAVY {
        (n("HEAVY FIGHTING"), MomentKind::Battle)
    } else if c.lost > 0 {
        (n("SKIRMISH"), MomentKind::Battle)
    } else {
        (n("CROSSFIRE"), MomentKind::Battle)
    };
    let raw: Vec<f32> = seconds.iter().map(|s| s.weight.sqrt()).collect();
    let most = raw.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    c.curve = raw.iter().map(|w| w / most).collect();
    c
}

/// Where a clip ideally starting at `ideal` (ticks) starts: there, or as near as can be (up to
/// three seconds sooner, one later, within `earliest` to `latest`), where no big blast went off in
/// view in the second before: a clip never opens on an explosion already under way.
fn clean_start(log: &ActionLog, plan: &CameraPlan, ideal: u64, earliest: u64, latest: u64, shape: Shape) -> u64 {
    let ideal = ideal.clamp(earliest, latest);
    let (at, spread) = plan.at(ideal as f32);
    // (What the opening picture shows: about its half-diagonal round its middle.)
    let reach = shape.span(spread) * 1.15;
    let noise = |tick: u64| -> f32 {
        let first = log.blasts.partition_point(|b| b.tick + 36 < tick);
        log.blasts[first..].iter().take_while(|b| b.tick < tick).filter(|b| b.at.distance(at) < reach).map(|b| b.size).sum()
    };
    let mut tries: Vec<u64> = (0..=40).map(|k| ideal.saturating_sub(90) + k * 3).filter(|&t| (earliest..=latest).contains(&t)).collect();
    tries.sort_by_key(|&t| (t.abs_diff(ideal), t > ideal));
    let mut best = (f32::MAX, ideal.min(latest));
    for t in tries {
        let n = noise(t);
        if n <= 0.0 {
            return t;
        }
        if n < best.0 {
            best = (n, t);
        }
    }
    best.1
}

// ---- The camera ------------------------------------------------------------------------------

/// The fight followed stays the one within this of where the camera is (world units), however it
/// moves; another is cut to only when nothing goes on there, and only after a shot of this long
/// (seconds); the plan reaches this far round the clip (seconds), to smooth across its ends.
const STAY: f32 = 760.0;
const MIN_SHOT: usize = 4;
const MARGIN: usize = 3;
/// A cut this close to the clip's start or end (seconds) is left out: the shot before or after
/// it is held instead.
const NO_CUT_NEAR_ENDS: usize = 3;

/// A second's place in the plan: where the camera looks, how far the action there spreads, and
/// whether it cuts there (a jump, not a glide).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    at: Vec2,
    spread: f32,
    cut: bool,
}

/// Where the camera looks through a clip (see the module documentation): a key a second, at the
/// middle of each second, smoothed, between which it glides.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraPlan {
    /// The tick of the first key.
    first: u64,
    keys: Vec<Key>,
}

/// The camera's plan for a clip from `from` to `to` (ticks) whose climax is at `climax`, where
/// `focus` is (when the log has no place for it).
pub fn plan(log: &ActionLog, from: u64, to: u64, climax: u64, focus: Vec2) -> CameraPlan {
    let beats = &log.beats;
    let (from_s, to_s) = ((from / SECOND) as usize, ((to / SECOND) as usize).max((from / SECOND) as usize + 1));
    let first = from_s.saturating_sub(MARGIN);
    let last = (to_s + MARGIN).min(beats.len().max(first + 1));
    let count = last - first;
    let c = ((climax / SECOND) as usize).clamp(first, last - 1) - first;
    let spots = |i: usize| beats.get(first + i).map_or(&[][..], |b| &b.spots[..]);
    let anchor = spots(c).first().map_or(Key { at: focus, spread: 400.0, cut: false }, |s| Key { at: s.at, spread: s.spread, cut: false });
    let mut keys = vec![anchor; count];
    // One step from `here` into second `i`: the fight followed if it goes on near, else held, else
    // (after a long enough shot) a cut to the busiest elsewhere.
    let step = |here: Key, i: usize, since: usize| -> (Key, bool) {
        let all = spots(i);
        let Some(busiest) = all.first() else { return (Key { cut: false, ..here }, false) };
        let near = all
            .iter()
            .filter(|s| s.at.distance(here.at) < STAY && s.weight >= 0.2 * busiest.weight)
            .max_by(|a, b| (a.weight / (1.0 + a.at.distance(here.at) / 300.0)).total_cmp(&(b.weight / (1.0 + b.at.distance(here.at) / 300.0))));
        match near {
            Some(s) => (Key { at: s.at, spread: s.spread, cut: false }, false),
            None if since >= MIN_SHOT && busiest.weight >= 4.0 => (Key { at: busiest.at, spread: busiest.spread, cut: false }, true),
            None => (Key { cut: false, ..here }, false),
        }
    };
    // Forward from the climax...
    let (mut here, mut since) = (anchor, 0);
    for i in c + 1..count {
        let (key, cut) = step(here, i, since);
        keys[i] = Key { cut, ..key };
        (here, since) = (key, if cut { 0 } else { since + 1 });
    }
    // ...and back from it (a switch going back is a cut into the second after, going forward).
    let (mut here, mut since) = (anchor, 0);
    for i in (0..c).rev() {
        let (key, cut) = step(here, i, since + 1);
        keys[i] = key;
        if cut {
            keys[i + 1].cut = true;
        }
        (here, since) = (key, if cut { 0 } else { since + 1 });
    }
    // No cut just after the clip's start or just before its end: the shot either side is held.
    let (start, end) = (from_s - first, to_s - first);
    for i in 1..count {
        if !keys[i].cut {
            continue;
        }
        if i > start && i <= start + NO_CUT_NEAR_ENDS {
            let held = keys[i];
            keys[..i].iter_mut().for_each(|k| *k = Key { cut: false, ..held });
            keys[i].cut = false;
        } else if i + NO_CUT_NEAR_ENDS >= end && i < end + MARGIN {
            let held = keys[i - 1];
            keys[i..].iter_mut().for_each(|k| *k = Key { cut: false, ..held });
        }
    }
    smooth_keys(&mut keys);
    CameraPlan { first: first as u64 * SECOND + SECOND / 2, keys }
}

/// The keys smoothed shot by shot (a cut is kept a cut): the places over about a second and a
/// half either way, the spreads over two and a half.
fn smooth_keys(keys: &mut [Key]) {
    let mut start = 0;
    while start < keys.len() {
        let end = (start + 1..keys.len()).find(|&i| keys[i].cut).unwrap_or(keys.len());
        let shot: Vec<Key> = keys[start..end].to_vec();
        let gauss = |sigma: f32, i: usize, value: &dyn Fn(&Key) -> Vec2| -> Vec2 {
            let reach = (sigma * 3.0).ceil() as isize;
            let (mut sum, mut weight) = (Vec2::ZERO, 0.0);
            for d in -reach..=reach {
                let j = (i as isize + d).clamp(0, shot.len() as isize - 1) as usize;
                let w = (-(d * d) as f32 / (2.0 * sigma * sigma)).exp();
                sum += value(&shot[j]) * w;
                weight += w;
            }
            sum / weight
        };
        for i in 0..shot.len() {
            keys[start + i].at = gauss(1.5, i, &|k| k.at);
            keys[start + i].spread = gauss(2.5, i, &|k| Vec2::splat(k.spread)).x;
        }
        start = end;
    }
}

impl CameraPlan {
    /// Where the camera looks at `tick` (world x and z), and how far the action there spreads:
    /// gliding through the keys (Catmull-Rom), jumping at a cut.
    pub fn at(&self, tick: f32) -> (Vec2, f32) {
        let Some(last) = self.keys.len().checked_sub(1) else { return (Vec2::ZERO, 400.0) };
        let f = ((tick - self.first as f32) / SECOND as f32).clamp(0.0, last as f32);
        let i = (f.floor() as usize).min(last);
        let u = f - i as f32;
        if i == last || u <= 0.0 {
            return (self.keys[i].at, self.keys[i].spread);
        }
        let (k1, k2) = (self.keys[i], self.keys[i + 1]);
        if k2.cut {
            return (k1.at, k1.spread);
        }
        let k0 = if i > 0 && !k1.cut { self.keys[i - 1] } else { k1 };
        let k3 = if i + 2 <= last && !self.keys[i + 2].cut { self.keys[i + 2] } else { k2 };
        let (u2, u3) = (u * u, u * u * u);
        let at = 0.5 * ((2.0 * k1.at) + (k2.at - k0.at) * u + (2.0 * k0.at - 5.0 * k1.at + 4.0 * k2.at - k3.at) * u2 + (3.0 * k1.at - k0.at - 3.0 * k2.at + k3.at) * u3);
        (at, k1.spread + (k2.spread - k1.spread) * u)
    }

    /// The ticks of the plan's cuts.
    pub fn cuts(&self) -> Vec<u64> {
        self.keys.iter().enumerate().filter(|(_, k)| k.cut).map(|(i, _)| self.first + i as u64 * SECOND).collect()
    }
}

/// The camera's heading `seconds` into a clip `length` seconds long: a slow sweep across the
/// battle's north, some twenty degrees in all, for the parallax.
pub fn yaw(seconds: f32, length: f32) -> f32 {
    let sweep = 0.36 * (30.0 / length.max(1.0)).sqrt();
    -sweep / 2.0 + sweep * (seconds / length.max(1.0)).clamp(0.0, 1.0)
}

// ---- The clip's titles -------------------------------------------------------------------------

/// A big moment named in the clip: when (ticks), its title (`n`) and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Callout {
    pub tick: u64,
    pub title: &'static str,
    pub detail: String,
    pub kind: MomentKind,
}

/// What a clip's titles say: its shape, length and first tick, the map and who fought (name,
/// side, colour), the big moments it names; whether the titles and the scoreboard are shown.
#[derive(Clone, Debug)]
pub struct ClipLook {
    pub shape: Shape,
    pub seconds: f32,
    pub start: u64,
    pub map: String,
    pub players: Vec<(String, String, Color32)>,
    pub callouts: Vec<Callout>,
    pub titles: bool,
    pub scoreboard: bool,
}

/// Whether the big moment `m` happens where a clip's camera (`plan`, `shape`) looks at its height.
fn seen(m: &Moment, plan: &CameraPlan, shape: Shape) -> bool {
    let (at, spread) = plan.at(crate::film::peak(m) as f32);
    shape.in_view(at, spread, Vec2::new(m.focus.x, m.focus.z))
}

/// The big moments a clip from `from` to `to` names: the worthiest whose height comes in it (not
/// in its opening seconds or its end tag) where its camera (`plan`) is looking, skirmishes left
/// out, at most [`CALLOUTS`], apart.
pub fn callouts(marks: &[Moment], from: u64, to: u64, plan: &CameraPlan, shape: Shape) -> Vec<Callout> {
    let open = from + (TITLE as u64 + 1) * SECOND;
    let close = to.saturating_sub((END_TAG + CALLOUT) as u64 * SECOND);
    let mut named: Vec<&Moment> = marks
        .iter()
        .filter(|m| (open..=close).contains(&crate::film::peak(m)))
        .filter(|m| m.kind != MomentKind::Battle || m.worth >= HEAVY)
        .filter(|m| seen(m, plan, shape))
        .collect();
    named.sort_by(|a, b| b.worth.total_cmp(&a.worth));
    let mut out: Vec<Callout> = Vec::new();
    for m in named {
        let tick = crate::film::peak(m);
        if out.len() < CALLOUTS && out.iter().all(|c| c.tick.abs_diff(tick) >= 6 * SECOND) {
            let (title, detail) = crate::highlights::describe(m);
            out.push(Callout { tick, title, detail, kind: m.kind });
        }
    }
    out.sort_by_key(|c| c.tick);
    out
}

/// Every player's army's worth (armed mobile units: metal, and energy at 60 to 1), as the cast's
/// tug of war weighs it.
pub fn armies(game: &crate::game::Game) -> Vec<f32> {
    let sim = &game.sim;
    let mut out = vec![0.0f32; sim.players.len()];
    for u in sim.units() {
        if u.dead || !u.is_complete() {
            continue;
        }
        let ty = sim.unit_type(u);
        if ty.locomotion != ta_sim::Locomotion::Static && !ty.def.is_commander && ty.weapons.iter().any(|w| w.is_some())
            && let Some(a) = out.get_mut(u.owner as usize)
        {
            *a += ty.cost_metal.to_f32() + ty.cost_energy.to_f32() / 60.0;
        }
    }
    out
}

/// How far into view something shown from `from` to `to` seconds is at `now`: in over `rise`,
/// out over `fall`.
fn shown(now: f32, from: f32, to: f32, rise: f32, fall: f32) -> f32 {
    ((now - from) / rise).min((to - now) / fall).clamp(0.0, 1.0)
}

/// Text laid out on one line, `spacing` between its letters.
fn line(painter: &egui::Painter, text: &str, font: FontId, colour: Color32, spacing: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::TextFormat { font_id: font, color: colour, extra_letter_spacing: spacing, ..Default::default() });
    painter.layout_job(job)
}

/// [`line`] made smaller (to `least` of its size at most) to fit `room` across.
fn fitted(painter: &egui::Painter, text: &str, font: FontId, colour: Color32, spacing: f32, room: f32, least: f32) -> Arc<egui::Galley> {
    let g = line(painter, text, font.clone(), colour, spacing);
    if g.size().x <= room {
        return g;
    }
    let k = (room / g.size().x).max(least);
    line(painter, text, FontId::new(font.size * k, font.family), colour, spacing * k)
}

/// A galley drawn with its middle at `centre`, over a soft shadow (to read over the battle).
fn put(painter: &egui::Painter, centre: Pos2, g: Arc<egui::Galley>, alpha: f32) {
    let at = centre - g.size() / 2.0;
    let shadow = Color32::from_black_alpha((200.0 * alpha) as u8);
    painter.galley_with_override_text_color(at + vec2(0.0, g.size().y * 0.05).max(vec2(1.5, 2.0)), g.clone(), shadow);
    painter.galley(at, g, Color32::WHITE);
}

/// A band darkening the picture behind titles, fading out at its top and bottom.
fn band(painter: &egui::Painter, rect: Rect, alpha: f32) {
    let dark = Color32::from_black_alpha((170.0 * alpha) as u8);
    let feather = rect.height() * 0.35;
    painter.add(ta_ui::metal::vgrad(Rect::from_x_y_ranges(rect.x_range(), (rect.top() - feather)..=rect.top()), Color32::TRANSPARENT, dark));
    painter.rect_filled(rect, 0.0, dark);
    painter.add(ta_ui::metal::vgrad(Rect::from_x_y_ranges(rect.x_range(), rect.bottom()..=(rect.bottom() + feather)), dark, Color32::TRANSPARENT));
}

/// A moment's colour.
fn kind_colour(kind: MomentKind) -> Color32 {
    match kind {
        MomentKind::Nuke => Color32::from_rgb(255, 96, 70),
        MomentKind::Commander => Color32::from_rgb(240, 236, 220),
        MomentKind::Battle => AMBER_BRIGHT,
    }
}

impl ClipLook {
    /// Draws the titles over the clip's picture (an interface pass at its size, a point a pixel at
    /// 1080 across its narrow side) `seconds` in, the battle at `tick`; `armies`: every player's
    /// army's worth now.
    pub fn draw(&self, ctx: &egui::Context, seconds: f32, tick: u64, armies: &[f32]) {
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clip_titles")));
        // (Sizes are given for 1080 across the narrow side.)
        let k = screen.width().min(screen.height()) / 1080.0;
        let safe = self.shape.safe();
        let room = safe.centred(screen);
        let tag_from = self.seconds - END_TAG;
        if self.scoreboard {
            let a = shown(seconds, TITLE - 0.6, tag_from, 0.5, 0.4);
            if a > 0.0 {
                self.versus(&painter, safe.centred(screen), k, a, tick, armies);
            }
        }
        if self.titles {
            // (Up from the first picture, which apps take for the clip's cover.)
            let a = shown(seconds, -1.0, TITLE, 1.0, 0.5);
            if a > 0.0 {
                self.opening(&painter, screen, room, k, a);
            }
            for c in &self.callouts {
                let at = c.tick.saturating_sub(self.start) as f32 / SECOND as f32 + 0.2;
                let a = shown(seconds, at, at + CALLOUT, 0.25, 0.4);
                if a > 0.0 {
                    self.callout(&painter, room, k, a, c, seconds - at);
                }
            }
            let a = shown(seconds, tag_from, self.seconds + 1.0, 0.6, 1.0);
            if a > 0.0 {
                self.end_tag(&painter, screen, room, k, a);
            }
        }
    }

    /// The map and who fought, over the clip's opening seconds.
    fn opening(&self, p: &egui::Painter, screen: Rect, room: Rect, k: f32, a: f32) {
        let vertical = self.shape == Shape::Vertical;
        let mid = if vertical { screen.top() + screen.height() * 0.36 } else { screen.top() + screen.height() * 0.43 };
        let rise = (1.0 - a) * 14.0 * k;
        let brand = fitted(p, t("TOTAL ANNIHILATION REBORN"), strong(28.0 * k), AMBER_BRIGHT.gamma_multiply(a), 3.0 * k, room.width(), 0.6);
        let map = fitted(p, &self.map, display(if vertical { 104.0 } else { 96.0 } * k), TEXT_BRIGHT.gamma_multiply(a), 1.0 * k, room.width(), 0.45);
        let players = self.players_line(p, k, room.width(), a);
        let gap = 14.0 * k;
        let height = brand.size().y + map.size().y + players.iter().map(|g| g.size().y).sum::<f32>() + gap * (1 + players.len()) as f32;
        let top = mid - height / 2.0 + rise;
        band(p, Rect::from_x_y_ranges(screen.x_range(), (top - 28.0 * k)..=(top + height + 28.0 * k)), a);
        // (Brass rules either side of the game's name, drawn out as it comes in.)
        let mut y = top + brand.size().y / 2.0;
        let reach = (room.width() * 0.5 - brand.size().x / 2.0 - 18.0 * k).max(0.0) * a;
        for side in [-1.0f32, 1.0] {
            let inner = screen.center().x + side * (brand.size().x / 2.0 + 18.0 * k);
            p.hline(inner.min(inner + side * reach)..=inner.max(inner + side * reach), y, Stroke::new(2.0 * k, AMBER.gamma_multiply(0.8 * a)));
        }
        put(p, pos2(screen.center().x, y), brand.clone(), a);
        y += brand.size().y / 2.0 + gap + map.size().y / 2.0;
        put(p, pos2(screen.center().x, y), map.clone(), a);
        y += map.size().y / 2.0 + gap;
        for g in players {
            put(p, pos2(screen.center().x, y + g.size().y / 2.0), g.clone(), a);
            y += g.size().y + gap;
        }
    }

    /// Who fought, in their colours: "A VS B" on a line where it fits (else a line each), more
    /// players a line each, at most four shown.
    fn players_line(&self, p: &egui::Painter, k: f32, room: f32, a: f32) -> Vec<Arc<egui::Galley>> {
        let size = 46.0 * k;
        let name = |i: usize| {
            let (name, _, colour) = &self.players[i];
            (name.clone(), colour.lerp_to_gamma(Color32::WHITE, 0.3).gamma_multiply(a))
        };
        let compose = |parts: &[(String, Color32, f32)]| {
            let mut job = egui::text::LayoutJob::default();
            for (text, colour, scale) in parts {
                job.append(text, 0.0, egui::TextFormat { font_id: strong(size * scale), color: *colour, valign: egui::Align::Center, ..Default::default() });
            }
            p.layout_job(job)
        };
        let vs = (format!("  {}  ", t("VS")), STEEL.gamma_multiply(a), 0.7);
        if self.players.len() == 2 {
            let (a_name, a_colour) = name(0);
            let (b_name, b_colour) = name(1);
            let one = compose(&[(a_name.clone(), a_colour, 1.0), vs.clone(), (b_name.clone(), b_colour, 1.0)]);
            if one.size().x <= room {
                return vec![one];
            }
            return vec![compose(&[(a_name, a_colour, 0.9)]), compose(&[(t("VS").to_string(), STEEL.gamma_multiply(a), 0.6)]), compose(&[(b_name, b_colour, 0.9)])];
        }
        let mut lines: Vec<Arc<egui::Galley>> = (0..self.players.len().min(4)).map(|i| {
            let (n, c) = name(i);
            compose(&[(n, c, 0.8)])
        }).collect();
        if self.players.len() > 4 {
            lines.push(compose(&[(tf("and {count} more", &[("count", &(self.players.len() - 4))]), STEEL.gamma_multiply(a), 0.6)]));
        }
        lines
    }

    /// The tug of war across the top of the safe part: the two names (or the colours of more),
    /// the battle's clock, each army's share.
    fn versus(&self, p: &egui::Painter, room: Rect, k: f32, a: f32, tick: u64, armies: &[f32]) {
        if self.players.len() < 2 {
            return;
        }
        let width = room.width().min(980.0 * k);
        let top = room.top() + 8.0 * k;
        let plate = Rect::from_min_size(pos2(room.center().x - width / 2.0, top), vec2(width, 118.0 * k));
        p.rect_filled(plate.translate(vec2(0.0, 4.0 * k)).expand(2.0 * k), 10.0 * k, Color32::from_black_alpha((90.0 * a) as u8));
        p.rect_filled(plate, 8.0 * k, Color32::from_rgba_unmultiplied(16, 15, 13, (215.0 * a) as u8));
        p.rect_stroke(plate, 8.0 * k, Stroke::new(1.5 * k, LINE_HI.gamma_multiply(a)), StrokeKind::Inside);
        let pad = 18.0 * k;
        let row = plate.top() + 36.0 * k;
        let colour = |i: usize| self.players.get(i).map_or(STEEL, |p| p.2);
        p.text(pos2(plate.center().x, row), Align2::CENTER_CENTER, clock(tick), display(42.0 * k), TEXT_BRIGHT.gamma_multiply(a));
        let shares: Vec<f32> = (0..self.players.len()).map(|i| armies.get(i).copied().unwrap_or(0.0).max(0.0)).collect();
        let total: f32 = shares.iter().sum();
        if self.players.len() == 2 {
            let name_room = width / 2.0 - pad - 70.0 * k;
            for (i, left) in [(0usize, true), (1, false)] {
                let name = fitted(p, &self.players[i].0, strong(40.0 * k), colour(i).lerp_to_gamma(Color32::WHITE, 0.35).gamma_multiply(a), 0.0, name_room, 0.6);
                let x = if left { plate.left() + pad } else { plate.right() - pad - name.size().x };
                p.galley(pos2(x, row - name.size().y / 2.0), name, Color32::WHITE);
            }
        }
        // Each army's share, side by side.
        let bar = Rect::from_min_max(pos2(plate.left() + pad, plate.top() + 66.0 * k), pos2(plate.right() - pad, plate.top() + 102.0 * k));
        p.rect_filled(bar, 4.0 * k, INSET.gamma_multiply(a));
        let inner = bar.shrink(3.0 * k);
        let mut x = inner.left();
        for (i, share) in shares.iter().enumerate() {
            let w = if total > 0.0 { inner.width() * share / total } else { inner.width() / shares.len() as f32 };
            let r = Rect::from_min_max(pos2(x, inner.top()), pos2((x + w).min(inner.right()), inner.bottom()));
            p.add(ta_ui::metal::vgrad(r, colour(i).lerp_to_gamma(Color32::WHITE, 0.2).gamma_multiply(a), colour(i).lerp_to_gamma(Color32::BLACK, 0.3).gamma_multiply(a)));
            x += w;
            if i + 1 < shares.len() {
                p.vline(x, inner.y_range().expand(3.0 * k), Stroke::new(2.5 * k, Color32::WHITE.gamma_multiply(0.85 * a)));
            }
        }
        if self.players.len() == 2 {
            for (i, align, at) in [(0usize, Align2::LEFT_CENTER, inner.left() + 8.0 * k), (1, Align2::RIGHT_CENTER, inner.right() - 8.0 * k)] {
                p.text(pos2(at, inner.center().y), align, ta_ui::i18n::grouped(shares[i].round() as u64), strong(26.0 * k), TEXT_BRIGHT.gamma_multiply(a));
            }
            p.text(inner.center(), Align2::CENTER_CENTER, t("ARMY"), strong(20.0 * k), Color32::from_white_alpha((210.0 * a) as u8));
        }
    }

    /// A big moment named in the lower part of the safe area, rising in.
    fn callout(&self, p: &egui::Painter, room: Rect, k: f32, a: f32, c: &Callout, age: f32) {
        let vertical = self.shape == Shape::Vertical;
        let centre_y = if vertical { room.bottom() - room.height() * 0.2 } else { room.bottom() - room.height() * 0.17 };
        let pop = 1.0 + 0.08 * (1.0 - (age / 0.3).clamp(0.0, 1.0));
        let rise = (1.0 - a) * 24.0 * k;
        let title = fitted(p, t(c.title), display(if vertical { 84.0 } else { 76.0 } * k * pop), TEXT_BRIGHT.gamma_multiply(a), 1.5 * k, room.width() - 40.0 * k, 0.5);
        let detail = fitted(p, &c.detail, strong(34.0 * k), TEXT.gamma_multiply(a), 0.0, room.width() - 40.0 * k, 0.6);
        let height = title.size().y + detail.size().y + 30.0 * k;
        let width = title.size().x.max(detail.size().x) + 64.0 * k;
        let plate = Rect::from_center_size(pos2(room.center().x, centre_y + rise), vec2(width, height));
        p.rect_filled(plate, 6.0 * k, Color32::from_black_alpha((175.0 * a) as u8));
        let accent = kind_colour(c.kind).gamma_multiply(a);
        p.rect_filled(Rect::from_min_size(plate.left_top(), vec2(plate.width(), 6.0 * k)), 0.0, accent);
        put(p, pos2(plate.center().x, plate.top() + 12.0 * k + title.size().y / 2.0), title.clone(), a);
        put(p, pos2(plate.center().x, plate.bottom() - 12.0 * k - detail.size().y / 2.0), detail, a);
    }

    /// The end tag: the picture darkening under the game's emblem and name.
    fn end_tag(&self, p: &egui::Painter, screen: Rect, room: Rect, k: f32, a: f32) {
        p.rect_filled(screen, 0.0, Color32::from_black_alpha((190.0 * a) as u8));
        let vertical = self.shape == Shape::Vertical;
        let emblem = if vertical { 220.0 } else { 190.0 } * k;
        let title = fitted(p, t("TOTAL ANNIHILATION REBORN"), display(if vertical { 76.0 } else { 72.0 } * k), TEXT_BRIGHT.gamma_multiply(a), 2.0 * k, room.width(), 0.5);
        let mut job = egui::text::LayoutJob::default();
        job.append(t("An unofficial reimplementation of Total Annihilation"), 0.0, egui::TextFormat { font_id: strong(32.0 * k), color: STEEL.gamma_multiply(a), ..Default::default() });
        job.wrap.max_width = room.width();
        job.halign = egui::Align::Center;
        let tagline = p.layout_job(job);
        let gap = 26.0 * k;
        let height = emblem + gap + title.size().y + gap + tagline.size().y;
        let mid = room.center().y - room.height() * 0.04;
        let scale = 0.92 + 0.08 * a;
        let top = mid - height / 2.0;
        let e = Rect::from_center_size(pos2(screen.center().x, top + emblem / 2.0), vec2(emblem, emblem) * scale);
        paint_emblem(p, e, a);
        let mut y = top + emblem + gap + title.size().y / 2.0;
        put(p, pos2(screen.center().x, y), title.clone(), a);
        y += title.size().y / 2.0 + gap * 0.6;
        let rule = room.width() * 0.3 * a;
        p.hline((screen.center().x - rule)..=(screen.center().x + rule), y, Stroke::new(2.0 * k, AMBER.gamma_multiply(0.8 * a)));
        y += gap * 0.6;
        // (Its rows centred on where it is put.)
        p.galley(pos2(screen.center().x, y), tagline, Color32::WHITE);
    }
}

/// The game's emblem in `rect` (square), `alpha` of the way in: its picture at the size drawn,
/// made once for each size (as `ta_ui::widgets::paint_emblem`, which draws it opaque).
fn paint_emblem(painter: &egui::Painter, rect: Rect, alpha: f32) {
    let px = (rect.width() * painter.ctx().pixels_per_point()).round().clamp(16.0, 1024.0) as u32;
    let id = egui::Id::new(("clip_emblem", px));
    let texture = painter.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied([px as usize; 2], &ta_ui::icon::emblem_rgba(px));
        let texture = painter.ctx().load_texture(format!("clip-emblem-{px}"), image, egui::TextureOptions::LINEAR);
        painter.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(alpha));
}

// ---- The sound ------------------------------------------------------------------------------

/// The level a clip's sound is ridden to (its RMS, dBFS: some -14 LUFS, where social sites set
/// what they play), how far it may be ridden either way (dB), and below what it is left alone
/// (silence is not lifted).
const RIDE_TO: f32 = -16.5;
const RIDE_MOST: f32 = 6.0;
const RIDE_GATE: f32 = -50.0;

/// A clip's sound brought to the level social sites play at and kept within a ceiling: ridden
/// slowly towards [`RIDE_TO`] (over some seconds, a battle's quiet stretch lifted, its loudest
/// lowered, within [`RIDE_MOST`]), and its peaks held under -1 dBFS as the sites ask, rather than
/// clipped (they would be after their encoders again): met at once, let go over a fifth of a
/// second.
#[derive(Clone, Debug)]
pub struct Limiter {
    /// The limiter's gain, the rider's, and the sound's power (mean square) as the rider hears it.
    gain: f32,
    level: f32,
    power: f32,
    ceiling: f32,
    /// Per stereo frame: the limiter's letting go, the power's following, the rider's moving.
    release: f32,
    hear: f32,
    ride: f32,
}

impl Limiter {
    pub fn new(rate: u32) -> Limiter {
        let per = |seconds: f32| 1.0 - (-1.0 / (seconds * rate as f32)).exp();
        Limiter {
            gain: 1.0,
            level: 1.0,
            power: 10f32.powf(RIDE_TO / 10.0),
            ceiling: 10f32.powf(-1.5 / 20.0),
            release: per(0.2),
            hear: per(3.0),
            ride: per(2.0),
        }
    }

    /// `samples` (interleaved stereo) through it.
    pub fn process(&mut self, samples: &mut [f32]) {
        let (target, gate) = (10f32.powf(RIDE_TO / 10.0), 10f32.powf(RIDE_GATE / 10.0));
        let (least, most) = (10f32.powf(-RIDE_MOST / 20.0), 10f32.powf(RIDE_MOST / 20.0));
        for frame in samples.chunks_exact_mut(2) {
            let power = (frame[0] * frame[0] + frame[1] * frame[1]) / 2.0;
            if power > gate {
                self.power += (power - self.power) * self.hear;
                let want = (target / self.power.max(1e-9)).sqrt().clamp(least, most);
                self.level += (want - self.level) * self.ride;
            }
            let peak = frame[0].abs().max(frame[1].abs()) * self.level;
            let fits = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
            self.gain = if fits < self.gain { fits } else { self.gain + (fits - self.gain) * self.release };
            let g = self.level * self.gain;
            frame[0] = (frame[0] * g).clamp(-self.ceiling, self.ceiling);
            frame[1] = (frame[1] * g).clamp(-self.ceiling, self.ceiling);
        }
    }
}

// ---- Where clips go ----------------------------------------------------------------------------

/// The folder clips go to: `Videos\TA Reborn\Clips`.
pub fn clips_dir() -> PathBuf {
    ta_config::videos_dir().join("Clips")
}

/// Where a clip of a battle on `map`, from `from` (ticks), goes in `dir`: "Metal Heck 12-40
/// vertical 30s 001.mp4" (the first number free).
pub fn clip_path(dir: &Path, map: &str, from: u64, spec: ClipSpec) -> PathBuf {
    let s = from / SECOND;
    let at = if s >= 3600 { format!("{}-{:02}-{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}-{:02}", s / 60, s % 60) };
    crate::film::film_path(dir, &format!("{map} {at} {} {}s", t(spec.shape.word()), spec.seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A beat with `weight` going on at `at`.
    fn beat(weight: f32, at: Vec2) -> Beat {
        let spots = if weight > 0.0 { vec![Spot { at, weight, spread: 300.0 }] } else { Vec::new() };
        Beat { weight, shots: (weight / 2.0) as u32, spots, ..Default::default() }
    }

    /// A battle `seconds` long, quiet but for skirmishing, with a fight building at `at` from
    /// `rise` to its climax at `peak`, then dying away.
    fn battle(seconds: usize, rise: usize, peak: usize, at: Vec2) -> ActionLog {
        let beats = (0..seconds)
            .map(|s| {
                if (rise..=peak + 4).contains(&s) {
                    let w = if s <= peak { 10.0 + 90.0 * (s - rise) as f32 / (peak - rise) as f32 } else { 40.0 };
                    let mut b = beat(w, at);
                    if s == peak {
                        (b.lost, b.worth) = (6, 2400.0);
                    }
                    b
                } else {
                    beat(if s % 7 == 0 { 3.0 } else { 0.0 }, Vec2::new(4000.0, 4000.0))
                }
            })
            .collect();
        ActionLog { beats, blasts: Vec::new() }
    }

    #[test]
    fn the_capture_switch_reads_length_and_shape() {
        assert_eq!(ClipSpec::parse("30v"), Some(ClipSpec { seconds: 30, shape: Shape::Vertical }));
        assert_eq!(ClipSpec::parse(" 60H "), Some(ClipSpec { seconds: 60, shape: Shape::Horizontal }));
        assert_eq!(ClipSpec::parse("60"), None);
        assert_eq!(ClipSpec::parse("v"), None);
        assert_eq!(ClipSpec::parse("2v"), None);
    }

    #[test]
    fn the_best_stretch_builds_to_its_climax() {
        let log = battle(600, 200, 260, Vec2::new(1000.0, 1200.0));
        let found = candidates(&log, &[], 600 * SECOND, ClipSpec::default());
        let best = found.first().expect("a stretch with a fight in it");
        // The climax comes late in it, clear of the end tag.
        let into = (best.climax - best.from) as f32 / (best.to - best.from) as f32;
        assert!((0.5..=0.9).contains(&into), "the climax {into:.2} of the way in: {best:?}");
        assert!((257..=261).contains(&(best.climax / SECOND)), "{best:?}");
        assert_eq!(best.to - best.from, 30 * SECOND);
        assert!(best.focus.distance(Vec2::new(1000.0, 1200.0)) < 1.0);
        assert_eq!((best.lost, best.title), (6, "HEAVY FIGHTING"));
        assert_eq!(best.curve.len(), 30);
    }

    #[test]
    fn stretches_offered_are_apart_and_the_best_first() {
        let mut log = battle(900, 100, 150, Vec2::new(500.0, 500.0));
        // A second, bigger fight later, elsewhere.
        let other = battle(900, 500, 560, Vec2::new(3000.0, 600.0));
        for s in 480..600 {
            let mut b = other.beats[s].clone();
            b.weight *= 2.0;
            if let Some(spot) = b.spots.first_mut() {
                spot.weight *= 2.0;
            }
            log.beats[s] = b;
        }
        let found = candidates(&log, &[], 900 * SECOND, ClipSpec { seconds: 60, shape: Shape::Horizontal });
        assert!(found.len() >= 2, "{found:?}");
        assert!(found.windows(2).all(|w| w[0].score >= w[1].score));
        // Every climax comes clear of the end tag (and not in the opening seconds).
        for c in &found {
            let into = (c.climax - c.from) as f32 / SECOND as f32;
            assert!((18.0..=60.0 - END_TAG - 1.5).contains(&into), "the climax {into:.1} s in: {c:?}");
        }
        assert!(found[0].climax / SECOND >= 540, "the bigger fight first: {:?}", found[0]);
        for (i, a) in found.iter().enumerate() {
            for b in &found[i + 1..] {
                let shared = a.to.min(b.to).saturating_sub(a.from.max(b.from));
                assert!(shared <= 60 * SECOND * 35 / 100, "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn a_nuclear_strike_makes_the_climax() {
        let mut log = battle(600, 100, 160, Vec2::new(800.0, 800.0));
        log.beats[400] = Beat { weight: 400.0, nuke: true, lost: 30, worth: 9000.0, spots: vec![Spot { at: Vec2::new(2500.0, 2500.0), weight: 400.0, spread: 700.0 }], ..Default::default() };
        // (The scan's mark of it: the strike lands seven seconds after the moment starts.)
        let strike = Moment { start: 393 * SECOND, end: 405 * SECOND, focus: glam::Vec3::new(2500.0, 0.0, 2500.0), spread: 400.0, kind: MomentKind::Nuke, units: 30, worth: 6000.0, slow: None, biggest: false };
        let found = candidates(&log, std::slice::from_ref(&strike), 600 * SECOND, ClipSpec::default());
        assert_eq!(found[0].title, "NUCLEAR STRIKE");
        assert_eq!(found[0].climax / SECOND, 400);
        // A strike the camera does not see does not name the clip.
        let elsewhere = Moment { focus: glam::Vec3::new(9000.0, 0.0, 9000.0), ..strike };
        let found = candidates(&log, &[elsewhere], 600 * SECOND, ClipSpec::default());
        assert_ne!(found[0].title, "NUCLEAR STRIKE");
    }

    #[test]
    fn a_quiet_battle_offers_nothing_and_a_short_one_all_of_it() {
        let quiet = ActionLog { beats: vec![Beat::default(); 300], blasts: Vec::new() };
        assert!(candidates(&quiet, &[], 300 * SECOND, ClipSpec::default()).is_empty());
        let short = battle(20, 2, 12, Vec2::new(100.0, 100.0));
        let found = candidates(&short, &[], 20 * SECOND, ClipSpec::default());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].from, found[0].to), (0, 20 * SECOND));
    }

    #[test]
    fn no_clip_opens_on_a_blast_going_off_in_view() {
        let mut log = battle(600, 200, 260, Vec2::new(1000.0, 1200.0));
        let first = candidates(&log, &[], 600 * SECOND, ClipSpec::default());
        let start = first[0].from;
        // Blasts in view just before that start, and one far away that does not count.
        log.blasts = vec![
            Blast { tick: start - 20, at: Vec2::new(1050.0, 1150.0), size: 3.0 },
            Blast { tick: start - 5, at: Vec2::new(950.0, 1250.0), size: 1.0 },
            Blast { tick: start - 2, at: Vec2::new(9000.0, 9000.0), size: 9.0 },
        ];
        let moved = candidates(&log, &[], 600 * SECOND, ClipSpec::default());
        let from = moved[0].from;
        assert_ne!(from, start);
        assert!(log.blasts[..2].iter().all(|b| !(b.tick + 36 >= from && b.tick < from)), "{from} still opens on a blast");
        assert!(from.abs_diff(start) <= 90, "it moves a little only: {start} to {from}");
    }

    #[test]
    fn the_camera_follows_the_fight_and_cuts_only_when_it_goes_quiet() {
        let a = Vec2::new(1000.0, 1000.0);
        let b = Vec2::new(5000.0, 1000.0);
        // A fight at `a` moving east 10 units a second, quiet after 40 s; then one at `b`.
        let beats: Vec<Beat> = (0..90)
            .map(|s| if s < 40 { beat(50.0, a + Vec2::new(10.0 * s as f32, 0.0)) } else if s >= 44 { beat(80.0, b) } else { Beat::default() })
            .collect();
        let log = ActionLog { beats, blasts: Vec::new() };
        let plan = plan(&log, 10 * SECOND, 70 * SECOND, 30 * SECOND + 15, a);
        // On the fight while it goes on, gliding with it...
        let (at, _) = plan.at((20 * SECOND + 15) as f32);
        assert!(at.distance(a + Vec2::new(200.0, 0.0)) < 30.0, "{at}");
        // ...held while nothing goes on, then one cut to the other fight.
        assert_eq!(plan.cuts().len(), 1, "{:?}", plan.cuts());
        let cut = plan.cuts()[0];
        assert!((44 * SECOND..=46 * SECOND + 15).contains(&cut), "{cut}");
        let (late, _) = plan.at((60 * SECOND) as f32);
        assert!(late.distance(b) < 1.0);
        // The path has no jumps but the cut.
        let mut last = plan.at((10 * SECOND) as f32).0;
        for tick in 10 * SECOND + 1..70 * SECOND {
            let (here, _) = plan.at(tick as f32);
            if !(cut - 1..=cut).contains(&tick) {
                assert!(here.distance(last) < 15.0, "a jump at {tick}: {last} to {here}");
            }
            last = here;
        }
    }

    #[test]
    fn no_cut_comes_just_after_the_start() {
        let a = Vec2::new(1000.0, 1000.0);
        let b = Vec2::new(6000.0, 1000.0);
        // Another fight for the clip's first second and a half, then the climax's from then on.
        let beats: Vec<Beat> = (0..80).map(|s| if s < 11 { beat(60.0, b) } else { beat(60.0, a) }).collect();
        let log = ActionLog { beats, blasts: Vec::new() };
        let plan = plan(&log, 10 * SECOND, 40 * SECOND, 30 * SECOND, a);
        assert!(plan.cuts().iter().all(|&c| c < 10 * SECOND || c > 13 * SECOND), "{:?}", plan.cuts());
        assert!(plan.at((10 * SECOND) as f32).0.distance(a) < 1.0);
    }

    #[test]
    fn the_recorder_sums_seconds_and_gathers_places() {
        let mut cells = BTreeMap::new();
        let mut add = |at: Vec2, w: f32| cells.entry(((at.x / CELL).floor() as i32, (at.y / CELL).floor() as i32)).or_insert_with(Cell::default).add(at, w);
        for i in 0..10 {
            add(Vec2::new(1000.0 + i as f32 * 20.0, 1000.0), 2.0);
        }
        add(Vec2::new(6000.0, 6000.0), 1.0);
        let found = spots(cells);
        assert_eq!(found.len(), 2);
        assert!((found[0].weight - 20.0).abs() < 1e-3 && found[0].at.distance(Vec2::new(1090.0, 1000.0)) < 1.0);
        assert!(found[0].spread >= 160.0);
        assert!(found[1].at.distance(Vec2::new(6000.0, 6000.0)) < 1.0);
    }

    /// Ten seconds of a tone `amplitude` high, through `limiter`; the last second's RMS (dBFS) and
    /// the loudest sample.
    fn tone(limiter: &mut Limiter, amplitude: f32) -> (f32, f32) {
        let mut sound: Vec<f32> = (0..480_000).flat_map(|i| {
            let v = (i as f32 * 0.0628).sin() * amplitude;
            [v, -v]
        }).collect();
        limiter.process(&mut sound);
        let last = &sound[sound.len() - 96_000..];
        let rms = (last.iter().map(|v| v * v).sum::<f32>() / last.len() as f32).sqrt();
        (20.0 * rms.log10(), sound.iter().fold(0.0f32, |m, v| m.max(v.abs())))
    }

    #[test]
    fn the_sound_is_ridden_to_the_sites_level_under_its_ceiling() {
        let ceiling = 10f32.powf(-1.5 / 20.0);
        // Far too loud: brought down as far as it goes, never over the ceiling.
        let (rms, peak) = tone(&mut Limiter::new(48_000), 1.4);
        let loud = 20.0 * (1.4f32 / std::f32::consts::SQRT_2).log10();
        assert!(peak <= ceiling + 1e-6 && (rms - (loud - RIDE_MOST)).abs() < 0.5, "{rms:.1} dB, peak {peak}");
        // At the level: left as it is.
        let at = 10f32.powf(RIDE_TO / 20.0) * std::f32::consts::SQRT_2;
        let (rms, _) = tone(&mut Limiter::new(48_000), at);
        assert!((rms - RIDE_TO).abs() < 0.5, "{rms:.1} dB");
        // Quiet: lifted, but by six decibels at the most.
        let (rms, _) = tone(&mut Limiter::new(48_000), at / 10.0);
        assert!((rms - (RIDE_TO - 20.0 + RIDE_MOST)).abs() < 0.5, "{rms:.1} dB");
        // Silence stays silent (it is not lifted to make noise of).
        let mut silence = vec![1e-4f32; 96_000];
        Limiter::new(48_000).process(&mut silence);
        assert!(silence.iter().all(|v| v.abs() < 2e-4));
    }

    #[test]
    fn clips_are_named_for_the_map_the_time_the_shape_and_the_length() {
        let dir = std::env::temp_dir().join(format!("ta-clip-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = clip_path(&dir, "Metal Heck", 760 * SECOND, ClipSpec { seconds: 30, shape: Shape::Vertical });
        assert_eq!(path.file_name().unwrap().to_string_lossy(), "Metal Heck 12-40 vertical 30s 001.mp4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_safe_areas_leave_the_middle_clear() {
        for shape in [Shape::Vertical, Shape::Horizontal] {
            let (w, h) = shape.size();
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32));
            let safe = shape.safe().rect(screen);
            assert!(safe.contains(screen.center()));
            assert!(shape.safe().centred(screen).width() <= safe.width());
            assert!(safe.area() > screen.area() * 0.45);
        }
        // About 7.5 Mbit/s: a minute is under 60 MB.
        assert!(ClipSpec { seconds: 60, shape: Shape::Vertical }.bytes(Some(192_000)) < 60.0e6);
    }
}
