//! The livery patterns drawn on the processor, as `ta_render`'s `shaders/livery.wgsl` draws them
//! on the units (the same noise, hashes and thresholds), for the skin editor's pattern tiles: a
//! flat cut through a pattern in a livery's colours, so each choice shows what it makes.

use glam::{IVec3, Vec2, Vec3};
use ta_skins::{Livery, Pattern};

/// How much of a pattern a tile spans, in the pattern's units at scale 1 (about two light
/// tanks across).
const SPAN: f32 = 2.4;

fn bits(c: IVec3) -> u32 {
    let mut h = (c.x as u32).wrapping_mul(0x8da6_b343) ^ (c.y as u32).wrapping_mul(0xd816_3841) ^ (c.z as u32).wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

fn hash(c: IVec3) -> f32 {
    (bits(c) >> 8) as f32 / 16_777_216.0
}

fn point(c: IVec3) -> Vec3 {
    let h = bits(c);
    Vec3::new((h & 1023) as f32, ((h >> 10) & 1023) as f32, ((h >> 20) & 1023) as f32) / 1023.0
}

fn noise(p: Vec3) -> f32 {
    let cell = p.floor();
    let i = cell.as_ivec3();
    let f = p - cell;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let corner = |k: i32| {
        let o = IVec3::new(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        (point(i + o) * 2.0 - 1.0).dot(f - o.as_vec3())
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x0 = lerp(corner(0), corner(1), u.x);
    let x1 = lerp(corner(2), corner(3), u.x);
    let x2 = lerp(corner(4), corner(5), u.x);
    let x3 = lerp(corner(6), corner(7), u.x);
    lerp(lerp(x0, x1, u.y), lerp(x2, x3, u.y), u.z) * 0.9 + 0.5
}

fn field(p: Vec3) -> f32 {
    let n = noise(p) * 0.57 + noise(p * 2.07 + Vec3::new(17.3, 3.1, 9.7)) * 0.29 + noise(p * 4.13 + Vec3::new(5.9, 23.1, 1.3)) * 0.14;
    (n - 0.5) * 2.4 + 0.5
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A livery's colours in linear light, each with whether it has it.
struct Colours {
    colours: [(Vec3, f32); 3],
    count: usize,
}

impl Colours {
    fn of(livery: &Livery) -> Self {
        let linear = |c: ta_skins::Colour| Vec3::from(c.0.map(egui::ecolor::linear_f32_from_gamma_u8));
        let first = livery.colours.first().copied().unwrap_or(ta_skins::Colour::rgb(0x808080));
        let count = livery.colours.len().clamp(1, 3);
        let colours = std::array::from_fn(|i| (linear(livery.colours.get(i).copied().unwrap_or(first)), if i < count { 1.0 } else { 0.0 }));
        Self { colours, count }
    }

    fn pick(&self, h: f32) -> Vec3 {
        match self.count {
            1 => self.colours[0].0,
            2 => if h < 0.58 { self.colours[0].0 } else { self.colours[1].0 },
            _ => if h < 0.45 { self.colours[0].0 } else if h < 0.78 { self.colours[1].0 } else { self.colours[2].0 },
        }
    }
}

fn blotch(c: &Colours, p: Vec3, soft: f32) -> Vec3 {
    let warp = Vec3::new(noise(p * 0.7 + Vec3::new(3.7, 1.1, 8.3)), noise(p * 0.7 + Vec3::new(9.1, 4.7, 2.9)), noise(p * 0.7 + Vec3::new(6.3, 7.9, 5.1))) - 0.5;
    let q = p * 1.6 + warp * 1.4;
    let a = smoothstep(0.5 - soft, 0.5 + soft, field(q));
    let b = smoothstep(0.66 - soft, 0.66 + soft, field(q * 1.3 + Vec3::new(41.0, 7.0, 19.0)));
    let colour = c.colours[0].0.lerp(c.colours[1].0, a * c.colours[1].1);
    colour.lerp(c.colours[2].0, b * c.colours[2].1)
}

fn splinter(c: &Colours, p: Vec3, soft: f32) -> Vec3 {
    let q = Vec3::new(p.x * 0.8 + p.z * 0.45, p.y * 1.7 + p.x * 0.3, p.z * 0.8 - p.x * 0.45) * 1.5;
    let cell = q.floor().as_ivec3();
    let (mut best, mut second, mut id, mut next) = (f32::MAX, f32::MAX, cell, cell);
    for k in 0..27 {
        let c = cell + IVec3::new(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        let d = q - (c.as_vec3() + 0.15 + point(c) * 0.7);
        let dist = d.dot(d);
        if dist < best {
            second = best;
            next = id;
            best = dist;
            id = c;
        } else if dist < second {
            second = dist;
            next = c;
        }
    }
    let edge = (second.sqrt() - best.sqrt()) * 0.5;
    let near = c.pick(hash(id));
    let far = c.pick(hash(next));
    far.lerp(near, 0.5).lerp(near, smoothstep(0.0, soft * 0.6 + 1e-4, edge))
}

fn stripes(c: &Colours, p: Vec3, soft: f32) -> Vec3 {
    let bend = (field(p * Vec3::new(0.45, 0.9, 0.45) + Vec3::new(2.3, 5.1, 7.7)) - 0.5) * 1.3;
    let d = p.z * 1.3 + p.x * 0.25 + bend;
    let along = Vec3::new(p.x * 0.8, p.y * 0.8, d * 2.0);
    let width = 0.34 + 0.5 * (noise(along + Vec3::new(11.0, 3.0, 5.0)) - 0.5);
    let s = ((d.rem_euclid(1.0)) - 0.5).abs() * 2.0;
    let band = 1.0 - smoothstep(width - soft, width + soft, s);
    let colour = c.colours[0].0.lerp(c.colours[1].0, band * c.colours[1].1);
    let thin = 1.0 - smoothstep(width * 0.35 - soft, width * 0.35 + soft, (((d + 0.5).rem_euclid(1.0)) - 0.5).abs() * 2.0);
    colour.lerp(c.colours[2].0, thin * c.colours[2].1)
}

fn hex(c: &Colours, p: Vec3, aa: f32) -> Vec3 {
    // (A tile is a face lying in the pattern's xy plane.)
    let uv = Vec2::new(p.x, p.y) * 2.4;
    let r = Vec2::new(1.0, 1.732_050_8);
    let h = r * 0.5;
    let modulo = |v: Vec2| v - r * (v / r).floor();
    let ga = modulo(uv) - h;
    let gb = modulo(uv - h) - h;
    let g = if ga.dot(ga) < gb.dot(gb) { ga } else { gb };
    let centre = uv - g;
    let q = g.abs();
    let edge = 0.5 - q.dot(Vec2::new(0.5, 0.866_025_4)).max(q.x);
    let id = IVec3::new((centre.x * 2.0).round() as i32, (centre.y * 2.0).round() as i32, 0);
    let gap = 1.0 - smoothstep(0.03, 0.03 + aa * 2.4 + 0.02, edge);
    c.pick(hash(id)) * (1.0 - 0.55 * gap * (1.0 - smoothstep(0.08, 0.2, aa)))
}

/// One of the colours, round by their count.
fn nth(c: &Colours, i: f32) -> Vec3 {
    c.colours[(i.rem_euclid(c.count as f32)) as usize % c.count].0
}

fn tricolour(c: &Colours, d: f32, soft: f32) -> Vec3 {
    // (`d` runs across the bands: the length on a unit, a tile's height here.)
    let count = c.count as f32;
    let t = (d * 0.7).rem_euclid(1.0) * count;
    let band = t.floor();
    let u = t - band;
    let w = soft * count * 0.7 + 1e-4;
    let colour = nth(c, band).lerp(nth(c, band + 1.0), smoothstep(1.0 - w, 1.0 + w, u));
    nth(c, band - 1.0).lerp(colour, smoothstep(-w, w, u))
}

fn union(c: &Colours, p: Vec3, soft: f32) -> Vec3 {
    // (A tile is a face lying in the pattern's xy plane.)
    let g = (Vec2::new(p.x, p.y) * 0.9).fract_gl() - 0.5;
    let upright = g.x.abs().min(g.y.abs());
    let diagonal = (g.x - g.y).abs().min((g.x + g.y).abs()) * std::f32::consts::FRAC_1_SQRT_2;
    let broad = 1.0 - smoothstep(-soft, soft, (upright - 0.1).min(diagonal - 0.065));
    let narrow = 1.0 - smoothstep(-soft, soft, (upright - 0.045).min(diagonal - 0.025));
    c.colours[0].0.lerp(c.colours[1].0, broad * c.colours[1].1).lerp(c.colours[2].0, narrow * c.colours[2].1)
}

/// How far `q` lies outside a five-pointed star `r` across its points (its inner corners at
/// `inner` of that; negative inside), its top point up, as the shader's `livery_star`.
fn star(q: Vec2, r: f32, inner: f32) -> f32 {
    let k1 = Vec2::new(0.809_017, -0.587_785_25);
    let k2 = Vec2::new(-k1.x, k1.y);
    let mut p = Vec2::new(q.x.abs(), q.y);
    p -= 2.0 * k1.dot(p).max(0.0) * k1;
    p -= 2.0 * k2.dot(p).max(0.0) * k2;
    p.x = p.x.abs();
    p.y -= r;
    let ba = inner * Vec2::new(-k1.y, k1.x) - Vec2::new(0.0, 1.0);
    let h = (p.dot(ba) / ba.dot(ba)).clamp(0.0, r);
    (p - ba * h).length() * (p.y * ba.x - p.x * ba.y).signum()
}

fn stars(c: &Colours, p: Vec3, soft: f32) -> Vec3 {
    let mut uv = Vec2::new(p.x, p.y) * 1.8;
    let row = uv.y.floor();
    uv.x += row * 0.5;
    let g = uv.fract_gl() - 0.5;
    if c.count > 2 && row.rem_euclid(2.0) > 0.5 {
        let band = 1.0 - smoothstep(0.2 - soft, 0.2 + soft, g.y.abs());
        return c.colours[1].0.lerp(c.colours[2].0, band);
    }
    let shape = 1.0 - smoothstep(-soft, soft, star(g, 0.3, 0.42));
    c.colours[0].0.lerp(c.colours[1].0, shape * c.colours[1].1)
}

fn festive(c: &Colours, p: Vec3, aa: f32, sharpness: f32) -> Vec3 {
    let q = p * 1.28;
    let phase = (q.x * 1.7 + q.y * 0.6 + q.z * 1.1).sin();
    let edge = 0.5 + (0.05 - 0.5) * sharpness + aa * 3.0;
    let colour = c.colours[0].0.lerp(c.colours[1].0, smoothstep(-edge, edge, phase) * c.colours[1].1);
    let uv = Vec2::new((q.x + q.z * 0.35).rem_euclid(1.0), (q.y + q.z * 0.65).rem_euclid(1.0)) - 0.5;
    let line = uv.x.abs().min(uv.y.abs()).min((uv.x - uv.y).abs().min((uv.x + uv.y).abs()) * std::f32::consts::FRAC_1_SQRT_2);
    let footprint = aa * 1.28;
    let width = (footprint * 0.6).max(0.008);
    let snow = (1.0 - smoothstep(0.018 - width, 0.018 + width, line)) * (1.0 - smoothstep(0.21 - width, 0.21 + width, uv.length()))
        * (1.0 - smoothstep(0.12, 0.35, footprint));
    colour.lerp(c.colours[2].0, snow * 0.88 * c.colours[2].1)
}

/// The colour of `pattern` (in `livery`'s colours, scale, sharpness and seed) at `p`, a point of
/// the pattern's space, where a pixel spans `aa` of it.
fn sample(livery: &Livery, pattern: Pattern, colours: &Colours, p: Vec3, aa: f32) -> Vec3 {
    let soft = 0.16 + (0.006 - 0.16) * livery.sharpness + aa * 1.5;
    match pattern {
        Pattern::Solid => colours.colours[0].0,
        Pattern::Blotch => blotch(colours, p, soft),
        Pattern::Splinter => splinter(colours, p, soft),
        Pattern::Digital => {
            let size = 0.12;
            let hard = blotch(colours, ((p / size).floor() + 0.5) * size, 1e-4);
            hard.lerp(blotch(colours, p, 0.02 + aa * 2.0), smoothstep(0.25, 0.5, aa / size))
        }
        Pattern::Stripes => stripes(colours, p, soft),
        Pattern::Hex => hex(colours, p, aa),
        Pattern::Festive => festive(colours, p, aa, livery.sharpness),
        Pattern::Tricolour => tricolour(colours, p.y, soft),
        Pattern::Union => union(colours, p, soft),
        Pattern::Stars => stars(colours, p, soft),
    }
}

/// A tile `width` × `height` pixels of `pattern` in `livery`'s colours (its own pattern aside),
/// lit a little from the top left as a painted plate is.
pub fn tile(livery: &Livery, pattern: Pattern, width: usize, height: usize) -> egui::ColorImage {
    let colours = Colours::of(livery);
    let scale = livery.scale.clamp(*ta_skins::SCALE.start(), *ta_skins::SCALE.end());
    let step = SPAN / scale / width.max(1) as f32;
    // (The seed moves the pattern as the shader's offset does.)
    let place = |k: u32| {
        let h = (livery.seed ^ k.wrapping_mul(0x9e37_79b9)).wrapping_mul(0x85eb_ca6b).rotate_left(13).wrapping_mul(0xc2b2_ae35);
        (h >> 8) as f32 / (1u32 << 24) as f32 * 256.0
    };
    let offset = Vec3::new(place(1), place(2), place(3));
    let mut image = egui::ColorImage::filled([width, height], egui::Color32::BLACK);
    for y in 0..height {
        for x in 0..width {
            let p = Vec3::new(x as f32 * step, y as f32 * step, 0.37) + offset;
            let light = 1.06 - 0.12 * (x as f32 / width as f32 + y as f32 / height as f32) * 0.5;
            let colour = sample(livery, pattern, &colours, p, step) * light;
            let [r, g, b] = colour.to_array().map(egui::ecolor::gamma_u8_from_linear_f32);
            image.pixels[y * width + x] = egui::Color32::from_rgb(r, g, b);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_draws_its_colours_and_solid_only_the_first() {
        let colours = [ta_skins::Colour::rgb(0xc4a676), ta_skins::Colour::rgb(0x5e4630), ta_skins::Colour::rgb(0x1c1712)];
        let livery = Livery::new(Pattern::Blotch, &colours);
        for pattern in Pattern::ALL {
            let image = tile(&livery, pattern, 48, 32);
            assert_eq!(image.size, [48, 32]);
            let distinct: std::collections::BTreeSet<[u8; 4]> = image.pixels.iter().map(|p| p.to_array()).collect();
            if pattern == Pattern::Solid {
                assert!(image.pixels.iter().all(|p| (i32::from(p.r()) - 0xc4).abs() < 24), "solid shows the first colour, lit");
            } else {
                assert!(distinct.len() > 40, "{pattern:?} draws a pattern");
            }
        }
    }

    #[test]
    fn a_seed_moves_the_pattern_and_scale_zooms_it() {
        let livery = Livery::new(Pattern::Splinter, &[ta_skins::Colour::rgb(0xffffff), ta_skins::Colour::rgb(0x000000)]);
        let first = tile(&livery, Pattern::Splinter, 32, 32);
        assert_ne!(first.pixels, tile(&Livery { seed: 9, ..livery.clone() }, Pattern::Splinter, 32, 32).pixels);
        assert_ne!(first.pixels, tile(&Livery { scale: 3.0, ..livery.clone() }, Pattern::Splinter, 32, 32).pixels);
        assert_eq!(first.pixels, tile(&livery, Pattern::Splinter, 32, 32).pixels, "the same livery, the same tile");
    }
}
