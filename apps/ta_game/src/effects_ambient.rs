//! World ambience: weather over the view (snow on ice worlds, embers rising off the lava as GPU
//! particles, showers with lightning on green and water worlds, dust on deserts, spores on
//! acid, glitter on crystal), the ways ships and hovercraft come for their wakes (drawn on the
//! water: `ta_render::wakes`), tread marks behind tanks, and the crackle of paralyzed units.
//!
//! Everything here is presentation only: it reads the simulation and never feeds back.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};
use ta_render::BeamInstance;
use ta_render::sprites::{SpriteId, SpriteInstance};
use ta_render::{ParticleBurst as Burst, ParticleKind as Kind};
use ta_sim::{Fx, Locomotion, Sim, UnitId};

use super::{Effects, Particle};

/// The weather at a moment, to be put back (see [`Effects::weather_state`]).
#[derive(Clone, Copy, Debug)]
pub struct WeatherState {
    weather: Weather,
    rain_phase: f32,
    wetness: Option<f32>,
}

/// Weather drawn over a world, chosen from the map's planet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Weather {
    #[default]
    None,
    Snow,
    Embers,
    /// Showers that come and go, with lightning while they are heavy.
    Rain,
    Dust,
    Spores,
    Glitter,
}

impl Weather {
    pub fn for_planet(planet: &str) -> Weather {
        let p = planet.to_ascii_lowercase();
        if p.contains("ice") {
            Weather::Snow
        } else if p.contains("lava") {
            Weather::Embers
        } else if p.contains("green") || p.contains("lush") || p.contains("archipelago") || p.contains("water world") || p.contains("earth") {
            Weather::Rain
        } else if p.contains("desert") || p.contains("red planet") {
            Weather::Dust
        } else if p.contains("acid") {
            Weather::Spores
        } else if p.contains("crystal") {
            Weather::Glitter
        } else {
            Weather::None
        }
    }

    /// Motes per million square pixels of view.
    fn density(self) -> f32 {
        match self {
            Weather::None => 0.0,
            Weather::Snow => 320.0,
            Weather::Embers | Weather::Rain => 70.0,
            Weather::Dust => 16.0,
            Weather::Spores => 90.0,
            Weather::Glitter => 140.0,
        }
    }
}

/// One weather particle.
pub(super) struct Mote {
    pos: Vec3,
    vel: Vec3,
    born: f32,
    life: f32,
    size: f32,
    phase: f32,
}

/// A ship's or hovercraft's hull at the waterline, for its wake: how far its bow lies ahead of
/// its middle and its stern behind, and its half-width (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hull {
    bow: f32,
    stern: f32,
    beam: f32,
}

impl Hull {
    /// The hull of a unit type from its model's pieces at rest (`bounds`, see
    /// `Scene::piece_bounds`): the pieces down at the waterline, not what stands on deck.
    fn of(t: &ta_sim::UnitType, bounds: &[Option<crate::scene::PieceBounds>]) -> Option<Hull> {
        let pieces = &t.model.pieces;
        let mut at: Vec<Vec3> = Vec::with_capacity(pieces.len());
        for p in pieces {
            let o = Vec3::from_array(p.offset.to_f32());
            let parent = p.parent.and_then(|q| at.get(q).copied()).unwrap_or(Vec3::ZERO);
            at.push(parent + o);
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (b, &o) in bounds.iter().zip(&at) {
            let Some(b) = b else { continue };
            if o.y + b.min.y > 4.0 {
                continue;
            }
            lo = lo.min(o + b.min);
            hi = hi.max(o + b.max);
        }
        (lo.x <= hi.x).then(|| Hull { bow: hi.z.max(2.0), stern: (-lo.z).max(2.0), beam: hi.x.max(-lo.x).max(2.0) })
    }

    /// A hull from a unit type's footprint alone (a model not loaded).
    fn guess(t: &ta_sim::UnitType) -> Hull {
        let r = t.radius.to_f32().max(4.0);
        Hull { bow: r, stern: r, beam: r * 0.45 }
    }
}

/// The way a ship or hovercraft came over the water, for its wake (see `ta_render::wakes`).
pub(super) struct Wake {
    /// Where its middle passed, oldest first: there, its heading then, when (game time), how
    /// fast it went (world units a second) and how far it had come along its way.
    points: VecDeque<(Vec2, f32, f32, f32, f32)>,
    /// Its middle and heading at the last two ticks it was followed at (it is drawn between the
    /// tick before the last and the last), the last tick and how many ticks lay between the two.
    ticks: [(Vec2, f32); 2],
    tick: u64,
    step: u64,
    hull: Hull,
    hover: bool,
    /// Still followed this frame (on the water, in sight); else its wake fades where it lies.
    live: bool,
}

impl Wake {
    fn overlaps(&self, now: f32, lo: Vec2, hi: Vec2) -> bool {
        let hull = ta_render::WakeTrail { first: 0, count: 0, bow: self.hull.bow, stern: self.hull.stern, beam: self.hull.beam, hover: self.hover };
        let (mut lower, mut upper) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        let mut include = |point: Vec2, age: f32, speed: f32| {
            let age = age.max(0.0) + hull.bow / speed.max(10.0);
            let bow = hull.bow + (hull.beam * 0.8).max(4.0) + hull.beam * 1.5 + 4.0;
            let radius = Vec2::splat(hull.reach(age, speed).max(bow));
            lower = lower.min(point - radius);
            upper = upper.max(point + radius);
        };
        for &(point, _, time, speed, _) in &self.points {
            include(point, now - time, speed);
        }
        if self.live {
            let speed = self.ticks[0].0.distance(self.ticks[1].0) / (self.step.max(1) as f32 * crate::game::TICK_SECONDS);
            include(self.ticks[0].0, 0.0, speed);
            include(self.ticks[1].0, 0.0, speed);
        }
        // Includes the straight ribbon between points, even if neither endpoint is on screen.
        lower.cmple(hi).all() && upper.cmpge(lo).all()
    }
}

/// How far a ship's or hovercraft's way may go between two points of its wake (world units;
/// more at speed and for a long hull: the wake is drawn straight between them, and where the
/// way bends, `WAKE_TURN` adds points).
fn wake_spacing(hull: &Hull, speed: f32) -> f32 {
    (0.3 * (hull.bow + hull.stern)).max(speed * 0.25).max(6.0)
}

/// How far its heading may turn between two points of its wake (radians).
const WAKE_TURN: f32 = 0.12;
/// The most points a wake keeps (a hull at full speed fills about this many in its life).
const WAKE_POINTS: usize = 192;

/// The difference from heading `a` to heading `b` (radians), brought into -pi..pi.
fn angle_between(a: f32, b: f32) -> f32 {
    (b - a + PI).rem_euclid(TAU) - PI
}

/// A piece of tread mark on the ground, or a walker's footprint.
pub(super) struct Track {
    pos: Vec3,
    rotation: f32,
    scale: f32,
    born: f32,
    /// The picture it is drawn with (the tread's, or a sole of its proportions).
    sprite: SpriteId,
    /// How deep it is pressed (its opacity, about 1).
    depth: f32,
}

/// Ambience state kept by [`Effects`].
#[derive(Default)]
pub struct Ambient {
    pub weather: Weather,
    /// The world's own weather, kept when a film's scene first changes it (see
    /// [`Effects::scene_weather`]).
    pub(super) own_weather: Option<Weather>,
    /// Horizontal wind direction (unit length), eased towards the simulation's.
    pub(super) wind_dir: Vec2,
    /// The wind's speed (the simulation's units: 20 a strength of 1), eased as its direction.
    pub(super) wind_speed: f32,
    pub(super) wind_synced: bool,
    pub(super) motes: Vec<Mote>,
    pub(super) tracks: VecDeque<Track>,
    /// Where each tank last laid a track, and when each hovercraft ashore last raised dust.
    pub(super) last_track: rustc_hash::FxHashMap<UnitId, Vec3>,
    pub(super) last_wake: rustc_hash::FxHashMap<UnitId, f32>,
    /// The ways the ships and hovercraft on the water came, for their wakes; those no longer
    /// followed (sunk, ashore, out of sight) fading where they lie; each type's hull.
    pub(super) wakes: rustc_hash::FxHashMap<UnitId, Wake>,
    pub(super) faded_wakes: Vec<Wake>,
    pub(super) hulls: rustc_hash::FxHashMap<usize, Hull>,
    /// The tick the wakes were last followed at.
    pub(super) wakes_tick: Option<u64>,
    /// Where each walker last set a foot down, and which of its feet comes down next.
    pub(super) last_step: rustc_hash::FxHashMap<UnitId, (Vec3, usize)>,
    /// How each walking type leaves its prints (from the feet it stands on, `Scene::feet`).
    pub(super) gaits: rustc_hash::FxHashMap<usize, std::sync::Arc<Gait>>,
    /// The dust ground units raise on this world (a tint), if any; when each vehicle last
    /// raised some, and where each aircraft's vapour trails last reached.
    pub(super) dust: Option<[f32; 3]>,
    pub(super) last_dust: rustc_hash::FxHashMap<UnitId, f32>,
    pub(super) last_vapour: rustc_hash::FxHashMap<UnitId, Vec3>,
    pub(super) track_sprite: Option<SpriteId>,
    /// The footprint pictures, by the proportions of the sole each draws (see [`FOOT_SHAPES`]).
    pub(super) foot_sprites: Vec<(f32, SpriteId)>,
    /// Where in the cycle of showers the world starts (0..1, from the game seed).
    pub(super) rain_phase: f32,
    /// The next lightning strike (game time), and the bolt on screen: its points and when
    /// it struck.
    pub(super) next_bolt: f32,
    pub(super) bolt: Option<(Vec<Vec3>, f32)>,
    /// How overcast the sky is now (the rain's strength; the renderer dims the sun with it).
    pub(super) overcast: f32,
    /// How wet the ground is (0..1): it soaks within seconds of the rain and takes a minute
    /// or so to dry (`None` until the first frame, so a game starting in a shower starts wet).
    pub(super) wetness: Option<f32>,
    /// How hard it rains now (0..1).
    pub(super) raining: f32,
    /// Thunder still to come: when, the sound, where, how loud.
    pub(super) thunder: Vec<(f32, &'static str, Vec3, f32)>,
}

impl Ambient {
    /// Forgets the tread marks, wakes and lightning (see `Effects::clear_battle`).
    pub(super) fn clear_marks(&mut self) {
        self.tracks.clear();
        self.last_track.clear();
        self.last_step.clear();
        self.last_wake.clear();
        self.wakes.clear();
        self.faded_wakes.clear();
        self.wakes_tick = None;
        self.last_dust.clear();
        self.last_vapour.clear();
        self.bolt = None;
        self.thunder.clear();
    }
}

const MOTES_MAX: usize = 1_800;
/// How far from where the perspective camera looks the weather falls.
const WEATHER_REACH: f32 = 1_500.0;

/// The ground the weather falls on: the view `lo`..`hi`, within [`WEATHER_REACH`] of where a
/// perspective camera looks (`focus`). (Its view reaches the horizon: spread over all of it,
/// the rain fell thin round the eye and filled the particles' room far off, where none showed.)
fn weather_area(focus: Option<Vec2>, lo: Vec2, hi: Vec2) -> (Vec2, Vec2) {
    let Some(focus) = focus else { return (lo, hi) };
    let (near_lo, near_hi) = (focus - Vec2::splat(WEATHER_REACH), focus + Vec2::splat(WEATHER_REACH));
    let (a, b) = (lo.max(near_lo), hi.min(near_hi));
    if a.x < b.x && a.y < b.y { (a, b) } else { (near_lo, near_hi) }
}
/// Seconds from one shower to the next, and how long each rains (ramping in and out).
const SHOWER_EVERY: f32 = 260.0;
const SHOWER_LENGTH: f32 = 110.0;
const SHOWER_RAMP: f32 = 20.0;
/// How long a lightning bolt stays on screen.
const BOLT_TIME: f32 = 0.3;
const TRACKS_MAX: usize = 12_000;
const TRACK_LIFE: f32 = 45.0;
/// Tread mark pieces are laid this fraction of their length apart (they fade in and out at
/// their ends, see `track_pixels`).
const TRACK_OVERLAP: f32 = 0.5;
/// A footprint is this much bigger than the sole it is made by (it presses the ground round it).
const FOOT_PRESS: f32 = 1.12;
/// Seconds between dust puffs behind a moving vehicle; pixels between an aircraft's vapour
/// puffs along its way (each wingtip), so the trails join up.
const DUST_EVERY: f32 = 0.11;
const VAPOUR_SPACING: f32 = 5.0;
/// Seconds between dust puffs behind a hovercraft ashore.
const WAKE_EVERY: f32 = 0.07;

/// The dust ground units raise on a world (a tint for the smoke sprite), if any: sand on
/// deserts, red on Mars-like worlds, ash on lava, snow on ice, grey on moons and slate; none
/// where the ground is green, wet or metal.
fn dust_for_planet(planet: &str) -> Option<[f32; 3]> {
    let p = planet.to_ascii_lowercase();
    if p.contains("desert") {
        Some([1.35, 1.2, 0.95])
    } else if p.contains("red planet") || p.contains("mars") {
        Some([1.3, 0.85, 0.62])
    } else if p.contains("lava") {
        Some([0.62, 0.58, 0.55])
    } else if p.contains("ice") || p.contains("snow") {
        Some([1.8, 1.85, 1.95])
    } else if p.contains("lunar") || p.contains("moon") || p.contains("slate") {
        Some([1.05, 1.07, 1.1])
    } else {
        None
    }
}

/// Tread-mark sprite: a piece of the two grooves a tank's treads press into the ground,
/// running along the texture's v axis. Black at a low opacity, so it darkens the ground in the
/// ground's own colour (as the light does), a little more along the grooves' outer edges, with
/// faint cleats; it fades out towards both ends, so pieces laid half a piece apart
/// (`TRACK_OVERLAP`) add up to an even, unbroken trail.
pub(super) fn track_pixels(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            // Each groove's distance from its middle, 0..1 across it.
            let across = ((u - 0.21).abs() / 0.13).min((u - 0.79).abs() / 0.13);
            let groove = 1.0 - smoothstep(0.8, 1.15, across);
            let rut = 1.0 + 0.3 * smoothstep(0.45, 0.9, across);
            let cleat = 1.0 + 0.1 * (v * TAU * 4.0).cos();
            // The piece's ends fade in and out (sin², summing evenly at half a piece apart).
            let along = (v * PI).sin().powi(2);
            let a = 0.26 * groove * rut * cleat * along;
            out.extend_from_slice(&[0, 0, 0, (a.clamp(0.0, 1.0) * 255.0) as u8]);
        }
    }
    out
}

/// How many texels across a footprint's picture is, and the margin of soft edge round the sole
/// in it (the sole is the rest).
pub(super) const FOOT_TEXELS: u32 = 32;
const FOOT_MARGIN: f32 = 1.5;
/// The proportions of sole (its length against its width) the footprint pictures come in; each
/// foot is drawn with the nearest, from a Flea's near square to a Thud's long boots and a Krogoth's
/// long wide foot.
pub(super) const FOOT_SHAPES: [f32; 9] = [0.7, 0.9, 1.1, 1.4, 1.8, 2.3, 3.0, 3.8, 6.5];

/// A footprint picture's size in texels (width, length) for a sole of proportions `shape`.
pub(super) fn foot_size(shape: f32) -> (u32, u32) {
    let sole = FOOT_TEXELS as f32 - 2.0 * FOOT_MARGIN;
    (FOOT_TEXELS, (sole * shape + 2.0 * FOOT_MARGIN).round() as u32)
}

/// The picture whose sole is nearest `shape` (length against width) among `sprites`, and the
/// world units across a texel when its sole is `width` wide.
pub(super) fn foot_sprite_for(sprites: &[(f32, SpriteId)], shape: f32, width: f32) -> Option<(SpriteId, f32)> {
    let nearest = sprites.iter().min_by(|a, b| (a.0 / shape).ln().abs().total_cmp(&(b.0 / shape).ln().abs()))?;
    Some((nearest.1, width / (FOOT_TEXELS as f32 - 2.0 * FOOT_MARGIN)))
}

/// A walker steps this far for each cell of its footprint's lesser side (world units): a Peewee
/// or a Commander (2 x 2) 22, a Krogoth (3 x 3) 33; farther when its prints are long (see
/// [`Gait::of`]).
const STRIDE_PER_CELL: f32 = 11.0;
/// The most a print may measure either way, for each cell of the walker's footprint that way
/// (world units): twice its footprint. A sole measured bigger is none (a model's body lying on
/// the ground until its script stands it up, a plate the measure did not know), and the walker
/// leaves the prints its footprint gives instead.
const MOST_PRINT_PER_CELL: f32 = 32.0;
/// Two prints of one foot lie at least this many times a print's length apart (from one's heel to
/// the next one's): they never overlap, so the ground is darkened once under each, never over and
/// over towards black.
const PRINT_SPACING: f32 = 1.1;

/// How a walker leaves its prints: the feet that come down in turn, each its pads (in the
/// walker's axes, as `scene::Pad`: across and ahead), and how far it goes between two steps.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Gait {
    pub(super) feet: Vec<Vec<crate::scene::Pad>>,
    pub(super) stride: f32,
    /// Its own feet, as measured on its model (else a pair from its footprint).
    pub(super) measured: bool,
}

impl Gait {
    /// The gait of a walker of `footprint` cells (and its movement class's, `class`, which TA
    /// gives a unit whose definition names none: TA: Enhanced's) on the `feet` measured on its
    /// model (`scene::feet`): its own soles, when it has some and they are of a size it could
    /// stand on (twice its footprint at most, [`MOST_PRINT_PER_CELL`]); else a squarish pair from
    /// its footprint. It steps [`STRIDE_PER_CELL`] for each cell of its footprint, or farther
    /// where its prints are long: a foot comes down again clear of its last print.
    pub(super) fn of(feet: Option<&[crate::scene::Foot]>, footprint: (u32, u32), class: Option<(u32, u32)>) -> Gait {
        use crate::scene::Pad;
        let size = footprint.0.min(footprint.1).max(1) as f32;
        let cells = [footprint.0.max(class.map_or(1, |c| c.0)).max(1), footprint.1.max(class.map_or(1, |c| c.1)).max(1)];
        let fits = |p: &Pad| (0..2).all(|k| p.size[k] > 0.0 && p.size[k] * FOOT_PRESS <= MOST_PRINT_PER_CELL * cells[k] as f32);
        let measured = feet.filter(|feet| !feet.is_empty() && feet.iter().all(|f| !f.pads.is_empty() && f.pads.iter().all(fits)));
        let feet: Vec<Vec<Pad>> = match measured {
            Some(feet) => feet.iter().map(|f| f.pads.clone()).collect(),
            // (Feet not known: a pair of squarish ones, left and right.)
            None => [1.0, -1.0].iter().map(|side| vec![Pad { at: [size * 3.5 * side, 0.0], size: [size * 3.0, size * 4.0] }]).collect(),
        };
        // How long a foot's print is: from the heel of its rearmost pad to the toe of its foremost.
        let length = |f: &[Pad]| {
            let heel = f.iter().map(|p| p.at[1] - p.size[1] * 0.5).fold(f32::MAX, f32::min);
            let toe = f.iter().map(|p| p.at[1] + p.size[1] * 0.5).fold(f32::MIN, f32::max);
            (toe - heel).max(0.0) * FOOT_PRESS
        };
        let longest = feet.iter().map(|f| length(f)).fold(0.0, f32::max);
        // (Each foot comes down every `feet.len()` steps.)
        let stride = (size * STRIDE_PER_CELL).max(PRINT_SPACING * longest / feet.len() as f32);
        Gait { feet, stride, measured: measured.is_some() }
    }
}

/// The footprint (cells) of a unit's movement class, if it has one.
fn class_footprint(sim: &Sim, t: &ta_sim::UnitType) -> Option<(u32, u32)> {
    t.move_class.and_then(|c| sim.data.move_classes.get(c)).map(|c| (c.footprint_x, c.footprint_z))
}

/// Footprint sprite `w` x `h` texels: the sole of a walker's foot pressed into the ground, a
/// rectangle with rounded corners (the toe, towards v = 1 where the decal's rotation faces, a
/// little rounder than the heel), pressed deepest along its rim and a little less in the middle,
/// crossed by a few faint lines of tread when it is long; black at a low opacity, so it darkens
/// the ground in the ground's own colour, with a soft edge a texel wide.
pub(super) fn foot_pixels(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let (half_w, half_h) = (w as f32 * 0.5 - FOOT_MARGIN, h as f32 * 0.5 - FOOT_MARGIN);
    let width = 2.0 * half_w;
    let shape = half_h / half_w;
    // (A boot's tread: lines across it, as far apart as a fifth of the sole's width, on a sole
    // clearly longer than it is wide.)
    let lines = smoothstep(1.2, 1.8, shape);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5 - w as f32 * 0.5, y as f32 + 0.5 - h as f32 * 0.5);
            let toe = ((py + half_h) / (2.0 * half_h)).clamp(0.0, 1.0);
            let radius = width * (0.12 + 0.1 * toe);
            let (qx, qy) = (px.abs() - (half_w - radius), py.abs() - (half_h - radius));
            // How far inside the sole's edge (negative: outside).
            let inside = -((qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius);
            let body = smoothstep(-0.9, 1.6, inside);
            let rim = 1.0 - smoothstep(0.0, 3.5, inside);
            let along = (py + half_h) / (width * 0.2);
            let line = (-((along - along.round()) / 0.16).powi(2)).exp() * lines * smoothstep(1.5, 3.0, inside);
            let a = body * (0.27 + 0.11 * rim + 0.05 * line);
            out.extend_from_slice(&[0, 0, 0, (a.clamp(0.0, 1.0) * 255.0) as u8]);
        }
    }
    out
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Lava worlds' embers (see `Effects::ember_weather`): spots looked at a second per million square
/// pixels of view at High effects quality, and the most a second however much is in view. Each
/// one on the lava gives an ember (now and then one the hot air carries higher,
/// [`RISING_SHARE`]), and a few burst a bubble ([`BUBBLE_SHARE`]); of those on the land, a few give
/// an ember ([`LAND_EMBER_SHARE`]).
const EMBER_SPOTS: f32 = 360.0;
const EMBER_SPOTS_MOST: f32 = 2700.0;
const RISING_SHARE: f32 = 0.04;
const BUBBLE_SHARE: f32 = 0.025;
const LAND_EMBER_SHARE: f32 = 0.02;
/// No bubble's glow within this far of the eye (world units): under a low camera it swelled into
/// a big soft disc across the view.
const FLASH_CLEAR: f32 = 250.0;

/// Spots a second to look at for embers over `area` million square pixels of view, at the
/// settings' weather density `weather` (the effects quality's: Low 0.4 to Ultra 1.4).
fn ember_spots(area: f32, weather: f32) -> f32 {
    (area.max(0.0) * EMBER_SPOTS).min(EMBER_SPOTS_MOST) * weather.max(0.0)
}

/// An ember lifting off the lava at `at` (just over its surface): small and dim, it rises a few
/// pixels on the hot air, drifts a little and cools within a second or two, a speck however near
/// the eye it passes.
fn lava_ember(at: [f32; 3]) -> Burst {
    Burst::new(Kind::Ember, at, 1)
        .radius(24.0)
        .vertical(0.1)
        .toward([0.0, 1.0, 0.0], 0.6)
        .speed(4.0, 16.0)
        .life(1.0, 2.4)
        .size(2.6, 0.9)
        .gravity(-9.0)
        .drag(1.5)
        .intensity(4.5)
        .collide(false)
        .speck()
}

/// Now and then an ember the hot air carries higher (see [`RISING_SHARE`]): up a few dozen pixels
/// over a few seconds, cooling as it goes.
fn rising_ember(at: [f32; 3]) -> Burst {
    Burst::new(Kind::Ember, at, 1)
        .radius(24.0)
        .vertical(0.1)
        .toward([0.0, 1.0, 0.0], 0.4)
        .speed(8.0, 20.0)
        .life(2.0, 3.4)
        .size(2.6, 0.8)
        .gravity(-16.0)
        .drag(1.2)
        .intensity(4.5)
        .collide(false)
        .speck()
}

/// A bubble of lava bursting at `at`: `droplets` molten drops tossed a little way up and falling
/// back in, and with `flash` a small glow where it burst.
fn lava_bubble(at: [f32; 3], droplets: u32, flash: bool) -> Vec<Burst> {
    let mut out = vec![
        Burst::new(Kind::Ember, at, droplets).radius(4.0).toward([0.0, 1.0, 0.0], 0.45).speed(25.0, 75.0).life(0.4, 0.8).size(2.4, 0.8).gravity(260.0).drag(0.3).intensity(4.5).speck(),
    ];
    if flash {
        out.push(Burst::new(Kind::Glow, at, 1).size(11.0, 19.0).life(0.2, 0.35).color([1.0, 0.45, 0.12]).intensity(1.8));
    }
    out
}

/// The highest a particle of burst `b` can climb over where it is born (world units): straight up
/// at its fastest, carried by its lift (a negative gravity) against its drag for its longest life.
#[cfg(test)]
fn highest_climb(b: &Burst) -> f32 {
    let (v0, lift, drag, life) = (b.speed[1], -b.gravity, b.drag, b.life[1]);
    let dt = 1.0 / 240.0;
    let (mut v, mut y, mut top) = (v0, 0.0f32, 0.0f32);
    let mut t = 0.0;
    while t < life {
        v = (v + lift * dt) * (-drag * dt).exp();
        y += v * dt;
        top = top.max(y);
        t += dt;
    }
    top + b.radius * b.vertical
}

impl Effects {
    /// Picks the weather for the map's planet and a prevailing wind direction.
    pub fn set_world(&mut self, planet: &str, seed: u64) {
        self.ambient.weather = Weather::for_planet(planet);
        self.ambient.dust = dust_for_planet(planet);
        self.lava_world = self.ambient.weather == Weather::Embers;
        let a = (seed % 6283) as f32 / 1000.0;
        self.ambient.wind_dir = Vec2::new(a.cos(), a.sin());
        self.ambient.rain_phase = (seed / 7 % 1000) as f32 / 1000.0;
        // Captures: TA_RAIN starts the game in the middle of a shower (`TA_RAIN=0`: in dry
        // weather, a couple of minutes before the first).
        if let Some(v) = std::env::var_os("TA_RAIN") {
            self.ambient.rain_phase = if v == "0" { 0.0 } else { 0.7 };
        }
    }

    /// A scene of the menu's film from game time `now` on (its scenes start at any time in its
    /// battle): with `rain`, in the middle of a shower, on any world; else in the world's own
    /// weather (its snow, its embers), and on a world of showers between two of them, the next
    /// minutes off.
    pub fn scene_weather(&mut self, rain: bool, now: f32) {
        let own = *self.ambient.own_weather.get_or_insert(self.ambient.weather);
        self.ambient.weather = if rain { Weather::Rain } else { own };
        let at = if rain { SHOWER_EVERY - SHOWER_LENGTH * 0.5 } else { 0.0 };
        self.ambient.rain_phase = ((at - now) / SHOWER_EVERY).rem_euclid(1.0);
    }

    /// The weather as it is (its kind, where its showers are, how wet the ground is), to be put
    /// back with [`Effects::set_weather_state`] (photo mode's).
    pub fn weather_state(&self) -> WeatherState {
        WeatherState { weather: self.ambient.weather, rain_phase: self.ambient.rain_phase, wetness: self.ambient.wetness }
    }

    pub fn set_weather_state(&mut self, state: WeatherState) {
        self.ambient.weather = state.weather;
        self.ambient.rain_phase = state.rain_phase;
        self.ambient.wetness = state.wetness;
    }

    /// Photo mode: no weather at all (no rain, snow, embers or dust), or in the middle of a
    /// shower on any world.
    pub fn photo_weather(&mut self, rain: bool, now: f32) {
        if rain {
            self.scene_weather(true, now);
        } else {
            self.ambient.weather = Weather::None;
        }
    }

    /// Photo mode's shower over a battle held still (its particles hanging where they are): the
    /// air over the view full of drops at once, as a shower keeps it, and the ground soaked.
    pub fn photo_rain(&mut self, sim: &Sim, now: f32) {
        let strength = self.rain_strength(now) * self.detail.weather.min(1.0);
        self.ambient.wetness = Some(self.ambient.wetness.unwrap_or(0.0).max(strength));
        let Some((lo, hi)) = self.view.filter(|_| strength > 0.01) else { return };
        let (lo, hi) = weather_area(self.focus, lo, hi);
        let (lo, hi) = (lo - Vec2::splat(100.0), hi + Vec2::splat(100.0));
        let (centre, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
        let area = (half.x * half.y * 4.0).max(0.0) / 1.0e6;
        let ground = Self::ground(sim, centre.x, centre.y).max(sim.world.sea_level.to_f32());
        let wind = self.wind(0.25);
        let fall = [wind.x * 0.12, -1.0, wind.y * 0.12];
        // (The rain's rate times the time its drops fall, in layers from the ground up to where
        // they start.)
        const LAYERS: u32 = 10;
        let drops = ((area * 2400.0 * strength * 0.68) as u32).min(20_000) / LAYERS;
        for i in 0..LAYERS {
            let height = ground + 380.0 * (i as f32 + 0.5) / LAYERS as f32;
            self.burst(Burst::new(Kind::Rain, [centre.x, height, centre.y], drops).radius(half.length()).vertical(0.05).toward(fall, 0.03));
        }
    }

    /// Staged scenes: in the middle of a shower from the start, on any world (`on`), or in dry
    /// weather, a couple of minutes before the first.
    pub fn set_rain(&mut self, on: bool) {
        if on {
            self.ambient.weather = Weather::Rain;
        }
        self.ambient.rain_phase = if on { 0.7 } else { 0.0 };
    }

    /// Measures the hulls of the ships and hovercraft for their wakes (once the scene has their
    /// models' pieces).
    pub fn measure_hulls(&mut self, sim: &Sim, scene: &crate::scene::Scene) {
        self.ambient.hulls.clear();
        for t in sim.types.types.iter().filter(|t| matches!(t.locomotion, Locomotion::Naval | Locomotion::Hover)) {
            if let Some(hull) = scene.piece_bounds(t.id).and_then(|b| Hull::of(t, &b)) {
                self.ambient.hulls.insert(t.id, hull);
            }
        }
    }

    /// Takes the feet the walkers' models stand on (once the scene has loaded them), for the
    /// prints they leave: each walking type's gait.
    pub fn measure_feet(&mut self, sim: &Sim, scene: &crate::scene::Scene) {
        self.ambient.gaits.clear();
        for t in sim.types.types.iter().filter(|t| t.locomotion == Locomotion::Ground && crate::scene::walks(t)) {
            let feet = scene.feet(t.id);
            let gait = Gait::of(feet.as_deref().map(|f| f.as_slice()), t.footprint, class_footprint(sim, t));
            if feet.as_ref().is_some_and(|f| !f.is_empty()) && !gait.measured {
                log::debug!("{}: its feet as measured ({:?}) are more than it could stand on; prints from its footprint", t.name, feet.as_deref().map(|f| f.iter().map(|f| f.pads.iter().map(|p| p.size).collect::<Vec<_>>()).collect::<Vec<_>>()));
            }
            self.ambient.gaits.insert(t.id, std::sync::Arc::new(gait));
        }
    }

    /// The wind as the effects show it (eased, see `update_ambient`): its direction at its
    /// strength, speed / 20, at most 1.5 (a gale) and never under `least`.
    pub(super) fn wind(&self, least: f32) -> Vec2 {
        self.ambient.wind_dir * (self.ambient.wind_speed / 20.0).clamp(least, 1.5)
    }

    /// Weather, wakes and tracks for this frame (`alpha` of the way through the tick).
    pub(super) fn update_ambient(&mut self, sim: &Sim, now: f32, dt: f32, alpha: f32) {
        // The wind the wind generators face (it turns every few seconds): eased in, except
        // at the start; its speed too (taken all at once, every tree in view lurched when it
        // changed, and while the menu's battle was fast-forwarded, every frame).
        let a = sim.wind_heading as f32 * std::f32::consts::TAU / 65536.0;
        let target = Vec2::new(a.sin(), a.cos());
        let k = (dt * 0.5).min(1.0);
        let eased = self.ambient.wind_dir + (target - self.ambient.wind_dir) * k;
        let speed = sim.wind.to_f32();
        (self.ambient.wind_dir, self.ambient.wind_speed) =
            if self.ambient.wind_synced { (eased.normalize_or(target), self.ambient.wind_speed + (speed - self.ambient.wind_speed) * k) } else { (target, speed) };
        self.ambient.wind_synced = true;
        self.follow_wakes(sim, now);
        let Some((lo, hi)) = self.view else {
            self.wake_points.clear();
            self.wake_trails.clear();
            return;
        };
        self.update_weather(sim, now, dt, lo, hi);
        self.update_wakes_and_tracks(sim, now, dt, lo, hi);
        self.wakes_in_view(now, alpha, lo, hi);
    }

    /// Follows the ships and hovercraft on the water (all of them, so that one coming into view
    /// brings its wake), once a tick: a point on each one's way as it goes, its wake forgetting
    /// what is older than a wake lasts. Wakes no longer followed fade where they lie.
    fn follow_wakes(&mut self, sim: &Sim, now: f32) {
        let sea = sim.world.sea_level.to_f32();
        let tick = sim.tick;
        let viewer = self.viewer;
        let a = &mut self.ambient;
        if a.wakes_tick == Some(tick) {
            return;
        }
        a.wakes_tick = Some(tick);
        for w in a.wakes.values_mut() {
            w.live = false;
        }
        for u in sim.units() {
            let t = sim.unit_type(u);
            // (Submarines go under the water, and make none.)
            let hover = match t.locomotion {
                Locomotion::Hover => true,
                Locomotion::Naval if t.def.water_line <= 0.0 => false,
                _ => continue,
            };
            if u.dead || u.carried_by.is_some() || !u.is_complete() || viewer.is_some_and(|pl| sim.seen_by(u, pl) != ta_sim::Seen::Visible) {
                continue;
            }
            let p = Vec3::from_array(u.pos.to_f32());
            if Self::ground(sim, p.x, p.z) > sea - 1.0 {
                continue;
            }
            let (at, heading) = (Vec2::new(p.x, p.z), u.heading as f32 * TAU / 65536.0);
            let hull = a.hulls.get(&u.type_id).copied().unwrap_or_else(|| Hull::guess(t));
            let fresh = || Wake { points: VecDeque::new(), ticks: [(at, heading); 2], tick, step: 1, hull, hover, live: true };
            let w = a.wakes.entry(u.id).or_insert_with(fresh);
            if w.tick != tick {
                // (Several ticks may go by in a frame. Placed, carried off, or the game sought: a
                // new way from where it is.)
                let step = tick.saturating_sub(w.tick);
                let jump = tick < w.tick || step > 30 || w.ticks[1].0.distance(at) > 12.0 * step as f32;
                if jump && !w.points.is_empty() {
                    let old = std::mem::replace(w, fresh());
                    a.faded_wakes.push(Wake { live: false, ..old });
                }
                (w.ticks, w.step) = if jump { ([(at, heading); 2], 1) } else { ([w.ticks[1], (at, heading)], step) };
                w.tick = tick;
            }
            w.live = true;
            let speed = w.ticks[0].0.distance(at) / (w.step as f32 * crate::game::TICK_SECONDS);
            let due = match w.points.back() {
                None => speed > 1.0,
                Some(&(q, h, ..)) => q.distance(at) >= wake_spacing(&w.hull, speed) || (q.distance(at) > 1.0 && angle_between(h, heading).abs() > WAKE_TURN),
            };
            if due {
                let travelled = w.points.back().map_or(0.0, |p| p.4 + p.0.distance(at));
                w.points.push_back((at, heading, now, speed, travelled));
            }
        }
        // What is no longer followed fades where it lies.
        let faded = &mut a.faded_wakes;
        a.wakes.retain(|_, w| {
            if !w.live && !w.points.is_empty() {
                faded.push(Wake { points: std::mem::take(&mut w.points), ..*w });
            }
            w.live
        });
        for w in a.wakes.values_mut().chain(a.faded_wakes.iter_mut()) {
            while w.points.front().is_some_and(|p| now - p.2 > ta_render::wakes::WAKE_LIFE) || w.points.len() > WAKE_POINTS {
                w.points.pop_front();
            }
        }
        a.faded_wakes.retain(|w| w.points.len() >= 2);
    }

    /// This frame's wakes in view (the ground `lo`..`hi`) for the renderer: each one's points,
    /// the newest first, from where its hull is drawn now (`alpha` of the way through the tick).
    fn wakes_in_view(&mut self, now: f32, alpha: f32, lo: Vec2, hi: Vec2) {
        self.wake_points.clear();
        self.wake_trails.clear();
        let a = &self.ambient;
        for w in a.wakes.values().chain(&a.faded_wakes) {
            if !w.overlaps(now, lo, hi) {
                continue;
            }
            let first = self.wake_points.len() as u32;
            // (The newest point may lie ahead of where the hull is drawn, between the ticks: the
            // way goes on from the one before.)
            let mut way = w.points.iter().rev().peekable();
            if w.live {
                // (Drawn between the tick before the last and the last.)
                let [(p0, h0), (p1, h1)] = w.ticks;
                let back = (1.0 - alpha) / w.step as f32;
                let at = p1 + (p0 - p1) * back;
                let speed = p0.distance(p1) / (w.step as f32 * crate::game::TICK_SECONDS);
                let heading = h1 - angle_between(h0, h1) * back;
                way.next_if(|p| p.2 >= now - crate::game::TICK_SECONDS && (p.0 - at).dot(Vec2::new(heading.sin(), heading.cos())) > 0.0);
                let travelled = way.peek().map_or(0.0, |p| p.4 + p.0.distance(at));
                self.wake_points.push(ta_render::WakePoint { pos: at.to_array(), heading, age: 0.0, speed, travelled });
            }
            self.wake_points.extend(way.map(|&(p, heading, time, speed, travelled)| ta_render::WakePoint { pos: p.to_array(), heading, age: now - time, speed, travelled }));
            let count = self.wake_points.len() as u32 - first;
            if count < 2 {
                self.wake_points.truncate(first as usize);
                continue;
            }
            self.wake_trails.push(ta_render::WakeTrail { first, count, bow: w.hull.bow, stern: w.hull.stern, beam: w.hull.beam, hover: w.hover });
        }
    }

    fn ground(sim: &Sim, x: f32, z: f32) -> f32 {
        crate::scene::drawn_ground(&sim.world, x, z)
    }

    fn update_weather(&mut self, sim: &Sim, now: f32, dt: f32, lo: Vec2, hi: Vec2) {
        let weather = self.ambient.weather;
        if weather == Weather::None {
            return;
        }
        let (lo, hi) = weather_area(self.focus, lo, hi);
        if weather == Weather::Embers {
            self.ember_weather(sim, dt, lo, hi);
            return;
        }
        if weather == Weather::Rain {
            self.rain_weather(sim, now, dt, lo, hi);
            return;
        }
        let margin = 200.0;
        let (mut lo, mut hi) = (lo - Vec2::splat(margin), hi + Vec2::splat(margin));
        // (The classic camera shows the void beyond the map: nothing falls through it.)
        if !self.modern {
            let (w, h) = sim.world.size_px();
            lo = lo.max(Vec2::ZERO);
            hi = hi.min(Vec2::new(w.to_f32(), h.to_f32())).max(lo);
        }
        // Keep a steady density over what is on screen.
        let area = ((hi.x - lo.x) * (hi.y - lo.y)).max(0.0) / 1.0e6;
        let want = ((area * weather.density() * self.detail.weather) as usize).min(MOTES_MAX);
        let wind = self.wind(0.25);
        let sea = sim.world.sea_level.to_f32();
        let mut spawn = want.saturating_sub(self.ambient.motes.len()).min(want / 20 + 4);
        while spawn > 0 {
            spawn -= 1;
            let (x, z) = (lo.x + self.rand() * (hi.x - lo.x), lo.y + self.rand() * (hi.y - lo.y));
            let g = Self::ground(sim, x, z).max(sea);
            let (r1, r2, r3) = (self.rand(), self.rand(), self.rand());
            let w3 = Vec3::new(wind.x, 0.0, wind.y);
            let mote = match weather {
                Weather::Snow => Mote { pos: Vec3::new(x, g + 180.0 + r1 * 260.0, z), vel: w3 * 28.0 + Vec3::new(0.0, -38.0 - r2 * 16.0, 0.0), born: now, life: 30.0, size: 4.0 + r3 * 3.5, phase: r1 * TAU },
                Weather::Dust => Mote { pos: Vec3::new(x, g + 8.0 + r1 * 30.0, z), vel: w3 * (30.0 + r2 * 25.0) + Vec3::new(0.0, 2.0, 0.0), born: now, life: 6.0 + r3 * 4.0, size: 70.0 + r2 * 90.0, phase: r1 * TAU },
                Weather::Spores => Mote { pos: Vec3::new(x, g + 10.0 + r1 * 110.0, z), vel: w3 * 8.0 + Vec3::new(0.0, 3.0 + r2 * 6.0, 0.0), born: now, life: 6.0 + r3 * 5.0, size: 2.5 + r3 * 1.5, phase: r1 * TAU },
                Weather::Glitter => Mote { pos: Vec3::new(x, g + 4.0 + r1 * 55.0, z), vel: w3 * 3.0, born: now, life: 1.5 + r3 * 2.0, size: 2.0 + r2 * 1.5, phase: r1 * TAU },
                Weather::None | Weather::Embers | Weather::Rain => unreachable!(),
            };
            self.ambient.motes.push(mote);
        }
        // Move; drop the expired, the grounded and those far out of view.
        let far = margin * 2.0;
        self.ambient.motes.retain(|m| {
            now - m.born < m.life && m.pos.x > lo.x - far && m.pos.x < hi.x + far && m.pos.z > lo.y - far && m.pos.z < hi.y + far
        });
        let mut i = 0;
        while i < self.ambient.motes.len() {
            let m = &mut self.ambient.motes[i];
            let age = now - m.born;
            let sway = Vec3::new((age * 1.7 + m.phase).sin(), 0.0, (age * 1.3 + m.phase).cos());
            m.pos += (m.vel + sway * if weather == Weather::Snow { 9.0 } else { 4.0 }) * dt;
            if weather == Weather::Snow && m.pos.y <= Self::ground(sim, m.pos.x, m.pos.z).max(sea) {
                self.ambient.motes.swap_remove(i);
                continue;
            }
            i += 1;
        }
        for m in &self.ambient.motes {
            let age = now - m.born;
            let t = (age / m.life).clamp(0.0, 1.0);
            let fade_in = (age * 2.0).min(1.0);
            let inst = SpriteInstance::new(m.pos.to_array(), self.glow).scale(m.size / 64.0);
            match weather {
                // Snowflakes: white motes, lit like smoke and as bright as the sunlit snow (a
                // touch of the sky's blue), so they read as snow against it rather than as dust.
                // Passing close to the eye a flake is kept to a speck a few pixels across (at
                // most about a fifth of a degree), a little fainter: drawn at its own size it grew
                // into a big soft disc there, a smudge on the lens.
                Weather::Snow => {
                    let near = self.eye.map_or(1.0, |eye| (eye.distance(m.pos) * 0.0035 / m.size).min(1.0));
                    self.smoke_sprites.push(SpriteInstance::new(m.pos.to_array(), self.flake).scale(m.size * near / 16.0).tint([1.0, 1.03, 1.1]).brightness(1.65).alpha(0.85 * fade_in * (0.6 + 0.4 * near)));
                }
                Weather::Dust => {
                    let a = 0.07 * fade_in * (1.0 - t).min(t * 4.0).min(1.0);
                    self.smoke_sprites.push(SpriteInstance::new(m.pos.to_array(), self.smoke[(m.phase * 4.0 / TAU) as usize % 4]).scale(m.size / 64.0).tint([1.35, 1.12, 0.8]).alpha(a).rotation(m.phase + age * 0.1));
                }
                Weather::Spores => self.additive.push(inst.tint([0.55, 1.0, 0.35]).brightness(1.8 * (1.0 - t)).alpha(fade_in)),
                Weather::Glitter => {
                    let twinkle = (age * 7.0 + m.phase).sin().max(0.0).powi(4);
                    self.additive.push(inst.tint([0.75, 0.9, 1.0]).brightness(6.0 * twinkle));
                }
                Weather::None | Weather::Embers | Weather::Rain => {}
            }
        }
    }

    /// How hard it rains at game time `now` (0..1): a shower of a couple of minutes every few
    /// minutes, from the game clock, so a replay sees the same weather.
    pub(super) fn rain_strength(&self, now: f32) -> f32 {
        if self.ambient.weather != Weather::Rain {
            return 0.0;
        }
        let t = (now + self.ambient.rain_phase * SHOWER_EVERY).rem_euclid(SHOWER_EVERY);
        let start = SHOWER_EVERY - SHOWER_LENGTH;
        ((t - start) / SHOWER_RAMP).min((SHOWER_EVERY - t) / SHOWER_RAMP).clamp(0.0, 1.0)
    }

    /// Rain over the view, `dt` seconds' worth of GPU particles: streaks falling on the wind,
    /// splashes on the water, and while it pours, lightning.
    fn rain_weather(&mut self, sim: &Sim, now: f32, dt: f32, lo: Vec2, hi: Vec2) {
        self.draw_bolt(now);
        // The thunder that follows a strike.
        let mut due = Vec::new();
        self.ambient.thunder.retain(|&(at, name, pos, volume)| {
            let now_due = now >= at;
            if now_due {
                due.push((name, pos, volume));
            }
            !now_due
        });
        self.sounds.extend(due);
        let strength = self.rain_strength(now) * self.detail.weather.min(1.0);
        let wet = self.ambient.wetness.unwrap_or(strength);
        let rate = if strength > wet { 1.0 / 10.0 } else { 1.0 / 70.0 };
        self.ambient.wetness = Some(wet + (strength - wet) * (1.0 - (-dt.max(0.0) * rate).exp()));
        self.ambient.raining = strength;
        // A strike lights up the land for an instant: the clouds part, as far as the light goes.
        let since = self.ambient.bolt.as_ref().map_or(f32::MAX, |(_, t)| now - t);
        let flash = if self.reduced_flashes { 0.0 } else { (1.0 - since / 0.2).clamp(0.0, 1.0) };
        self.ambient.overcast = strength * (1.0 - 0.85 * flash);
        if strength <= 0.01 || dt <= 0.0 {
            return;
        }
        let (lo, hi) = (lo - Vec2::splat(100.0), hi + Vec2::splat(100.0));
        let (centre, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
        let area = (half.x * half.y * 4.0).max(0.0) / 1.0e6;
        let sea = sim.world.sea_level.to_f32();
        let wind = self.wind(0.25);
        // One flat burst high over the middle of the view, as wide as the view.
        let drops = ((area * 2400.0 * strength * dt + self.rand()) as u32).min(5000);
        if drops > 0 {
            let top = Self::ground(sim, centre.x, centre.y).max(sea) + 380.0;
            let fall = [wind.x * 0.12, -1.0, wind.y * 0.12];
            self.burst(Burst::new(Kind::Rain, [centre.x, top, centre.y], drops).radius(half.length()).vertical(0.1).toward(fall, 0.03));
        }
        // Rings of spray where drops hit the sea, and little splashes on the land.
        let splashes = ((area * 70.0 * strength * dt + self.rand()) as u32).min(40);
        for _ in 0..splashes {
            let (x, z) = (lo.x + self.rand() * (hi.x - lo.x), lo.y + self.rand() * (hi.y - lo.y));
            let ground = Self::ground(sim, x, z);
            if ground < sea - 2.0 {
                self.burst(Burst::new(Kind::Spray, [x, sea + 0.5, z], 4).radius(1.5).toward([0.0, 1.0, 0.0], 0.6).speed(20.0, 45.0).life(0.25, 0.45).size(1.5, 0.8).intensity(0.35));
            } else {
                self.burst(Burst::new(Kind::Spray, [x, ground + 0.4, z], 3).radius(1.0).toward([0.0, 1.0, 0.0], 0.8).speed(12.0, 30.0).life(0.16, 0.3).size(1.2, 0.6).intensity(0.3));
            }
        }
        // Lightning while it pours: a jagged bolt from the clouds, a flash over the land.
        if strength > 0.6 && now >= self.ambient.next_bolt {
            self.ambient.next_bolt = now + 7.0 + self.rand() * 16.0;
            let strike = Vec2::new(lo.x + self.rand() * (hi.x - lo.x), lo.y + self.rand() * (hi.y - lo.y));
            let ground = Self::ground(sim, strike.x, strike.y).max(sea);
            let foot = Vec3::new(strike.x, ground, strike.y);
            let top = foot + Vec3::new(self.rand() * 160.0 - 80.0, 900.0, self.rand() * 160.0 - 80.0);
            let mut points = vec![top];
            for i in 1..10 {
                let t = i as f32 / 10.0;
                let jitter = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * 70.0 * (1.0 - t * 0.6);
                points.push(top.lerp(foot, t) + jitter);
            }
            points.push(foot);
            self.ambient.bolt = Some((points, now));
            // The crack of the strike, then the rumble rolling in.
            self.sounds.push(("lghthvy1", foot, 0.7));
            let rumble = now + 0.6 + self.rand() * 0.9;
            self.ambient.thunder.push((rumble, "xplolrg3", foot, 0.55));
            self.light(foot + Vec3::Y * 110.0, now, 0.4, 700.0, [7.0, 7.5, 9.0]);
            self.burst(Burst::new(Kind::Glow, (foot + Vec3::Y * 4.0).to_array(), 1).size(60.0, 90.0).life(0.2, 0.3).color([0.7, 0.8, 1.0]).intensity(4.0));
            self.burst(Burst::new(Kind::Spark, (foot + Vec3::Y * 3.0).to_array(), 40).toward([0.0, 1.0, 0.0], 1.3).speed(60.0, 220.0).life(0.2, 0.5).size(1.8, 0.8).color([0.75, 0.85, 1.0]).intensity(8.0).cold());
        }
    }

    /// The lightning bolt on screen, flickering, for the moment it lasts.
    fn draw_bolt(&mut self, now: f32) {
        if self.reduced_flashes { return; }
        let Some((points, start)) = &self.ambient.bolt else { return };
        let age = now - start;
        if !(0.0..BOLT_TIME).contains(&age) {
            self.ambient.bolt = None;
            return;
        }
        // On, off, on: a bolt flickers as it strikes again down the same channel.
        let lit = age < 0.06 || (0.1..0.2).contains(&age) || age > 0.24;
        if !lit {
            return;
        }
        let fade = 1.0 - age / BOLT_TIME;
        let segments: Vec<(Vec3, Vec3)> = points.windows(2).map(|w| (w[0], w[1])).collect();
        for (a, b) in segments {
            // A white-hot core in a wide blue glow.
            self.beams.push(BeamInstance::new(a.to_array(), b.to_array(), 20.0, [0.9 * fade, 1.1 * fade, 1.8 * fade], 1.0));
            self.beams.push(BeamInstance::new(a.to_array(), b.to_array(), 6.0, [6.0 * fade, 6.5 * fade, 8.0 * fade], 1.0));
        }
    }

    /// Lava worlds, `dt` seconds' worth of GPU particles: embers lifting off the lava, small and
    /// dim, cooling within a couple of dozen pixels of it (the odd one carried higher); now and
    /// then a bubble that bursts with a few droplets and a little glow; and once in a while an
    /// ember off the scorched ground. (They rose by the thousand, a hundred pixels over the melt,
    /// and swelled into big soft discs past a low camera: a glittering curtain over every lava
    /// lake, and the view cluttered.)
    fn ember_weather(&mut self, sim: &Sim, dt: f32, lo: Vec2, hi: Vec2) {
        let margin = 150.0;
        let (lo, hi) = (lo - Vec2::splat(margin), hi + Vec2::splat(margin));
        let area = ((hi.x - lo.x) * (hi.y - lo.y)).max(0.0) / 1.0e6;
        let sea = sim.world.sea_level.to_f32();
        // (A long frame's spots are not all made up for at once.)
        let tries = (ember_spots(area, self.detail.weather) * dt.clamp(0.0, 0.1) + self.rand()).floor() as u32;
        for _ in 0..tries {
            let (x, z) = (lo.x + self.rand() * (hi.x - lo.x), lo.y + self.rand() * (hi.y - lo.y));
            let g = Self::ground(sim, x, z);
            if g >= sea {
                if self.rand() < LAND_EMBER_SHARE {
                    self.burst(lava_ember([x, g + 1.5, z]).life(0.8, 1.6).intensity(2.6));
                }
                continue;
            }
            let at = [x, sea + 1.5, z];
            if self.rand() < BUBBLE_SHARE {
                // A bubble bursts: a few droplets tossed up fall back into the lava, and (not
                // right under a low camera, where it would fill the view) a little glow.
                let droplets = 3 + (self.rand() * 4.0) as u32;
                let flash = self.eye.is_none_or(|eye| eye.distance(Vec3::from_array(at)) > FLASH_CLEAR);
                for b in lava_bubble(at, droplets, flash) {
                    self.burst(b);
                }
            }
            let rising = self.rand() < RISING_SHARE;
            self.burst(if rising { rising_ember(at) } else { lava_ember(at) });
        }
    }

    fn update_wakes_and_tracks(&mut self, sim: &Sim, now: f32, dt: f32, lo: Vec2, hi: Vec2) {
        let sea = sim.world.sea_level.to_f32();
        let margin = 120.0;
        let mut wakes = Vec::new();
        let mut new_tracks = Vec::new();
        for u in sim.units() {
            let p = Vec3::from_array(u.pos.to_f32());
            if p.x < lo.x - margin || p.x > hi.x + margin || p.z < lo.y - margin || p.z > hi.y + margin {
                continue;
            }
            if u.carried_by.is_some() || u.dead {
                continue;
            }
            if self.viewer.is_some_and(|pl| sim.seen_by(u, pl) != ta_sim::Seen::Visible) {
                continue;
            }
            let t = sim.unit_type(u);
            // Paralyzed units crackle: little arcs flash over the hull and throw blue sparks.
            if u.paralyzed_until > sim.tick {
                let r = t.radius.to_f32();
                for _ in 0..(dt * 14.0 + self.rand()).floor() as u32 {
                    let at = p + Vec3::new((self.rand() - 0.5) * r * 1.6, 4.0 + self.rand() * t.model.height.to_f32().max(8.0), (self.rand() - 0.5) * r * 1.6);
                    let (blue, flash) = ([0.45, 0.65, 1.0], 7.0 + self.rand() * 6.0);
                    self.burst(Burst::new(Kind::Glow, at.to_array(), 1).size(flash, 10.0).life(0.05, 0.1).color(blue).intensity(5.0));
                    self.burst(Burst::new(Kind::Spark, at.to_array(), 4).radius(2.0).spread(PI).speed(50.0, 160.0).life(0.06, 0.2).size(1.8, 0.6).color(blue).intensity(9.0).gravity(0.0).drag(3.0).collide(false).cold());
                }
            }
            if u.speed <= Fx::from_ratio(1, 4) {
                continue;
            }
            match t.locomotion {
                // (On the water their wakes are drawn on it: see `follow_wakes`.)
                Locomotion::Hover => {
                    let on_water = Self::ground(sim, p.x, p.z) < sea - 1.0;
                    let last = self.ambient.last_wake.get(&u.id).copied().unwrap_or(0.0);
                    if now - last >= WAKE_EVERY && !on_water {
                        self.ambient.last_wake.insert(u.id, now);
                        let heading = u.heading as f32 * std::f32::consts::TAU / 65536.0;
                        let fwd = Vec3::new(heading.sin(), 0.0, heading.cos());
                        wakes.push((p, fwd, t.radius.to_f32(), u.speed.to_f32()));
                    }
                }
                Locomotion::Ground => {
                    // Walkers (kbots, the Commander) leave footprints, vehicles treads (see
                    // `scene::walks`).
                    let walker = crate::scene::walks(t);
                    let tracked = !walker && matches!(t.def.tedclass.as_str(), "TANK" | "CNSTR");
                    let Some(tread) = self.ambient.track_sprite else { continue };
                    if !self.detail.tracks || p.y < sea || !(tracked || walker) {
                        continue;
                    }
                    let size = t.footprint.0.min(t.footprint.1) as f32;
                    let fast = u.speed.to_f32() > t.max_speed.to_f32() * 0.5;
                    if tracked {
                        // Pieces (`track_pixels`: 32 wide, 16 long) laid at even steps along the
                        // way the tank went since the last one, however far it went in the frame,
                        // so the trail runs on without gaps at any frame rate or game speed.
                        let width = size * 14.0;
                        let step = width * 16.0 / 32.0 * TRACK_OVERLAP;
                        match self.ambient.last_track.get(&u.id) {
                            Some(&q) if q.distance_squared(p) < step * step => {}
                            // (Carried off or warped: a new trail.)
                            Some(&q) if q.distance_squared(p) > (step * 8.0).powi(2) => {
                                self.ambient.last_track.insert(u.id, p);
                            }
                            Some(&q) => {
                                let d = p - q;
                                let way = d.normalize_or_zero();
                                let steps = (d.length() / step) as usize;
                                for k in 1..=steps {
                                    let at = q + way * (step * k as f32);
                                    let ground = Self::ground(sim, at.x, at.z);
                                    // (A little each way in depth, so the trail is not too even.)
                                    let depth = 0.85 + 0.3 * self.rand();
                                    new_tracks.push(Track { pos: Vec3::new(at.x, ground, at.z), rotation: (-way.x).atan2(way.z), scale: width / 32.0, born: now, sprite: tread, depth });
                                }
                                self.ambient.last_track.insert(u.id, q + way * (step * steps as f32));
                            }
                            None => {
                                self.ambient.last_track.insert(u.id, p);
                            }
                        }
                        // Dust off the treads at speed, on dry worlds.
                        if let Some(tint) = self.ambient.dust
                            && fast
                            && now - self.ambient.last_dust.get(&u.id).copied().unwrap_or(0.0) >= DUST_EVERY
                        {
                            self.ambient.last_dust.insert(u.id, now);
                            let heading = u.heading as f32 * TAU / 65536.0;
                            let fwd = Vec3::new(heading.sin(), 0.0, heading.cos());
                            let side = Vec3::new(fwd.z, 0.0, -fwd.x) * width * 0.3 * if self.rand() < 0.5 { 1.0 } else { -1.0 };
                            let at = p - fwd * t.radius.to_f32() * 0.8 + side;
                            self.dust_puff(at, -fwd * (6.0 + u.speed.to_f32() * 4.0), size * 5.0, tint, now);
                        }
                        continue;
                    }
                    // A foot down every stride, one after another (left and right in turn), each in
                    // its own place under the walker and as big as the sole of its model's foot
                    // (see `Gait::of`).
                    let gait = match self.ambient.gaits.get(&u.type_id) {
                        Some(gait) => gait.clone(),
                        None => std::sync::Arc::new(Gait::of(None, t.footprint, class_footprint(sim, t))),
                    };
                    let stride = gait.stride;
                    match self.ambient.last_step.get(&u.id).copied() {
                        Some((q, _)) if q.distance_squared(p) < stride * stride => {}
                        Some((q, next)) => {
                            let d = (p - q).normalize_or_zero();
                            let side = Vec3::new(d.z, 0.0, -d.x);
                            for pad in gait.feet[next % gait.feet.len()].iter().copied() {
                                let at = p + side * pad.at[0] + d * pad.at[1];
                                let ground = Self::ground(sim, at.x, at.z);
                                let depth = 0.85 + 0.3 * self.rand();
                                // (A little bigger than the sole: it presses the ground beyond itself.)
                                let (width, length) = (pad.size[0] * FOOT_PRESS, pad.size[1] * FOOT_PRESS);
                                if let Some((sprite, scale)) = foot_sprite_for(&self.ambient.foot_sprites, length / width, width) {
                                    new_tracks.push(Track { pos: Vec3::new(at.x, ground, at.z), rotation: (-d.x).atan2(d.z), scale, born: now, sprite, depth });
                                }
                                // A little dust where the foot came down, on dry worlds.
                                if let Some(tint) = self.ambient.dust
                                    && fast
                                {
                                    self.dust_puff(Vec3::new(at.x, ground, at.z), Vec3::ZERO, width * 1.6, tint, now);
                                }
                            }
                            self.ambient.last_step.insert(u.id, (p, next + 1));
                        }
                        None => {
                            self.ambient.last_step.insert(u.id, (p, 0));
                        }
                    }
                }
                // Vapour trails off the wingtips of aircraft flying fast and high.
                Locomotion::Air => {
                    let altitude = p.y - Self::ground(sim, p.x, p.z).max(sea);
                    if u.landed || altitude < 40.0 || u.speed.to_f32() < t.max_speed.to_f32() * 0.6 {
                        // (A new trail starts where it takes up speed again.)
                        self.ambient.last_vapour.remove(&u.id);
                        continue;
                    }
                    let Some(q) = self.ambient.last_vapour.get(&u.id).copied().filter(|q| q.distance_squared(p) < 200.0 * 200.0) else {
                        self.ambient.last_vapour.insert(u.id, p);
                        continue;
                    };
                    let gap = q.distance(p);
                    if gap < VAPOUR_SPACING {
                        continue;
                    }
                    self.ambient.last_vapour.insert(u.id, p);
                    let heading = u.heading as f32 * TAU / 65536.0;
                    let fwd = Vec3::new(heading.sin(), 0.0, heading.cos());
                    let side = Vec3::new(fwd.z, 0.0, -fwd.x);
                    let span = t.radius.to_f32() * 0.75;
                    // Puffs all the way from where the trails reached, a few pixels apart.
                    let n = ((gap / VAPOUR_SPACING) as usize).clamp(1, 12);
                    for k in 1..=n {
                        let at = q.lerp(p, k as f32 / n as f32) - fwd * t.radius.to_f32() * 0.3;
                        for s in [-1.0f32, 1.0] {
                            let sprite = self.smoke_puff();
                            self.emit(Particle {
                                sprite,
                                pos: at + side * span * s,
                                vel: Vec3::ZERO,
                                start: now,
                                life: 1.3,
                                size0: 6.0,
                                size1: 13.0,
                                alpha: 0.42,
                                additive: false,
                                gravity: 0.0,
                                tint: [2.2, 2.25, 2.3],
                                brightness: 1.0,
                                rotation: 0.0,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        for tr in new_tracks {
            if self.ambient.tracks.len() >= TRACKS_MAX {
                self.ambient.tracks.pop_front();
            }
            self.ambient.tracks.push_back(tr);
        }
        // Dust off both sides of the stern of hovercraft on land.
        for (p, fwd, radius, speed) in wakes {
            let side = Vec3::new(fwd.z, 0.0, -fwd.x);
            let stern = p - fwd * radius * 0.8;
            for s in [-1.0f32, 1.0] {
                let rotation = self.rand() * TAU;
                let jitter = self.rand() * 4.0;
                let (y, tint, alpha, life) = (p.y + 2.0, [1.3, 1.12, 0.85], 0.22, 1.1);
                let sprite = self.smoke_puff();
                self.emit(Particle {
                    sprite,
                    pos: Vec3::new(stern.x, y, stern.z) + side * s * (radius * 0.45 + jitter),
                    vel: side * s * (10.0 + speed * 6.0) - fwd * 4.0,
                    start: now,
                    life,
                    size0: radius * 0.5 + 4.0,
                    size1: radius * 1.2 + 12.0,
                    alpha,
                    additive: false,
                    gravity: 0.0,
                    tint,
                    brightness: 1.0,
                    rotation,
                });
            }
        }
        // Forget units that stopped existing.
        if sim.tick.is_multiple_of(300) {
            self.ambient.last_track.retain(|id, _| sim.unit(*id).is_some());
            self.ambient.last_wake.retain(|id, _| sim.unit(*id).is_some());
            self.ambient.last_step.retain(|id, _| sim.unit(*id).is_some());
            self.ambient.last_dust.retain(|id, _| sim.unit(*id).is_some());
            self.ambient.last_vapour.retain(|id, _| sim.unit(*id).is_some());
        }
    }

    /// A puff of the world's dust at `at`, drifting with `vel` and the wind.
    fn dust_puff(&mut self, at: Vec3, vel: Vec3, size: f32, tint: [f32; 3], now: f32) {
        let wind = Vec3::new(self.ambient.wind_dir.x, 0.0, self.ambient.wind_dir.y) * 6.0;
        let rotation = self.rand() * TAU;
        let rise = 3.0 + self.rand() * 4.0;
        let life = 1.0 + self.rand() * 0.6;
        let sprite = self.smoke_puff();
        self.emit(Particle {
            sprite,
            pos: at + Vec3::new(0.0, 2.0, 0.0),
            vel: vel + wind + Vec3::new(0.0, rise, 0.0),
            start: now,
            life,
            size0: size * 0.5,
            size1: size * 1.6,
            alpha: 0.2,
            additive: false,
            gravity: 0.0,
            tint,
            brightness: 1.0,
            rotation,
        });
    }

    /// Tread marks and footprints as ground decals, fading out.
    pub(super) fn track_decals(&mut self, now: f32) {
        while self.ambient.tracks.front().is_some_and(|t| now - t.born > TRACK_LIFE) {
            self.ambient.tracks.pop_front();
        }
        let view = self.view;
        for t in &self.ambient.tracks {
            if let Some((lo, hi)) = view && (t.pos.x < lo.x || t.pos.x > hi.x || t.pos.z < lo.y || t.pos.z > hi.y) {
                continue;
            }
            let fade = (1.0 - (now - t.born) / TRACK_LIFE).clamp(0.0, 1.0).powf(0.7);
            self.decals.push(SpriteInstance::new([t.pos.x, t.pos.y + 0.1, t.pos.z], t.sprite).scale(t.scale).alpha(fade * t.depth).rotation(t.rotation));
        }
    }
}

#[cfg(test)]
mod visual_tests {
    use super::*;

    /// A footprint picture's opacity at texel (`x`, `y`).
    fn print_alpha(px: &[u8], w: u32, x: u32, y: u32) -> u8 {
        px[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_footprint_is_a_rounded_rectangle_not_an_oval() {
        for shape in FOOT_SHAPES {
            let (w, h) = foot_size(shape);
            let px = foot_pixels(w, h);
            let middle = print_alpha(&px, w, w / 2, h / 2);
            assert!(middle > 40 && middle < 120, "a light press in the middle: {middle}");
            // The soft edge lies in the margin, and the corners of the sole, a rectangle's, are
            // well pressed, not empty as an oval's would be.
            assert_eq!(print_alpha(&px, w, 0, 0), 0);
            assert_eq!(print_alpha(&px, w, w - 1, h - 1), 0);
            for (x, y) in [(3, 3), (w - 4, 3), (3, h - 4), (w - 4, h - 4)] {
                assert!(print_alpha(&px, w, x, y) as f32 > middle as f32 * 0.5, "{shape}: the corner at {x},{y} is empty: {}", print_alpha(&px, w, x, y));
            }
            // Left and right alike.
            for y in [h / 3, h / 2, h - 5] {
                assert_eq!(print_alpha(&px, w, 4, y), print_alpha(&px, w, w - 5, y));
            }
            // Pressed deepest along the rim (over the middle rows, its lines of tread averaged out).
            let mean = |xs: std::ops::Range<u32>| {
                let rows = h / 4..h - h / 4;
                let sum: u32 = rows.clone().flat_map(|y| xs.clone().map(move |x| (x, y))).map(|(x, y)| print_alpha(&px, w, x, y) as u32).sum();
                sum as f32 / (rows.len() * xs.len()) as f32
            };
            assert!(mean(2..4) > mean(w / 2 - 3..w / 2 + 3), "{shape}: pressed deepest along its rim");
            // It fills the picture to its margin: most of the texels across the middle row are pressed.
            let pressed = (0..w).filter(|&x| print_alpha(&px, w, x, h / 2) as f32 > middle as f32 * 0.5).count();
            assert!(pressed as u32 >= w - 4, "{shape}: {pressed} of {w}");
        }
    }

    #[test]
    fn a_sole_is_drawn_with_the_picture_nearest_its_proportions() {
        let sprites = [(1.0, SpriteId(1)), (2.0, SpriteId(2)), (3.0, SpriteId(3))];
        let pick = |shape: f32| foot_sprite_for(&sprites, shape, 8.7).map(|p| p.0);
        assert_eq!(pick(0.9), Some(SpriteId(1)));
        assert_eq!(pick(1.3), Some(SpriteId(1)));
        assert_eq!(pick(1.5), Some(SpriteId(2)));
        assert_eq!(pick(2.4), Some(SpriteId(2)));
        assert_eq!(pick(2.6), Some(SpriteId(3)));
        // The sole 8.7 units wide covers the picture's 29 texels across: 0.3 a texel.
        assert!((foot_sprite_for(&sprites, 1.0, 8.7).unwrap().1 - 0.3).abs() < 1e-5);
        assert!(foot_sprite_for(&[], 1.0, 8.7).is_none());
        // A picture's length follows its proportions, its width is the same.
        let sizes: Vec<(u32, u32)> = FOOT_SHAPES.iter().map(|&s| foot_size(s)).collect();
        assert!(sizes.iter().all(|s| s.0 == FOOT_TEXELS) && sizes.windows(2).all(|w| w[0].1 < w[1].1));
        assert_eq!(foot_size(1.0), (FOOT_TEXELS, FOOT_TEXELS));
    }

    fn foot(across: f32, w: f32, l: f32) -> crate::scene::Foot {
        crate::scene::Foot { pads: vec![crate::scene::Pad { at: [across, 0.0], size: [w, l] }] }
    }

    /// How long a foot's print is, heel to toe (world units).
    fn print_length(f: &[crate::scene::Pad]) -> f32 {
        let heel = f.iter().map(|p| p.at[1] - p.size[1] * 0.5).fold(f32::MAX, f32::min);
        let toe = f.iter().map(|p| p.at[1] + p.size[1] * 0.5).fold(f32::MIN, f32::max);
        (toe - heel) * FOOT_PRESS
    }

    #[test]
    fn a_walker_prints_its_own_feet_when_it_could_stand_on_them() {
        use crate::scene::Pad;
        // TA's Commander (2 x 2): its own feet, 7.4 x 6.9, a step every 22 units as before.
        let com = Gait::of(Some(&[foot(4.8, 7.4, 6.9), foot(-4.8, 7.4, 6.9)]), (2, 2), Some((2, 2)));
        assert!(com.measured);
        assert_eq!(com.feet, vec![vec![Pad { at: [4.8, 0.0], size: [7.4, 6.9] }], vec![Pad { at: [-4.8, 0.0], size: [7.4, 6.9] }]]);
        assert_eq!(com.stride, 22.0);
        // TA's own walkers keep their steps (a cell's 11 units): the longest prints for their
        // footprints, a Core Spy's 30.2 on 2 x 2 and a Krogoth's 41.5 on 3 x 3, fit between two
        // steps of one foot; a Spectre (1 x 1) stands on feet 9.3 x 10.8.
        for (feet, cells) in [(vec![foot(7.5, 13.2, 30.2), foot(-7.5, 13.2, 30.2)], 2), (vec![foot(7.0, 11.7, 41.5), foot(-7.0, 11.7, 41.5)], 3), (vec![foot(7.5, 9.3, 10.8), foot(-7.5, 9.3, 10.8)], 1)] {
            let gait = Gait::of(Some(&feet), (cells, cells), Some((2.max(cells), 2.max(cells))));
            assert!(gait.measured && gait.stride == 11.0 * cells as f32, "{gait:?}");
        }
        // A foot of a heel and toes (the Krogoth's, as two pads): both come down at once.
        let heel_and_toes = crate::scene::Foot { pads: vec![Pad { at: [7.0, -12.0], size: [11.0, 14.0] }, Pad { at: [7.0, 14.0], size: [11.0, 10.0] }] };
        let gait = Gait::of(Some(&[heel_and_toes.clone(), foot(-7.0, 11.0, 38.0)]), (3, 3), None);
        assert!(gait.measured && gait.feet[0].len() == 2);
        assert!((print_length(&gait.feet[0]) - 38.0 * FOOT_PRESS).abs() < 1e-3);
    }

    #[test]
    fn a_sole_bigger_than_a_walker_could_stand_on_leaves_its_footprints_pair() {
        use crate::scene::Pad;
        // TA: Escalation's Commander as it was measured with its plate (one sole 108 units square
        // on a 2 x 2 footprint): no foot of its; a pair of 6 x 8 from its footprint, 7 either side
        // (as for a walker whose feet are not known), a step every 22.
        let pair = vec![vec![Pad { at: [7.0, 0.0], size: [6.0, 8.0] }], vec![Pad { at: [-7.0, 0.0], size: [6.0, 8.0] }]];
        let plate = Gait::of(Some(&[foot(0.0, 108.3, 108.3)]), (2, 2), Some((2, 2)));
        assert!(!plate.measured);
        assert_eq!((&plate.feet, plate.stride), (&pair, 22.0));
        assert_eq!(Gait::of(None, (2, 2), Some((2, 2))), plate);
        assert_eq!(Gait::of(Some(&[]), (2, 2), None), plate);
        assert_eq!(Gait::of(Some(&[crate::scene::Foot { pads: Vec::new() }]), (2, 2), None), plate);
        // One sole too big spoils the measure (the real ones were dropped beside it).
        assert!(!Gait::of(Some(&[foot(5.0, 8.7, 11.7), foot(-5.0, 8.7, 11.7), foot(0.0, 70.0, 20.0)]), (2, 2), None).measured);
        // Twice the footprint is the most either way: 64 units for 2 x 2, across and along.
        let edge = 64.0 / FOOT_PRESS;
        assert!(Gait::of(Some(&[foot(20.0, edge - 0.01, edge - 0.01), foot(-20.0, 5.0, 5.0)]), (2, 2), None).measured);
        assert!(!Gait::of(Some(&[foot(20.0, edge + 0.01, 5.0), foot(-20.0, 5.0, 5.0)]), (2, 2), None).measured);
        assert!(!Gait::of(Some(&[foot(20.0, 5.0, edge + 0.01), foot(-20.0, 5.0, 5.0)]), (2, 2), None).measured);
        // A big walker's big feet are its own (TA: Escalation's Krogoth, 6 x 6: 19.5 x 62.4).
        assert!(Gait::of(Some(&[foot(12.7, 18.7, 62.7), foot(-13.7, 19.5, 62.4)]), (6, 6), Some((6, 6))).measured);
        // A 4 x 4 walker's pair, from its footprint: 12 x 16, 14 either side; steps of 44.
        let big = Gait::of(None, (4, 6), None);
        assert_eq!(big.feet[0], vec![Pad { at: [14.0, 0.0], size: [12.0, 16.0] }]);
        assert_eq!(big.stride, 44.0);
    }

    #[test]
    fn a_unit_naming_no_footprint_is_measured_against_its_movement_class() {
        // TA: Enhanced's definitions name no footprint (read as 1 x 1; TA takes the movement
        // class's): its Krogoth's feet, 13 x 43.5, are more than a 1 x 1 unit stands on, and
        // within its class's 3 x 3.
        let feet = [foot(8.8, 13.0, 43.5), foot(-9.5, 13.6, 43.3)];
        assert!(!Gait::of(Some(&feet), (1, 1), None).measured);
        let gait = Gait::of(Some(&feet), (1, 1), Some((3, 3)));
        assert!(gait.measured);
        // Its steps (11 units for its 1 x 1) lengthen so that a foot comes down clear of its last
        // print: 43.5 long, x 1.12 pressed, x 1.1 apart, a foot every second step.
        assert!((gait.stride - 43.5 * FOOT_PRESS * PRINT_SPACING / 2.0).abs() < 1e-4, "{}", gait.stride);
    }

    #[test]
    fn a_foots_prints_never_overlap_one_another() {
        // Whatever the walker: two prints of one foot lie its number of steps apart, at least
        // a print's length (and a tenth) apart, so none darkens the ground another has darkened.
        let cases: Vec<(Vec<crate::scene::Foot>, (u32, u32))> = vec![
            (vec![foot(4.8, 7.4, 6.9), foot(-4.8, 7.4, 6.9)], (2, 2)),
            (vec![foot(0.0, 12.0, 30.0)], (2, 2)),
            (vec![foot(9.0, 5.0, 30.0), foot(-9.0, 5.0, 30.0), foot(9.0, 5.0, 30.0), foot(-9.0, 5.0, 30.0)], (1, 1)),
            (vec![foot(15.0, 23.9, 42.6), foot(-15.0, 22.1, 42.9)], (1, 1)),
            (vec![foot(23.8, 35.3, 65.7), foot(-23.8, 35.3, 65.7)], (7, 7)),
        ];
        for (feet, footprint) in cases {
            let gait = Gait::of(Some(&feet), footprint, Some((3, 3)));
            for f in &gait.feet {
                let apart = gait.stride * gait.feet.len() as f32;
                assert!(apart + 1e-3 >= print_length(f) * PRINT_SPACING, "{footprint:?}: {apart} for a print {} long", print_length(f));
            }
        }
    }

    #[test]
    fn embers_over_the_lava_stay_few_small_and_low() {
        // The old ember: it climbed past ninety pixels over the melt, for up to five seconds, 4.6
        // across at full brightness 6, two of them for every spot.
        let old = Burst::new(Kind::Ember, [0.0; 3], 2).radius(24.0).vertical(0.1).toward([0.0, 1.0, 0.0], 0.5).speed(15.0, 50.0).life(2.0, 5.0).size(4.6, 1.5).gravity(-14.0).drag(0.6).intensity(6.0);
        assert!(highest_climb(&old) > 90.0, "{}", highest_climb(&old));
        // Now one, smaller, dimmer and shorter-lived, keeping within a couple of dozen pixels of
        // the surface (the odd one carried higher, still under sixty), a speck however near the eye.
        let (ember, rising) = (lava_ember([0.0; 3]), rising_ember([0.0; 3]));
        for b in [&ember, &rising] {
            assert!(b.count == 1 && b.speck && b.size[0] < old.size[0] && b.intensity < old.intensity && b.life[1] < old.life[1], "{b:?}");
        }
        assert!(highest_climb(&ember) < 25.0, "{}", highest_climb(&ember));
        assert!(highest_climb(&rising) < 60.0, "{}", highest_climb(&rising));
        // A bubble's droplets fall back from fifteen pixels at most (they went past forty); its glow
        // is left out under a low camera.
        let bubble = lava_bubble([0.0; 3], 6, true);
        assert!(bubble.iter().filter(|b| b.kind == Kind::Ember).all(|b| b.speck && highest_climb(b) < 16.0), "{:?}", bubble.iter().map(highest_climb).collect::<Vec<_>>());
        assert_eq!(bubble.len(), 2);
        assert_eq!(lava_bubble([0.0; 3], 6, false).len(), 1);
        // Spots a second by the area in view, up to a most, and by the effects quality: the same
        // at any frame rate (they were capped at 64 a frame, so a slower one had fewer).
        assert_eq!(ember_spots(2.0, 1.0), 2.0 * EMBER_SPOTS);
        assert_eq!(ember_spots(1000.0, 1.0), EMBER_SPOTS_MOST);
        assert!(ember_spots(2.0, 0.4) < ember_spots(2.0, 1.0) && ember_spots(2.0, 1.0) < ember_spots(2.0, 1.4));
        assert_eq!(ember_spots(-1.0, 1.0), 0.0);
        assert_eq!(ember_spots(2.0, 0.0), 0.0, "weather off");
    }

    #[test]
    fn the_weather_falls_round_where_a_perspective_camera_looks() {
        let (lo, hi) = (Vec2::new(-20_000.0, 0.0), Vec2::new(20_000.0, 40_000.0));
        assert_eq!(weather_area(None, lo, hi), (lo, hi), "the classic view: the ground in view");
        assert_eq!(weather_area(Some(Vec2::new(0.0, 1000.0)), lo, hi), (Vec2::new(-1500.0, 0.0), Vec2::new(1500.0, 2500.0)), "near the focus, in view");
        assert_eq!(weather_area(Some(Vec2::new(0.0, -5000.0)), lo, hi), (Vec2::new(-1500.0, -6500.0), Vec2::new(1500.0, -3500.0)), "looking where nothing is in view: round the focus");
    }

    #[test]
    fn wake_visibility_covers_mature_arms_and_segments_between_points() {
        let mut wake = Wake {
            points: VecDeque::from([(Vec2::new(500.0, 50.0), 0.0, 0.0, 90.0, 0.0)]),
            ticks: [(Vec2::new(500.0, 50.0), 0.0); 2], tick: 0, step: 1,
            hull: Hull { bow: 24.0, stern: 16.0, beam: 8.0 }, hover: false, live: false,
        };
        let (lo, hi) = (Vec2::ZERO, Vec2::splat(100.0));
        assert!(!wake.overlaps(0.0, lo, hi), "the fresh narrow wake is outside the view");
        assert!(wake.overlaps(13.0, lo, hi), "a mature wake reaches in from beyond the old 320-unit margin");
        wake.points = VecDeque::from([(Vec2::new(-500.0, 50.0), 0.0, 0.0, 10.0, 0.0), (Vec2::new(500.0, 50.0), 0.0, 0.0, 10.0, 1000.0)]);
        assert!(wake.overlaps(0.0, lo, hi), "the ribbon crosses the view with both endpoints outside it");
    }
}
