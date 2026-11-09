//! Space, as the film's openings show it (TA's intro movie begins there): a sky of stars, nebulae
//! and distant galaxies, planets with TA's worlds on their faces and air round them, their moons,
//! asteroids tumbling past (TA Reborn's own 3D rocks, cratered) and the sun.
//!
//! The game reads a scene file (`ta_game::space`) into a [`SpaceScene`], what the scene is made
//! of, and hands the renderer a [`SpaceFrame`] every frame, where everything is then; the renderer
//! draws it in place of the battlefield ([`crate::Renderer::render_space`]) at the window's size,
//! through the post-processing's space look (a filmic curve, glow, dither: [`crate::post`]).
//!
//! Everything is drawn round the eye (camera-relative, so f32 keeps its precision), with depth
//! reversed and the far plane at infinity (1 at the near plane, 0 at infinity), into the HDR
//! target, in one pass (`shaders/space.wgsl`):
//! 1. the sky (`fs_sky`, a full-screen triangle): nebulae (warped fractal noise in layers over
//!    the sphere of directions, ridged filaments, bright cores, dust lanes in front), galaxies (an
//!    inclined disc: a bulge, logarithmic arms broken into clumps, dust lanes along them, pink
//!    knots) and the sun's disc (limb-darkened, its edge antialiased);
//! 2. the stars (`vs_star`): a catalogue made once from the scene's seed (directions spread over
//!    the sky, more along a band if asked; brightness from a power law; colour from a black body's
//!    temperature), each a Gaussian integrated over the pixel, so it moves across the pixels
//!    without shimmering, dimmed behind the nebulae's dust;
//! 3. the asteroids (`vs_rock`): the boulders of `features.glb` subdivided, lumped and cratered
//!    on the CPU ([`RockShape`]), the rock texture laid on along their own axes (its cracks as
//!    relief), lit by the sun (lunar-Lambert), by the light the planets throw back, in the
//!    shadows the spheres cast; the nearest few with shadow maps of their own (their craters'
//!    shadows);
//! 4. planets and moons, farthest first (`fs_sphere`, a full-screen triangle each, every pixel's
//!    ray meeting the sphere): the world's surface as Conquest paints it (`ta_game::planet`'s
//!    recipes ported to the GPU, `shaders/space_worlds.wgsl`, their detail growing with the size
//!    on screen), its relief, the sea's glint, a cloud layer turning on its own and shadowing the
//!    ground, lights by night, eclipses, and the air between the eye and the ground; edges
//!    antialiased by their coverage, depth tested against the asteroids (not written); then the
//!    air round each (`fs_halo_tint`, `fs_halo_glow`): single scattering marched through an
//!    exponential atmosphere (Rayleigh and Mie, the sunlight reaching each step dimmed by a
//!    Chapman approximation), laid over what is behind it: multiplied by its transmittance, then
//!    its glow added.
//!
//! Then the sun's glare (`fs_glare`, a pass of its own over the resolved picture): a halo, streaks
//! and ghosts, as much as the sun's disc is seen (32 points of it tested against the depth, the
//! asteroids, and the spheres, through their air).
//!
//! `TA_SPACE_SHADERS=<folder>` reads `space*.wgsl` from that folder instead of the built-in ones
//! (tuning the look without rebuilding).

use std::f32::consts::TAU;
use std::ops::Range;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Quat, Vec3};
use crate::gpu::CheckedBufferInit;

use crate::util::{DEPTH_FORMAT, HDR_FORMAT};

/// TA's worlds as the space shader numbers them (`space_worlds.wgsl`'s `ground`; the game's
/// `planet::World` in this order).
pub const WORLDS: [&str; 15] = ["green", "lush", "archipelago", "water", "wet desert", "desert", "red", "lava", "metal", "urban", "ice", "slate", "lunar", "crystal", "acid"];

/// At most this many spheres (planets and moons) and asteroids with shadow maps of their own.
pub const MAX_SPHERES: usize = 16;
pub const SHADOWED_ROCKS: usize = 4;
/// Each asteroid shadow map's size (texels square).
const ROCK_SHADOW: u32 = 2048;
/// The near plane (scene units).
const NEAR: f32 = 0.002;

/// What a scene is made of.
#[derive(Clone, Debug)]
pub struct SpaceScene {
    /// The sky's random numbers (the stars' places).
    pub seed: u64,
    pub stars: Stars,
    pub nebulae: Vec<Nebula>,
    pub galaxies: Vec<Galaxy>,
    pub sun: Sun,
    /// Planets and moons, in the order [`SpaceFrame::spheres`] places them.
    pub spheres: Vec<SphereLook>,
    /// The asteroids' shapes, which [`RockPose::shape`] picks from.
    pub rocks: Vec<RockShape>,
    pub look: Look,
}

/// The stars.
#[derive(Clone, Debug)]
pub struct Stars {
    /// How many over the whole sky.
    pub count: u32,
    /// Their brightness (1: as the examples have them).
    pub brightness: f32,
    /// How coloured they are (0 all white, 1 a black body's colour).
    pub colour: f32,
    /// Their blur's radius (pixels at 1080 lines; scaled with the picture).
    pub size: f32,
    /// A band where they crowd (a galaxy's plane, seen from inside it), if any: its pole, its half
    /// width (radians) and the share of the stars in it.
    pub band: Option<(Vec3, f32, f32)>,
}

/// A nebula: a band of glowing gas and dust across the sky.
#[derive(Clone, Debug)]
pub struct Nebula {
    /// Towards its middle, and along it (unit, square to `centre`).
    pub centre: Vec3,
    pub along: Vec3,
    /// Half its length and half its width (radians), and how far it bends across as it goes.
    pub length: f32,
    pub width: f32,
    pub bend: f32,
    /// Its gas's two colours (linear), its cores' colour, and how bright each is.
    pub colour: Vec3,
    pub colour2: Vec3,
    pub core: Vec3,
    pub brightness: f32,
    pub core_brightness: f32,
    /// How strong its filaments are, how much dust darkens it, its structure's scale (how many
    /// billows across the sky's sphere), how much they swirl.
    pub filaments: f32,
    pub dust: f32,
    pub scale: f32,
    pub warp: f32,
    pub seed: u32,
    /// Its core's length along it (radians, end to end).
    pub core_length: f32,
}

/// A distant spiral galaxy.
#[derive(Clone, Debug)]
pub struct Galaxy {
    /// Towards its middle; its radius on the sky (radians).
    pub centre: Vec3,
    pub radius: f32,
    /// How far its disc is turned from face on, and the angle of its long axis on the sky
    /// (radians, from the sky's horizontal).
    pub tilt: f32,
    pub angle: f32,
    pub arms: f32,
    /// How tightly its arms wind.
    pub twist: f32,
    pub brightness: f32,
    /// Its bulge's, arms' and star-forming knots' colours (linear).
    pub core: Vec3,
    pub arm: Vec3,
    pub knots: Vec3,
    pub dust: f32,
    pub seed: u32,
    /// How strong the great clouds of glowing gas along its arms are (1: as the examples have).
    pub clouds: f32,
}

/// The sun, infinitely far.
#[derive(Clone, Debug)]
pub struct Sun {
    /// Towards it (unit).
    pub direction: Vec3,
    /// Its light's colour (linear) and strength: what a white surface facing it shows.
    pub colour: Vec3,
    pub intensity: f32,
    /// Its disc's radius (radians), and how bright the disc is over its light.
    pub radius: f32,
    pub disc: f32,
    /// The glare when it is in view (1: as the examples have it), and the lens's ghosts.
    pub glare: f32,
    pub ghosts: f32,
    /// A fill light on the asteroids and the airless bodies (a second, dim light from elsewhere, as
    /// a film's fill lamp: their shadow sides seen): towards it, and its colour times its strength
    /// (linear; none by default).
    pub fill: Vec3,
    pub fill_light: Vec3,
}

/// What a planet or a moon looks like.
#[derive(Clone, Debug)]
pub struct SphereLook {
    /// Its world (an index of [`WORLDS`]) and the seed of its surface.
    pub world: u32,
    pub seed: u32,
    pub radius: f32,
    pub air: Option<Air>,
    /// The cloud cover's strength (1: the world's own, 0 none), the relief's, the sea's roughness
    /// (its glint's spread), the lights by night.
    pub clouds: f32,
    pub relief: f32,
    pub roughness: f32,
    pub lights: f32,
    /// Its colours multiplied by this (linear), and its sea's (shallow, deep; linear) in place
    /// of the world's.
    pub tint: Vec3,
    pub ocean: Option<[Vec3; 2]>,
    /// Its mean colour as it throws the sun's light back (linear): how it lights the others.
    pub albedo: Vec3,
    /// An airless body's light: 0 the moon's dusty light (flat to its terminator), 1 Lambert's
    /// (falling off softly towards it).
    pub soft: f32,
    /// Octaves of its surface more (or, negative, fewer: calmer) than its size on screen shows.
    pub detail: f32,
    /// Light on all of it from nowhere in particular (linear, times its colours): its night side
    /// seen dimly.
    pub fill: f32,
}

/// A sphere's air.
#[derive(Clone, Copy, Debug)]
pub struct Air {
    /// How high it reaches (of the radius).
    pub height: f32,
    /// Its scattering by molecules: the optical depth straight up from the ground, per channel
    /// (the bluer, the bluer the sky), and by haze (grey), and the haze's forward lean.
    pub rayleigh: Vec3,
    pub mie: f32,
    pub mie_g: f32,
}

/// An asteroid's shape: one of the 3D rocks, roughened and cratered.
#[derive(Clone, Debug)]
pub struct RockShape {
    /// The rock of `features.glb` it starts from (`boulder_round`, `boulder_angular`...).
    pub model: String,
    pub seed: u32,
    /// How many craters (1: about seventy), how lumpy it is, how much it is stretched along its
    /// own axes, and how often its triangles are split (each time four from one).
    pub craters: f32,
    pub lumps: f32,
    pub stretch: Vec3,
    pub detail: u32,
    /// Its colour (linear albedo), and the rock texture's tiles across its radius.
    pub colour: Vec3,
    pub texture: f32,
}

/// The picture's look.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// Times the light (2^stops), the glow's strength, the light from everywhere (linear), the
    /// corners' darkening.
    pub exposure: f32,
    pub bloom: f32,
    pub ambient: Vec3,
    pub vignette: f32,
}

/// Where everything is in a frame.
#[derive(Clone, Debug)]
pub struct SpaceFrame {
    /// Seconds (the capture's clock).
    pub time: f32,
    pub eye: Vec3,
    /// The camera's turn (it looks along its -z, its up +y).
    pub rotation: Quat,
    /// The vertical field of view (radians).
    pub fov_y: f32,
    /// Each sphere of the scene, in its order.
    pub spheres: Vec<SpherePose>,
    pub rocks: Vec<RockPose>,
    /// The picture's brightness (1; down to 0 fades to black), as a video fades: the finished
    /// picture's values times it.
    pub fade: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpherePose {
    pub centre: Vec3,
    /// The surface's turn and the clouds' (from the sphere's own axes to the world's).
    pub rotation: Quat,
    pub clouds: Quat,
}

#[derive(Clone, Copy, Debug)]
pub struct RockPose {
    /// The index of its shape in [`SpaceScene::rocks`].
    pub shape: usize,
    pub centre: Vec3,
    pub rotation: Quat,
    /// Its radius (its farthest point from its middle).
    pub size: f32,
}

// ---- GPU layouts (`space_common.wgsl`) ---------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct FrameGpu {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    sun: [f32; 4],
    sun_light: [f32; 4],
    screen: [f32; 4],
    ambient: [f32; 4],
    counts: [u32; 4],
    stars: [f32; 4],
    misc: [f32; 4],
    /// The fill light: towards it (xyz), and its colour (rgb).
    fill: [f32; 4],
    fill_light: [f32; 4],
    rock_shadow: [[[f32; 4]; 4]; SHADOWED_ROCKS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SphereGpu {
    centre: [f32; 4],
    body: [[f32; 4]; 3],
    cloud: [[f32; 4]; 3],
    kind: [u32; 4],
    air: [f32; 4],
    haze: [f32; 4],
    surface: [f32; 4],
    tint: [f32; 4],
    sea: [[f32; 4]; 2],
    shine: [f32; 4],
    /// x how Lambertian an airless body's light is, y octaves more (fewer) than its size shows, z
    /// its fill light.
    look: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct NebulaGpu {
    centre: [f32; 4],
    along: [f32; 4],
    across: [f32; 4],
    colour: [f32; 4],
    colour2: [f32; 4],
    core: [f32; 4],
    shape: [f32; 4],
    /// x how tightly its core gathers along it (its length over its core's, squared).
    extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GalaxyGpu {
    centre: [f32; 4],
    major: [f32; 4],
    minor: [f32; 4],
    core: [f32; 4],
    arm: [f32; 4],
    knots: [f32; 4],
    extra: [f32; 4],
}

/// A star: towards it, its brightness (its peak, centred on a pixel, at the scene's blur) and its
/// colour (linear, a luminance of 1 at 255... as unorm) with its depth among the dust in alpha.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub(crate) struct StarGpu {
    dir: [f32; 3],
    flux: f32,
    colour: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub(crate) struct RockVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    /// How deep in a crater's bowl (0..1): darker, less of the light from everywhere.
    cavity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub(crate) struct RockInstanceGpu {
    /// Rows of the transform (turn × size | place, camera-relative).
    m: [[f32; 4]; 3],
    /// rgb its colour over the texture's mean, w the texture's tiles per unit of its own space.
    tint: [f32; 4],
    /// x its shadow map's slot (-1: none), y its radius, z its relief's strength, w unused.
    extra: [f32; 4],
}

// ---- The sky's stars --------------------------------------------------------------------------

/// SplitMix64: the scene's random numbers.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// 0 to 1 (never 0).
    fn unit(&mut self) -> f32 {
        ((self.next() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }

    fn direction(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let a = self.range(0.0, TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), z, r * a.sin())
    }

    fn gaussian(&mut self) -> f32 {
        (-2.0 * self.unit().ln()).sqrt() * (TAU * self.unit()).cos()
    }
}

/// A black body's colour at `kelvin` (linear, luminance 1): Tanner Helland's fit of the sRGB
/// colours.
fn black_body(kelvin: f32) -> Vec3 {
    let t = kelvin / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.699 * (t - 60.0).powf(-0.133_205) };
    let g = if t <= 66.0 { 99.4708 * t.ln() - 161.1196 } else { 288.122 * (t - 60.0).powf(-0.075_515) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.5177 * (t - 10.0).ln() - 305.0448
    };
    let c = Vec3::new(r, g, b).clamp(Vec3::ZERO, Vec3::splat(255.0)) / 255.0;
    let c = Vec3::new(crate::util::srgb_to_linear(c.x), crate::util::srgb_to_linear(c.y), crate::util::srgb_to_linear(c.z));
    c / c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-4)
}

/// Two unit vectors square to `n` and to each other.
fn basis(n: Vec3) -> (Vec3, Vec3) {
    let a = if n.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let t = a.cross(n).normalize();
    (t, n.cross(t))
}

/// The scene's stars: directions, brightness from a power law (many faint, few bright), colour
/// from a black body's temperature (most yellowish white, some orange, a few blue).
fn star_catalogue(stars: &Stars, seed: u64) -> Vec<StarGpu> {
    let mut rng = Rng(seed ^ 0x5717_a25e);
    let (t1, t2) = stars.band.map_or((Vec3::X, Vec3::Z), |(pole, _, _)| basis(pole.normalize_or(Vec3::Y)));
    (0..stars.count)
        .map(|_| {
            let dir = match stars.band {
                Some((pole, width, share)) if rng.unit() < share => {
                    // Crowding towards the band's middle, thinning out to either side.
                    let pole = pole.normalize_or(Vec3::Y);
                    let a = rng.range(0.0, TAU);
                    let lat = (rng.gaussian() * width).clamp(-1.5, 1.5);
                    (t1 * a.cos() + t2 * a.sin()) * lat.cos() + pole * lat.sin()
                }
                _ => rng.direction(),
            };
            // Brightness: N(> b) falls as b^-1.6, from the faintest at 0.012 to some thousand
            // times that; the very brightest few rarer still.
            let flux = (0.012 * rng.unit().powf(-1.0 / 1.6)).min(14.0);
            let kelvin = if rng.unit() < 0.1 { rng.range(9000.0, 28000.0) } else { 2700.0 + 6200.0 * rng.unit().powf(1.4) };
            let colour = Vec3::ONE.lerp(black_body(kelvin), stars.colour.clamp(0.0, 1.0));
            let colour = colour / colour.max_element().max(1.0);
            let depth = rng.unit();
            let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            StarGpu { dir: dir.normalize().to_array(), flux, colour: [unorm(colour.x), unorm(colour.y), unorm(colour.z), unorm(depth)] }
        })
        .collect()
}

// ---- The asteroids' shapes ---------------------------------------------------------------------

/// Gradient noise in 3D (about -1 to 1), for the rocks' lumps.
fn noise3(p: Vec3, seed: u32) -> f32 {
    let hash = |x: i32, y: i32, z: i32| {
        let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9e37_79b9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297a_2d39);
        h ^ (h >> 15)
    };
    let grad = |h: u32, d: Vec3| {
        let g = Vec3::new((h & 1023) as f32, ((h >> 10) & 1023) as f32, ((h >> 20) & 1023) as f32) / 511.5 - Vec3::ONE;
        g.dot(d)
    };
    let i = p.floor();
    let f = p - i;
    let (x, y, z) = (i.x as i32, i.y as i32, i.z as i32);
    let u = f * f * f * (f * (f * 6.0 - Vec3::splat(15.0)) + Vec3::splat(10.0));
    let c = |dx: i32, dy: i32, dz: i32| grad(hash(x + dx, y + dy, z + dz), f - Vec3::new(dx as f32, dy as f32, dz as f32));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), u.x);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), u.x);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), u.x);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z)
}

fn fbm3(p: Vec3, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut f) = (0.0, 0.5, 1.0);
    for o in 0..octaves {
        sum += amp * noise3(p * f, seed.wrapping_add(o * 7919));
        amp *= 0.5;
        f *= 2.07;
    }
    sum
}

/// A crater's height at `x`, the distance from its middle over its radius: a bowl (-1 at the
/// middle, its floor flattened in the biggest), a raised rim, and its ejecta falling away.
fn crater_height(x: f32, flat: f32) -> (f32, f32) {
    let bowl = if x < 1.0 { (x * x - 1.0).max(-1.0 + flat) } else { 0.0 };
    let w = if x < 1.0 { 0.16 } else { 0.32 };
    let rim = 0.28 * (-((x - 1.0) / w).powi(2)).exp();
    (bowl + rim, -bowl)
}

/// Draws each corner `times` towards its neighbours' middle, half way.
fn smooth_mesh(points: &mut [Vec3], triangles: &[[u32; 3]], times: u32) {
    let mut neighbours = vec![Vec::new(); points.len()];
    for t in triangles {
        for k in 0..3 {
            neighbours[t[k] as usize].push(t[(k + 1) % 3]);
            neighbours[t[(k + 1) % 3] as usize].push(t[k]);
        }
    }
    for _ in 0..times {
        let before = points.to_vec();
        for (i, p) in points.iter_mut().enumerate() {
            let n = &neighbours[i];
            if !n.is_empty() {
                let mean = n.iter().map(|&j| before[j as usize]).sum::<Vec3>() / n.len() as f32;
                *p = p.lerp(mean, 0.5);
            }
        }
    }
}

/// Splits every triangle into four at its edges' middles.
fn subdivide(points: &mut Vec<Vec3>, triangles: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let mut middle: rustc_hash::FxHashMap<(u32, u32), u32> = Default::default();
    let mut mid = |a: u32, b: u32, points: &mut Vec<Vec3>| {
        let key = (a.min(b), a.max(b));
        *middle.entry(key).or_insert_with(|| {
            points.push((points[a as usize] + points[b as usize]) * 0.5);
            points.len() as u32 - 1
        })
    };
    let mut out = Vec::with_capacity(triangles.len() * 4);
    for &[a, b, c] in triangles {
        let ab = mid(a, b, points);
        let bc = mid(b, c, points);
        let ca = mid(c, a, points);
        out.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
    }
    out
}

/// An asteroid's mesh: its rock centred and scaled to a radius of 1, stretched, split
/// `detail` times, rounded a little, then lumped and cratered along the directions from its middle;
/// normals from its faces.
pub(crate) fn rock_mesh(shape: &RockShape) -> Result<(Vec<RockVertex>, Vec<u32>)> {
    let (mut points, mut triangles) = crate::foliage::model_mesh(&shape.model)?;
    anyhow::ensure!(!triangles.is_empty(), "{}: no triangles", shape.model);
    let (min, max) = points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    let middle = (min + max) * 0.5;
    for p in &mut points {
        *p = (*p - middle) * shape.stretch;
    }
    let reach = points.iter().map(|p| p.length()).fold(0.0f32, f32::max).max(1e-6);
    for p in &mut points {
        *p /= reach;
    }
    // Worn round: the boulder's planes bulged out half way to a ball and their creases smoothed
    // away on its own mesh (an asteroid's faces are old), then split, and the splits' creases
    // smoothed too.
    let mean = points.iter().map(|p| p.length()).sum::<f32>() / points.len().max(1) as f32;
    for p in &mut points {
        let r = p.length();
        if r > 1e-6 {
            *p *= (r + (mean - r) * 0.5) / r;
        }
    }
    smooth_mesh(&mut points, &triangles, 4);
    for _ in 0..shape.detail.min(4) {
        triangles = subdivide(&mut points, &triangles);
    }
    smooth_mesh(&mut points, &triangles, 1);
    // Craters: many small, a few large, scattered over it.
    let mut rng = Rng(shape.seed as u64 ^ 0xc4a7_e125);
    let count = (shape.craters * 70.0).round().max(0.0) as usize;
    let craters: Vec<(Vec3, f32, f32)> = (0..count)
        .map(|_| {
            let r = 0.05 + 0.45 * rng.unit().powf(2.2);
            (rng.direction(), r, (r - 0.25).max(0.0) * 1.2)
        })
        .collect();
    let seed = shape.seed;
    let mut cavity = vec![0.0f32; points.len()];
    for (p, cav) in points.iter_mut().zip(cavity.iter_mut()) {
        let dir = p.normalize_or(Vec3::Y);
        let mut h = shape.lumps * (0.24 * fbm3(dir * 1.2, 3, seed) + 0.06 * fbm3(dir * 4.1, 3, seed ^ 0x51) + 0.02 * fbm3(dir * 11.0, 2, seed ^ 0x93));
        let mut deep = 0.0f32;
        for &(c, r, flat) in &craters {
            let cos = dir.dot(c);
            if cos < (r * 1.6).min(3.0).cos() {
                continue;
            }
            let x = cos.clamp(-1.0, 1.0).acos() / r;
            let (dh, bowl) = crater_height(x, flat.min(0.6));
            // Deeper for its size the smaller it is (the big ones are old and filled).
            let depth = r * (0.36 - 0.16 * r);
            h += dh * depth;
            deep = deep.max(bowl);
        }
        *p *= 1.0 + h;
        *cav = deep;
    }
    let reach = points.iter().map(|p| p.length()).fold(0.0f32, f32::max).max(1e-6);
    for p in &mut points {
        *p /= reach;
    }
    let mut normals = vec![Vec3::ZERO; points.len()];
    for t in &triangles {
        let (a, b, c) = (points[t[0] as usize], points[t[1] as usize], points[t[2] as usize]);
        let n = (b - a).cross(c - a);
        for &k in t {
            normals[k as usize] += n;
        }
    }
    let vertices = points
        .iter()
        .zip(&normals)
        .zip(&cavity)
        .map(|((p, n), c)| RockVertex { pos: p.to_array(), normal: n.normalize_or(p.normalize_or(Vec3::Y)).to_array(), cavity: c.clamp(0.0, 1.0) })
        .collect();
    let indices = triangles.iter().flatten().copied().collect();
    Ok((vertices, indices))
}

// ---- Shaders ----------------------------------------------------------------------------------

/// The space shader, its depth texture read with `samples` samples (the glare's).
pub(crate) fn shader_source(samples: u32) -> String {
    let file = |name: &str, built_in: &'static str| -> String {
        match std::env::var("TA_SPACE_SHADERS") {
            Ok(dir) => std::fs::read_to_string(std::path::Path::new(&dir).join(name)).unwrap_or_else(|e| {
                log::warn!("TA_SPACE_SHADERS: {name}: {e}; using the built-in one");
                built_in.to_string()
            }),
            Err(_) => built_in.to_string(),
        }
    };
    let source = format!(
        "{}\n{}\n{}",
        file("space_common.wgsl", include_str!("shaders/space_common.wgsl")),
        file("space_worlds.wgsl", include_str!("shaders/space_worlds.wgsl")),
        file("space.wgsl", include_str!("shaders/space.wgsl"))
    );
    if samples > 1 { source } else { source.replace("texture_depth_multisampled_2d", "texture_depth_2d") }
}

// ---- The renderer -----------------------------------------------------------------------------

/// What the frame's draws need to know besides the buffers.
struct Draws {
    /// Spheres farthest first (their order in the buffer), and which have air.
    spheres: Vec<bool>,
    /// (shape, first instance, count), and the shadowed ones' (shape, instance).
    rocks: Vec<(usize, u32, u32)>,
    shadowed: Vec<(usize, u32)>,
}

pub(crate) struct SpaceRenderer {
    scene: SpaceScene,
    sky: wgpu::RenderPipeline,
    stars: wgpu::RenderPipeline,
    rocks: wgpu::RenderPipeline,
    rock_shadow: wgpu::RenderPipeline,
    sphere: wgpu::RenderPipeline,
    halo_tint: wgpu::RenderPipeline,
    halo_glow: wgpu::RenderPipeline,
    glare: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    /// The same with a stand-in for the shadow maps, while they are drawn.
    shadow_group: wgpu::BindGroup,
    glare_layout: wgpu::BindGroupLayout,
    frame_buffer: wgpu::Buffer,
    sphere_buffer: wgpu::Buffer,
    star_buffer: wgpu::Buffer,
    star_count: u32,
    rock_vertices: wgpu::Buffer,
    rock_indices: wgpu::Buffer,
    /// Each shape's indices and first vertex.
    rock_ranges: Vec<(Range<u32>, i32)>,
    rock_instances: wgpu::Buffer,
    rock_capacity: usize,
    /// The asteroids' shadow maps, a layer each, and a view of each layer to draw into.
    shadow_layers: Vec<wgpu::TextureView>,
    /// The texture's mean colour (linear): asteroids' colours are over it.
    rock_mean: Vec3,
    draws: Draws,
    /// The sun as seen this frame: whether it is in front of the eye (for the glare).
    sun_ahead: bool,
}

impl SpaceRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &SpaceScene, samples: u32) -> Result<SpaceRenderer> {
        let shader = crate::util::shader_standalone(device, "space", &shader_source(samples));
        use wgpu::ShaderStages as S;
        let entry = |binding: u32, visibility: S, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let storage = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("space"),
            entries: &[
                entry(0, S::VERTEX_FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                entry(1, S::VERTEX_FRAGMENT, storage),
                entry(2, S::VERTEX_FRAGMENT, storage),
                entry(3, S::VERTEX_FRAGMENT, storage),
                entry(4, S::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                entry(5, S::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(6, S::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
                entry(7, S::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
            ],
        });
        let glare_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("space glare"),
            entries: &[entry(0, S::VERTEX_FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: samples > 1 })],
        });
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("space"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let glare_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("space glare"), bind_group_layouts: &[Some(&layout), Some(&glare_layout)], immediate_size: 0 });

        let additive = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let keep = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let target = |blend: Option<wgpu::BlendState>| [Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })];
        let opaque = target(None);
        let add = target(Some(wgpu::BlendState { color: additive, alpha: keep }));
        let over = target(Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: keep }));
        let multiply = target(Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add }, alpha: keep }));
        // Reversed depth: nearer is greater.
        let depth = |write: bool, compare: wgpu::CompareFunction| Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil: Default::default(), bias: Default::default() });
        let multisample = wgpu::MultisampleState { count: samples, ..Default::default() };
        let star_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<StarGpu>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Unorm8x4],
        };
        let rock_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RockVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32],
        };
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<RockInstanceGpu>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4],
        };
        let pipeline = |label: &str, vs: &str, fs: Option<&str>, buffers: &[Option<wgpu::VertexBufferLayout>], targets: &[Option<wgpu::ColorTargetState>], depth_stencil: Option<wgpu::DepthStencilState>, layout: &wgpu::PipelineLayout, multisample: wgpu::MultisampleState, cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil,
                multisample,
                fragment: fs.map(|fs| wgpu::FragmentState { module: &shader, entry_point: Some(fs), targets, compilation_options: Default::default() }),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction as C;
        let sky = pipeline("space sky", "vs_fullscreen", Some("fs_sky"), &[], &opaque, depth(false, C::Always), &main_layout, multisample, None);
        let stars = pipeline("space stars", "vs_star", Some("fs_star"), &[Some(star_layout)], &add, depth(false, C::Always), &main_layout, multisample, None);
        let rocks = pipeline("space rocks", "vs_rock", Some("fs_rock"), &[Some(rock_layout.clone()), Some(instance_layout.clone())], &opaque, depth(true, C::GreaterEqual), &main_layout, multisample, Some(wgpu::Face::Back));
        let rock_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("space rock shadows"),
            layout: Some(&main_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_rock_shadow"), buffers: &[Some(rock_layout), Some(instance_layout)], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(C::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.0 },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let sphere = pipeline("space spheres", "vs_sphere", Some("fs_sphere"), &[], &over, depth(false, C::GreaterEqual), &main_layout, multisample, None);
        let halo_tint = pipeline("space air (its tint)", "vs_sphere", Some("fs_halo_tint"), &[], &multiply, depth(false, C::GreaterEqual), &main_layout, multisample, None);
        let halo_glow = pipeline("space air (its glow)", "vs_sphere", Some("fs_halo_glow"), &[], &add, depth(false, C::GreaterEqual), &main_layout, multisample, None);
        let glare = pipeline("space glare", "vs_glare", Some("fs_glare"), &[], &add, None, &glare_pipeline_layout, wgpu::MultisampleState::default(), None);

        // The sky: its stars, nebulae and galaxies, made once.
        let mut catalogue = star_catalogue(&scene.stars, scene.seed);
        let star_count = catalogue.len() as u32;
        if catalogue.is_empty() {
            catalogue.push(StarGpu::zeroed());
        }
        let star_buffer = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("space stars"), contents: bytemuck::cast_slice(&catalogue), usage: wgpu::BufferUsages::VERTEX });
        let nebulae: Vec<NebulaGpu> = scene.nebulae.iter().map(nebula_gpu).collect();
        let galaxies: Vec<GalaxyGpu> = scene.galaxies.iter().map(galaxy_gpu).collect();
        let storage_of = |label: &str, bytes: &[u8], fallback: usize| {
            let blank = vec![0u8; fallback];
            device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: if bytes.is_empty() { &blank } else { bytes }, usage: wgpu::BufferUsages::STORAGE })
        };
        let nebula_buffer = storage_of("space nebulae", bytemuck::cast_slice(&nebulae), std::mem::size_of::<NebulaGpu>());
        let galaxy_buffer = storage_of("space galaxies", bytemuck::cast_slice(&galaxies), std::mem::size_of::<GalaxyGpu>());
        let sphere_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("space spheres"),
            size: (std::mem::size_of::<SphereGpu>() * MAX_SPHERES) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("space frame"), size: std::mem::size_of::<FrameGpu>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        // The asteroids' shapes, made once.
        let mut vertices: Vec<RockVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut rock_ranges = Vec::new();
        for shape in &scene.rocks {
            let (v, i) = rock_mesh(shape).with_context(|| format!("asteroid {}", shape.model))?;
            let first = indices.len() as u32;
            rock_ranges.push((first..first + i.len() as u32, vertices.len() as i32));
            vertices.extend(v);
            indices.extend(i);
        }
        if vertices.is_empty() {
            vertices.push(RockVertex::zeroed());
            indices.extend([0, 0, 0]);
        }
        let rock_vertices = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("space rock vertices"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        let rock_indices = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("space rock indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
        let rock_capacity = 256;
        let rock_instances = device.create_buffer(&wgpu::BufferDescriptor { label: Some("space rocks"), size: (std::mem::size_of::<RockInstanceGpu>() * rock_capacity) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        // The rock texture, with its mipmaps.
        let (texels, mean) = crate::foliage::rock_texture()?;
        let size = crate::foliage::TEXTURE_SIZE;
        let mips = crate::terrain::byte_mips(size, size, 4, &texels);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("space rock"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in mips.iter().enumerate() {
            crate::util::write_texture_level(queue, &texture, level as u32, *w, *h, data, 4);
        }
        let texture_view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("space rock"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("space rock shadows"),
            size: wgpu::Extent3d { width: ROCK_SHADOW, height: ROCK_SHADOW, depth_or_array_layers: SHADOWED_ROCKS as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_array = shadow_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let shadow_layers = (0..SHADOWED_ROCKS as u32)
            .map(|layer| shadow_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: layer, array_layer_count: Some(1), ..Default::default() }))
            .collect();
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("space rock shadows"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let bind = |label: &str, shadows: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: frame_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: sphere_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: nebula_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: galaxy_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&texture_view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(shadows) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
                ],
            })
        };
        let group = bind("space", &shadow_array);
        // (The shadow maps are drawn with a stand-in bound where they are read.)
        let stand_in = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("space no shadows"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let shadow_group = bind("space (its shadow maps)", &stand_in);
        Ok(SpaceRenderer {
            scene: scene.clone(),
            sky,
            stars,
            rocks,
            rock_shadow,
            sphere,
            halo_tint,
            halo_glow,
            glare,
            group,
            shadow_group,
            glare_layout,
            frame_buffer,
            sphere_buffer,
            star_buffer,
            star_count,
            rock_vertices,
            rock_indices,
            rock_ranges,
            rock_instances,
            rock_capacity,
            shadow_layers,
            rock_mean: Vec3::from(mean),
            draws: Draws { spheres: Vec::new(), rocks: Vec::new(), shadowed: Vec::new() },
            sun_ahead: false,
        })
    }

    /// The picture's look this frame (for the post-processing).
    pub fn look(&self, frame: &SpaceFrame) -> crate::post::SpaceLook {
        let l = &self.scene.look;
        crate::post::SpaceLook { exposure: l.exposure, bloom: l.bloom, vignette: l.vignette, fade: frame.fade }
    }

    /// Uploads `frame`: the camera, the spheres farthest first, the asteroids by shape.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &SpaceFrame, size: (u32, u32)) {
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let fov = frame.fov_y.clamp(0.01, 3.0);
        let view = Mat4::from_quat(frame.rotation.normalize().inverse());
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, w / h, NEAR);
        let view_proj = proj * view;
        let sun = &self.scene.sun;
        let sun_dir = sun.direction.normalize_or(Vec3::Y);
        let forward = frame.rotation * Vec3::NEG_Z;
        self.sun_ahead = sun_dir.dot(forward) > 0.0;

        // Spheres: farthest first.
        let mut order: Vec<usize> = (0..frame.spheres.len().min(self.scene.spheres.len()).min(MAX_SPHERES)).collect();
        order.sort_by(|&a, &b| {
            let d = |i: usize| (frame.spheres[i].centre - frame.eye).length();
            d(b).total_cmp(&d(a))
        });
        let rows = |q: Quat| {
            let m = Mat3::from_quat(q.normalize().inverse());
            [0, 1, 2].map(|i| m.row(i).extend(0.0).to_array())
        };
        let spheres: Vec<SphereGpu> = order
            .iter()
            .map(|&i| {
                let (look, pose) = (&self.scene.spheres[i], &frame.spheres[i]);
                let c = pose.centre - frame.eye;
                let air = look.air;
                let (sea, ocean) = match look.ocean {
                    Some([a, b]) => ([a.extend(1.0).to_array(), b.extend(1.0).to_array()], 2),
                    None => ([[0.0; 4]; 2], 0),
                };
                let flags = air.map_or(0, |_| 1) | ocean;
                let height = air.map_or(0.0, |a| a.height.max(1e-4));
                SphereGpu {
                    centre: [c.x, c.y, c.z, look.radius],
                    body: rows(pose.rotation),
                    cloud: rows(pose.clouds),
                    kind: [look.world, look.seed, flags, 0],
                    air: air.map_or([0.0; 4], |a| [a.rayleigh.x, a.rayleigh.y, a.rayleigh.z, height]),
                    // (The molecules' scale height a quarter of the air's, the haze's a fifth of that.)
                    haze: air.map_or([0.0; 4], |a| [a.mie, a.mie_g.clamp(-0.95, 0.95), height * 0.25, height * 0.05]),
                    surface: [look.clouds, look.relief, look.roughness.max(0.02), look.lights],
                    tint: look.tint.extend(1.0).to_array(),
                    sea,
                    shine: look.albedo.extend(1.0).to_array(),
                    look: [look.soft.clamp(0.0, 1.0), look.detail, look.fill.max(0.0), 0.0],
                }
            })
            .collect();
        if !spheres.is_empty() {
            queue.write_buffer(&self.sphere_buffer, 0, bytemuck::cast_slice(&spheres));
        }
        self.draws.spheres = order.iter().map(|&i| self.scene.spheres[i].air.is_some()).collect();

        // Asteroids, by shape; the biggest few on screen shadowed (only those in view compete: one
        // out of it taking a slot took the craters' shadows off one in it, which then popped back).
        let pixel = 2.0 * (fov * 0.5).tan() / h;
        let mut poses: Vec<(usize, &RockPose)> = frame.rocks.iter().enumerate().filter(|(_, r)| r.shape < self.rock_ranges.len()).collect();
        poses.sort_by_key(|(_, r)| r.shape);
        let screen_size = |r: &RockPose| r.size / ((r.centre - frame.eye).length().max(1e-3) * pixel);
        let in_view = |r: &RockPose| {
            let c = view_proj * (r.centre - frame.eye).extend(1.0);
            if c.w <= 0.0 {
                return (r.centre - frame.eye).length() < r.size;
            }
            // (Its middle within the picture, widened by its radius on screen: in each axis's own
            // units, the picture 2 across.)
            let r_px = screen_size(r);
            c.x.abs() <= c.w * (1.0 + 2.0 * r_px / w) && c.y.abs() <= c.w * (1.0 + 2.0 * r_px / h)
        };
        let mut by_size: Vec<usize> = (0..poses.len()).filter(|&k| in_view(poses[k].1)).collect();
        by_size.sort_by(|&a, &b| screen_size(poses[b].1).total_cmp(&screen_size(poses[a].1)));
        let mut slot_of = vec![-1.0f32; poses.len()];
        let mut shadow_mats = [[[0.0f32; 4]; 4]; SHADOWED_ROCKS];
        self.draws.shadowed.clear();
        for (slot, &k) in by_size.iter().take(SHADOWED_ROCKS).enumerate() {
            let r = poses[k].1;
            // (Only those big enough on screen for their craters' shadows to show.)
            if screen_size(r) < 24.0 {
                break;
            }
            slot_of[k] = slot as f32;
            shadow_mats[slot] = rock_shadow_matrix(r.centre - frame.eye, r.size, sun_dir).to_cols_array_2d();
            self.draws.shadowed.push((r.shape, k as u32));
        }
        let instances: Vec<RockInstanceGpu> = poses
            .iter()
            .enumerate()
            .map(|(k, (_, r))| {
                let shape = &self.scene.rocks[r.shape];
                let m = Mat3::from_quat(r.rotation.normalize()) * r.size;
                let t = r.centre - frame.eye;
                let tint = shape.colour / self.rock_mean.max(Vec3::splat(1e-3));
                RockInstanceGpu {
                    m: [m.row(0).extend(t.x).to_array(), m.row(1).extend(t.y).to_array(), m.row(2).extend(t.z).to_array()],
                    tint: tint.extend(shape.texture.max(0.1)).to_array(),
                    extra: [slot_of[k], r.size, 1.0, 0.0],
                }
            })
            .collect();
        if instances.len() > self.rock_capacity {
            self.rock_capacity = instances.len().next_power_of_two();
            self.rock_instances = device.create_buffer(&wgpu::BufferDescriptor { label: Some("space rocks"), size: (std::mem::size_of::<RockInstanceGpu>() * self.rock_capacity) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.rock_instances, 0, bytemuck::cast_slice(&instances));
        }
        self.draws.rocks.clear();
        for (k, (_, r)) in poses.iter().enumerate() {
            match self.draws.rocks.last_mut() {
                Some((shape, _, count)) if *shape == r.shape => *count += 1,
                _ => self.draws.rocks.push((r.shape, k as u32, 1)),
            }
        }

        let l = &self.scene.look;
        let gpu = FrameGpu {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            sun: sun_dir.extend(sun.radius.max(1e-4)).to_array(),
            sun_light: (sun.colour * sun.intensity).extend(sun.disc).to_array(),
            screen: [w, h, pixel, frame.time],
            ambient: l.ambient.extend(self.scene.stars.brightness).to_array(),
            counts: [spheres.len() as u32, self.scene.nebulae.len() as u32, self.scene.galaxies.len() as u32, (frame.time * 60.0).round() as u32],
            stars: [self.scene.stars.size * h / 1080.0, self.scene.stars.colour, sun.glare, sun.ghosts],
            misc: [NEAR, 0.35, 0.0, 0.0],
            fill: sun.fill.normalize_or(Vec3::Z).extend(0.0).to_array(),
            fill_light: sun.fill_light.extend(0.0).to_array(),
            rock_shadow: shadow_mats,
        };
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&gpu));
    }

    /// Draws the frame into the scene's targets (`color` with its resolve target; `resolved` the
    /// single-sampled picture, which the glare is laid over; `depth` the scene's depth).
    pub fn draw(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, (color, resolve): (&wgpu::TextureView, Option<&wgpu::TextureView>), resolved: &wgpu::TextureView, depth: &wgpu::TextureView) {
        // The asteroids' shadow maps.
        for (slot, &(shape, instance)) in self.draws.shadowed.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("space rock shadows"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_layers[slot],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.rock_shadow);
            pass.set_bind_group(0, &self.shadow_group, &[]);
            pass.set_vertex_buffer(0, self.rock_vertices.slice(..));
            pass.set_vertex_buffer(1, self.rock_instances.slice(..));
            pass.set_index_buffer(self.rock_indices.slice(..), wgpu::IndexFormat::Uint32);
            let (range, base) = &self.rock_ranges[shape];
            pass.draw_indexed(range.clone(), *base, instance..instance + 1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("space"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, resolve_target: resolve, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store }, depth_slice: None })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
            if self.star_count > 0 {
                pass.set_pipeline(&self.stars);
                pass.set_vertex_buffer(0, self.star_buffer.slice(..));
                pass.draw(0..6, 0..self.star_count);
            }
            if !self.draws.rocks.is_empty() {
                pass.set_pipeline(&self.rocks);
                pass.set_vertex_buffer(0, self.rock_vertices.slice(..));
                pass.set_vertex_buffer(1, self.rock_instances.slice(..));
                pass.set_index_buffer(self.rock_indices.slice(..), wgpu::IndexFormat::Uint32);
                for &(shape, first, count) in &self.draws.rocks {
                    let (range, base) = &self.rock_ranges[shape];
                    pass.draw_indexed(range.clone(), *base, first..first + count);
                }
            }
            for (i, &air) in self.draws.spheres.iter().enumerate() {
                let i = i as u32;
                pass.set_pipeline(&self.sphere);
                pass.draw(0..3, i..i + 1);
                if air {
                    pass.set_pipeline(&self.halo_tint);
                    pass.draw(0..3, i..i + 1);
                    pass.set_pipeline(&self.halo_glow);
                    pass.draw(0..3, i..i + 1);
                }
            }
        }
        // The sun's glare, as much as it is seen.
        if self.sun_ahead && self.scene.sun.glare > 0.0 {
            let glare_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("space glare"),
                layout: &self.glare_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("space glare"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: resolved, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }, depth_slice: None })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.glare);
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(1, &glare_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// An asteroid's shadow map's projection: along the sunlight, fitted round it (`centre`
/// camera-relative, `radius` its size).
fn rock_shadow_matrix(centre: Vec3, radius: f32, sun: Vec3) -> Mat4 {
    let eye = centre + sun * radius * 2.0;
    let up = if sun.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
    let view = glam::camera::rh::view::look_to_mat4(eye, -sun, up);
    let r = radius * 1.02;
    glam::camera::rh::proj::directx::orthographic(-r, r, -r, r, radius * 0.5, radius * 3.5) * view
}

fn nebula_gpu(n: &Nebula) -> NebulaGpu {
    let c = n.centre.normalize_or(Vec3::NEG_Z);
    let along = (n.along - c * n.along.dot(c)).normalize_or(basis(c).0);
    let across = c.cross(along);
    NebulaGpu {
        centre: c.extend(n.length.max(1e-3)).to_array(),
        along: along.extend(n.width.max(1e-3)).to_array(),
        across: across.extend(n.bend).to_array(),
        colour: n.colour.extend(n.brightness).to_array(),
        colour2: n.colour2.extend(n.filaments).to_array(),
        core: n.core.extend(n.core_brightness).to_array(),
        shape: [n.dust, n.scale.max(0.1), n.warp, (n.seed % 4096) as f32],
        extra: [(n.length.max(1e-3) * 2.0 / n.core_length.max(1e-3)).powi(2), 0.0, 0.0, 0.0],
    }
}

fn galaxy_gpu(g: &Galaxy) -> GalaxyGpu {
    let c = g.centre.normalize_or(Vec3::NEG_Z);
    // The sky's horizontal and vertical at the galaxy, turned by its angle.
    let right = Vec3::Y.cross(-c).normalize_or(Vec3::X);
    let up = (-c).cross(right).normalize_or(Vec3::Y);
    let (s, co) = g.angle.sin_cos();
    let major = right * co + up * s;
    let minor = c.cross(major).normalize_or(up);
    GalaxyGpu {
        centre: c.extend(g.radius.clamp(1e-3, 1.4).tan()).to_array(),
        major: major.extend(g.tilt.cos().abs()).to_array(),
        minor: minor.extend(g.twist).to_array(),
        core: g.core.extend(g.brightness).to_array(),
        arm: g.arm.extend(g.arms.max(1.0)).to_array(),
        knots: g.knots.extend(g.dust).to_array(),
        extra: [(g.seed % 4096) as f32, g.clouds.max(0.0), 0.0, 0.0],
    }
}

/// The buffers' sizes (the shader's layout is tested against them).
#[cfg(test)]
pub(crate) const FRAME_SIZE: usize = std::mem::size_of::<FrameGpu>();
#[cfg(test)]
pub(crate) const SPHERE_SIZE: usize = std::mem::size_of::<SphereGpu>();
#[cfg(test)]
pub(crate) const NEBULA_SIZE: usize = std::mem::size_of::<NebulaGpu>();
#[cfg(test)]
pub(crate) const GALAXY_SIZE: usize = std::mem::size_of::<GalaxyGpu>();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_are_spread_and_varied() {
        let stars = Stars { count: 20000, brightness: 1.0, colour: 0.8, size: 1.0, band: None };
        let cat = star_catalogue(&stars, 7);
        assert_eq!(cat.len(), 20000);
        // Over the whole sky: as many above as below, the mean direction near nothing.
        let mean = cat.iter().map(|s| Vec3::from(s.dir)).sum::<Vec3>() / cat.len() as f32;
        assert!(mean.length() < 0.03, "{mean}");
        // Many faint, few bright.
        let faint = cat.iter().filter(|s| s.flux < 0.05).count();
        let bright = cat.iter().filter(|s| s.flux > 1.0).count();
        assert!(faint > cat.len() / 2 && bright > 10 && bright < cat.len() / 50, "{faint} faint, {bright} bright");
        // Some red, some blue (compared in u16: a channel near 255 plus 40 overflows a u8).
        let red = cat.iter().filter(|s| u16::from(s.colour[0]) > u16::from(s.colour[2]) + 40).count();
        let blue = cat.iter().filter(|s| u16::from(s.colour[2]) > u16::from(s.colour[0]) + 40).count();
        assert!(red > 100 && blue > 100, "{red} red, {blue} blue");
        // The same seed, the same sky.
        assert_eq!(star_catalogue(&stars, 7)[123].dir, cat[123].dir);
    }

    #[test]
    fn a_band_gathers_stars() {
        let stars = Stars { count: 20000, brightness: 1.0, colour: 0.5, size: 1.0, band: Some((Vec3::Y, 0.15, 0.5)) };
        let cat = star_catalogue(&stars, 3);
        let near = cat.iter().filter(|s| s.dir[1].abs() < 0.15).count();
        // Without the band about 15% lie that near the plane; with half of them in it (two thirds
        // of those within a width of it), some 40%.
        assert!(near > cat.len() * 36 / 100, "{near}");
    }

    #[test]
    fn asteroids_are_cratered_rocks() {
        let shape = RockShape { model: "boulder_angular".into(), seed: 5, craters: 1.0, lumps: 1.0, stretch: Vec3::new(1.3, 0.9, 1.0), detail: 2, colour: Vec3::splat(0.3), texture: 2.0 };
        let (vertices, indices) = rock_mesh(&shape).expect("a rock");
        assert_eq!(indices.len() % 3, 0);
        assert!(indices.len() / 3 >= 1280 * 16, "{} triangles", indices.len() / 3);
        // Its radius is 1, its normals point outwards, and it has craters' bowls.
        let reach = vertices.iter().map(|v| Vec3::from(v.pos).length()).fold(0.0f32, f32::max);
        assert!((reach - 1.0).abs() < 1e-4);
        let outward = vertices.iter().filter(|v| Vec3::from(v.pos).dot(Vec3::from(v.normal)) > 0.0).count();
        assert!(outward > vertices.len() * 95 / 100);
        assert!(vertices.iter().filter(|v| v.cavity > 0.5).count() > 100);
        // Every boulder makes one.
        for model in ["boulder_round", "boulder_flat", "boulder_tall"] {
            rock_mesh(&RockShape { model: model.into(), detail: 1, ..shape.clone() }).unwrap_or_else(|e| panic!("{model}: {e}"));
        }
    }
}
