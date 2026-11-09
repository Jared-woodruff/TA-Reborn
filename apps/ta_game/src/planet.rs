//! The galaxy map's planets (Conquest): each the world its map's planet names (TA groups its
//! maps by planet: green, lush, desert, lava, metal, ice, ...), its surface grown from noise
//! (nothing of TA's art), lit as a ball from the upper left as the plain ones were, and kept
//! as a small texture made at the size it is shown (egui has no mipmaps).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

/// The kinds of world, as TA's maps name their planets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum World {
    Green,
    Lush,
    Archipelago,
    Water,
    WetDesert,
    Desert,
    Red,
    Lava,
    Metal,
    Urban,
    Ice,
    Slate,
    Lunar,
    Crystal,
    Acid,
    /// TA Reborn's maps of real places: oceans, continents green to desert, ice at the poles.
    Earth,
    /// TA Reborn's worlds in space: a rock adrift, cratered, without air.
    Asteroid,
}

impl World {
    /// The world a map's planet names (a map that names none, or one not known, is green, as
    /// the terrain takes it).
    pub fn of(planet: &str) -> World {
        let p = planet.to_ascii_lowercase();
        const KEYS: [(&str, World); 17] = [
            ("earth", World::Earth),
            ("asteroid", World::Asteroid),
            ("wet desert", World::WetDesert),
            ("desert", World::Desert),
            ("green", World::Green),
            ("lush", World::Lush),
            ("red", World::Red),
            ("lava", World::Lava),
            ("metal", World::Metal),
            ("ice", World::Ice),
            ("archipelago", World::Archipelago),
            ("water", World::Water),
            ("slate", World::Slate),
            ("lunar", World::Lunar),
            ("crystal", World::Crystal),
            ("acid", World::Acid),
            ("urban", World::Urban),
        ];
        KEYS.iter().find(|(k, _)| p.contains(k)).map_or(World::Green, |&(_, w)| w)
    }

    /// Its number as space's shader knows the worlds (`ta_render::space::WORLDS`, in this
    /// enum's order). The shader has no Earth or asteroid of its own: it draws them as the
    /// worlds nearest them, an archipelago and a moon (the galaxy map draws them as they are).
    pub fn index(self) -> u32 {
        match self {
            World::Earth => World::Archipelago as u32,
            World::Asteroid => World::Lunar as u32,
            _ => self as u32,
        }
    }

    /// The air along its limb, and how thick it is (0: none).
    pub fn air(self) -> ([f32; 3], f32) {
        match self {
            World::Green | World::Lush => ([0.50, 0.70, 1.00], 0.60),
            World::Archipelago | World::Water => ([0.45, 0.72, 1.00], 0.65),
            World::WetDesert => ([0.85, 0.80, 0.70], 0.40),
            World::Desert => ([0.95, 0.80, 0.58], 0.38),
            World::Red => ([0.95, 0.62, 0.45], 0.32),
            World::Lava => ([1.00, 0.45, 0.20], 0.40),
            World::Metal => ([0.66, 0.74, 0.86], 0.25),
            World::Urban => ([0.70, 0.76, 0.90], 0.32),
            World::Ice => ([0.72, 0.90, 1.00], 0.55),
            World::Slate => ([0.62, 0.68, 0.80], 0.22),
            World::Lunar => ([0.0, 0.0, 0.0], 0.0),
            World::Crystal => ([0.62, 0.92, 1.00], 0.50),
            World::Acid => ([0.78, 0.95, 0.35], 0.55),
            World::Earth => ([0.50, 0.70, 1.00], 0.62),
            World::Asteroid => ([0.0, 0.0, 0.0], 0.0),
        }
    }
}

/// How long making planets may take of a frame: more wait for the next (a galaxy's dozen or
/// two take a few frames, each fading in over its stand-in).
const BUDGET: Duration = Duration::from_millis(5);
/// How long a planet takes to fade in over its stand-in, in seconds.
const FADE: f64 = 0.3;

type Key = (World, u64, u32);

/// The planets' textures, by world, seed and size in pixels: made when first shown (a few a
/// frame), those not shown for a while let go.
#[derive(Default)]
pub struct Planets {
    /// Each with the frame it was last shown on and when it was made.
    textures: HashMap<Key, (TextureHandle, u64, f64)>,
    frame: u64,
    spent: Duration,
}

impl Planets {
    /// Paints the planet of `planet` (a map's planet name) seeded by `seed` (its map's), `r`
    /// points round `p`; `stand_in` paints what shows until it is made, and under it while it
    /// fades in.
    pub fn paint(&mut self, painter: &egui::Painter, p: egui::Pos2, r: f32, planet: &str, seed: u64, stand_in: impl FnOnce()) {
        let ctx = painter.ctx();
        let frame = ctx.cumulative_pass_nr();
        if frame != self.frame {
            self.frame = frame;
            self.spent = Duration::ZERO;
            // (Galaxies come and go with NEW GALAXY: what has not been shown for a minute's
            // frames goes.)
            self.textures.retain(|_, (_, seen, _)| frame.saturating_sub(*seen) < 3600);
        }
        let ppp = ctx.pixels_per_point();
        // The ball `r` + 0.5 points round, on a sprite with a pixel's room round it.
        let radius = (r + 0.5) * ppp;
        let px = (2.0 * radius + 2.0).ceil() as u32;
        let world = World::of(planet);
        let key = (world, seed, px);
        let now = ctx.input(|i| i.time);
        if !self.textures.contains_key(&key) && self.spent < BUDGET {
            let start = Instant::now();
            let image = image(world, seed, px as usize, radius);
            self.spent += start.elapsed();
            let texture = ctx.load_texture(format!("planet-{world:?}-{seed:x}-{px}"), image, TextureOptions::LINEAR);
            self.textures.insert(key, (texture, frame, now));
        }
        let Some((texture, seen, made)) = self.textures.get_mut(&key) else {
            stand_in();
            ctx.request_repaint();
            return;
        };
        *seen = frame;
        let shown = ((now - *made) / FADE).clamp(0.0, 1.0) as f32;
        if shown < 1.0 {
            stand_in();
            ctx.request_repaint();
        }
        let size = px as f32 / ppp;
        let rect = egui::Rect::from_center_size(p, egui::vec2(size, size));
        painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(shown));
    }
}

/// A seed for a planet from its map's name (the same world every time it comes up).
pub fn seed_of(map: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in map.to_ascii_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A world's mean colour (linear), its clouds counted: how its face throws the sunlight back (the
/// light a planet sheds on its moons in space, `crate::space`). From its surface at points spread
/// evenly over the ball.
pub fn mean_albedo(world: World, seed: u64) -> [f32; 3] {
    let s = seed as u32 ^ (seed >> 32) as u32;
    let n = 2000;
    let mut sum = [0.0f32; 3];
    for i in 0..n {
        // (A Fibonacci lattice: as many points on every part of the ball.)
        let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
        let r = (1.0 - y * y).max(0.0).sqrt();
        let a = i as f32 * 2.399_963;
        let g = ground(world, [r * a.cos(), y, r * a.sin()], s, 4, 0.01);
        for (total, albedo) in sum.iter_mut().zip(g.albedo) {
            *total += ta_render::util::srgb_to_linear(albedo * (1.0 - g.cloud) + 0.94 * g.cloud) / n as f32;
        }
    }
    sum
}

/// What a point of a world's surface is: its colour by day, how much it shines back the sun
/// (water, ice), what glows of itself (lava, city lights; brightest by night) and the cloud
/// over it.
struct Ground {
    albedo: [f32; 3],
    shine: f32,
    glow: [f32; 3],
    cloud: f32,
}

/// The planet as premultiplied RGBA, `px` pixels square, the ball `radius` pixels round its
/// middle: lit from the upper left, a soft terminator, the night side dim, what glows of
/// itself brightest there, a glint on water and ice, and the lit air along the limb.
pub fn image(world: World, seed: u64, px: usize, radius: f32) -> ColorImage {
    let light = norm([-0.45, -0.55, 0.7]);
    let half = norm([light[0], light[1], light[2] + 1.0]);
    let (air, thick) = world.air();
    // Each world turned its own way: its axis tipped and spun round (from the seed).
    let s = seed as u32 ^ (seed >> 32) as u32;
    let tilt = (unit(hash(s, 1)) - 0.5) * 0.9;
    let spin = unit(hash(s, 2)) * std::f32::consts::TAU;
    let (ct, st, cs, ss) = (tilt.cos(), tilt.sin(), spin.cos(), spin.sin());
    // A pixel's width on the ball; and as much detail as the size shows (each octave of the
    // noise twice as fine: none finer than a pixel).
    let pixel = 1.0 / radius;
    let detail = (radius.log2() + 0.5).clamp(2.0, 6.0) as u32;
    let centre = px as f32 / 2.0;
    let mut pixels = Vec::with_capacity(px * px);
    for j in 0..px {
        for i in 0..px {
            let x = (i as f32 + 0.5 - centre) / radius;
            let y = (j as f32 + 0.5 - centre) / radius;
            let d = (x * x + y * y).sqrt();
            // (The edge's coverage of the pixel.)
            let alpha = ((1.0 - d) / pixel + 0.5).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                pixels.push(Color32::TRANSPARENT);
                continue;
            }
            let (x, y) = if d > 1.0 { (x / d, y / d) } else { (x, y) };
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            let n = [x, y, z];
            // The point on the world: tipped about x, then spun about its axis.
            let (y1, z1) = (y * ct - z * st, y * st + z * ct);
            let q = [x * cs + z1 * ss, y1, -x * ss + z1 * cs];
            let g = ground(world, q, s, detail, pixel / z.max(0.25));

            let lit = dot(n, light);
            let day = ((lit + 0.12) / 1.12).clamp(0.0, 1.0);
            let sun = 0.10 + 0.95 * day.powf(0.85);
            let night = 1.0 - day;
            let cloud = [0.94, 0.95, 0.97];
            let mut c = [0.0f32; 3];
            for k in 0..3 {
                let surface = g.albedo[k] * (1.0 - g.cloud) + cloud[k] * g.cloud;
                c[k] = surface * sun + g.glow[k] * (0.45 + 0.55 * night) * (1.0 - 0.7 * g.cloud);
            }
            let glint = dot(n, half).max(0.0).powi(64) * g.shine * (1.0 - g.cloud) * day * 0.5;
            // The air: lit along the limb on the day side, faint round the night side.
            let rim = (1.0 - z).powf(2.4) * thick * (0.2 + 0.8 * (lit * 1.6 + 0.35).clamp(0.0, 1.0));
            // (A world without air darkens towards its limb.)
            let limb = if thick > 0.0 { 1.0 } else { 0.72 + 0.28 * z };
            for k in 0..3 {
                c[k] = ((c[k] * limb + glint) * (1.0 - rim) + air[k] * rim * (0.35 + 0.65 * day)).clamp(0.0, 1.0);
            }
            let to = |v: f32| (v * alpha * 255.0 + 0.5) as u8;
            pixels.push(Color32::from_rgba_premultiplied(to(c[0]), to(c[1]), to(c[2]), (alpha * 255.0 + 0.5) as u8));
        }
    }
    ColorImage::new([px, px], pixels)
}

/// What the world is at `q` (a point of the unit ball), seeded by `s`, in noise of at most
/// `detail` octaves; `fine` is a pixel's width there, the least a line's softness.
fn ground(world: World, q: [f32; 3], s: u32, detail: u32, fine: f32) -> Ground {
    let at = |f: f32, o: f32| [q[0] * f + o, q[1] * f + o * 0.7, q[2] * f - o * 1.3];
    let noise = |p: [f32; 3], octaves: u32, salt: u32| fbm(p, octaves.min(detail), s ^ salt);
    let plain = |albedo: [f32; 3]| Ground { albedo, shine: 0.0, glow: [0.0; 3], cloud: 0.0 };
    // Clouds: big soft swirls (stretched along the lines of latitude).
    let clouds = |cover: f32| {
        let p = [q[0] * 2.6, q[1] * 4.7, q[2] * 2.6];
        let w = noise([p[0] + 5.1, p[1], p[2] - 2.3], 3, 0x55);
        smooth(cover, cover + 0.18, noise([p[0] + w * 1.6, p[1] + w, p[2]], 5, 0x77)) * 0.85
    };
    // A network of lines where the noise crosses its middle: 1 on them, 0 away.
    let lines = |p: [f32; 3], octaves: u32, salt: u32, width: f32| {
        let v = (noise(p, octaves, salt) - 0.5).abs();
        smooth(width + fine * 1.5, (width - fine * 1.5).max(0.0), v)
    };
    match world {
        World::Green | World::Lush | World::Archipelago | World::Water | World::Acid => {
            let (sea, cover) = match world {
                World::Green => (0.50, 0.56),
                World::Lush => (0.42, 0.56),
                World::Archipelago => (0.60, 0.58),
                World::Water => (0.67, 0.56),
                _ => (0.50, 0.66),
            };
            let h = noise(at(1.9, 0.0), 6, 0);
            let acid = world == World::Acid;
            if h < sea {
                // The sea: deeper away from the shore.
                let deep = smooth(0.0, 0.12, sea - h);
                let (shallow, abyss) = if acid { ([0.70, 0.80, 0.20], [0.42, 0.56, 0.08]) } else { ([0.16, 0.42, 0.56], [0.05, 0.16, 0.36]) };
                let glow = if acid { [0.06, 0.09, 0.0] } else { [0.0; 3] };
                return Ground { albedo: mix3(shallow, abyss, deep), shine: if acid { 0.5 } else { 1.0 }, glow, cloud: clouds(cover) };
            }
            let up = smooth(sea, sea + 0.22, h);
            let wood = noise(at(4.3, 7.0), 4, 0x13);
            let albedo = if acid {
                mix3([0.30, 0.31, 0.16], [0.17, 0.16, 0.10], wood)
            } else {
                let lush = if world == World::Lush { 0.25 } else { 0.0 };
                let low = mix3([0.31, 0.52, 0.22], [0.14, 0.35, 0.16], (wood - 0.15 + lush).clamp(0.0, 1.0));
                let beach = smooth(0.025, 0.0, h - sea) * 0.7;
                let ground = mix3(low, [0.47, 0.41, 0.29], (up * 1.3 - 0.35 - lush).clamp(0.0, 1.0));
                mix3(ground, [0.72, 0.66, 0.48], beach)
            };
            // Snow on the peaks and towards the poles.
            let snow = if acid { 0.0 } else { smooth(0.80, 0.90, q[1].abs() + (wood - 0.5) * 0.2).max(smooth(0.80, 0.86, h) * 0.8) };
            Ground { albedo: mix3(albedo, [0.90, 0.93, 0.96], snow), shine: 0.0, glow: [0.0; 3], cloud: clouds(cover) }
        }
        World::WetDesert => {
            let h = noise(at(2.1, 0.0), 6, 0);
            let rock = noise(at(5.0, 3.0), 4, 0x21);
            if h < 0.42 {
                let deep = smooth(0.0, 0.1, 0.42 - h);
                return Ground { albedo: mix3([0.26, 0.48, 0.52], [0.10, 0.26, 0.38], deep), shine: 0.9, glow: [0.0; 3], cloud: clouds(0.68) };
            }
            let green = smooth(0.06, 0.0, h - 0.42) * 0.8;
            let sand = mix3([0.78, 0.64, 0.42], [0.56, 0.44, 0.30], smooth(0.45, 0.7, rock));
            Ground { albedo: mix3(sand, [0.36, 0.44, 0.24], green), shine: 0.0, glow: [0.0; 3], cloud: clouds(0.68) }
        }
        World::Desert => {
            // Dunes in bands round the world, rock showing through.
            let w = noise(at(2.4, 1.0), 4, 0x31);
            let band = 0.5 + 0.5 * (q[1] * 26.0 + w * 9.0).sin();
            let rock = noise(at(3.2, 4.0), 5, 0x33);
            let sand = mix3([0.86, 0.68, 0.42], [0.72, 0.53, 0.31], band * 0.6);
            plain(mix3(sand, [0.48, 0.34, 0.22], smooth(0.56, 0.70, rock)))
        }
        World::Red => {
            let h = noise(at(2.0, 0.0), 6, 0);
            let base = mix3([0.68, 0.32, 0.19], [0.40, 0.19, 0.13], smooth(0.42, 0.62, h));
            let base = mix3(base, [0.82, 0.48, 0.30], smooth(0.62, 0.78, noise(at(3.6, 2.0), 4, 0x41)) * 0.6);
            let base = crater(base, q, s ^ 0x43, 5.0, 0.55);
            let pole = smooth(0.86, 0.93, q[1].abs() + (h - 0.5) * 0.15);
            plain(mix3(base, [0.92, 0.86, 0.84], pole))
        }
        World::Lava => {
            // A dark crust in plates, split by glowing rifts, melt pooled in the low ground and
            // the crust's own heat dull red near them.
            let crust = noise(at(3.0, 0.0), 5, 0);
            let albedo = mix3([0.17, 0.14, 0.13], [0.07, 0.06, 0.06], crust);
            let rift = lines(at(1.7, 9.0), 4, 0x51, 0.022);
            let near = lines(at(1.7, 9.0), 4, 0x51, 0.09) * 0.35;
            let pool = smooth(0.71, 0.77, noise(at(1.4, 4.0), 3, 0x53));
            let heat = rift.max(pool);
            let hot = mix3([0.80, 0.20, 0.03], [1.0, 0.70, 0.26], heat * heat);
            let glow = [hot[0] * heat + 0.45 * near, hot[1] * heat + 0.06 * near, hot[2] * heat];
            Ground { albedo: mix3(albedo, [0.30, 0.10, 0.04], heat), shine: 0.0, glow, cloud: 0.0 }
        }
        World::Metal | World::Urban => {
            // Plating laid in courses round the world, each plate its own shade, the seams
            // between faint; dark canyons across it; and lights by night, in clusters (cities).
            let lon = q[2].atan2(q[0]);
            let lat = q[1].clamp(-1.0, 1.0).asin();
            let rows = 13.0;
            let v = (lat / std::f32::consts::PI + 0.5) * rows;
            let row = v.floor();
            let cols = (26.0 * lat.cos()).round().max(3.0);
            let u = (lon / std::f32::consts::TAU + 0.5) * cols + unit(hash(s ^ 0x61, row as i32 as u32));
            let plate = hash(s ^ 0x62, (u.floor() as i32 as u32).wrapping_mul(131) ^ row as i32 as u32);
            let (fu, fv) = (u.fract().min(1.0 - u.fract()), v.fract().min(1.0 - v.fract()));
            let seam = smooth(fine * cols / std::f32::consts::TAU / lat.cos().max(0.2) * 1.1, 0.0, fu).max(smooth(fine * rows / std::f32::consts::PI * 1.1, 0.0, fv));
            let shade = 0.84 + 0.30 * unit(plate);
            let (base, dark) = if world == World::Metal { ([0.50, 0.52, 0.55], [0.22, 0.24, 0.27]) } else { ([0.42, 0.46, 0.53], [0.18, 0.20, 0.25]) };
            let canyon = lines(at(1.6, 3.0), 4, 0x63, 0.03);
            let albedo = mix3([base[0] * shade, base[1] * shade, base[2] * shade], dark, (seam * 0.45).max(canyon * 0.9));
            let city = smooth(if world == World::Urban { 0.42 } else { 0.56 }, 0.68, noise(at(2.2, 5.0), 3, 0x65));
            let spark = smooth(0.62, 0.74, noise(at(14.0, 1.0), 2, 0x67));
            let lamp = city * spark;
            Ground { albedo, shine: 0.25, glow: [1.0 * lamp, 0.74 * lamp, 0.40 * lamp], cloud: 0.0 }
        }
        World::Ice => {
            let h = noise(at(2.2, 0.0), 5, 0);
            let crevasse = lines(at(2.6, 6.0), 4, 0x71, 0.02);
            let floe = smooth(0.40, 0.33, h);
            let albedo = mix3([0.90, 0.94, 0.98], [0.68, 0.80, 0.90], smooth(0.45, 0.65, h));
            let albedo = mix3(albedo, [0.30, 0.48, 0.66], floe.max(crevasse * 0.7));
            Ground { albedo, shine: 0.35 + floe * 0.4, glow: [0.0; 3], cloud: clouds(0.68) * 0.7 }
        }
        World::Slate => {
            // Grey-blue rock laid down in folded bands, dark faults across them.
            let w = noise(at(1.8, 0.0), 4, 0);
            let band = 0.5 + 0.5 * (q[0] * 7.0 + q[1] * 11.0 + w * 8.0).sin();
            let base = mix3([0.46, 0.48, 0.54], [0.29, 0.31, 0.37], smooth(0.4, 0.65, noise(at(3.0, 5.0), 5, 0x81)));
            let base = mix3(base, [0.58, 0.61, 0.66], smooth(0.55, 0.95, band) * 0.5);
            let fault = lines(at(2.4, 2.0), 4, 0x83, 0.018);
            plain(mix3(base, [0.14, 0.15, 0.18], fault * 0.75))
        }
        World::Lunar => {
            let h = noise(at(1.8, 0.0), 5, 0);
            let base = mix3([0.62, 0.62, 0.60], [0.36, 0.36, 0.37], smooth(0.48, 0.60, h));
            plain(crater(crater(base, q, s ^ 0x91, 4.0, 0.7), q, s ^ 0x93, 9.0, 0.6))
        }
        World::Earth => {
            // Oceans, deep away from the coasts, frozen towards the poles; land green in the
            // tropics and the temperate belts, desert either side of them, tundra and ice
            // beyond, mountains brown; clouds, and the cities' lights by night.
            let sea = 0.54;
            let h = noise(at(1.7, 0.0), 6, 0);
            let lat = q[1].abs();
            if h < sea {
                let deep = smooth(0.0, 0.14, sea - h);
                let ice = smooth(0.80, 0.88, lat + (h - 0.5) * 0.3);
                let water = mix3([0.12, 0.38, 0.58], [0.03, 0.12, 0.34], deep);
                return Ground { albedo: mix3(water, [0.86, 0.90, 0.94], ice), shine: 1.0 - ice, glow: [0.0; 3], cloud: clouds(0.54) };
            }
            let up = smooth(sea, sea + 0.24, h);
            let wet = noise(at(3.1, 2.0), 4, 0x15);
            let desert = smooth(0.16, 0.26, lat) * smooth(0.48, 0.36, lat) * smooth(0.62, 0.42, wet);
            let tropic = smooth(0.18, 0.08, lat);
            let woods = mix3([0.30, 0.48, 0.22], [0.12, 0.34, 0.14], tropic.max(smooth(0.4, 0.7, wet) * 0.6));
            let tundra = smooth(0.62, 0.72, lat);
            let mut albedo = mix3(woods, [0.80, 0.66, 0.42], desert);
            albedo = mix3(albedo, [0.52, 0.50, 0.44], tundra);
            albedo = mix3(albedo, [0.46, 0.40, 0.31], (up * 1.2 - 0.4).clamp(0.0, 1.0));
            albedo = mix3(albedo, [0.74, 0.68, 0.50], smooth(0.02, 0.0, h - sea) * 0.6);
            let snow = smooth(0.78, 0.86, lat + (wet - 0.5) * 0.15).max(smooth(0.82, 0.88, h) * 0.8);
            let city = smooth(0.70, 0.80, noise(at(3.0, 8.0), 3, 0x17)) * (1.0 - desert) * (1.0 - tundra) * (1.0 - snow) * 0.6;
            Ground { albedo: mix3(albedo, [0.92, 0.94, 0.97], snow), shine: 0.0, glow: [city, 0.72 * city, 0.38 * city], cloud: clouds(0.54) }
        }
        World::Asteroid => {
            // Brown-grey regolith, lighter where the dust lies thick, cratered thrice over.
            let h = noise(at(2.4, 0.0), 5, 0);
            let base = mix3([0.46, 0.42, 0.38], [0.24, 0.22, 0.21], smooth(0.40, 0.66, h));
            let base = mix3(base, [0.58, 0.52, 0.44], smooth(0.62, 0.80, noise(at(6.0, 3.0), 3, 0xb1)) * 0.5);
            plain(crater(crater(crater(base, q, s ^ 0xb3, 3.0, 0.85), q, s ^ 0xb5, 6.0, 0.75), q, s ^ 0xb7, 12.0, 0.6))
        }
        World::Crystal => {
            // Facets, each catching the light its own way, bright along their edges.
            let (near, second, id) = cells([q[0] * 5.0, q[1] * 5.0, q[2] * 5.0], s ^ 0xa1);
            let tone = unit(id);
            let face = 0.62 + 0.62 * unit(hash(id, 7));
            let edge = smooth(0.05 + fine * 3.0, 0.0, second - near);
            let base = mix3([0.50, 0.78, 0.92], [0.64, 0.52, 0.88], tone);
            let albedo = mix3([base[0] * face, base[1] * face, base[2] * face], [0.92, 0.97, 1.0], edge * 0.45);
            Ground { albedo, shine: 0.6, glow: [0.04 * tone, 0.07, 0.10], cloud: 0.0 }
        }
    }
}

/// Craters over `base`: some of the cells of a lattice `f` to the ball's radius hold one, its
/// floor dark and its rim light; `amount` how many.
fn crater(base: [f32; 3], q: [f32; 3], s: u32, f: f32, amount: f32) -> [f32; 3] {
    let (near, _, id) = cells([q[0] * f, q[1] * f, q[2] * f], s);
    if unit(hash(id, 3)) > amount {
        return base;
    }
    let r = 0.18 + 0.22 * unit(hash(id, 5));
    let floor = smooth(r, r * 0.6, near) * 0.32;
    let rim = smooth(r * 0.75, r, near) * smooth(r * 1.4, r, near) * 0.22;
    let k = 1.0 - floor + rim;
    [base[0] * k, base[1] * k, base[2] * k]
}

/// Cellular noise: the distances to the nearest and second nearest of points scattered one to
/// a unit cell, and the nearest one's cell's hash.
fn cells(p: [f32; 3], s: u32) -> (f32, f32, u32) {
    let base = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let (mut near, mut second, mut id) = (f32::MAX, f32::MAX, 0);
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [base[0] + dx, base[1] + dy, base[2] + dz];
                let h = hash3(c, s);
                let at = [c[0] as f32 + unit(h), c[1] as f32 + unit(hash(h, 1)), c[2] as f32 + unit(hash(h, 2))];
                let d = ((at[0] - p[0]).powi(2) + (at[1] - p[1]).powi(2) + (at[2] - p[2]).powi(2)).sqrt();
                if d < near {
                    (second, near, id) = (near, d, h);
                } else if d < second {
                    second = d;
                }
            }
        }
    }
    (near, second, id)
}

/// Fractal noise of `octaves` (gradient noise, each octave twice as fine and half as strong),
/// about 0 to 1 round 0.5.
fn fbm(p: [f32; 3], octaves: u32, s: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut total) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves.max(1) {
        sum += amp * gradient([p[0] * f, p[1] * f, p[2] * f], s.wrapping_add(o.wrapping_mul(0x9e37)));
        total += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    (0.5 + 0.75 * sum / total).clamp(0.0, 1.0)
}

/// Gradient (Perlin) noise, about -1 to 1.
fn gradient(p: [f32; 3], s: u32) -> f32 {
    let i = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = [p[0] - i[0], p[1] - i[1], p[2] - i[2]];
    let c = [i[0] as i32, i[1] as i32, i[2] as i32];
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(f[0]), fade(f[1]), fade(f[2]));
    let g = |dx: i32, dy: i32, dz: i32| {
        let (x, y, z) = (f[0] - dx as f32, f[1] - dy as f32, f[2] - dz as f32);
        match hash3([c[0] + dx, c[1] + dy, c[2] + dz], s) % 12 {
            0 => x + y,
            1 => y - x,
            2 => x - y,
            3 => -x - y,
            4 => x + z,
            5 => z - x,
            6 => x - z,
            7 => -x - z,
            8 => y + z,
            9 => z - y,
            10 => y - z,
            _ => -y - z,
        }
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

fn hash3(c: [i32; 3], s: u32) -> u32 {
    hash(hash(hash(s, c[0] as u32), c[1] as u32), c[2] as u32)
}

fn hash(a: u32, b: u32) -> u32 {
    let mut x = a ^ b.wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// A hash as 0 to 1.
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// 0 at `a`, 1 at `b` (either way round), smoothly between.
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = dot(v, v).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLDS: [&str; 15] = ["Green planet", "Lush", "Archipelago", "Water World", "Wet Desert", "Desert", "Red Planet", "Lava", "Metal", "Urban", "Ice", "Slate", "Lunar", "Crystal", "Acid"];

    /// Space's shader numbers the worlds as this enum does.
    #[test]
    fn space_knows_the_worlds_by_number() {
        for (i, name) in ta_render::space::WORLDS.iter().enumerate() {
            assert_eq!(World::of(name).index(), i as u32, "{name}");
        }
        assert_eq!(WORLDS.len(), ta_render::space::WORLDS.len());
        // And their colours: the water world bluer than the desert, the lava world dark.
        let water = mean_albedo(World::Water, 3);
        let desert = mean_albedo(World::Desert, 3);
        assert!(water[2] > water[0] && desert[0] > desert[2], "{water:?} {desert:?}");
        assert!(mean_albedo(World::Lava, 3).iter().all(|&c| c < 0.1));
    }

    #[test]
    fn every_world_is_round_and_its_own() {
        assert_eq!(World::of(""), World::Green);
        assert_eq!(World::of("Red Planet"), World::Red);
        assert_eq!(World::of("Wet Desert"), World::WetDesert);
        let mut means = Vec::new();
        for name in WORLDS {
            let image = image(World::of(name), seed_of(name), 48, 23.0);
            let at = |x: usize, y: usize| image.pixels[y * 48 + x];
            assert_eq!(at(0, 0), Color32::TRANSPARENT, "{name}: clear outside the ball");
            assert_eq!(at(24, 24).a(), 255, "{name}: solid inside");
            let solid: Vec<&Color32> = image.pixels.iter().filter(|p| p.a() == 255).collect();
            let n = solid.len() as f32;
            let mean = solid.iter().fold([0.0f32; 3], |m, p| [m[0] + p.r() as f32 / n, m[1] + p.g() as f32 / n, m[2] + p.b() as f32 / n]);
            assert!(mean.iter().any(|&c| c > 30.0), "{name}: not black ({mean:?})");
            means.push((World::of(name), mean));
        }
        // Distinct worlds look distinct.
        for (i, (a, ma)) in means.iter().enumerate() {
            for (b, mb) in &means[i + 1..] {
                let diff: f32 = (0..3).map(|k| (ma[k] - mb[k]).abs()).sum();
                assert!(diff > 4.0, "{a:?} and {b:?} look alike ({ma:?}, {mb:?})");
            }
        }
    }

    /// A sheet of every world, three of each, at the galaxy's sizes and larger, on space:
    /// `TA_PLANETS_OUT=<png> cargo test -p ta_game planet::tests::sheet -- --ignored`.
    #[test]
    #[ignore]
    fn sheet() {
        let Ok(out) = std::env::var("TA_PLANETS_OUT") else { return };
        let sizes = [(34usize, 16.5f32), (46, 22.5), (128, 62.0)];
        let cell = 140;
        let (w, h) = (cell * 3 * sizes.len(), cell * WORLDS.len());
        let mut sheet = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([6, 8, 16, 255]));
        for (row, name) in WORLDS.iter().enumerate() {
            for k in 0..3usize {
                for (col, &(px, radius)) in sizes.iter().enumerate() {
                    let planet = image(World::of(name), seed_of(&format!("{name}{k}")), px, radius);
                    let (x0, y0) = ((col * 3 + k) * cell + (cell - px) / 2, row * cell + (cell - px) / 2);
                    for (i, p) in planet.pixels.iter().enumerate() {
                        let (x, y) = ((x0 + i % px) as u32, (y0 + i / px) as u32);
                        let under = *sheet.get_pixel(x, y);
                        let a = p.a() as u32;
                        let over = |c: u8, u: u8| (c as u32 + u as u32 * (255 - a) / 255).min(255) as u8;
                        sheet.put_pixel(x, y, image::Rgba([over(p.r(), under[0]), over(p.g(), under[1]), over(p.b(), under[2]), 255]));
                    }
                }
            }
        }
        sheet.save(out).expect("the sheet saved");
    }
}
