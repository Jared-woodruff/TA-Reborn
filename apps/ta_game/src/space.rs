//! Space scenes for the film (`--open space`, `TA_SPACE=<scene.toml>`). TA's intro movie opens in
//! space (a blue planet and its moon, an asteroid tumbling past, a band of nebula and a distant
//! galaxy), and these are such shots made by the game: a scene file says what is there and how the
//! camera moves, and the renderer draws it (`ta_render::space`). The capture options work as for a
//! battle (`--screenshot`, `--frames`, `TA_SHOTS`, `TA_SHOT_STEP`, `TA_CAPTURE_FPS`): the scene's
//! clock is the capture's, frame n shown at n / `TA_CAPTURE_FPS` seconds. Opened by hand, Escape
//! leaves, Space pauses, the arrow keys step a second back or on, Home starts it again, and the
//! scene is read again whenever its file changes.
//!
//! # The scene file
//!
//! TOML. Lengths in any unit (the examples' planet is 100 across), times in seconds of the scene's
//! clock, angles in degrees, colours as on screen (sRGB, 0 to 1). A direction is `[x, y, z]` (y up;
//! a camera with no yaw looks along -z) or `[azimuth, elevation]` (azimuth 0 along -z, 90 along
//! +x). Everything has a default but the camera's keys; unknown names are errors (typos show).
//!
//! ```toml
//! seed = 7                      # the stars' places
//! exposure = 0.0                # stops
//! bloom = 0.25                  # the glow round bright light
//! ambient = [0.0012, 0.0012, 0.0016]  # the light from everywhere (on the dark sides)
//! vignette = 0.15
//! ease = "none"                 # the camera's path eased at its ends: none, in, out, both
//!
//! [sun]
//! direction = [-60, 20]         # towards it
//! colour = [1.0, 0.98, 0.95]
//! intensity = 2.6               # what a white surface facing it shows
//! size = 0.4                    # its disc's radius
//! disc = 40                     # its disc's brightness over its light
//! glare = 1.0                   # the lens's glare when it is in view
//! ghosts = 1.0                  # and the lens's ghosts
//!
//! [fill]                        # a fill light on the rocks and moons (their shadow sides seen)
//! direction = [-40, 10]         # towards it
//! colour = [0.8, 0.85, 1.0]
//! intensity = 0.3               # none by default
//!
//! [stars]
//! count = 150000                # over the whole sky
//! brightness = 1.0
//! colour = 0.7                  # 0 white, 1 a black body's colour
//! size = 0.75                   # their blur's radius (pixels at 1080 lines)
//! band = [30, 60]               # where they crowd: the band's pole (none by default)
//! band_width = 10               # its half width
//! band_share = 0.3              # of the stars
//!
//! [[nebula]]                    # a band of glowing gas and dust (any number)
//! direction = [20, 10]          # its middle
//! angle = 30                    # its run across the sky (0 level, 90 up)
//! length = 70                   # end to end
//! width = 18
//! bend = 0.2                    # how far it curves
//! colour = [0.35, 0.55, 1.0]    # its gas, drifting from one colour to the other
//! colour2 = [0.85, 0.45, 0.75]
//! core = [1.0, 0.95, 0.9]       # its bright core
//! brightness = 0.5
//! core_brightness = 3.0
//! filaments = 1.0
//! dust = 0.6                    # dark lanes in front (and the stars behind dimmed)
//! scale = 3.0                   # its billows (more: smaller)
//! swirl = 1.2
//! core_length = 14              # its core's length along it (a fifth of its length by default)
//! seed = 3
//!
//! [[galaxy]]                    # a distant spiral (any number)
//! direction = [10, -15]
//! size = 8                      # across
//! tilt = 60                     # from face on
//! angle = 20                    # its long axis on the sky
//! arms = 2
//! twist = 3                     # how tightly they wind
//! brightness = 1.0
//! core = [1.0, 0.92, 0.8]
//! arm = [0.65, 0.75, 1.0]
//! knots = [1.0, 0.45, 0.6]      # where its stars are born (and its gas clouds' colour)
//! clouds = 1.0                  # its great clouds of glowing gas along the arms
//! dust = 0.6
//! seed = 11
//!
//! [[body]]                      # a planet or moon
//! kind = "planet"               # planet, moon (airless and lunar by default) or asteroid
//! world = "Water World"         # its TA world: Green, Lush, Archipelago, Water, Wet Desert,
//!                               # Desert, Red, Lava, Metal, Urban, Ice, Slate, Lunar, Crystal, Acid
//! seed = 3                      # its face (or `map = "Coast To Coast"`: that map's in Conquest)
//! position = [0, 0, 0]
//! velocity = [0, 0, 0]          # units a second
//! radius = 100
//! axis = [20, 70]               # its spin's axis (up by default)
//! spin = 1.5                    # degrees a second
//! turn = 40                     # its turn about its axis at 0 seconds
//! clouds = 1.0                  # its world's cloud cover (0 none), turning on their own:
//! cloud_spin = 2.0              # degrees a second
//! atmosphere = 0.015            # its air's height (of the radius; 0 none; its world's by default)
//! air = [0.06, 0.14, 0.33]      # the air's optical depth straight up (rgb: bluer, bluer sky)
//! haze = 0.05                   # the haze's (white), and how it leans forward (0 to 1)
//! haze_lean = 0.75
//! relief = 1.0                  # how much the ground's shape shows in the light
//! roughness = 0.12              # the sea's (how wide its glint spreads)
//! lights = 1.0                  # what glows by night (cities, lava)
//! tint = [1, 1, 1]              # its colours times this
//! ocean = [[0.1, 0.3, 0.9], [0.02, 0.06, 0.4]]  # its sea (shallow, deep) for its world's
//! albedo = [0.1, 0.2, 0.4]      # its colour as it lights the others (its world's mean)
//! soft = 0                      # without air: 0 the moon's dusty light, 1 soft (Lambert's)
//! detail_bias = 0               # octaves more than its size shows (negative: calmer)
//! fill = 0                      # light on all of it (its night side seen dimly)
//!
//! [[body]]                      # an asteroid: one of TA Reborn's 3D rocks, cratered
//! kind = "asteroid"
//! model = "boulder_angular"     # boulder_round, boulder_flat, boulder_angular, boulder_tall
//! position = [0, -20, 60]       # and velocity, radius, axis, spin, turn as a planet's
//! craters = 1.0                 # (1: some seventy)
//! lumps = 1.0
//! stretch = [1.4, 0.9, 1.0]     # along its own axes
//! detail = 3                    # its triangles split this often (each four from one)
//! colour = [0.42, 0.38, 0.34]
//! texture = 3                   # the rock's texture's tiles across its radius
//!
//! [[field]]                     # asteroids scattered through a box
//! count = 40
//! centre = [0, -30, -200]
//! size = [300, 120, 200]        # the box
//! radius = [0.4, 3]             # smallest and largest (many small)
//! spin = [5, 30]                # slowest and fastest
//! velocity = [0, 0, 0]          # all together
//! scatter = 1.0                 # each its own way too, up to this fast
//! models = ["boulder_round", "boulder_angular"]
//! craters = 1.0                 # and lumps, detail (2), colour, texture as an asteroid's; seed
//!
//! [[camera]]                    # where the camera is at `t` (keys; a key's missing fields keep
//! t = 0                         # the last key's); between keys a smooth curve through them all
//! position = [0, 40, 260]
//! look_at = [0, 0, 0]           # or yaw, pitch, roll (look_at in every key: the point is
//! fov = 40                      # followed; vertical field of view)
//!
//! [[fade]]                      # the picture's brightness at `t` (1; 0 black), straight between
//! t = 4.5                       # keys, as a video fades (its values scaled): a cut from black
//! level = 1                     # is two keys a moment apart
//! ```

use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use glam::{Quat, Vec3};
use serde::Deserialize;
use ta_render::space::{Air, Galaxy, Look, Nebula, RockPose, RockShape, SpaceFrame, SpaceScene, SphereLook, SpherePose, Stars, Sun};

use crate::planet::World;

/// The scene `--open space` shows without `TA_SPACE`: a blue world so near its limb sweeps across
/// the picture, its moon, rocks drifting by, the camera tilting down past them (the intro's opening).
const DEFAULT_SCENE: &str = r#"
seed = 7
exposure = 0.2
[sun]
direction = [0.75, -0.25, 0.6]
[stars]
count = 170000
brightness = 1.3
size = 0.85
[[nebula]]
direction = [30, -40]
angle = 32
length = 80
width = 14
brightness = 0.5
core_brightness = 1.5
seed = 5
[[body]]
kind = "planet"
world = "Water World"
seed = 3
position = [36, 106, -100]
radius = 100
axis = [80, 10]
spin = 0.8
turn = 210
tint = [0.5, 0.62, 1.25]
ocean = [[0.10, 0.28, 0.85], [0.02, 0.06, 0.45]]
clouds = 0.8
atmosphere = 0.03
air = [0.015, 0.04, 0.12]
haze = 0.05
[[body]]
kind = "moon"
seed = 21
position = [-92, 10, -195]
radius = 28
spin = 1.5
tint = [0.46, 0.5, 0.6]
[[field]]
count = 26
centre = [10, 10, -60]
size = [120, 70, 50]
radius = [0.2, 1.6]
velocity = [0.6, 0.3, 0]
scatter = 0.4
[[camera]]
t = 0
position = [0, 0, 0]
yaw = 0
pitch = 5
fov = 40
[[camera]]
t = 7
position = [0, -3, 2]
yaw = 3
pitch = -13
"#;

// ---- The file --------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SceneFile {
    seed: u64,
    exposure: f32,
    bloom: Option<f32>,
    ambient: Option<[f32; 3]>,
    vignette: Option<f32>,
    ease: Option<String>,
    sun: SunFile,
    fill: FillFile,
    stars: StarsFile,
    nebula: Vec<NebulaFile>,
    galaxy: Vec<GalaxyFile>,
    body: Vec<BodyFile>,
    field: Vec<FieldFile>,
    camera: Vec<CameraKey>,
    fade: Vec<FadeKey>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SunFile {
    direction: Option<Vec<f32>>,
    colour: Option<[f32; 3]>,
    intensity: Option<f32>,
    size: Option<f32>,
    disc: Option<f32>,
    glare: Option<f32>,
    ghosts: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct FillFile {
    direction: Option<Vec<f32>>,
    colour: Option<[f32; 3]>,
    intensity: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct StarsFile {
    count: Option<u32>,
    brightness: Option<f32>,
    colour: Option<f32>,
    size: Option<f32>,
    band: Option<Vec<f32>>,
    band_width: Option<f32>,
    band_share: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct NebulaFile {
    direction: Option<Vec<f32>>,
    angle: f32,
    length: Option<f32>,
    width: Option<f32>,
    bend: f32,
    colour: Option<[f32; 3]>,
    colour2: Option<[f32; 3]>,
    core: Option<[f32; 3]>,
    brightness: Option<f32>,
    core_brightness: Option<f32>,
    filaments: Option<f32>,
    dust: Option<f32>,
    scale: Option<f32>,
    swirl: Option<f32>,
    seed: u32,
    core_length: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct GalaxyFile {
    direction: Option<Vec<f32>>,
    size: Option<f32>,
    tilt: Option<f32>,
    angle: f32,
    arms: Option<f32>,
    twist: Option<f32>,
    brightness: Option<f32>,
    core: Option<[f32; 3]>,
    arm: Option<[f32; 3]>,
    knots: Option<[f32; 3]>,
    dust: Option<f32>,
    seed: u32,
    clouds: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct BodyFile {
    kind: String,
    world: Option<String>,
    seed: Option<u64>,
    map: Option<String>,
    position: [f32; 3],
    velocity: [f32; 3],
    radius: Option<f32>,
    axis: Option<Vec<f32>>,
    spin: f32,
    turn: f32,
    clouds: Option<f32>,
    cloud_spin: Option<f32>,
    atmosphere: Option<f32>,
    air: Option<[f32; 3]>,
    haze: Option<f32>,
    haze_lean: Option<f32>,
    relief: Option<f32>,
    roughness: Option<f32>,
    lights: Option<f32>,
    tint: Option<[f32; 3]>,
    ocean: Option<[[f32; 3]; 2]>,
    albedo: Option<[f32; 3]>,
    soft: Option<f32>,
    detail_bias: Option<f32>,
    fill: Option<f32>,
    model: Option<String>,
    craters: Option<f32>,
    lumps: Option<f32>,
    stretch: Option<[f32; 3]>,
    detail: Option<u32>,
    colour: Option<[f32; 3]>,
    texture: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct FieldFile {
    count: u32,
    centre: [f32; 3],
    size: Option<[f32; 3]>,
    radius: Option<[f32; 2]>,
    spin: Option<[f32; 2]>,
    velocity: [f32; 3],
    scatter: f32,
    models: Option<Vec<String>>,
    craters: Option<f32>,
    lumps: Option<f32>,
    detail: Option<u32>,
    colour: Option<[f32; 3]>,
    texture: Option<f32>,
    seed: u32,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
struct CameraKey {
    t: f32,
    position: Option<[f32; 3]>,
    look_at: Option<[f32; 3]>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    roll: Option<f32>,
    fov: Option<f32>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct FadeKey {
    t: f32,
    level: f32,
}

/// A direction as the file gives it: `[x, y, z]`, or `[azimuth, elevation]` in degrees.
fn direction(v: &[f32]) -> Result<Vec3> {
    match *v {
        [x, y, z] => Vec3::new(x, y, z).try_normalize().ok_or_else(|| anyhow!("the direction [{x}, {y}, {z}] has no length")),
        [azimuth, elevation] => Ok(along(azimuth.to_radians(), elevation.to_radians())),
        _ => bail!("a direction is [x, y, z] or [azimuth, elevation], not {v:?}"),
    }
}

/// The way an azimuth and an elevation (radians) point: azimuth 0 along -z, a quarter turn along +x.
fn along(azimuth: f32, elevation: f32) -> Vec3 {
    let (sa, ca) = azimuth.sin_cos();
    let (se, ce) = elevation.sin_cos();
    Vec3::new(sa * ce, se, -ca * ce)
}

/// A colour as the file gives it (sRGB) in linear light.
fn linear(c: [f32; 3]) -> Vec3 {
    Vec3::from(c.map(|v| ta_render::util::srgb_to_linear(v.max(0.0))))
}

// ---- Motion ------------------------------------------------------------------------------------

/// How a body moves: its place at 0 seconds and its velocity; its turn about its axis.
#[derive(Clone, Copy, Debug)]
struct Motion {
    position: Vec3,
    velocity: Vec3,
    /// From its own axes (its pole +y) to its axis's.
    tilt: Quat,
    axis: Vec3,
    /// Radians at 0 seconds, and a second.
    turn: f32,
    spin: f32,
    cloud_spin: f32,
}

impl Motion {
    fn new(position: [f32; 3], velocity: [f32; 3], axis: Vec3, turn: f32, spin: f32, cloud_spin: f32) -> Motion {
        Motion { position: Vec3::from(position), velocity: Vec3::from(velocity), tilt: Quat::from_rotation_arc(Vec3::Y, axis), axis, turn: turn.to_radians(), spin: spin.to_radians(), cloud_spin: cloud_spin.to_radians() }
    }

    fn at(&self, t: f32) -> (Vec3, Quat, Quat) {
        let spun = |rate: f32| Quat::from_axis_angle(self.axis, self.turn + rate * t) * self.tilt;
        (self.position + self.velocity * t, spun(self.spin), spun(self.cloud_spin))
    }
}

/// SplitMix64: the scene's own random numbers (the fields' asteroids).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        ((self.next() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }

    fn direction(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let a = self.range(0.0, std::f32::consts::TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), z, r * a.sin())
    }
}

// ---- The camera's path ------------------------------------------------------------------------

/// A key of the camera's path, every field filled: its place, what it looks at or its angles
/// (yaw, pitch, roll in radians), its field of view (radians).
#[derive(Clone, Copy, Debug)]
struct Key {
    t: f32,
    position: Vec3,
    look_at: Option<Vec3>,
    angles: Vec3,
    fov: f32,
}

/// A curve through keys (time, value): cubic, each key's slope from its neighbours'
/// (Catmull-Rom's, for keys unevenly spaced in time); at the ends the slope of the first and last
/// stretches, or none where eased (it sets off and comes to rest). Held before the first key and
/// after the last.
fn curve<const N: usize>(keys: &[(f32, [f32; N])], t: f32, ease: (bool, bool)) -> [f32; N] {
    let Some(first) = keys.first() else { return [0.0; N] };
    let last = keys[keys.len() - 1];
    if keys.len() == 1 || t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = keys.iter().rposition(|k| k.0 <= t).unwrap_or(0).min(keys.len() - 2);
    let (a, b) = (keys[i], keys[i + 1]);
    let slope = |k: usize| -> [f32; N] {
        let n = keys.len();
        if (k == 0 && ease.0) || (k == n - 1 && ease.1) {
            return [0.0; N];
        }
        let (p, q) = (keys[k.saturating_sub(1)], keys[(k + 1).min(n - 1)]);
        let dt = (q.0 - p.0).max(1e-6);
        std::array::from_fn(|c| (q.1[c] - p.1[c]) / dt)
    };
    let (ma, mb) = (slope(i), slope(i + 1));
    let h = (b.0 - a.0).max(1e-6);
    let s = (t - a.0) / h;
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
    std::array::from_fn(|c| h00 * a.1[c] + h10 * h * ma[c] + h01 * b.1[c] + h11 * h * mb[c])
}

/// Straight lines through keys (time, value), held before the first and after the last.
fn straight(keys: &[(f32, [f32; 1])], t: f32) -> f32 {
    let Some(first) = keys.first() else { return 1.0 };
    let i = keys.iter().rposition(|k| k.0 <= t);
    match i {
        None => first.1[0],
        Some(i) if i + 1 == keys.len() => keys[i].1[0],
        Some(i) => {
            let (a, b) = (keys[i], keys[i + 1]);
            let s = ((t - a.0) / (b.0 - a.0).max(1e-6)).clamp(0.0, 1.0);
            a.1[0] + (b.1[0] - a.1[0]) * s
        }
    }
}

/// The camera's turn: looking along `angles` (yaw, pitch, roll).
fn camera_turn(angles: Vec3) -> Quat {
    Quat::from_rotation_y(-angles.x) * Quat::from_rotation_x(angles.y) * Quat::from_rotation_z(angles.z)
}

/// The yaw and pitch of looking along `forward`.
fn angles_of(forward: Vec3) -> (f32, f32) {
    let f = forward.normalize_or(Vec3::NEG_Z);
    (f.x.atan2(-f.z), f.y.clamp(-1.0, 1.0).asin())
}

// ---- The scene ---------------------------------------------------------------------------------

/// A scene read from its file: what the renderer draws, and how it moves.
pub struct Scene {
    pub scene: SpaceScene,
    spheres: Vec<Motion>,
    /// Each asteroid: its shape, how it moves, its radius.
    rocks: Vec<(usize, Motion, f32)>,
    keys: Vec<Key>,
    /// Every key looks at a point (the point is followed between them).
    follow: bool,
    ease: (bool, bool),
    fades: Vec<(f32, [f32; 1])>,
}

impl Scene {
    pub fn parse(text: &str) -> Result<Scene> {
        let file: SceneFile = toml::from_str(text)?;
        let s = &file.sun;
        let sun = Sun {
            direction: s.direction.as_deref().map(direction).transpose().context("the sun")?.unwrap_or(along(-1.1, 0.4)),
            colour: linear(s.colour.unwrap_or([1.0, 0.98, 0.95])),
            intensity: s.intensity.unwrap_or(2.6),
            radius: s.size.unwrap_or(0.4).to_radians(),
            disc: s.disc.unwrap_or(40.0),
            glare: s.glare.unwrap_or(1.0),
            ghosts: s.ghosts.unwrap_or(1.0),
            fill: file.fill.direction.as_deref().map(direction).transpose().context("the fill light")?.unwrap_or(Vec3::Z),
            fill_light: linear(file.fill.colour.unwrap_or([0.8, 0.85, 1.0])) * file.fill.intensity.unwrap_or(0.0),
        };
        let st = &file.stars;
        let band = match &st.band {
            Some(pole) => Some((direction(pole).context("the stars' band")?, st.band_width.unwrap_or(10.0).to_radians(), st.band_share.unwrap_or(0.3))),
            None => None,
        };
        let stars = Stars { count: st.count.unwrap_or(150_000).min(2_000_000), brightness: st.brightness.unwrap_or(1.0), colour: st.colour.unwrap_or(0.7), size: st.size.unwrap_or(0.75), band };
        let mut nebulae = Vec::new();
        for (i, n) in file.nebula.iter().enumerate() {
            let centre = n.direction.as_deref().map(direction).transpose().with_context(|| format!("nebula {}", i + 1))?.unwrap_or(Vec3::NEG_Z);
            nebulae.push(Nebula {
                centre,
                along: sky_way(centre, n.angle.to_radians()),
                length: n.length.unwrap_or(70.0).to_radians() * 0.5,
                width: n.width.unwrap_or(18.0).to_radians() * 0.5,
                bend: n.bend,
                colour: linear(n.colour.unwrap_or([0.35, 0.55, 1.0])),
                colour2: linear(n.colour2.unwrap_or([0.85, 0.45, 0.75])),
                core: linear(n.core.unwrap_or([1.0, 0.95, 0.9])),
                brightness: n.brightness.unwrap_or(0.5),
                core_brightness: n.core_brightness.unwrap_or(3.0),
                filaments: n.filaments.unwrap_or(1.0),
                dust: n.dust.unwrap_or(0.6),
                scale: n.scale.unwrap_or(3.0),
                warp: n.swirl.unwrap_or(1.2),
                seed: n.seed,
                // (A fifth of its length by default.)
                core_length: n.core_length.map_or(n.length.unwrap_or(70.0).to_radians() * 0.204, f32::to_radians),
            });
        }
        let mut galaxies = Vec::new();
        for (i, g) in file.galaxy.iter().enumerate() {
            galaxies.push(Galaxy {
                centre: g.direction.as_deref().map(direction).transpose().with_context(|| format!("galaxy {}", i + 1))?.unwrap_or(Vec3::NEG_Z),
                radius: g.size.unwrap_or(8.0).to_radians() * 0.5,
                tilt: g.tilt.unwrap_or(60.0).to_radians(),
                angle: g.angle.to_radians(),
                arms: g.arms.unwrap_or(2.0),
                twist: g.twist.unwrap_or(3.0),
                brightness: g.brightness.unwrap_or(1.0),
                core: linear(g.core.unwrap_or([1.0, 0.92, 0.8])),
                arm: linear(g.arm.unwrap_or([0.65, 0.75, 1.0])),
                knots: linear(g.knots.unwrap_or([1.0, 0.45, 0.6])),
                dust: g.dust.unwrap_or(0.6),
                seed: g.seed,
                clouds: g.clouds.unwrap_or(1.0),
            });
        }

        // Planets, moons and asteroids.
        let (mut spheres, mut sphere_motion) = (Vec::new(), Vec::new());
        let (mut shapes, mut rocks) = (Vec::new(), Vec::new());
        for (i, b) in file.body.iter().enumerate() {
            let what = || format!("body {} ({})", i + 1, b.kind);
            let axis = b.axis.as_deref().map(direction).transpose().with_context(what)?.unwrap_or(Vec3::Y);
            match b.kind.as_str() {
                "planet" | "moon" => {
                    let moon = b.kind == "moon";
                    let world = World::of(b.world.as_deref().unwrap_or(if moon { "Lunar" } else { "Green" }));
                    let seed = match (&b.map, b.seed) {
                        (Some(map), _) => crate::planet::seed_of(map),
                        (None, Some(seed)) => seed,
                        (None, None) => i as u64 * 7919 + 17,
                    };
                    let (air_colour, thick) = world.air();
                    let height = b.atmosphere.unwrap_or(if moon || thick <= 0.0 { 0.0 } else { 0.015 });
                    let air = (height > 0.0).then(|| {
                        let k = (thick / 0.6).max(0.3);
                        let a = Vec3::from(air_colour);
                        Air { height, rayleigh: b.air.map(Vec3::from).unwrap_or(a * a * 0.3 * k), mie: b.haze.unwrap_or(0.05 * k), mie_g: b.haze_lean.unwrap_or(0.75) }
                    });
                    spheres.push(SphereLook {
                        world: world.index(),
                        seed: (seed as u32) ^ (seed >> 32) as u32,
                        radius: b.radius.unwrap_or(100.0).max(1e-3),
                        air,
                        clouds: b.clouds.unwrap_or(1.0),
                        relief: b.relief.unwrap_or(1.0),
                        roughness: b.roughness.unwrap_or(0.12),
                        lights: b.lights.unwrap_or(1.0),
                        tint: b.tint.map_or(Vec3::ONE, Vec3::from),
                        ocean: b.ocean.map(|[a, c]| [linear(a), linear(c)]),
                        albedo: b.albedo.map(linear).unwrap_or_else(|| Vec3::from(crate::planet::mean_albedo(world, seed)) * b.tint.map_or(Vec3::ONE, Vec3::from)),
                        soft: b.soft.unwrap_or(0.0),
                        detail: b.detail_bias.unwrap_or(0.0),
                        fill: b.fill.unwrap_or(0.0),
                    });
                    sphere_motion.push(Motion::new(b.position, b.velocity, axis, b.turn, b.spin, b.cloud_spin.unwrap_or(b.spin * 1.3)));
                }
                "asteroid" => {
                    shapes.push(RockShape {
                        model: b.model.clone().unwrap_or_else(|| "boulder_angular".into()),
                        seed: b.seed.unwrap_or(i as u64 * 131 + 5) as u32,
                        craters: b.craters.unwrap_or(1.0),
                        lumps: b.lumps.unwrap_or(1.0),
                        stretch: Vec3::from(b.stretch.unwrap_or([1.3, 0.9, 1.05])),
                        detail: b.detail.unwrap_or(3).min(4),
                        colour: linear(b.colour.unwrap_or([0.42, 0.38, 0.34])),
                        texture: b.texture.unwrap_or(3.0),
                    });
                    rocks.push((shapes.len() - 1, Motion::new(b.position, b.velocity, axis, b.turn, b.spin, 0.0), b.radius.unwrap_or(5.0)));
                }
                other => bail!("{}: kind is planet, moon or asteroid, not {other:?}", what()),
            }
        }
        anyhow::ensure!(spheres.len() <= ta_render::space::MAX_SPHERES, "at most {} planets and moons", ta_render::space::MAX_SPHERES);
        for (i, f) in file.field.iter().enumerate() {
            let mut rng = Rng(f.seed as u64 * 0x9e37 + i as u64 * 7 + 1);
            let models = f.models.clone().unwrap_or_else(|| ["boulder_round", "boulder_angular", "boulder_tall", "boulder_flat"].map(String::from).to_vec());
            anyhow::ensure!(!models.is_empty(), "field {}: no models", i + 1);
            let first = shapes.len();
            let kinds = (f.count as usize).clamp(1, 6);
            for k in 0..kinds {
                shapes.push(RockShape {
                    model: models[k % models.len()].clone(),
                    seed: f.seed.wrapping_mul(31).wrapping_add(k as u32 * 7 + 3),
                    craters: f.craters.unwrap_or(1.0),
                    lumps: f.lumps.unwrap_or(1.0),
                    stretch: Vec3::new(rng.range(1.0, 1.5), rng.range(0.75, 1.0), rng.range(0.85, 1.15)),
                    detail: f.detail.unwrap_or(2).min(4),
                    colour: linear(f.colour.unwrap_or([0.42, 0.38, 0.34])),
                    texture: f.texture.unwrap_or(2.5),
                });
            }
            let size = Vec3::from(f.size.unwrap_or([200.0, 100.0, 200.0]));
            let [small, large] = f.radius.unwrap_or([0.4, 3.0]);
            let [slow, fast] = f.spin.unwrap_or([5.0, 30.0]);
            for _ in 0..f.count {
                let at = Vec3::from(f.centre) + (Vec3::new(rng.unit(), rng.unit(), rng.unit()) - Vec3::splat(0.5)) * size;
                // Many small, few large.
                let radius = small * (large / small.max(1e-3)).powf(rng.unit().powi(2));
                let drift = Vec3::from(f.velocity) + rng.direction() * f.scatter * rng.unit();
                let spin = rng.range(slow, fast) * if rng.unit() < 0.5 { -1.0 } else { 1.0 };
                let shape = first + (rng.next() % kinds as u64) as usize;
                rocks.push((shape, Motion::new(at.to_array(), drift.to_array(), rng.direction(), rng.range(0.0, 360.0), spin, 0.0), radius));
            }
        }

        // The camera's keys, each filled from the last.
        anyhow::ensure!(!file.camera.is_empty(), "a scene needs a [[camera]] key at least");
        let mut keys: Vec<Key> = Vec::new();
        let mut cams = file.camera.clone();
        cams.sort_by(|a, b| a.t.total_cmp(&b.t));
        for c in &cams {
            let last = keys.last().copied();
            let position = c.position.map(Vec3::from).or(last.map(|k| k.position)).unwrap_or(Vec3::new(0.0, 0.0, 300.0));
            let given_angles = c.yaw.is_some() || c.pitch.is_some();
            let look_at = match (c.look_at, given_angles) {
                (Some(p), _) => Some(Vec3::from(p)),
                (None, true) => None,
                (None, false) => last.and_then(|k| k.look_at),
            };
            let previous = last.map_or(Vec3::ZERO, |k| k.angles);
            let mut angles = Vec3::new(
                c.yaw.map_or(previous.x, f32::to_radians),
                c.pitch.map_or(previous.y, f32::to_radians),
                c.roll.map_or(previous.z, f32::to_radians),
            );
            if let Some(p) = look_at {
                let (yaw, pitch) = angles_of(p - position);
                angles.x = yaw;
                angles.y = pitch;
            }
            // (Yaw the short way round from the last key.)
            if last.is_some() {
                angles.x = previous.x + (angles.x - previous.x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            }
            let fov = c.fov.map(f32::to_radians).or(last.map(|k| k.fov)).unwrap_or(45f32.to_radians());
            keys.push(Key { t: c.t, position, look_at, angles, fov });
        }
        let follow = keys.iter().all(|k| k.look_at.is_some());
        let ease = match file.ease.as_deref().unwrap_or("none") {
            "none" => (false, false),
            "in" => (true, false),
            "out" => (false, true),
            "both" => (true, true),
            other => bail!("ease is none, in, out or both, not {other:?}"),
        };
        let mut fades: Vec<(f32, [f32; 1])> = file.fade.iter().map(|f| (f.t, [f.level])).collect();
        fades.sort_by(|a, b| a.0.total_cmp(&b.0));

        let scene = SpaceScene {
            seed: file.seed,
            stars,
            nebulae,
            galaxies,
            sun,
            spheres,
            rocks: shapes,
            look: Look {
                exposure: 2f32.powf(file.exposure),
                bloom: file.bloom.unwrap_or(0.25),
                ambient: file.ambient.map(Vec3::from).unwrap_or(Vec3::new(0.0012, 0.0012, 0.0016)),
                vignette: file.vignette.unwrap_or(0.15),
            },
        };
        Ok(Scene { scene, spheres: sphere_motion, rocks, keys, follow, ease, fades })
    }

    /// The camera at `t`: its place, its turn, its field of view.
    fn camera(&self, t: f32) -> (Vec3, Quat, f32) {
        let pick = |f: &dyn Fn(&Key) -> [f32; 3]| -> Vec<(f32, [f32; 3])> { self.keys.iter().map(|k| (k.t, f(k))).collect() };
        let eye = Vec3::from(curve(&pick(&|k| k.position.to_array()), t, self.ease));
        let fov = curve(&self.keys.iter().map(|k| (k.t, [k.fov])).collect::<Vec<_>>(), t, self.ease)[0];
        let angles = Vec3::from(curve(&pick(&|k| k.angles.to_array()), t, self.ease));
        let turn = if self.follow {
            // The point looked at, followed along its own curve; the roll as keyed.
            let target = Vec3::from(curve(&pick(&|k| k.look_at.unwrap_or(Vec3::ZERO).to_array()), t, self.ease));
            let (yaw, pitch) = angles_of(target - eye);
            camera_turn(Vec3::new(yaw, pitch, angles.z))
        } else {
            camera_turn(angles)
        };
        (eye, turn, fov)
    }

    /// Where everything is at `t` seconds.
    pub fn frame(&self, t: f32) -> SpaceFrame {
        let (eye, rotation, fov_y) = self.camera(t);
        let spheres = self
            .spheres
            .iter()
            .map(|m| {
                let (centre, rotation, clouds) = m.at(t);
                SpherePose { centre, rotation, clouds }
            })
            .collect();
        let rocks = self
            .rocks
            .iter()
            .map(|&(shape, m, size)| {
                let (centre, rotation, _) = m.at(t);
                RockPose { shape, centre, rotation, size }
            })
            .collect();
        let fade = if self.fades.is_empty() { 1.0 } else { straight(&self.fades, t).max(0.0) };
        SpaceFrame { time: t, eye, rotation, fov_y, spheres, rocks, fade }
    }
}

/// The way along the sky at `centre`, turned `angle` (radians) from level towards up.
fn sky_way(centre: Vec3, angle: f32) -> Vec3 {
    let right = centre.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(centre).normalize_or(Vec3::Y);
    right * angle.cos() + up * angle.sin()
}

// ---- The page ---------------------------------------------------------------------------------

/// A space scene on screen: its file (if any), its clock.
pub struct SpaceView {
    pub scene: Scene,
    path: Option<std::path::PathBuf>,
    modified: Option<SystemTime>,
    /// Seconds on the scene's clock, and whether it stands still.
    pub time: f32,
    pub paused: bool,
    /// When the file was last looked at.
    checked: Instant,
}

impl SpaceView {
    /// The scene `TA_SPACE` names (or the built-in one), up in the renderer.
    pub fn open(renderer: &mut ta_render::Renderer) -> Result<SpaceView> {
        let path = std::env::var_os("TA_SPACE").map(std::path::PathBuf::from);
        let (scene, modified) = match &path {
            Some(p) => (read(p)?, modified(p)),
            None => (Scene::parse(DEFAULT_SCENE).context("the built-in scene")?, None),
        };
        renderer.set_space(Some(&scene.scene))?;
        log::info!(
            "space: {} (Escape leaves, Space pauses, the arrow keys step a second, Home starts again)",
            path.as_ref().map_or("the built-in scene".to_string(), |p| p.display().to_string())
        );
        Ok(SpaceView { scene, path, modified, time: 0.0, paused: false, checked: Instant::now() })
    }

    /// Reads the scene again if its file changed (looked at twice a second); a scene that does
    /// not read is said in the log, and the last one stays.
    pub fn reload(&mut self, renderer: &mut ta_render::Renderer) {
        if self.checked.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.checked = Instant::now();
        let Some(path) = &self.path else { return };
        let stamp = modified(path);
        if stamp == self.modified {
            return;
        }
        self.modified = stamp;
        match read(path).and_then(|scene| renderer.set_space(Some(&scene.scene)).map(|_| scene)) {
            Ok(scene) => {
                log::info!("space: {} read again", path.display());
                self.scene = scene;
            }
            Err(e) => {
                log::error!("space: {}: {e:#}", path.display());
                let _ = renderer.set_space(Some(&self.scene.scene));
            }
        }
    }
}

fn read(path: &std::path::Path) -> Result<Scene> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Scene::parse(&text).with_context(|| path.display().to_string())
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_scene_reads() {
        let scene = Scene::parse(DEFAULT_SCENE).expect("the built-in scene");
        assert_eq!(scene.scene.spheres.len(), 2);
        assert!(scene.scene.spheres[0].air.is_some() && scene.scene.spheres[1].air.is_none(), "the planet has air, its moon none");
        assert_eq!(scene.scene.spheres[1].world, World::Lunar.index());
        assert_eq!(scene.rocks.len(), 26);
        let f = scene.frame(5.0);
        assert_eq!(f.spheres.len(), 2);
        assert!(f.fade == 1.0 && f.fov_y > 0.5);
    }

    /// The example at the top of this file is a scene that reads, with every name it shows.
    #[test]
    fn the_documented_format_reads() {
        let source = include_str!("space.rs");
        let doc: String = source.lines().take_while(|l| l.starts_with("//!")).map(|l| l.strip_prefix("//! ").unwrap_or(l.trim_start_matches("//!"))).collect::<Vec<_>>().join("
");
        let start = doc.find("```toml").expect("the format's example") + "```toml".len();
        let end = start + doc[start..].find("```").expect("its end");
        let scene = Scene::parse(&doc[start..end]).unwrap_or_else(|e| panic!("the documented scene: {e:#}"));
        assert_eq!((scene.scene.nebulae.len(), scene.scene.galaxies.len(), scene.scene.spheres.len()), (1, 1, 1));
        assert_eq!(scene.rocks.len(), 1 + 40);
        assert!(scene.scene.stars.band.is_some() && scene.fades.len() == 1);
    }

    #[test]
    fn directions_and_typos() {
        assert!((direction(&[0.0, 0.0]).unwrap() - Vec3::NEG_Z).length() < 1e-6);
        assert!((direction(&[90.0, 0.0]).unwrap() - Vec3::X).length() < 1e-6);
        assert!((direction(&[0.0, 90.0]).unwrap() - Vec3::Y).length() < 1e-6);
        assert!(direction(&[1.0]).is_err());
        // A misspelt name is an error, not a silent default.
        assert!(Scene::parse("[[camera]]\nt = 0\n[[body]]\nkind = \"planet\"\nradios = 3\n").is_err());
        assert!(Scene::parse("[[body]]\nkind = \"planet\"\n").is_err(), "no camera");
    }

    #[test]
    fn the_camera_moves_smoothly_through_its_keys() {
        let scene = Scene::parse("[[camera]]\nt = 0\nposition = [0, 0, 0]\nyaw = 0\n[[camera]]\nt = 2\nposition = [10, 0, 0]\n[[camera]]\nt = 4\nposition = [20, 10, 0]\nyaw = 90\n").unwrap();
        // Through the keys, held past them.
        assert!((scene.frame(2.0).eye - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4);
        assert!((scene.frame(9.0).eye - Vec3::new(20.0, 10.0, 0.0)).length() < 1e-4);
        // Without easing the first stretch sets off at its own speed (5 a second).
        let v = (scene.frame(0.01).eye - scene.frame(0.0).eye) / 0.01;
        assert!((v.x - 5.0).abs() < 0.3, "{v}");
        // No jumps: the speed is continuous through the middle key.
        let speed = |t: f32| (scene.frame(t + 0.001).eye - scene.frame(t - 0.001).eye) / 0.002;
        assert!((speed(1.999) - speed(2.001)).length() < 0.05);
        // Yaw 90 looks along +x.
        let forward = scene.frame(4.0).rotation * Vec3::NEG_Z;
        assert!((forward - Vec3::X).length() < 1e-4, "{forward}");
        // Easing both ends: it sets off from rest.
        let eased = Scene::parse("ease = \"both\"\n[[camera]]\nt = 0\nposition = [0, 0, 0]\n[[camera]]\nt = 2\nposition = [10, 0, 0]\n").unwrap();
        assert!((eased.frame(0.01).eye.x / 0.01) < 0.2);
    }

    #[test]
    fn looking_at_a_point_follows_it() {
        let scene = Scene::parse("[[camera]]\nt = 0\nposition = [0, 0, 100]\nlook_at = [0, 0, 0]\n[[camera]]\nt = 4\nposition = [100, 0, 0]\nlook_at = [0, 0, 0]\n").unwrap();
        for t in [0.0, 1.0, 2.0, 3.0, 4.0] {
            let f = scene.frame(t);
            let forward = f.rotation * Vec3::NEG_Z;
            assert!(forward.dot(-f.eye.normalize()) > 0.999, "at {t} it looks away");
        }
    }

    #[test]
    fn fades_are_straight_and_cut_where_asked() {
        let scene = Scene::parse("[[camera]]
t = 0
[[fade]]
t = 1.99
level = 0
[[fade]]
t = 2
level = 0.5
[[fade]]
t = 4
level = 1
").unwrap();
        assert_eq!(scene.frame(1.0).fade, 0.0);
        assert!((scene.frame(2.0).fade - 0.5).abs() < 1e-6);
        assert!((scene.frame(3.0).fade - 0.75).abs() < 1e-6);
        assert_eq!(scene.frame(9.0).fade, 1.0);
    }

    #[test]
    fn fields_scatter_their_asteroids() {
        let scene = Scene::parse("[[camera]]\nt = 0\n[[field]]\ncount = 50\ncentre = [0, 0, -100]\nsize = [100, 50, 20]\nradius = [1, 4]\n").unwrap();
        assert_eq!(scene.rocks.len(), 50);
        assert_eq!(scene.scene.rocks.len(), 6);
        let f = scene.frame(0.0);
        for r in &f.rocks {
            assert!((r.centre.x).abs() <= 50.0 && (r.centre.z + 100.0).abs() <= 10.0 && (1.0..=4.0).contains(&r.size));
        }
    }
}
