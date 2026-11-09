//! Presentation of combat: projectiles, explosions, smoke, muzzle glows, and the GPU particle
//! bursts (sparks, embers, flames, dust, grit; `effects_particles.rs`) that go with them.
//!
//! Everything here is client-side eye candy driven by simulation events and projectile
//! state; it never feeds back into the deterministic simulation.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use ta_data::RenderType;
use ta_render::{BeamInstance, DrawInstance, ModelId, ParticleBurst, Renderer, SpriteId, SpriteInstance};
use ta_sim::{Sim, SimEvent};
use ta_vfs::Vfs;

/// A playing sprite animation (explosions).
struct Anim {
    frames: usize,
    start: f32,
    pos: Vec3,
    fps: f32,
    scale: f32,
    brightness: f32,
}

/// A weapon's model in flight (a missile's, a rocket's, a torpedo's, a bomb's) and where its
/// tail is.
struct WeaponModel {
    model: ModelId,
    /// Its pieces' matrices as it rests, in its flight's frame: nose along +z, the body's middle
    /// on the line it flies. (TA's lie off it: the Arm's nuke 6 px up and 5 to one side of its
    /// projectile's place, so its exhaust, put there, came out from under it.)
    pieces: Vec<Mat4>,
    /// Where its tail ends, along its flight from the projectile's place (px; behind it, so less
    /// than nothing), and how thick its body is.
    tail: f32,
    girth: f32,
}

impl WeaponModel {
    /// Its tail and girth, and its body moved onto the flight's line, from its pieces as they rest
    /// (in the flight's frame), their names and its triangles' corners on them. The body is what
    /// is not its flare: TA's rockets and some missiles carry their exhaust as a piece of their
    /// own (`flare`, `missile flare`, `misflare`: the Merl's 35 px long behind a 30 px rocket, a
    /// thin stalk to a yellow-green leaf), the flame starting where the body ends. A missile
    /// drawn with Reborn's flame (`flame`: one with a smoke trail) is drawn without it.
    fn new(model: ModelId, rest: Vec<Mat4>, names: &[&str], corners: impl Iterator<Item = (u32, Vec3)>, flame: bool) -> WeaponModel {
        let flare = |piece: u32| names.get(piece as usize).is_some_and(|n| n.to_ascii_lowercase().contains("flare"));
        let body = names.iter().enumerate().any(|(i, _)| !flare(i as u32));
        let (lo, hi) = corners
            .filter(|&(piece, _)| !body || !flare(piece))
            .filter_map(|(piece, at)| rest.get(piece as usize).map(|m| m.transform_point3(at)))
            .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        if lo.x > hi.x {
            // (Nothing drawn: a missile's old place for its flame.)
            return WeaponModel { model, pieces: rest, tail: -6.0, girth: 6.0 };
        }
        let middle = Mat4::from_translation(-Vec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, 0.0));
        // (A piece left out is drawn shrunk to nothing where it was.)
        let gone = Mat4::from_scale(Vec3::splat(1e-4));
        let pieces = rest.iter().enumerate().map(|(i, m)| if flame && body && flare(i as u32) { middle * *m * gone } else { middle * *m }).collect();
        WeaponModel { model, pieces, tail: lo.z, girth: (hi.x - lo.x).min(hi.y - lo.y).max(1.0) }
    }
}

/// How a projectile's model lies along `dir`, the way it flies: its nose along it, its top kept
/// up (no roll; turned from +z the shortest way, a missile rolled as it turned and flew upside
/// down northwards); straight up or down, facing `towards` (the way it is headed for, and turns
/// over to).
fn flight_rotation(dir: Vec3, towards: Vec3) -> Quat {
    let level = if dir.x.abs() + dir.z.abs() > 1e-4 { dir } else { towards };
    Quat::from_rotation_y(level.x.atan2(level.z)) * Quat::from_rotation_x(-dir.y.clamp(-1.0, 1.0).asin())
}

/// What one of TA's explosion animations looks like, measured from its frames when it loads.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AnimLook {
    /// Half the size of its biggest frame (world units at scale 1: a sprite pixel is one).
    radius: f32,
    /// A blast of fire (its painted colours white-hot to red, smoke-grey), rather than a splash,
    /// a shock ring or an energy burst (blue or green). With the perspective camera fire is
    /// drawn as fireballs (`fireball_bursts`) in place of TA's painting, which magnified shows
    /// its pixels.
    fire: bool,
}

/// An [`AnimLook`] measured a frame at a time, as its frames are read (`effects_anims.rs`).
#[derive(Default)]
struct LookSum {
    biggest: f32,
    /// The opaque pixels' colour (each weighted by how bright it is) and their weights, summed.
    sum: [f64; 3],
    weight: f64,
}

impl LookSum {
    /// A frame as painted.
    fn add(&mut self, f: &ta_formats::gaf::GafFrame, palette: &ta_formats::image::Palette) {
        let img = &f.image;
        self.biggest = self.biggest.max((img.width + img.height) as f32 * 0.5);
        for (&p, _) in img.pixels.iter().zip(&img.opaque).filter(|&(_, &o)| o) {
            let [r, g, b, _] = palette.rgba(p);
            self.colour([r, g, b], 1.0);
        }
    }

    /// A frame read reduced: as big as painted, each texel's colour standing for the opaque
    /// pixels it covers.
    fn add_reduced(&mut self, f: &ta_formats::gaf::ReducedFrame) {
        self.biggest = self.biggest.max((f.full_width + f.full_height) as f32 * 0.5);
        let per_alpha = f.scale * f.scale / 255.0;
        for t in f.rgba.as_chunks::<4>().0.iter().filter(|t| t[3] > 0) {
            self.colour([t[0], t[1], t[2]], f64::from(t[3]) * per_alpha);
        }
    }

    fn colour(&mut self, [r, g, b]: [u8; 3], pixels: f64) {
        let w = (r as f64 + g as f64 + b as f64) / 765.0 * pixels;
        self.sum[0] += r as f64 * w;
        self.sum[1] += g as f64 * w;
        self.sum[2] += b as f64 * w;
        self.weight += w;
    }

    /// The opaque pixels' mean colour, the bright ones counting most: fire is red over blue and
    /// about as red as green, or redder.
    fn look(&self) -> AnimLook {
        let [r, g, b] = self.sum.map(|c| c / self.weight.max(1e-9));
        AnimLook { radius: self.biggest * 0.5, fire: self.weight > 0.0 && r >= b * 1.05 && r >= g * 0.95 }
    }
}

/// A smoke puff / glow particle.
/// An expanding shockwave ring on the ground (large blasts).
/// A scorch mark left on the ground by an explosion.
struct Scorch {
    pos: Vec3,
    size: f32,
    rotation: f32,
    start: f32,
}

/// A destroyed unit's wreck burning, then smouldering: seconds of flames, and of both (the
/// bigger the unit, the longer).
struct Burning {
    pos: Vec3,
    start: f32,
    big: bool,
    next_puff: f32,
    fire: f32,
    smoulder: f32,
}

/// Tint of the smoke of burning fuel and wrecks (on the smoke sprite's grey).
const DARK_SMOKE: [f32; 3] = [0.42, 0.39, 0.36];
/// How charred a flying piece of a destroyed unit is drawn (see `Debris::damage`): as it is
/// thrown off, its cracks glowing, and three seconds on, cooled.
const DEBRIS_DAMAGE: (f32, f32) = (0.95, 0.7);
/// The flags of what is left of the dead (debris, a charred hull): their lights out, their
/// animated textures held (`playing: 0`).
const DEAD_LIGHTS: u32 = 255 << ta_render::model::FLAG_LIGHTS_OUT_SHIFT;

/// How fast smoke drifts on the wind (world units a second at a moderate wind).
const SMOKE_WIND: f32 = 16.0;

/// Seconds scorch marks take to fade away, and how many are kept.
const SCORCH_LIFE: f32 = 60.0;
const MAX_SCORCHES: usize = 400;
/// Seconds a wreck burns with flames, and smoulders in total (a unit without a death blast:
/// its fuel; a wreck's own by its size, see `unit_died`), and the most burning at once.
const FIRE_TIME: f32 = 7.0;
const SMOULDER_TIME: f32 = 30.0;
const MOST_BURNING: usize = 160;

/// A short-lived dynamic light (explosion fireball, muzzle flash) that fades out.
struct TimedLight {
    pos: Vec3,
    color: [f32; 3],
    radius: f32,
    start: f32,
    life: f32,
}

/// Hot air over a blast, shimmering for a while (heat haze).
struct TimedHeat {
    pos: Vec3,
    radius: f32,
    strength: f32,
    start: f32,
    life: f32,
}

/// A model piece thrown off by a dying unit.
struct Debris {
    model: ModelId,
    piece: usize,
    /// The flying piece's world matrix.
    matrix: Mat4,
    surface: std::sync::Arc<crate::scene::PieceSurface>,
    pos: Vec3,
    vel: Vec3,
    heading: f32,
    axis: Vec3,
    spin: f32,
    angle: f32,
    start: f32,
    flags: i32,
    /// Time the piece came to rest (it then sinks away).
    landed: Option<f32>,
    trail: f32,
    /// Its unit's player colour, and how charred it is drawn (a unit's battle damage: soot,
    /// and cracks glowing while it is hot, cooling as it flies).
    team: u32,
    damage: f32,
}

impl Debris {
    fn transform(&self) -> Mat4 {
        Mat4::from_translation(self.pos) * Mat4::from_rotation_y(self.heading) * Mat4::from_axis_angle(self.axis, self.angle)
    }
}

/// Clearance of the actual mesh above the terrain and its lowest contact point. The model's
/// pivot can be outside its geometry (arms and guns often are), so stopping the pivot at the
/// terrain leaves some fragments hanging in the air and buries others. Sampling each vertex
/// also seats a tilted piece on sloping ground instead of on a plane through its origin.
fn debris_contact(vertices: &[Vec3], matrix: Mat4, ground: impl Fn(f32, f32) -> f32) -> (f32, Vec3) {
    vertices.iter().map(|&v| {
        let p = matrix.transform_point3(v);
        let floor = ground(p.x, p.z);
        (p.y - floor, Vec3::new(p.x, floor, p.z))
    }).min_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or((f32::INFINITY, Vec3::ZERO))
}

/// Live particles above which new ones thin out, and the most there can be (at High effects
/// quality; Low keeps a quarter, Ultra twice as many).
const PARTICLES_SOFT: usize = 16_000;
const PARTICLES_MAX: usize = 32_000;
/// The same for flying debris (each piece is a model instance).
const DEBRIS_SOFT: usize = 600;
const DEBRIS_MAX: usize = 1_500;

/// How much effects detail the player wants.
#[derive(Clone, Copy, Debug)]
pub struct Detail {
    /// Scales the particle and debris budgets.
    pub budget: f32,
    /// Scales weather density (0 = none).
    pub weather: f32,
    pub tracks: bool,
    /// Most GPU particles alive at once, and the share of each burst's particles emitted.
    pub particles: u32,
    pub particle_density: f32,
}

impl Detail {
    pub fn from_settings(g: &ta_config::Graphics) -> Detail {
        let (budget, weather, tracks, particles, particle_density) = match g.effects {
            ta_config::Quality::Low => (0.25, 0.4, false, 1 << 15, 0.35),
            ta_config::Quality::Medium => (0.5, 0.7, true, 1 << 16, 0.6),
            ta_config::Quality::High => (1.0, 1.0, true, 1 << 17, 1.0),
            ta_config::Quality::Ultra => (2.0, 1.4, true, 1 << 18, 1.5),
        };
        Detail { budget, weather: if g.weather { weather } else { 0.0 }, tracks, particles, particle_density }
    }
}

impl Default for Detail {
    fn default() -> Detail {
        Detail { budget: 1.0, weather: 1.0, tracks: true, particles: 1 << 17, particle_density: 1.0 }
    }
}

const EXPLODE_ON_HIT: i32 = 2;
const FALL: i32 = 4;
const SMOKE: i32 = 8;
const FIRE: i32 = 16;
const BITMAPONLY: i32 = 32;

struct Particle {
    sprite: SpriteId,
    pos: Vec3,
    vel: Vec3,
    start: f32,
    life: f32,
    size0: f32,
    size1: f32,
    alpha: f32,
    additive: bool,
    /// How fast it falls (units a second, each second): sparks do, a glow or a flame does not.
    gravity: f32,
    tint: [f32; 3],
    brightness: f32,
    rotation: f32,
}

/// Sparks falling (see `Particle::gravity`).
const SPARK_FALL: f32 = 180.0;

#[path = "effects_ambient.rs"]
mod ambient;
#[path = "effects_anims.rs"]
mod anims;
pub use ambient::{Weather, WeatherState};
#[path = "effects_fireworks.rs"]
mod fireworks;
#[path = "effects_nano.rs"]
mod nano;
#[path = "effects_damage.rs"]
mod damage;
#[path = "effects_impacts.rs"]
mod impacts;
#[path = "effects_particles.rs"]
mod particles;
#[path = "effects_remains.rs"]
mod remains;
pub use remains::Death;

use particles::Blast;

pub struct Effects {
    /// TA's explosion animations (`effects_anims.rs`): each one's frames, and what it looks like.
    anims: Vec<Vec<SpriteId>>,
    anim_looks: Vec<AnimLook>,
    /// TA's own blasts Reborn's effects use ("fx/explode3"...), where they are TA's size.
    generic_blasts: HashMap<String, usize>,
    /// Per weapon: (land, water, lava) animation indices.
    weapon_anims: Vec<[Option<usize>; 3]>,
    /// Each weapon's model in flight (missiles, rockets and torpedoes have two or three pieces:
    /// the body, its flare or propeller).
    weapon_models: Vec<Option<WeaponModel>>,
    /// Each projectile's velocity a tick ago and now, and the tick of now: its model turns
    /// between them as it is drawn between those ticks (a missile turning over after its climb
    /// turned in steps of 6°, two frames each).
    flight_dirs: HashMap<u32, (u64, Vec3, Vec3)>,
    glow: SpriteId,
    /// A snowflake: a small soft white mote.
    flake: SpriteId,
    /// Puffs of smoke: billows of four shapes (see `billow_pixels`), one picked for each.
    smoke: [SpriteId; 4],
    /// The perspective camera draws the battle (its effects in 3D: fireballs and spray in place
    /// of TA's painted blasts and splashes); the classic camera keeps TA's.
    pub modern: bool,
    active: Vec<Anim>,
    particles: Vec<Particle>,
    /// Blasts of projectiles that met what they hit partway through a tick, each with the
    /// moment (game seconds) the projectile drawn gets there: they go off then.
    due_blasts: Vec<(f32, SimEvent)>,
    trail_timer: HashMap<u32, f32>,
    rng: u32,
    pub additive: Vec<SpriteInstance>,
    pub smoke_sprites: Vec<SpriteInstance>,
    pub beams: Vec<BeamInstance>,
    /// This frame's lightning bolts (by projectile id) and how each is drawn among the others
    /// crowding it (see [`BoltLook`]).
    bolts: rustc_hash::FxHashMap<u32, BoltLook>,
    model_matrices: Vec<Vec<Mat4>>,
    model_ids: Vec<ModelId>,
    /// Fog of war: only effects inside this player's sight are shown (None = all).
    pub viewer: Option<u8>,
    /// Ground rectangle in view (with a margin); particles outside it are not drawn.
    pub view: Option<(glam::Vec2, glam::Vec2)>,
    /// The perspective camera's eye (snowflakes passing close to it are kept to specks).
    pub eye: Option<Vec3>,
    /// Where the perspective camera looks, on the ground (the weather falls round it).
    pub focus: Option<glam::Vec2>,
    /// Effects quality from the settings.
    pub detail: Detail,
    /// Presentation only: soften luminous combat effects without hiding projectiles.
    pub reduced_flashes: bool,
    debris: Vec<Debris>,
    timed_lights: Vec<TimedLight>,
    /// This frame's dynamic lights, for `Renderer::set_lights`.
    pub lights: Vec<ta_render::PointLight>,
    timed_heat: Vec<TimedHeat>,
    /// This frame's heat haze sources (position, radius, strength), for
    /// `Renderer::add_heat_source`.
    pub heat: Vec<(Vec3, f32, f32)>,
    /// Sounds the effects want played (name, where, volume): thunder. The app hands them to
    /// the audio engine each frame.
    pub sounds: Vec<(&'static str, Vec3, f32)>,
    scorch_sprite: SpriteId,
    scorches: Vec<Scorch>,
    burning: Vec<Burning>,
    /// Badly damaged units (see `effects_damage.rs`): when each next gives off flame or smoke;
    /// and where those burning in view are this frame (position, size, how hard: 0..1), for
    /// their firelight.
    damage_fx: rustc_hash::FxHashMap<ta_sim::UnitId, f32>,
    unit_fires: Vec<(Vec3, f32, f32)>,
    /// Burning trees (feature index): when they next give off embers and smoke.
    tree_fires: rustc_hash::FxHashMap<usize, f32>,
    /// Burning trees drawn in 3D (feature index, the middle of the crown): their flames go there.
    pub tree_flames: Vec<(usize, Vec3)>,
    /// Rings running out over the ground and the water (shockwaves, ripples, an EMP's pulse),
    /// and this frame's of them for the renderer (with the selection's, see `rings.rs`).
    ripples: Vec<impacts::Ripple>,
    pub rings: Vec<ta_render::RingInstance>,
    /// Camera shake to apply this frame (world units) and its decay.
    pub shake: f32,
    /// Ground decals (scorch marks) for this frame.
    pub decals: Vec<SpriteInstance>,
    /// Weather, wakes and tread marks.
    ambient: ambient::Ambient,
    /// This frame's wakes behind ships and hovercraft, for `Renderer::prepare_wakes`: their
    /// ways' points and the wakes over them.
    pub wake_points: Vec<ta_render::WakePoint>,
    pub wake_trails: Vec<ta_render::WakeTrail>,
    /// Nanolathe particles, and each unit type's nano emitter piece.
    nano: Vec<nano::Nano>,
    /// Where builders' nano streams land this frame (and how far they spread): each glows
    /// green on what it builds.
    nano_lights: Vec<(Vec3, f32)>,
    nano_pieces: HashMap<usize, Option<usize>>,
    /// This frame's GPU particle bursts (handed over by `submit_particles`).
    bursts: Vec<ParticleBurst>,
    /// Per weapon: some unit blows up with it when it dies (a bigger shower of sparks).
    death_weapons: Vec<bool>,
    /// The map's sea is lava (blasts in it throw up glowing rock, not spray).
    lava_world: bool,
    /// What is left of the dead: falling, sinking, collapsing (`effects_remains.rs`).
    remains: Vec<remains::Remains>,
    /// Big units' later blasts, due.
    pops: Vec<remains::Pop>,
    /// Fireworks for the New Year (`effects_fireworks.rs`): the shells in flight, when each rack
    /// (feature index) sends up its next, and a volley under way (round where, how far, until).
    shells: Vec<fireworks::Shell>,
    firework_due: rustc_hash::FxHashMap<usize, f32>,
    volley: Option<(Vec3, f32, f32)>,
}

impl Effects {
    /// How hard it rains at game time `now` (0..1; see the weather in `effects_ambient.rs`).
    pub fn rain(&self, now: f32) -> f32 {
        self.rain_strength(now)
    }

    /// The weather over the world as it is drawn: snow, embers, showers, dust, spores or glitter,
    /// or none (the cockpit's instruments say so).
    pub fn weather_kind(&self) -> Weather {
        self.ambient.weather
    }

    /// Forgets the battle's effects (the highlights go back in time): whatever burns, flies,
    /// smokes, glows or marks the ground now.
    pub fn clear_battle(&mut self) {
        self.active.clear();
        self.particles.clear();
        self.shells.clear();
        self.firework_due.clear();
        self.volley = None;
        self.trail_timer.clear();
        self.flight_dirs.clear();
        self.additive.clear();
        self.smoke_sprites.clear();
        self.beams.clear();
        self.debris.clear();
        self.timed_lights.clear();
        self.lights.clear();
        self.timed_heat.clear();
        self.heat.clear();
        self.sounds.clear();
        self.scorches.clear();
        self.burning.clear();
        self.damage_fx.clear();
        self.unit_fires.clear();
        self.tree_fires.clear();
        self.ripples.clear();
        self.rings.clear();
        self.shake = 0.0;
        self.decals.clear();
        self.nano.clear();
        self.bursts.clear();
        self.ambient.clear_marks();
        self.wake_points.clear();
        self.wake_trails.clear();
        self.remains.clear();
        self.pops.clear();
        self.due_blasts.clear();
    }
}

/// Where an event happens, for line-of-sight filtering.
fn event_position(e: &SimEvent, sim: &Sim) -> Option<ta_sim::FxVec3> {
    match e {
        SimEvent::Explosion { pos, .. }
        | SimEvent::UnitExploded { pos, .. }
        | SimEvent::WeaponFired { pos, .. }
        | SimEvent::Sfx { pos, .. }
        | SimEvent::PieceExplode { pos, .. }
        | SimEvent::UnitDestroyed { pos, .. }
        | SimEvent::Intercepted { pos, .. } => Some(*pos),
        SimEvent::NanoSpray { builder, .. } => sim.unit(*builder).map(|u| u.pos),
        _ => None,
    }
}

/// Whether an event out of `player`'s sight is theirs to see anyway: the blast of a missile of
/// their side's that a defence could shoot at, or of its interceptor, and an interception either
/// side of which was theirs (see `nuclear::strategic`).
fn ours(e: &SimEvent, sim: &Sim, player: u8) -> bool {
    match *e {
        SimEvent::Explosion { weapon, player: Some(owner), .. } => crate::game::nuclear::strategic(&sim.weapon_types.types[weapon]) && sim.allied(player, owner),
        SimEvent::Intercepted { defender, attacker, .. } => sim.allied(player, defender) || sim.allied(player, attacker),
        _ => false,
    }
}

/// A puff of smoke `size` texels square, of shape `shape` (each its own lumps): a billow of
/// fractal noise, denser inside and soft-edged, shading itself (its dense heart darker, its
/// thin fringes lighter, as lit smoke is), and nothing at the square's border.
fn billow_pixels(size: u32, shape: u32) -> Vec<u8> {
    let hash = |x: i32, y: i32| {
        let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ shape.wrapping_mul(0xCB1A_B31F);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5BD1_E995);
        h ^= h >> 15;
        (h & 0xFFFF) as f32 / 65535.0
    };
    let value_noise = |x: f32, y: f32| {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let (ix, iy) = (x0 as i32, y0 as i32);
        let a = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * sx;
        let b = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * sx;
        a + (b - a) * sy
    };
    let smooth = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let d = (u * u + v * v).sqrt();
            let (mut n, mut amplitude, mut frequency) = (0.0, 0.5, 2.4);
            for octave in 0..4 {
                n += value_noise(u * frequency + 17.0 + octave as f32 * 5.3, v * frequency + 31.0) * amplitude;
                amplitude *= 0.5;
                frequency *= 2.0;
            }
            let n = n / 0.9375;
            let edge = 1.0 - smooth(0.25, 0.95, d + (n - 0.5) * 0.6);
            let density = (edge * (0.3 + 1.0 * n) * (1.0 - smooth(0.85, 1.0, d))).clamp(0.0, 1.0);
            let shade = 1.12 - 0.4 * density;
            let c = |k: f32| ((k * shade).clamp(0.0, 1.0) * 255.0) as u8;
            out.extend_from_slice(&[c(0.62), c(0.6), c(0.58), (density.powf(1.2) * 0.95 * 255.0) as u8]);
        }
    }
    out
}

fn radial(size: u32, f: impl Fn(f32) -> [f32; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / c).min(1.0);
            let [r, g, b, a] = f(d);
            out.extend_from_slice(&[(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, (a * 255.0) as u8]);
        }
    }
    out
}

impl Effects {
    pub fn new(vfs: &Vfs, renderer: &mut Renderer, sim: &Sim) -> Effects {
        let palette = &sim.data.palette;
        let atlas = &mut renderer.sprites.atlas;
        let glow = atlas
            .add_rgba(64, 64, &radial(64, |d| {
                let a = (1.0 - d).powf(2.2);
                [1.0, 1.0, 1.0, a]
            }), 31.5, 31.5)
            .expect("atlas");
        let smoke = [0, 1, 2, 3].map(|shape| atlas.add_rgba(64, 64, &billow_pixels(64, shape), 31.5, 31.5).expect("atlas"));
        let flake = atlas.add_rgba(16, 16, &radial(16, |d| [1.0, 1.0, 1.0, (1.0 - d * d).max(0.0).powi(2)]), 7.5, 7.5).expect("atlas");
        // Scorch: a dark, ragged blotch (value noise breaks up the circle).
        let scorch_size = 64u32;
        let mut scorch = Vec::with_capacity((scorch_size * scorch_size * 4) as usize);
        let hash = |x: i32, y: i32| ((x.wrapping_mul(374761393).wrapping_add(y.wrapping_mul(668265263)) as u32 ^ 0x5bd1e995).wrapping_mul(0x27d4eb2d) >> 24) as f32 / 255.0;
        for y in 0..scorch_size as i32 {
            for x in 0..scorch_size as i32 {
                let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
                let d = (dx * dx + dy * dy).sqrt() / 31.5;
                let n = hash(x / 4, y / 4) * 0.5 + hash(x / 9, y / 9) * 0.5;
                let a = ((1.0 - d) * 1.6 - 0.35 + (n - 0.5) * 0.7).clamp(0.0, 1.0).powf(1.3);
                let v = 0.06 + n * 0.05;
                scorch.extend_from_slice(&[(v * 255.0) as u8, (v * 0.9 * 255.0) as u8, (v * 0.8 * 255.0) as u8, (a * 0.85 * 255.0) as u8]);
            }
        }
        let scorch_sprite = atlas.add_rgba(scorch_size, scorch_size, &scorch, 31.5, 31.5).expect("atlas");

        let track_sprite = atlas.add_rgba(32, 16, &ambient::track_pixels(32, 16), 15.5, 7.5).expect("atlas");
        // The soles' prints: one picture for each of a few proportions (a Flea's near-square foot,
        // a Thud's long one), each foot drawn with the nearest.
        let foot_sprites: Vec<(f32, SpriteId)> = ambient::FOOT_SHAPES
            .iter()
            .map(|&shape| {
                let (w, h) = ambient::foot_size(shape);
                (shape, atlas.add_rgba(w, h, &ambient::foot_pixels(w, h), (w as f32 - 1.0) * 0.5, (h as f32 - 1.0) * 0.5).expect("atlas"))
            })
            .collect();

        // Explosion animations: the weapons', and TA's own blasts for units dying without an
        // explosion weapon, their pieces and their wrecks.
        let anims::Explosions { anims, looks: anim_looks, weapon_anims, generic: generic_blasts } = anims::load(vfs, atlas, palette, sim);

        let weapon_models = sim
            .weapon_types
            .types
            .iter()
            .map(|w| {
                if w.def.model.is_empty() || !matches!(w.def.render_type, RenderType::Model | RenderType::Bomb) {
                    return None;
                }
                let models = renderer.models.as_mut()?;
                let name = w.def.model.to_ascii_lowercase();
                let m = ta_formats::model3do::Model3do::parse(&vfs.read_opt(&format!("objects3d/{name}.3do"))?).ok()?;
                let id = models.library.add(&name, &m, &models.textures, palette);
                // (The root at the projectile's place; the rest where they sit on it. Then the
                // body's middle on the flight's line, see `WeaponModel`.)
                let info = models.library.info(id);
                let rest = info.rest_pose();
                let names: Vec<&str> = info.pieces.iter().map(|p| p.name.as_str()).collect();
                let root = rest.first().copied().unwrap_or(Mat4::IDENTITY).inverse();
                Some(WeaponModel::new(id, rest.iter().map(|m| root * *m).collect(), &names, models.library.corners(id), w.def.smoke_trail))
            })
            .collect();
        let death_weapons = sim
            .weapon_types
            .types
            .iter()
            .map(|w| sim.types.types.iter().any(|t| t.def.explode_as == w.name || t.def.self_destruct_as == w.name))
            .collect();
        Effects {
            anims,
            anim_looks,
            generic_blasts,
            weapon_anims,
            weapon_models,
            flight_dirs: HashMap::new(),
            glow,
            flake,
            smoke,
            modern: false,
            active: Vec::new(),
            particles: Vec::new(),
            due_blasts: Vec::new(),
            trail_timer: HashMap::new(),
            rng: 0x1234_5678,
            additive: Vec::new(),
            smoke_sprites: Vec::new(),
            beams: Vec::new(),
            bolts: Default::default(),
            model_matrices: Vec::new(),
            model_ids: Vec::new(),
            viewer: None,
            debris: Vec::new(),
            view: None,
            eye: None,
            focus: None,
            detail: Detail::default(),
            timed_lights: Vec::new(),
            timed_heat: Vec::new(),
            heat: Vec::new(),
            lights: Vec::new(),
            scorch_sprite,
            scorches: Vec::new(),
            burning: Vec::new(),
            damage_fx: Default::default(),
            unit_fires: Vec::new(),
            tree_fires: Default::default(),
            tree_flames: Vec::new(),
            ripples: Vec::new(),
            rings: Vec::new(),
            shake: 0.0,
            reduced_flashes: false,
            decals: Vec::new(),
            ambient: ambient::Ambient { track_sprite: Some(track_sprite), foot_sprites, ..Default::default() },
            wake_points: Vec::new(),
            wake_trails: Vec::new(),
            nano: Vec::new(),
            nano_lights: Vec::new(),
            nano_pieces: HashMap::new(),
            bursts: Vec::new(),
            death_weapons,
            lava_world: false,
            remains: Vec::new(),
            pops: Vec::new(),
            sounds: Vec::new(),
            shells: Vec::new(),
            firework_due: Default::default(),
            volley: None,
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift32: visual randomness only, never used by the simulation.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Keeps a shrinking share of new particles as the pool fills, so huge battles degrade
    /// gracefully instead of stalling the frame.
    fn emit(&mut self, p: Particle) {
        let n = self.particles.len();
        let (soft, max) = ((PARTICLES_SOFT as f32 * self.detail.budget) as usize, (PARTICLES_MAX as f32 * self.detail.budget) as usize);
        if n >= soft {
            if n >= max {
                return;
            }
            let keep = (max - n) as f32 / (max - soft).max(1) as f32;
            if self.rand() > keep {
                return;
            }
        }
        self.particles.push(p);
    }

    fn rand_dir(&mut self) -> Vec3 {
        Vec3::new(self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn puff(&mut self, pos: Vec3, vel: Vec3, now: f32, life: f32, size0: f32, size1: f32, alpha: f32) {
        self.puff_tinted(pos, vel, now, life, size0, size1, alpha, [1.0; 3]);
    }

    /// Smoke of burning fuel and wrecks: dark, brown-black.
    #[allow(clippy::too_many_arguments)]
    fn dark_puff(&mut self, pos: Vec3, vel: Vec3, now: f32, life: f32, size0: f32, size1: f32, alpha: f32) {
        self.puff_tinted(pos, vel, now, life, size0, size1, alpha, DARK_SMOKE);
    }

    /// One of the smoke puff's shapes, at random.
    fn smoke_puff(&mut self) -> SpriteId {
        self.smoke[((self.rand() * 4.0) as usize).min(3)]
    }

    /// TA's explosion animation `anim` at `pos`, or with the perspective camera, in place of a
    /// fiery one, a fireball as big and as long-lived (`fireball_bursts`): magnified, TA's
    /// low-resolution painting showed its pixels.
    fn blast_anim(&mut self, anim: usize, pos: Vec3, fps: f32, scale: f32, brightness: f32, now: f32) {
        let look = self.anim_looks[anim];
        if self.modern && look.fire {
            let life = self.anims[anim].len() as f32 / fps;
            self.fireball(pos, look.radius * scale, life);
            return;
        }
        self.active.push(Anim { frames: anim, start: now, pos, fps, scale, brightness });
    }

    #[allow(clippy::too_many_arguments)]
    fn puff_tinted(&mut self, pos: Vec3, vel: Vec3, now: f32, life: f32, size0: f32, size1: f32, alpha: f32, tint: [f32; 3]) {
        let rotation = self.rand() * std::f32::consts::TAU;
        let sprite = self.smoke_puff();
        self.emit(Particle {
            sprite,
            pos,
            vel,
            start: now,
            life,
            size0,
            size1,
            alpha,
            additive: false,
            gravity: 0.0,
            tint,
            brightness: 1.0,
            rotation,
        });
    }

    /// Bubbles let out at `p` under the sea (at `sea`): they break the surface above it a moment
    /// later as a little froth. (What lies under the water is seen through it, which sprites are
    /// not: drawn on their way up they hung over the water as grey smudges.)
    fn bubbles(&mut self, p: Vec3, sea: f32, now: f32) {
        let depth = sea - p.y;
        if depth < -8.0 {
            return;
        }
        let rise = (depth.max(0.0) / 40.0).min(2.5);
        for k in 0..2 {
            if k == 1 && self.rand() < 0.5 {
                break;
            }
            let off = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * 8.0;
            let size = 2.0 + self.rand() * 2.0;
            let rotation = self.rand() * std::f32::consts::TAU;
            let (start, life) = (now + rise + self.rand() * 0.3, 0.8 + self.rand() * 0.5);
            let sprite = self.smoke_puff();
            self.emit(Particle {
                sprite,
                pos: Vec3::new(p.x, sea + 0.6, p.z) + off,
                vel: Vec3::ZERO,
                start,
                life,
                size0: size,
                size1: size * 3.0,
                alpha: 0.2,
                additive: false,
                gravity: 0.0,
                tint: [1.9, 1.95, 2.0],
                brightness: 1.0,
                rotation,
            });
        }
    }

    fn light(&mut self, pos: Vec3, now: f32, life: f32, radius: f32, color: [f32; 3]) {
        self.timed_lights.push(TimedLight { pos, color, radius, start: now, life });
    }

    fn heat(&mut self, pos: Vec3, now: f32, life: f32, radius: f32, strength: f32) {
        if self.timed_heat.len() < 256 {
            self.timed_heat.push(TimedHeat { pos, radius, strength, start: now, life });
        }
    }

    fn flash(&mut self, pos: Vec3, now: f32, life: f32, size: f32, tint: [f32; 3], brightness: f32) {
        if self.reduced_flashes { return; }
        self.particles.push(Particle {
            sprite: self.glow,
            pos,
            vel: Vec3::ZERO,
            start: now,
            life,
            size0: size,
            size1: size * 1.4,
            alpha: 1.0,
            additive: true,
            gravity: 0.0,
            tint,
            brightness,
            rotation: 0.0,
        });
    }

    /// Throws a piece of a dying unit's model (`model` is the unit's render model).
    pub fn spawn_debris(&mut self, model: Option<ModelId>, team: u32, surface: Option<std::sync::Arc<crate::scene::PieceSurface>>, sim: &Sim, e: &SimEvent, now: f32) {
        let SimEvent::PieceExplode { model_piece, pos, heading, flags, .. } = *e else { return };
        if self.viewer.is_some_and(|p| !sim.in_sight(p, pos)) {
            return;
        }
        let p = Vec3::from_array(pos.to_f32());
        // A small blast graphic for every exploding piece (all of it for BITMAPONLY).
        let key = if flags & 0x1F00 != 0 { "fx/explode3" } else { "fx/explode2" };
        if let Some(&a) = self.generic_blasts.get(key) {
            self.blast_anim(a, p, 24.0, 0.6, 1.0, now);
        }
        let Some(model) = model else { return };
        let Some(surface) = surface else { return };
        if flags & BITMAPONLY != 0 {
            return;
        }
        let n = self.debris.len();
        let (soft, max) = ((DEBRIS_SOFT as f32 * self.detail.budget) as usize, (DEBRIS_MAX as f32 * self.detail.budget) as usize);
        if n >= soft && (n >= max || self.rand() > (max - n) as f32 / (max - soft).max(1) as f32) {
            return;
        }
        let out = self.rand_dir();
        let speed = 50.0 + self.rand() * 90.0;
        let vel = Vec3::new(out.x * speed, 110.0 + self.rand() * 160.0, out.z * speed);
        let axis = self.rand_dir().normalize_or(Vec3::X);
        let spin = 3.0 + self.rand() * 9.0;
        self.debris.push(Debris {
            model,
            piece: model_piece,
            matrix: Mat4::ZERO,
            surface,
            pos: p,
            vel,
            heading: heading as f32 * std::f32::consts::TAU / 65536.0,
            axis,
            spin,
            angle: 0.0,
            start: now,
            flags,
            landed: None,
            trail: 0.0,
            team,
            damage: DEBRIS_DAMAGE.0,
        });
    }

    /// This frame's point lights: fading timed lights, glowing projectiles and burning debris;
    /// and the heat haze over blasts and burning wrecks.
    fn collect_lights(&mut self, sim: &Sim, now: f32, alpha: f32) {
        self.heat.clear();
        self.timed_heat.retain(|h| now - h.start < h.life);
        for h in &self.timed_heat {
            let t = ((now - h.start) / h.life).clamp(0.0, 1.0);
            self.heat.push((h.pos, h.radius * (0.7 + 0.5 * t), h.strength * (1.0 - t)));
        }
        for b in &self.burning {
            let age = now - b.start;
            if age < b.fire {
                let fade = 1.0 - age / b.fire;
                self.heat.push((b.pos, if b.big { 70.0 } else { 42.0 }, fade * if b.big { 0.45 } else { 0.3 }));
            }
        }
        for &(p, size, k) in &self.unit_fires {
            self.heat.push((p, (size * 1.6).clamp(24.0, 90.0), 0.28 * k));
        }
        self.lights.clear();
        self.timed_lights.retain(|l| now - l.start < l.life);
        for l in &self.timed_lights {
            let t = ((now - l.start) / l.life).clamp(0.0, 1.0);
            let fade = (1.0 - t) * (1.0 - t);
            self.lights.push(ta_render::PointLight::new(l.pos.to_array(), l.color.map(|c| c * fade), l.radius));
        }
        for p in in_flight(sim, alpha) {
            if self.viewer.is_some_and(|pl| !sim.in_sight(pl, p.pos)) {
                continue;
            }
            let w = &sim.weapon_types.types[p.weapon];
            let pos = Vec3::from_array(p.prev_pos.to_f32()).lerp(Vec3::from_array(p.pos.to_f32()), alpha);
            // (The air shimmers over the hottest of them.)
            match w.def.render_type {
                RenderType::Disintegrator => self.heat.push((pos, 60.0, 0.5)),
                RenderType::Flame => self.heat.push((pos, 30.0, 0.35)),
                RenderType::Model if w.def.smoke_trail => self.heat.push((pos - Vec3::from_array(p.vel.to_f32()).normalize_or(Vec3::Z) * 10.0, 18.0, 0.25)),
                _ => {}
            }
            let (color, radius) = match w.def.render_type {
                RenderType::Laser | RenderType::Lightning => {
                    // (A bolt in a crowd lights the ground as faintly as it is drawn, and not in
                    // the strokes it does not show in.)
                    let k = self.bolts.get(&p.id).map_or(1.0, |b| if b.shown { b.glow } else { 0.0 });
                    if k <= 0.0 {
                        continue;
                    }
                    let c = weapon_color(w, sim).map(|c| c * k);
                    // The beam lights the ground along its length, not just at its head.
                    let vel = Vec3::from_array(p.vel.to_f32());
                    let shape = LaserShape::of(w);
                    let len = shape.length.min(travelled(p, alpha));
                    let middle = pos - vel.normalize_or(Vec3::Z) * len * 0.55;
                    let reach = 1.0 + 0.6 * shape.heavy;
                    self.lights.push(ta_render::PointLight::new(middle.to_array(), [c[0] * 1.1, c[1] * 1.1, c[2] * 1.1], 70.0 * reach));
                    ([c[0] * 1.8, c[1] * 1.8, c[2] * 1.8], 60.0 * reach)
                }
                RenderType::Disintegrator => ([6.0, 3.2, 1.2], 140.0),
                RenderType::Flame => ([3.0, 1.4, 0.4], 70.0),
                RenderType::Shell | RenderType::Point => ([1.6, 1.0, 0.45], 40.0 * shell_size(w).powf(0.7)),
                RenderType::Model if w.def.smoke_trail => ([2.0, 1.2, 0.5], 50.0),
                _ => continue,
            };
            self.lights.push(ta_render::PointLight::new(pos.to_array(), color, radius));
        }
        // Burning units: their firelight, flickering.
        for &(p, size, k) in &self.unit_fires {
            let flicker = 0.7 + 0.3 * ((now * 15.0 + p.x * 0.37 + p.z * 0.11).sin() * 0.5 + 0.5);
            let i = k * flicker;
            self.lights.push(ta_render::PointLight::new((p + Vec3::Y * 8.0).to_array(), [1.7 * i, 0.72 * i, 0.2 * i], (50.0 + size * 2.0).min(150.0)));
        }
        for b in &self.burning {
            let age = now - b.start;
            if age < b.fire {
                let fade = 1.0 - age / b.fire;
                let flicker = 0.7 + 0.3 * ((now * 17.0 + b.start * 13.0).sin() * 0.5 + 0.5);
                let k = fade * flicker * if b.big { 1.6 } else { 1.0 };
                self.lights.push(ta_render::PointLight::new((b.pos + Vec3::Y * 12.0).to_array(), [1.9 * k, 0.8 * k, 0.2 * k], if b.big { 150.0 } else { 90.0 }));
            }
        }
        // Nano streams glow green on what they build.
        for &(spot, radius) in &self.nano_lights {
            let flicker = 0.85 + 0.15 * ((now * 11.0 + spot.x * 0.1).sin() * 0.5 + 0.5);
            self.lights.push(ta_render::PointLight::new((spot + Vec3::Y * 10.0).to_array(), [0.35 * flicker, 1.5 * flicker, 0.55 * flicker], 50.0 + radius * 1.5).without_shadows());
        }
        for d in self.debris.iter().filter(|d| d.flags & FIRE != 0 && d.landed.is_none()) {
            let flicker = 0.75 + 0.25 * ((now * 23.0 + d.start * 7.0).sin() * 0.5 + 0.5);
            self.lights.push(ta_render::PointLight::new(d.pos.to_array(), [2.5 * flicker, 1.0 * flicker, 0.25 * flicker], 50.0));
        }
        // Burning trees: firelight and shimmering air.
        for &i in sim.burning_features() {
            let Some(f) = sim.features.get(i).filter(|f| f.alive) else { continue };
            let p = self.tree_flame(i).map_or(Vec3::from_array(f.pos.to_f32()), |c| c - Vec3::Y * 24.0);
            if self.view.is_some_and(|(lo, hi)| p.x < lo.x - 96.0 || p.x > hi.x + 96.0 || p.z < lo.y - 96.0 || p.z > hi.y + 96.0) || !self.sees(sim, p) {
                continue;
            }
            let flicker = 0.7 + 0.3 * ((now * 15.0 + i as f32 * 1.7).sin() * 0.5 + 0.5);
            self.lights.push(ta_render::PointLight::new((p + Vec3::Y * 24.0).to_array(), [1.5 * flicker, 0.62 * flicker, 0.16 * flicker], 85.0));
            self.heat.push((p + Vec3::Y * 12.0, 30.0, 0.3));
        }
        self.remains_lights(now);
    }

    /// A nuclear detonation: blinding flash, ground shockwave, rising fireball and a
    /// mushroom cloud of smoke, plus camera shake.
    fn nuclear_blast(&mut self, p: Vec3, radius: f32, now: f32) {
        // Light: a hard pulse over the whole area, then a long orange afterglow. (Bright, but
        // short of white: the fireball and the shockwave have to read through it.)
        self.light(p + Vec3::Y * 120.0, now, 1.0, (radius * 3.5).min(1800.0), [7.0, 5.2, 3.6]);
        self.light(p + Vec3::Y * 160.0, now + 0.3, 7.0, (radius * 2.4).min(1300.0), [3.2, 1.3, 0.35]);
        self.heat(p, now, 9.0, (radius * 1.4).min(700.0), 1.0);
        // A white-hot core, brief.
        self.flash(p + Vec3::Y * 30.0, now, 0.3, radius * 0.8, [1.0, 0.9, 0.75], 3.0);
        // Its shockwaves, running out over the ground.
        self.ripple(p, now, 1.8, radius * 2.8, [3.0, 2.0, 1.1], 0.25, 34.0);
        self.ripple(p, now, 1.8, radius * 2.8, [4.0, 3.2, 2.4], 0.7, 10.0);
        self.ripple(p, now + 0.2, 2.6, radius * 1.7, [3.0, 2.2, 1.5], 0.55, 6.0);
        let rotation = self.rand() * std::f32::consts::TAU;
        self.scorches.push(Scorch { pos: p, size: radius * 2.2, rotation, start: now });
        // Fireball: saturated orange-red billows rising into the stem.
        for i in 0..28 {
            let t = i as f32 / 28.0;
            let lateral = self.rand_dir() * radius * 0.3;
            let rise = Vec3::new(lateral.x * 0.12, 40.0 + t * 50.0, lateral.z * 0.12);
            let rotation = self.rand() * std::f32::consts::TAU;
            let heat = self.rand();
            let sprite = self.smoke_puff();
            self.emit(Particle {
                sprite,
                pos: p + Vec3::new(lateral.x * 0.6, 12.0, lateral.z * 0.6),
                vel: rise,
                start: now + t * 0.5,
                life: 3.2,
                size0: radius * 0.35,
                size1: radius * 0.8,
                alpha: 0.95,
                additive: true,
                gravity: 0.0,
                tint: [1.0, 0.35 + heat * 0.25, 0.08],
                brightness: 1.7,
                rotation,
            });
        }
        // Dust wall racing out with the shockwave.
        for i in 0..48 {
            let a = i as f32 / 48.0 * std::f32::consts::TAU + self.rand() * 0.1;
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let speed = radius * (0.9 + self.rand() * 0.3);
            let rotation = self.rand() * std::f32::consts::TAU;
            let sprite = self.smoke_puff();
            self.emit(Particle {
                sprite,
                pos: p + dir * radius * 0.15 + Vec3::Y * 10.0,
                vel: dir * speed + Vec3::Y * 8.0,
                start: now + 0.05,
                life: 2.6,
                size0: radius * 0.18,
                size1: radius * 0.55,
                alpha: 0.6,
                additive: false,
                gravity: 0.0,
                tint: [0.78, 0.68, 0.55],
                brightness: 1.0,
                rotation,
            });
        }
        // Mushroom cap: dark smoke billowing high above, lingering.
        for _ in 0..26 {
            let a = self.rand() * std::f32::consts::TAU;
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let pos = p + Vec3::Y * radius * 0.9 + dir * radius * 0.35 * self.rand();
            let vel = dir * 18.0 + Vec3::Y * 12.0;
            let delay = 1.0 + self.rand() * 0.8;
            let life = 10.0 + self.rand() * 4.0;
            self.dark_puff(pos, vel, now + delay, life, radius * 0.35, radius * 0.9, 0.55);
        }
        self.shake = self.shake.max((radius / 12.0).min(40.0));
    }

    /// Scorch marks fade out; burning wrecks emit flames and embers, then a thinning smoke
    /// column.
    /// A wreck set burning: at most [`MOST_BURNING`] at once, the oldest giving way.
    fn burn(&mut self, b: Burning) {
        if self.burning.len() >= MOST_BURNING {
            self.burning.remove(0);
        }
        self.burning.push(b);
    }

    fn update_aftermath(&mut self, sim: &Sim, now: f32) {
        self.decals.clear();
        self.update_ripples(now);
        self.scorches.retain(|s| now - s.start < SCORCH_LIFE);
        for s in &self.scorches {
            let fade = (1.0 - (now - s.start) / SCORCH_LIFE).clamp(0.0, 1.0).powf(0.6);
            self.decals.push(SpriteInstance::new([s.pos.x, s.pos.y + 0.15, s.pos.z], self.scorch_sprite).scale(s.size / 64.0).alpha(fade).rotation(s.rotation));
        }
        self.burning.retain(|b| now - b.start < b.smoulder);
        let mut puffs = Vec::new();
        for b in &mut self.burning {
            let age = now - b.start;
            if now < b.next_puff {
                continue;
            }
            let fire = age < b.fire;
            // Denser early, sparser as the wreck cools.
            b.next_puff = now + if fire { 0.12 } else { 0.25 + age / b.smoulder * 0.6 };
            puffs.push((b.pos, fire, b.big, age / b.smoulder));
        }
        // (Made only near the view: a wreck's smoke far off took particles from the battle in
        // sight. Its column builds up again as the camera comes to it.)
        let view = self.view;
        for (pos, fire, big, aged) in puffs {
            if view.is_some_and(|(lo, hi)| pos.x < lo.x - 300.0 || pos.x > hi.x + 300.0 || pos.z < lo.y - 300.0 || pos.z > hi.y + 300.0) {
                continue;
            }
            if self.sees(sim, pos) {
                self.wreck_fire_particles(pos, big, fire);
            }
            let jitter = Vec3::new(self.rand() * 10.0 - 5.0, 4.0, self.rand() * 10.0 - 5.0);
            if fire {
                let rotation = self.rand() * std::f32::consts::TAU;
                self.emit(Particle {
                    sprite: self.glow,
                    pos: pos + jitter,
                    vel: Vec3::new(0.0, 22.0, 0.0),
                    start: now,
                    life: 0.5,
                    size0: if big { 26.0 } else { 16.0 },
                    size1: 6.0,
                    alpha: 0.85,
                    additive: true,
                    gravity: 0.0,
                    tint: [1.0, 0.45, 0.12],
                    brightness: 2.6,
                    rotation,
                });
            }
            let thin = (1.0 - aged).clamp(0.2, 1.0);
            let (s0, s1) = if big { (22.0, 90.0) } else { (14.0, 55.0) };
            let drift = Vec3::new(self.rand() * 6.0 - 3.0, 26.0, self.rand() * 6.0 - 3.0);
            self.dark_puff(pos + jitter, drift, now, 3.5, s0, s1, 0.5 * thin);
        }
    }

    /// Where a burning tree drawn in 3D burns: the middle of its crown.
    fn tree_flame(&self, index: usize) -> Option<Vec3> {
        self.tree_flames.iter().find(|f| f.0 == index).map(|f| f.1)
    }

    /// Burning trees in view: embers and licks of flame rising from the crown, and smoke.
    fn update_tree_fires(&mut self, sim: &Sim, now: f32) {
        let fires: Vec<usize> = sim.burning_features().to_vec();
        self.tree_fires.retain(|i, _| fires.contains(i));
        let view = self.view;
        for i in fires {
            let Some(f) = sim.features.get(i).filter(|f| f.alive) else { continue };
            let def = &sim.data.features[f.def];
            let p = self.tree_flame(i).unwrap_or_else(|| Vec3::from_array(f.pos.to_f32()) + Vec3::Y * (def.height as f32 * 0.55).max(8.0));
            if view.is_some_and(|(lo, hi)| p.x < lo.x - 64.0 || p.x > hi.x + 64.0 || p.z < lo.y - 64.0 || p.z > hi.y + 64.0) || !self.sees(sim, p) {
                continue;
            }
            let next = self.tree_fires.entry(i).or_insert(now);
            if now < *next {
                continue;
            }
            *next = now + 0.15;
            // (As big as the crown: about as wide as the tree's footprint, at least a few cells.)
            let crown = (def.footprint_x.max(def.footprint_z) as f32 * 9.0).clamp(14.0, 40.0);
            self.tree_fire_particles(p, crown);
            let drift = Vec3::new(self.rand() * 8.0 - 4.0, 24.0, self.rand() * 8.0 - 4.0);
            self.dark_puff(p + Vec3::Y * crown * 0.6, drift, now, 3.5, crown * 0.8, crown * 3.0, 0.4);
        }
    }

    /// Integrates flying debris; trails smoke/fire and settles pieces on the ground.
    fn update_debris(&mut self, sim: &Sim, now: f32, dt: f32) {
        let mut debris = std::mem::take(&mut self.debris);
        debris.retain(|d| now - d.start < 6.0 && d.landed.is_none_or(|t| now - t < 2.5));
        // Share one terrain snapshot for the pass, without a global height-field lock for
        // every contact vertex in a large battle. Ascending pieces need no contact search.
        let ground = crate::scene::Ground::of(&sim.world);
        let sea_floor = sim.world.sea_level.to_f32() - 30.0;
        for d in &mut debris {
            d.damage = DEBRIS_DAMAGE.0 + (DEBRIS_DAMAGE.1 - DEBRIS_DAMAGE.0) * ((now - d.start) / 3.0).min(1.0);
            if d.landed.is_none() {
                let g = if d.flags & FALL != 0 { 300.0 } else { 220.0 };
                d.vel.y -= g * dt;
                d.pos += d.vel * dt;
                d.angle += d.spin * dt;
                let (clearance, contact) = if d.vel.y < 0.0 && !ground.clear_above(d.pos, d.surface.radius, sea_floor) {
                    debris_contact(&d.surface.points, d.transform(), |x, z| ground.drawn(x, z).max(sea_floor))
                } else {
                    (f32::INFINITY, Vec3::ZERO)
                };
                if clearance <= 0.0 {
                    d.pos.y -= clearance;
                    if d.flags & EXPLODE_ON_HIT != 0 || d.vel.y < -260.0 && d.flags & FIRE != 0 {
                        if let Some(&a) = self.generic_blasts.get("fx/explode2") {
                            self.blast_anim(a, contact, 22.0, 0.7, 2.2, now);
                        }
                        self.blast_particles(contact, 14.0, contact.y, false, Blast::Shell);
                        d.landed = Some(now - 2.0);
                    } else if d.vel.y > -90.0 {
                        d.landed = Some(now);
                    } else {
                        // Bounce with friction.
                        d.vel = Vec3::new(d.vel.x * 0.45, -d.vel.y * 0.3, d.vel.z * 0.45);
                        d.spin *= 0.5;
                    }
                }
                d.trail += dt;
            } else if let Some(t) = d.landed {
                // Rest, then sink into the ground.
                let k = ((now - t) - 1.6).max(0.0);
                d.pos.y -= k * 12.0 * dt;
            }
        }
        // Trails.
        for d in &mut debris {
            if d.landed.is_some() || d.trail < 0.05 {
                continue;
            }
            d.trail = 0.0;
            if d.flags & FIRE != 0 {
                self.debris_trail_particles(d.pos, d.vel);
                let rotation = self.rand() * std::f32::consts::TAU;
                self.emit(Particle {
                    sprite: self.glow,
                    pos: d.pos,
                    vel: Vec3::new(0.0, 8.0, 0.0),
                    start: now,
                    life: 0.35,
                    size0: 14.0,
                    size1: 6.0,
                    alpha: 0.9,
                    additive: true,
                    gravity: 0.0,
                    tint: [1.0, 0.5, 0.15],
                    brightness: 3.0,
                    rotation,
                });
            }
            if d.flags & (SMOKE | FIRE) != 0 {
                let dark = d.flags & FIRE != 0;
                let rotation = self.rand() * std::f32::consts::TAU;
                let sprite = self.smoke_puff();
                self.emit(Particle {
                    sprite,
                    pos: d.pos,
                    vel: Vec3::new(0.0, 10.0, 0.0),
                    start: now,
                    life: 1.3,
                    size0: 8.0,
                    size1: 26.0,
                    alpha: 0.5,
                    additive: false,
                    gravity: 0.0,
                    tint: if dark { [0.35, 0.33, 0.3] } else { [0.8, 0.8, 0.8] },
                    brightness: 1.0,
                    rotation,
                });
            }
        }
        for d in &mut debris {
            d.matrix = d.transform();
        }
        self.debris = debris;
    }

    /// Reacts to simulation events.
    pub fn on_event(&mut self, e: &SimEvent, sim: &Sim, now: f32) {
        // A projectile's blast goes off as the projectile drawn gets to where it hit: the world
        // is drawn a tick behind the game (between the last tick and this one), and the
        // projectile `when` of the way through this tick's flight.
        if let SimEvent::Explosion { when, .. } = e
            && *when > ta_sim::Fx::ZERO
        {
            self.due_blasts.push((now + when.to_f32() / 30.0, e.clone()));
            return;
        }
        self.happened(e, sim, now);
    }

    /// An event's effects, `now` its moment.
    fn happened(&mut self, e: &SimEvent, sim: &Sim, now: f32) {
        if let (Some(player), Some(pos)) = (self.viewer, event_position(e, sim)) && !sim.in_sight(player, pos) && !ours(e, sim, player) {
            return;
        }
        match e {
            SimEvent::Explosion { weapon, pos, water, .. } => {
                let p = Vec3::from_array(pos.to_f32());
                let w = &sim.weapon_types.types[*weapon];
                // (In the sea of a lava world, the weapon's lava splash.)
                let lava = *water && self.lava_world;
                let set = self.weapon_anims.get(*weapon).copied().unwrap_or([None; 3]);
                let anim = if lava { set[2].or(set[0]) } else if *water { set[1].or(set[0]) } else { set[0] };
                let radius = w.aoe_radius.to_f32();
                let big = radius > 40.0;
                let ground = crate::scene::drawn_ground(&sim.world, p.x, p.z);
                // (The perspective camera's splashes are spray and molten droplets, see
                // `blast_bursts`.)
                if let Some(a) = anim.filter(|_| !(self.modern && *water)) {
                    self.blast_anim(a, p, 22.0, 1.0, 1.0, now);
                }
                // Light flash, sparks and lingering smoke scale with the blast.
                // (A battle sets off a dozen at once, over pale sand as often as not: kept small,
                // brief and below white, the flashes and the light they cast mark each blast
                // without washing the whole fight out; a fireball, with the perspective camera,
                // carries most of the blast's light itself.)
                let size = (radius * if self.modern { 0.9 } else { 1.2 }).clamp(16.0, 220.0);
                // (In the weapon's colour: a green laser's hit is green, an EMP's blue.)
                let tint = impacts::impact_tint(w, sim);
                self.flash(p + Vec3::Y * 4.0, now, if big { 0.22 } else { 0.1 }, size, tint, if big { 1.6 } else { 1.25 });
                let beam = matches!(w.def.render_type, RenderType::Laser | RenderType::Lightning);
                let on_ground = !water && p.y - ground < 12.0;
                // (An EMP burns nothing: no scorch, no smoke, no shimmering air.)
                let emp = w.def.paralyzer;
                if on_ground && !emp && (radius >= 12.0 || beam) {
                    let rotation = self.rand() * std::f32::consts::TAU;
                    // (A beam's hit burns a small mark where a shell's blast scorches wide.)
                    let mark = if radius >= 12.0 { (radius * 2.2).clamp(24.0, 260.0) } else { 12.0 + radius };
                    self.scorches.push(Scorch { pos: Vec3::new(p.x, ground, p.z), size: mark, rotation, start: now });
                    if self.scorches.len() > MAX_SCORCHES {
                        self.scorches.remove(0);
                    }
                }
                if on_ground && !emp && radius >= 40.0 {
                    self.shockwave(Vec3::new(p.x, ground, p.z), radius, now);
                }
                if *water && !lava {
                    self.splash_ripples(p, sim.world.sea_level.to_f32(), radius, now);
                }
                // Nuclear missiles, fusion plants, Commanders and crawling bombs (a factory
                // blowing itself up, 200 across, is only a very big blast), and a mod's smaller
                // nuclear weapons (see `nuclear`); an EMP missile pulses instead.
                if w.def.paralyzer && radius >= 60.0 {
                    self.emp_pulse(p, radius, now);
                } else if nuclear(w) {
                    self.nuclear_blast(p, radius, now);
                    self.nuke_particles(p, radius, ground);
                }
                self.weapon_shake(w, p);
                let (light_radius, light_life) = ((radius * 3.0).clamp(50.0, 240.0), if big { 0.5 } else { 0.25 });
                let heat = if *water { [1.2, 1.4, 1.6] } else if tint == [1.0, 0.62, 0.3] { [1.7, 0.72, 0.2] } else { tint.map(|c| c * 1.7) };
                self.light(p + Vec3::Y * 20.0, now, light_life, light_radius, heat);
                if !water && !emp && radius >= 16.0 {
                    self.heat(p, now, if big { 2.2 } else { 1.2 }, (radius * 2.5).clamp(30.0, 220.0), if big { 0.9 } else { 0.5 });
                }
                // Sparks, flames, embers, grit and dust (GPU particles), by what went off.
                let blast = match w.def.render_type {
                    _ if self.death_weapons.get(*weapon).copied().unwrap_or(false) => Blast::Death,
                    RenderType::Disintegrator => Blast::Death,
                    RenderType::Laser | RenderType::Lightning => Blast::Beam(beam_color(weapon_color(w, sim))),
                    _ if w.def.paralyzer => Blast::Beam([0.55, 0.72, 1.0]),
                    RenderType::Flame => Blast::Flame,
                    _ => Blast::Shell,
                };
                self.blast_particles(p, radius, ground, *water, blast);
                if !water && !emp {
                    let n = if big { 8 } else { 2 };
                    for _ in 0..n {
                        let off = self.rand_dir() * radius * 0.4;
                        let v = Vec3::new(self.rand() * 16.0 - 8.0, 14.0 + self.rand() * 10.0, self.rand() * 16.0 - 8.0);
                        let life = 2.5 + self.rand() * 2.0;
                        self.dark_puff(p + off, v, now + 0.1, life, radius.max(16.0) * 0.8, radius.max(16.0) * 2.2, 0.55);
                    }
                }
            }
            SimEvent::UnitExploded { pos, big, .. } => {
                let p = Vec3::from_array(pos.to_f32());
                let ground = crate::scene::drawn_ground(&sim.world, p.x, p.z);
                let sea = sim.world.sea_level.to_f32();
                self.blast_particles(p, if *big { 90.0 } else { 40.0 }, ground, ground < sea - 2.0 && p.y < sea + 6.0, Blast::Death);
                // (Its wreck's fire, if `unit_died` lit one already, burns as long as this.)
                match self.burning.iter_mut().find(|b| b.pos.distance(p) < 8.0 && (now - b.start).abs() < 1.0) {
                    Some(b) => {
                        (b.fire, b.smoulder, b.big) = (b.fire.max(FIRE_TIME), b.smoulder.max(SMOULDER_TIME), b.big || *big);
                    }
                    None => self.burn(Burning { pos: p, start: now, big: *big, next_puff: now, fire: FIRE_TIME, smoulder: SMOULDER_TIME }),
                }
                let rotation = self.rand() * std::f32::consts::TAU;
                self.scorches.push(Scorch { pos: p, size: if *big { 150.0 } else { 70.0 }, rotation, start: now });
                let key = if *big { "fx/explode4" } else { "fx/explode3" };
                if let Some(&a) = self.generic_blasts.get(key) {
                    self.blast_anim(a, p + Vec3::Y * 8.0, 20.0, if *big { 2.0 } else { 1.2 }, 1.1, now);
                }
                self.flash(p + Vec3::Y * 10.0, now, 0.18, if *big { 120.0 } else { 55.0 }, [1.0, 0.6, 0.3], 1.8);
                self.light(p + Vec3::Y * 24.0, now, if *big { 0.8 } else { 0.5 }, if *big { 280.0 } else { 160.0 }, [2.2, 0.95, 0.25]);
                self.heat(p, now, if *big { 2.5 } else { 1.6 }, if *big { 130.0 } else { 70.0 }, if *big { 1.0 } else { 0.7 });
                for _ in 0..6 {
                    let off = self.rand_dir() * 16.0;
                    let life = 3.0 + self.rand() * 2.0;
                    self.dark_puff(p + off, Vec3::new(0.0, 18.0, 0.0), now + 0.15, life, 30.0, 80.0, 0.6);
                }
            }
            SimEvent::WeaponFired { weapon, pos, target, .. } => {
                let w = &sim.weapon_types.types[*weapon];
                let p = Vec3::from_array(pos.to_f32());
                if matches!(w.def.render_type, RenderType::Shell | RenderType::Point | RenderType::Disintegrator) {
                    let big = matches!(w.def.render_type, RenderType::Disintegrator) || w.aoe_radius.to_f32() >= 30.0;
                    self.muzzle_particles(p, Vec3::from_array(target.to_f32()), big);
                }
                if matches!(w.def.render_type, RenderType::Shell | RenderType::Point) {
                    let dir = (Vec3::from_array(target.to_f32()) - p).normalize_or(Vec3::Z);
                    self.muzzle_blast(p, dir, shell_size(w), now);
                }
                let tint = weapon_color(w, sim);
                let (size, light_radius) = match w.def.render_type {
                    RenderType::Disintegrator => (34.0, 150.0),
                    RenderType::Laser | RenderType::Lightning => (9.0, 55.0),
                    RenderType::Bomb => (0.0, 0.0),
                    _ => (14.0, 80.0),
                };
                if w.def.stockpile && !w.def.interceptor {
                    // A missile leaving its silo: a column of exhaust and a glare.
                    for i in 0..14 {
                        let v = Vec3::new(self.rand() * 30.0 - 15.0, 30.0 + self.rand() * 40.0, self.rand() * 30.0 - 15.0);
                        self.puff(p + Vec3::Y * (i as f32 * 3.0), v, now + i as f32 * 0.03, 4.0, 30.0, 110.0, 0.5);
                    }
                    self.light(p + Vec3::Y * 20.0, now, 1.2, 260.0, [7.0, 4.5, 2.0]);
                }
                if size > 0.0 {
                    self.flash(p, now, 0.07, size, tint, 2.5);
                    let c = if matches!(w.def.render_type, RenderType::Laser | RenderType::Lightning) { tint } else { [1.0, 0.7, 0.3] };
                    self.light(p, now, 0.09, light_radius, [c[0] * 5.0, c[1] * 5.0, c[2] * 5.0]);
                }
                if w.def.start_smoke {
                    self.puff(p, Vec3::new(0.0, 10.0, 0.0), now, 1.2, 8.0, 26.0, 0.45);
                }
            }
            SimEvent::Intercepted { pos, .. } => self.interception(Vec3::from_array(pos.to_f32()), now),
            SimEvent::Sfx { pos, kind, .. }
                // A submarine's or an underwater plant's bubbles (256|3, TA's SFXTYPE_SUBBUBBLES).
                if *kind == (256 | 3) => {
                    self.bubbles(Vec3::from_array(pos.to_f32()), sim.world.sea_level.to_f32(), now);
                }
            SimEvent::Sfx { pos, kind, .. }
                // Damage smoke from SmokeUnit scripts: 256|1 white, 256|2 black.
                if *kind & 256 != 0 => {
                    let dark = *kind & 2 != 0;
                    let p = Vec3::from_array(pos.to_f32());
                    let rotation = self.rand() * std::f32::consts::TAU;
                    let sprite = self.smoke_puff();
                    self.emit(Particle {
                        sprite,
                        pos: p,
                        vel: Vec3::new(4.0, 22.0, -3.0),
                        start: now,
                        life: 2.2,
                        size0: 10.0,
                        size1: 36.0,
                        alpha: 0.5,
                        additive: false,
                        gravity: 0.0,
                        tint: if dark { [0.25, 0.24, 0.23] } else { [1.0, 1.0, 1.0] },
                        brightness: 1.0,
                        rotation,
                    });
                }
            _ => {}
        }
    }

    /// Builds this frame's effect sprites, beams and projectile models.
    pub fn update(&mut self, sim: &Sim, now: f32, dt: f32, alpha: f32) {
        if self.due_blasts.iter().any(|(at, _)| *at <= now) {
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.due_blasts).into_iter().partition(|(at, _)| *at <= now);
            self.due_blasts = later;
            for (at, e) in due {
                self.happened(&e, sim, at);
            }
        }
        self.additive.clear();
        self.smoke_sprites.clear();
        self.beams.clear();
        self.model_matrices.clear();
        self.model_ids.clear();

        // Explosion animations.
        self.active.retain(|a| (now - a.start) * a.fps < self.anims[a.frames].len() as f32);
        for a in &self.active {
            let frames = &self.anims[a.frames];
            let i = (((now - a.start) * a.fps) as usize).min(frames.len() - 1);
            // (TA's explosion art hides most of what lies behind it, as TA drew it: added to
            // pale sand it would glare white.)
            self.additive.push(SpriteInstance::new(a.pos.to_array(), frames[i]).scale(a.scale).brightness(a.brightness).cover(0.85));
        }

        // The lightning bolts in sight, each with how many crowd it (see `BoltLook`): their
        // middles in cells as wide as the crowding reach, each counting those in its cell and the
        // eight round it.
        self.bolts.clear();
        let middles: Vec<(u32, u32, Vec3)> = in_flight(sim, alpha)
            .filter(|p| matches!(sim.weapon_types.types[p.weapon].def.render_type, RenderType::Lightning))
            .filter(|p| !self.viewer.is_some_and(|pl| !sim.in_sight(pl, p.pos)))
            .map(|p| (p.id, p.age / 2, (bolt_start(p) + Vec3::from_array(p.pos.to_f32())) * 0.5))
            .collect();
        if !middles.is_empty() {
            let cell = |m: Vec3| ((m.x / BOLT_CROWD_RADIUS).floor() as i32, (m.z / BOLT_CROWD_RADIUS).floor() as i32);
            let mut cells: rustc_hash::FxHashMap<(i32, i32), Vec<Vec3>> = Default::default();
            for &(_, _, m) in &middles {
                cells.entry(cell(m)).or_default().push(m);
            }
            for &(id, stroke, m) in &middles {
                let (cx, cz) = cell(m);
                let mut crowd = 0;
                'count: for dz in -1..=1 {
                    for dx in -1..=1 {
                        for o in cells.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                            if o.distance_squared(m) < BOLT_CROWD_RADIUS * BOLT_CROWD_RADIUS {
                                crowd += 1;
                                if crowd >= BOLT_CROWD_MOST {
                                    break 'count;
                                }
                            }
                        }
                    }
                }
                self.bolts.insert(id, BoltLook::among(crowd).in_stroke(id, stroke));
            }
        }

        // Projectiles, and those that ended this tick on their way to where they hit.
        let mut trails = Vec::new();
        let tick = sim.tick;
        let mut flying = rustc_hash::FxHashSet::default();
        for p in in_flight(sim, alpha) {
            flying.insert(p.id);
            let vel = Vec3::from_array(p.vel.to_f32());
            // (Its velocity a tick ago, and now: see `flight_dirs`. Back in a replay, afresh.)
            let flight = self.flight_dirs.entry(p.id).or_insert((tick, vel, vel));
            if flight.0 != tick {
                *flight = if flight.0 < tick { (tick, flight.2, vel) } else { (tick, vel, vel) };
            }
            let turning = flight.1;
            let w = &sim.weapon_types.types[p.weapon];
            // (Out of sight, only a side's own missiles a defence shoots at, and its interceptors.)
            if self.viewer.is_some_and(|pl| !sim.in_sight(pl, p.pos) && !(crate::game::nuclear::strategic(w) && sim.allied(pl, p.owner_player))) {
                continue;
            }
            let prev = Vec3::from_array(p.prev_pos.to_f32());
            let cur = Vec3::from_array(p.pos.to_f32());
            let pos = prev.lerp(cur, alpha);
            let dir = turning.normalize_or(vel).lerp(vel.normalize_or(Vec3::Z), alpha).normalize_or(Vec3::Z);
            let tint = weapon_color(w, sim);
            match w.def.render_type {
                RenderType::Laser => {
                    // A bolt: a white-hot core in a glow of its colour (TA's `color2`, the heavy
                    // lasers' blue, the tachyon beams' violet; not so bright that the glow clips
                    // to a flat bar, or a green laser to yellow), its head rounded and hot, its
                    // tail fading out; as long as its speed over its time, never reaching back
                    // behind the barrel it left; the harder it hits, the broader and longer.
                    let shape = LaserShape::of(w);
                    let len = shape.length.min(travelled(p, alpha));
                    let width = shape.width;
                    let beam = BeamInstance::new(pos.to_array(), (pos - dir * len).to_array(), width * 2.0, tint.map(|c| c * 1.6), 1.0);
                    self.beams.push(beam.core(0.14 + 0.06 * shape.heavy).tapered(width, 0.6).cover(0.6));
                    self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(0.035 * width).tint(beam_color(tint)).brightness(2.5 + 1.5 * shape.heavy));
                    if shape.heavy > 0.0 {
                        // (A heavy beam's light about it as it passes.)
                        self.additive.push(SpriteInstance::new((pos - dir * len * 0.5).to_array(), self.glow).scale(0.05 * width).tint(beam_color(tint)).brightness(0.8 * shape.heavy));
                    }
                }
                RenderType::Lightning => {
                    // From the gun it was fired from (where the bolt left it) to its head, in
                    // jagged segments that keep their shape for two ticks at a time: it crackles,
                    // a fifteenth of a second a stroke, rather than taking a new shape every frame
                    // (which strobed at the frame rate). A glow round a hot core, as the storm's;
                    // fainter, and in some strokes only, in a crowd of bolts (see `BoltLook`).
                    let look = self.bolts.get(&p.id).copied().unwrap_or(BoltLook::among(1));
                    if !look.shown {
                        continue;
                    }
                    let start = bolt_start(p);
                    let stroke = p.age / 2;
                    let mut a = start;
                    let segments = 6;
                    for i in 1..=segments {
                        let t = i as f32 / segments as f32;
                        let jitter = if i == segments {
                            Vec3::ZERO
                        } else {
                            let k = |axis: u32| crackle(p.id, stroke, i as u32 * 3 + axis) - 0.5;
                            Vec3::new(k(0), k(1), k(2)) * 18.0
                        };
                        let b = start.lerp(pos, t) + jitter;
                        self.beams.push(BeamInstance::new(a.to_array(), b.to_array(), 8.0, [0.55, 0.65, 1.2], look.glow).cover(0.3));
                        self.beams.push(BeamInstance::new(a.to_array(), b.to_array(), 2.5, [2.5, 3.0, 6.0], look.core).core(0.4));
                        a = b;
                    }
                }
                RenderType::Disintegrator => {
                    self.dgun_particles(pos, dir, dt);
                    // (Its fireball roils, a few times a second; a new size every frame strobed.)
                    let seed = p.id as f32 * 1.7;
                    let flicker = 1.0 + 0.1 * (now * 23.0 + seed).sin() + 0.05 * (now * 37.0 + seed * 2.3).sin();
                    self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(0.9 * flicker).tint([1.0, 0.6, 0.25]).brightness(8.0));
                    self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(0.35).brightness(10.0));
                }
                RenderType::Model | RenderType::Bomb => {
                    let model = self.weapon_models.get(p.weapon).and_then(Option::as_ref);
                    // (Where its tail is, for its flame, and how thick it is.)
                    let (tail, girth) = model.map_or((-6.0, 6.0), |m| (m.tail, m.girth));
                    if let Some(m) = model {
                        // The model's nose (+z) along its flight, every piece in its place on it
                        // (one matrix each, as the model's shader reads them).
                        let at = Mat4::from_rotation_translation(flight_rotation(dir, Vec3::from_array(p.target_pos.to_f32()) - pos), pos);
                        let (id, matrices) = (m.model, m.pieces.iter().map(|piece| at * *piece).collect());
                        self.model_matrices.push(matrices);
                        self.model_ids.push(id);
                    } else {
                        self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(0.25).tint(tint).brightness(4.0));
                    }
                    // A torpedo's wake of bubbles under the water.
                    if w.def.water_weapon && pos.y < sim.world.sea_level.to_f32() - 2.0 && self.rand() < dt * 25.0 {
                        self.bubbles(pos, sim.world.sea_level.to_f32(), now);
                    }
                    if w.def.smoke_trail {
                        // Its exhaust from its tail, back along its body; its smoke from there on.
                        let climbing = w.vlaunch && p.age < w.launch_ticks;
                        let smoke = self.missile_flame(p.id, pos + dir * tail, dir, girth, climbing, now, dt);
                        trails.push((p.id, smoke));
                    }
                }
                RenderType::Flame => {
                    // (Growing as it flies, between the ticks too, not in steps.)
                    let s = 0.25 + ((p.age as f32 - 1.0 + alpha).max(0.0) * 0.04).min(0.8);
                    self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(s).tint([1.0, 0.45, 0.12]).brightness(4.0));
                    // Its stream full of rolling flame (about forty a second each).
                    if self.rand() < dt * 40.0 {
                        self.burst(ParticleBurst::new(ta_render::ParticleKind::Fire, pos.to_array(), 1).radius(3.0 + s * 8.0).velocity((dir * 30.0).to_array()).speed(4.0, 14.0).life(0.2, 0.4).size(8.0 + s * 14.0, 3.0).gravity(-30.0));
                    }
                }
                _ => {
                    // Plasma shells: hot glowing ball with a short tail (orange, not white: a
                    // battle has dozens in the air at once), as big as the gun that fired it: a
                    // light tank's spark, a Bertha's or an Intimidator's fireball.
                    let size = shell_size(w);
                    self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(0.2 * size).tint([1.0, 0.55, 0.22]).brightness(3.0));
                    // (Its tail narrows and fades out behind it, a comet's, not a stick ending
                    // in a square cut; never reaching back behind the barrel it left.)
                    let tail = (20.0 * size).min(travelled(p, alpha));
                    self.beams.push(BeamInstance::new(pos.to_array(), (pos - dir * tail).to_array(), 4.0 * size, [2.0, 0.95, 0.3], 0.7).tapered(2.0 * size, 1.0));
                }
            }
        }
        self.update_debris(sim, now, dt);
        self.update_remains(sim, now, dt);
        self.update_aftermath(sim, now);
        self.update_damage(sim, now);
        self.update_tree_fires(sim, now);
        self.track_decals(now);
        self.update_ambient(sim, now, dt, alpha);
        self.update_nano(sim, now, dt);
        self.collect_lights(sim, now, alpha);

        // Missile smoke trails: a puff every ~40 ms per projectile, where its flame ends.
        let mut live = std::collections::HashSet::new();
        for (id, behind) in trails {
            live.insert(id);
            let t = self.trail_timer.entry(id).or_insert(0.0);
            *t += dt;
            let mut puffs = 0;
            while *t > 0.04 {
                *t -= 0.04;
                puffs += 1;
            }
            for _ in 0..puffs {
                let sprite = self.smoke_puff();
                self.emit(Particle {
                    sprite,
                    pos: behind,
                    vel: Vec3::new(0.0, 6.0, 0.0),
                    start: now,
                    life: 1.2,
                    size0: 5.0,
                    size1: 18.0,
                    alpha: 0.3,
                    additive: false,
                    gravity: 0.0,
                    tint: [0.8, 0.8, 0.82],
                    brightness: 1.0,
                    rotation: 0.0,
                });
            }
        }
        self.trail_timer.retain(|id, _| live.contains(id));
        self.flight_dirs.retain(|id, _| flying.contains(id));

        // Particles: all move (smoke drifting on the wind), those in view are drawn.
        self.particles.retain(|p| now - p.start < p.life);
        let view = self.view;
        let wind = self.wind(0.2) * SMOKE_WIND;
        for p in &mut self.particles {
            let age = now - p.start;
            if age < 0.0 {
                continue;
            }
            p.pos += p.vel * dt;
            if !p.additive {
                p.pos.x += wind.x * dt;
                p.pos.z += wind.y * dt;
            }
            if !p.additive {
                p.vel *= 1.0 - (0.6 * dt).min(0.5);
            }
            p.vel.y -= p.gravity * dt;
            if let Some((lo, hi)) = view {
                let m = p.size1 + p.pos.y.max(0.0);
                if p.pos.x < lo.x - m || p.pos.x > hi.x + m || p.pos.z < lo.y - m || p.pos.z > hi.y + m {
                    continue;
                }
            }
            let t = (age / p.life).clamp(0.0, 1.0);
            let size = p.size0 + (p.size1 - p.size0) * t;
            let fade = if p.additive { 1.0 - t } else { (t * 6.0).min(1.0) * (1.0 - t).powf(1.4) };
            let inst = SpriteInstance::new(p.pos.to_array(), p.sprite)
                .scale(size / 64.0)
                .alpha(p.alpha * fade)
                .tint(p.tint)
                .brightness(p.brightness)
                .rotation(p.rotation + age * 0.3);
            if p.additive {
                self.additive.push(inst);
            } else {
                self.smoke_sprites.push(inst);
            }
        }
        if self.reduced_flashes {
            soften_flashes(&mut self.additive, &mut self.beams, &mut self.lights);
        }
    }

    /// Missiles and flying debris in view, as model instances.
    pub fn models(&self) -> Vec<DrawInstance<'_>> {
        let in_view = |p: Vec3| self.view.is_none_or(|(lo, hi)| p.x >= lo.x && p.x <= hi.x && p.z >= lo.y && p.z <= hi.y);
        self.model_ids
            .iter()
            .zip(&self.model_matrices)
            .map(|(&model, m)| DrawInstance { model, pieces: m, team: 0, flags: 0, build: 1.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL })
            .chain(self.debris.iter().filter(|d| in_view(d.pos)).map(|d| DrawInstance { model: d.model, pieces: std::slice::from_ref(&d.matrix), team: d.team, flags: DEAD_LIGHTS, build: 1.0, damage: d.damage, travel: [0.0; 2], piece: Some(d.piece as u32), playing: 0 }))
            .chain(self.remains_instances())
            .collect()
    }
}

/// Apply at submission, so effects already alive also respond to the preference.
fn soften_flashes(sprites: &mut [SpriteInstance], beams: &mut [BeamInstance], lights: &mut [ta_render::PointLight]) {
    for s in sprites { s.params[2] *= 0.2; }
    for b in beams { for c in &mut b.color[..3] { *c *= 0.25; } }
    for l in lights { l.color = l.color.map(|c| c * 0.15); }
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;

    #[test]
    fn reduced_flashes_keep_effect_locations_and_coverage() {
        let mut sprites = [SpriteInstance::new([3.0, 4.0, 5.0], SpriteId(7)).brightness(3.0).alpha(0.6).cover(0.85)];
        let mut beams = [BeamInstance::new([1.0; 3], [2.0; 3], 6.0, [5.0, 2.0, 1.0], 0.8)];
        let mut lights = [ta_render::PointLight::new([9.0; 3], [8.0, 4.0, 2.0], 240.0)];
        let (sprite, beam, light) = (sprites[0], beams[0], lights[0]);
        soften_flashes(&mut sprites, &mut beams, &mut lights);
        assert!((sprites[0].params[2] - 0.6).abs() < 1e-6);
        assert_eq!(sprites[0].params[0], sprite.params[0]);
        assert_eq!(sprites[0].tint, sprite.tint);
        assert_eq!((beams[0].a, beams[0].b, beams[0].shape, beams[0].color[3]), (beam.a, beam.b, beam.shape, beam.color[3]));
        assert_eq!(&beams[0].color[..3], &[1.25, 0.5, 0.25]);
        assert_eq!((lights[0].position, lights[0].radius, lights[0].shadows), (light.position, light.radius, light.shadows));
        assert!(lights[0].color.iter().zip(light.color).all(|(after, before)| *after > 0.0 && *after < before * 0.2));
    }
}

/// A beam's colour at full strength (its brightest channel at 1), for the sparks it strikes.
fn beam_color(c: [f32; 3]) -> [f32; 3] {
    let peak = c[0].max(c[1]).max(c[2]).max(1e-3);
    c.map(|v| v / peak)
}

/// Weapon colour: for lasers the colour of their glow, warm tones otherwise. TA drew a laser in
/// two palette colours, `color` its bright core (near white for the heavy lasers and the
/// tachyon weapons) and `color2` round it (their blue, their violet; the light lasers' green):
/// the glow is `color2`'s, or `color`'s where it has none (the yellow lasers).
fn weapon_color(w: &ta_sim::WeaponType, sim: &Sim) -> [f32; 3] {
    let rgb = |index: u8| {
        let [r, g, b] = sim.data.palette.rgb(index);
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
    };
    let lit = |c: &[f32; 3]| c.iter().sum::<f32>() >= 0.2;
    match w.def.render_type {
        RenderType::Laser => {
            let rim = rgb(w.def.color2);
            let core = rgb(w.def.color);
            if w.def.color2 != 0 && lit(&rim) {
                // (TA drew its lasers a pixel or two wide, where its pale blues and lilacs read;
                // as a glow round a white core, brightened past the tone map's shoulder, they
                // came out white: their hue is brought out until their weakest channel is a
                // third of their strongest.)
                saturated(rim, 0.3)
            } else if lit(&core) {
                core
            } else {
                [1.0, 0.2, 0.1]
            }
        }
        RenderType::Lightning => [0.6, 0.7, 1.0],
        _ => [1.0, 0.75, 0.45],
    }
}

/// A colour as saturated as its weakest channel `ratio` of its strongest, at most (its hue pushed
/// away from its grey, the brightest channel kept); one already that saturated as it is.
fn saturated(c: [f32; 3], ratio: f32) -> [f32; 3] {
    let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    if hi <= 0.0 || lo <= hi * ratio {
        return c;
    }
    let lum = c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
    // How far to push: the weakest channel then stands at `ratio` of the strongest.
    let k = (1.0 - ratio) * lum / (ratio * (hi - lum) + (lum - lo)).max(1e-4);
    let pushed = c.map(|v| (lum + (v - lum) * k).max(0.0));
    let peak = pushed[0].max(pushed[1]).max(pushed[2]).max(1e-4);
    pushed.map(|v| v / peak * hi)
}

/// How a laser bolt is drawn: as long as its speed over its time (TA's `duration`, at least
/// 0.06 s), broad for a heavy laser; `heavy` (0..1) grows from 400 damage to 2,500 (the
/// Annihilator's, the Doomsday Machine's, the Krogoth's head beam), broadening and lengthening
/// it (TA drew those as heavy beams, not thin dashes).
struct LaserShape {
    width: f32,
    length: f32,
    heavy: f32,
}

impl LaserShape {
    fn of(w: &ta_sim::WeaponType) -> LaserShape {
        let damage = w.def.default_damage as f32;
        let heavy = ((damage / 400.0).max(1.0).log10() / (2500.0f32 / 400.0).log10()).clamp(0.0, 1.0);
        let width = if damage > 100.0 { 5.0 } else { 3.0 } + 5.0 * heavy;
        let length = ((w.def.velocity * w.def.duration.max(0.06)) as f32 * (1.0 + 1.4 * heavy)).clamp(10.0, 200.0);
        LaserShape { width, length, heavy }
    }
}

/// How big a shell is drawn (1 for a Bulldog's): by its blast and its damage, 0.6 for the
/// lightest guns to 2.2 for the heaviest (Big Bertha, the Intimidator, the Buzzsaw).
fn shell_size(w: &ta_sim::WeaponType) -> f32 {
    let blast = (w.aoe_radius.to_f32().max(1.0) / 48.0).sqrt();
    let punch = (w.def.default_damage as f32).max(1.0) / 120.0;
    (blast * punch.powf(0.2)).clamp(0.6, 2.2)
}

/// The projectiles to draw `alpha` of the way through the tick: those flying, and those that
/// ended in it until they get to where they hit (the world is drawn between the last tick and
/// this one, so their last stretch is still to be seen).
fn in_flight(sim: &Sim, alpha: f32) -> impl Iterator<Item = &ta_sim::Projectile> {
    sim.projectiles.iter().chain(sim.spent.iter().filter(move |(_, when)| alpha < when.to_f32()).map(|(p, _)| p))
}

/// How far a projectile has come from where it was fired, as drawn (`alpha` of the way from its
/// last tick to this one): a bolt's tail reaches back no further.
fn travelled(p: &ta_sim::Projectile, alpha: f32) -> f32 {
    (p.age as f32 - 1.0 + alpha).max(0.0) * Vec3::from_array(p.vel.to_f32()).length()
}

/// Blasts this big (px from the middle) look nuclear whatever they are: a nuclear missile's, a
/// fusion plant's or a Commander's death, a crawling bomb's.
const NUCLEAR_RADIUS: f32 = 240.0;
/// Blasts this big or more look nuclear when their data calls them so (see [`nuclear`]).
const NAMED_NUCLEAR_RADIUS: f32 = 128.0;

/// Whether a weapon's blast is drawn as a nuclear one (its flash, shockwaves and mushroom
/// cloud, all to its size): by its size alone from [`NUCLEAR_RADIUS`], or from
/// [`NAMED_NUCLEAR_RADIUS`] when its name, the name players see or its explosion's art says
/// nuke, nuclear or atomic. Total Mayhem's Orion drops `NUKEBOMB` (its art NUKEBOMB, "Nuclear
/// Bomb"), a blast of 225: under the size, and drawn as a very big ordinary one; its tactical
/// nukes are 190 to 200. TA's own Big Bertha, Intimidator and Buzzsaw shells burst in art named
/// NUKE1, but 40 to 60 across: never nuclear.
fn nuclear(w: &ta_sim::WeaponType) -> bool {
    nuclear_by(w.aoe_radius.to_f32(), [&w.def.name, &w.def.display_name, &w.def.explosion_gaf, &w.def.explosion_art])
}

/// [`nuclear`] by a blast's radius and its names (the weapon's, the one players see, its
/// explosion's art file and animation).
fn nuclear_by(radius: f32, names: [&str; 4]) -> bool {
    let named = || {
        names.iter().any(|s| {
            let s = s.to_ascii_lowercase();
            ["nuke", "nuclear", "atomic"].iter().any(|word| s.contains(word))
        })
    };
    radius >= NUCLEAR_RADIUS || radius >= NAMED_NUCLEAR_RADIUS && named()
}

/// How near (world px) another lightning bolt's middle must be to a bolt's to crowd it: about as
/// far as the bloom spreads a bolt's light at the distances battles are watched from.
const BOLT_CROWD_RADIUS: f32 = 160.0;
/// As many bolts as may stand together drawn each as a lone one is (a Zeus's, a pair of them).
const BOLT_ALONE: f32 = 3.0;
/// More than this many crowding a bolt draw it no fainter (and are not counted on).
const BOLT_CROWD_MOST: usize = 256;

/// How a lightning bolt is drawn among the `crowd` bolts (itself included) whose middles lie
/// within [`BOLT_CROWD_RADIUS`] of its own. Alone, or with a couple of others, as a bolt always
/// was: a glow round a hot core, crackling in two-tick strokes all its way. In a crowd (a line of
/// Tigers' five-bolt bursts, a battery of Zeuses firing up at aircraft) their glows, laid over one
/// another and bloomed, made a white wall: so each one's glow and core are fainter the more bolts
/// crowd it (their light together growing as the root of their number, not with it), and after
/// its first stroke a bolt shows in only some of its strokes, struck again in a new shape each
/// time: lightning flickering, not a curtain of light.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BoltLook {
    glow: f32,
    core: f32,
    /// The share of its strokes after the first it shows in.
    duty: f32,
    /// Whether it shows in this frame's stroke.
    shown: bool,
}

impl BoltLook {
    fn among(crowd: usize) -> Self {
        let k = (BOLT_ALONE / crowd.max(1) as f32).min(1.0);
        BoltLook { glow: k.sqrt(), core: k.powf(0.25), duty: (k * 1.5).sqrt().clamp(0.35, 1.0), shown: true }
    }

    /// As in `stroke` of projectile `id`: always in its first stroke, in others as its crackle
    /// draws.
    fn in_stroke(self, id: u32, stroke: u32) -> Self {
        BoltLook { shown: stroke == 0 || crackle(id, stroke, 97) <= self.duty, ..self }
    }
}

/// Where a lightning bolt is drawn from: the gun it left (its head now less its way since).
fn bolt_start(p: &ta_sim::Projectile) -> Vec3 {
    Vec3::from_array(p.pos.to_f32()) - Vec3::from_array(p.vel.to_f32()) * p.age as f32
}

/// A lightning bolt's crackle: a number in 0..1 for projectile `id`, its stroke (two ticks
/// long) and a slot, the same every frame of the stroke.
fn crackle(id: u32, stroke: u32, slot: u32) -> f32 {
    let mut h = id.wrapping_mul(0x9E37_79B9) ^ stroke.wrapping_mul(0x85EB_CA6B) ^ slot.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod nuclear_tests {
    use super::*;

    /// TA's own nuclear blasts by their size; a mod's smaller nuclear weapons by what their data
    /// calls them; never a shell whose art only happens to be named NUKE1, nor a big ordinary blast.
    #[test]
    fn a_blast_is_nuclear_by_its_size_or_its_names() {
        // TA: the nuclear missile and the fusion plant's death; a Commander's, by size alone.
        assert!(nuclear_by(256.0, ["NUCLEAR_MISSILE", "Nuclear Missile", "commboom", "commboom"]));
        assert!(nuclear_by(258.0, ["ATOMIC_BLAST", "Atomic Blast Weapon", "fx", "explode2"]));
        assert!(nuclear_by(475.0, ["COMMANDER_BLAST", "Commander Blast", "commboom", "commboom"]));
        // Total Mayhem: the Orion's bomb, its tactical nukes and its nuclear cannon.
        assert!(nuclear_by(225.0, ["NUKEBOMB", "Nuclear Bomb", "NUKEBOMB", "NUKEBOMB"]));
        assert!(nuclear_by(191.0, ["ARMSCUDD_ROCKET", "Tactical Nuke Rocket", "fx2", "explode8"]));
        assert!(nuclear_by(200.0, ["CORE_NUKECANNON", "Nuclear Cannon", "fx2", "explode8"]));
        // TA's Big Bertha, Intimidator and Buzzsaw burst in art named NUKE1: not nuclear.
        assert!(!nuclear_by(40.0, ["ARM_BERTHACANNON", "Bertha Cannon", "fx", "NUKE1"]));
        assert!(!nuclear_by(60.0, ["CORBUZZ_WEAPON", "Bertha Cannon", "fx", "NUKE1"]));
        // A factory's death, 200 across, is a very big ordinary blast.
        assert!(!nuclear_by(200.0, ["LARGE_BUILDING", "Large building", "fx", "explode2"]));
    }
}

#[cfg(test)]
mod missile_tests {
    use super::*;

    /// A missile's tail is where its body ends, not its flare piece (TA's exhaust cone), and its
    /// body is moved onto the flight's line: the Arm nuke's lies 6 px up and 5 to one side of its
    /// projectile's place, so a flame put there came out from under it.
    #[test]
    fn a_missiles_tail_is_its_bodys_end_on_the_flight_line() {
        let rest = vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::new(0.0, 0.0, -15.0))];
        // The body: x 1..11, y 1..12, z -11..29; the flare behind it, z -50..-15.
        let body = [Vec3::new(1.0, 1.0, -11.0), Vec3::new(11.0, 12.0, 29.0), Vec3::new(6.0, 6.0, 0.0)].map(|v| (0, v));
        let flare = [Vec3::new(-1.0, -1.0, -35.0), Vec3::new(1.0, 1.0, 0.0)].map(|v| (1, v));
        let m = WeaponModel::new(0, rest.clone(), &["MissileOnly", "flare"], body.into_iter().chain(flare), false);
        assert_eq!(m.tail, -11.0, "the body's tail, not the flare's");
        assert_eq!(m.girth, 10.0);
        // The body's middle (6, 6.5) now on the line; the flare moved with it.
        assert_eq!(m.pieces[0].transform_point3(Vec3::new(6.0, 6.5, 0.0)), Vec3::ZERO);
        assert_eq!(m.pieces[1].transform_point3(Vec3::ZERO), Vec3::new(-6.0, -6.5, -15.0));
        assert!(m.pieces[1].transform_point3(Vec3::new(0.0, 0.0, -35.0)).distance(Vec3::new(-6.0, -6.5, -50.0)) < 1e-4, "its flare drawn whole");
        // With Reborn's flame, the flare is drawn shrunk to nothing.
        let m = WeaponModel::new(0, rest, &["MissileOnly", "flare"], body.into_iter().chain(flare), true);
        assert_eq!(m.tail, -11.0);
        assert!(m.pieces[1].transform_point3(Vec3::new(0.0, 0.0, -35.0)).distance(Vec3::new(-6.0, -6.5, -15.0)) < 0.01, "no flare");
        // A model of nothing but a flare is measured and drawn whole; one with nothing drawn keeps
        // the old place.
        let only = WeaponModel::new(0, vec![Mat4::IDENTITY], &["flare"], flare.map(|(_, v)| (0, v)).into_iter(), true);
        assert_eq!(only.tail, -35.0);
        assert!(only.pieces[0].transform_point3(Vec3::new(0.0, 0.0, -35.0)).z == -35.0);
        assert_eq!(WeaponModel::new(0, vec![Mat4::IDENTITY], &["base"], std::iter::empty(), true).tail, -6.0);
    }

    /// A projectile's model lies nose first along its flight with its top up, whichever way it
    /// flies (turned the shortest way from +z, one flying north was upside down); straight up, it
    /// faces the way it will turn over.
    #[test]
    fn a_missile_lies_along_its_flight_its_top_up() {
        let close = |a: Vec3, b: Vec3| a.distance(b) < 1e-4;
        for dir in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z, Vec3::new(1.0, -1.0, 0.3).normalize(), Vec3::new(-0.2, 0.7, -0.9).normalize()] {
            let q = flight_rotation(dir, Vec3::X);
            assert!(close(q * Vec3::Z, dir), "nose along {dir}");
            assert!((q * Vec3::Y).y > 0.0 && (q * Vec3::X).y.abs() < 1e-4, "top up, no roll, flying {dir}");
        }
        // Straight up, headed east: its belly (-y) faces the way it turns over, east.
        let q = flight_rotation(Vec3::Y, Vec3::new(500.0, -700.0, 0.0));
        assert!(close(q * Vec3::Z, Vec3::Y));
        assert!(close(q * Vec3::NEG_Y, Vec3::X), "{}", q * Vec3::NEG_Y);
        // ...and just turning over that way, the same.
        let q2 = flight_rotation(Vec3::new(0.05, 1.0, 0.0).normalize(), Vec3::new(500.0, -700.0, 0.0));
        assert!((q * Vec3::NEG_Y).distance(q2 * Vec3::NEG_Y) < 0.06);
    }
}

#[cfg(test)]
mod bolt_tests {
    use super::*;

    /// A lone bolt, or two or three together (a Zeus's, the Commander's), is drawn as one always
    /// was: whole, in every stroke. A crowd's bolts are fainter the more crowd them, their light
    /// together growing as the root of their number, and show in fewer strokes (never under a
    /// third of them), but always in their first.
    #[test]
    fn a_lone_bolt_is_drawn_whole_and_a_crowd_fainter_and_flickering() {
        let full = BoltLook { glow: 1.0, core: 1.0, duty: 1.0, shown: true };
        for crowd in [0, 1, 2, 3] {
            assert_eq!(BoltLook::among(crowd), full, "{crowd} bolts");
        }
        let mut last = full;
        for crowd in [4, 8, 20, 50, 100, 256] {
            let look = BoltLook::among(crowd);
            assert!(look.glow < last.glow && look.core < last.core && look.duty <= last.duty, "{crowd} bolts: {look:?}");
            assert!(look.duty >= 0.35);
            // (Their glows together: the root of their number, three alone's worth at most.)
            let together = look.glow * crowd as f32;
            assert!((together - (BOLT_ALONE * crowd as f32).sqrt()).abs() < 1e-3, "{crowd} bolts: {together}");
            last = look;
        }
        let crowded = BoltLook::among(100);
        assert!((0..50).all(|id| crowded.in_stroke(id, 0).shown), "every bolt strikes first");
        let shown = (0..2000u32).filter(|&i| crowded.in_stroke(i % 50, 1 + i / 50).shown).count() as f32 / 2000.0;
        assert!((shown - crowded.duty).abs() < 0.05, "shows in about {} of its strokes, not {shown}", crowded.duty);
        assert!((0..2000u32).all(|i| full.in_stroke(i % 50, 1 + i / 50).shown), "a lone bolt shows in every stroke");
    }
}

#[cfg(test)]
mod contact_tests {
    use super::*;

    #[test]
    fn debris_ground_contact_uses_the_tumbled_mesh_not_its_pivot_or_box() {
        // A gun with its pivot outside the mesh, and a diagonal face whose bounding box
        // has empty corners. Rotating that box would itself make a floating contact.
        let points = [Vec3::new(10.0, 0.0, -2.0), Vec3::new(20.0, 4.0, -2.0), Vec3::new(12.0, 6.0, 2.0)];
        let rotation = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let (old_gap, _) = debris_contact(&points, rotation, |_, _| 0.0);
        assert!((old_gap - 10.0).abs() < 1e-4, "stopping this pivot on the floor leaves the entire gun floating");

        for angle in [0.0, 0.4, 1.5, 2.8, 4.9] {
            let matrix = Mat4::from_translation(Vec3::new(40.0, -30.0, 60.0))
                * Mat4::from_rotation_y(0.7) * Mat4::from_rotation_z(angle);
            for slope in [0.0, 0.35, -0.4] {
                let ground = |x: f32, z: f32| 12.0 + x * slope - z * slope * 0.5;
                let (clearance, _) = debris_contact(&points, matrix, ground);
                let seated = Mat4::from_translation(-Vec3::Y * clearance) * matrix;
                let (gap, point) = debris_contact(&points, seated, ground);
                assert!(gap.abs() < 1e-4, "the visible mesh must touch the slope: {gap}");
                assert!((point.y - ground(point.x, point.z)).abs() < 1e-4);
                for vertex in points {
                    let p = seated.transform_point3(vertex);
                    assert!(p.y >= ground(p.x, p.z) - 1e-4, "a fragment vertex went underground");
                }
            }
        }
    }
}
