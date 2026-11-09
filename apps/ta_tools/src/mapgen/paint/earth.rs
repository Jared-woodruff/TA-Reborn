//! Real places (TA Reborn's maps of Australia, the USA and the world, `mapgen/geo.rs`): each part
//! of the land painted by the painter of its biome. Forest is the wild painter's, its colours
//! temperate, tropical or boreal by how warm it is; grassland the wild painter's prairie or
//! savanna; desert the desert painter's, hot or cold; the polar lands the wild painter's tundra
//! or the winter painter's ice caps. Snow lies on the peaks above the snow line, salt crusts the
//! dry lakes, and the shallows are turquoise in the tropics, the deep blue everywhere. Where two biomes
//! meet, their painters' colours meet over a short, wandering edge: the tiles share between alike
//! places, and a gradient across many tiles would be cut into squares.

use super::super::noise::{fbm, mix3, smoothstep, tvalue, tworley};
use super::desert::Desert;
use super::snow::Snow;
use super::wild::Wild;
use super::{Ground, scale};

/// A real place's painters, by biome.
#[derive(Clone)]
pub struct Earth {
    pub temperate: Wild,
    pub tropical: Wild,
    pub boreal: Wild,
    pub prairie: Wild,
    pub savanna: Wild,
    pub desert: Desert,
    pub steppe: Desert,
    pub tundra: Wild,
    pub ice: Snow,
    /// The salt crusting the dry lakes.
    pub salt: [f32; 3],
}

/// How much more one biome must have than the next for its painter alone to paint.
const EDGE: f32 = 0.012;

/// The biomes a place may be: forest, grassland, desert, cold.
const FOREST: usize = 0;
const GRASS: usize = 1;
const DESERT: usize = 2;

impl Ground<'_> {
    pub fn earth(&self, s: &Earth, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let Some(geo) = self.land.geo.as_ref() else { return self.wild(&s.temperate, x, z, h, row) };
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        // How warm, its bounds wandering.
        let warm = geo.warmth.sample(cx, cz) + 0.05 * fbm(x / 420.0, z / 420.0, seed ^ 0x92, 3);
        if self.sea - h > 0.0 {
            return if warm > 0.7 { self.wild(&s.tropical, x, z, h, row) } else { self.wild(&s.temperate, x, z, h, row) };
        }
        // The biomes here, each wandering by noise of its own, the two strongest.
        let b = geo.biome_at(self.land.w, self.land.h, cx, cz);
        let mut weights = [0.0f32; 4];
        for (k, w) in weights.iter_mut().enumerate() {
            *w = b[k] + 0.05 * fbm(x / 320.0, z / 320.0, seed ^ (0xA0 + k as u32), 3) + 0.02 * fbm(x / 80.0, z / 80.0, seed ^ (0xB0 + k as u32), 2);
        }
        let mut order = [0usize, 1, 2, 3];
        order.sort_by(|&a, &b| weights[b].total_cmp(&weights[a]));
        let (first, second) = (order[0], order[1]);
        let paint = |k: usize| -> [f32; 3] {
            match k {
                FOREST if warm > 0.72 => self.wild(&s.tropical, x, z, h, row),
                FOREST if warm < 0.34 => self.wild(&s.boreal, x, z, h, row),
                FOREST => self.wild(&s.temperate, x, z, h, row),
                GRASS if warm > 0.66 => self.wild(&s.savanna, x, z, h, row),
                GRASS => self.wild(&s.prairie, x, z, h, row),
                DESERT if warm > 0.42 => self.desert(&s.desert, x, z, h, row),
                DESERT => self.desert(&s.steppe, x, z, h, row),
                _ if warm > 0.06 => self.wild(&s.tundra, x, z, h, row),
                _ => self.snow(&s.ice, x, z, h, row),
            }
        };
        // (A short edge between the two, a cell or so across though the biomes change over many:
        // only near it are both painted, half and half where they are level.)
        let lead = weights[first] - weights[second];
        let mut c = paint(first);
        if lead < EDGE {
            c = mix3(paint(second), c, 0.5 + 0.5 * smoothstep(0.0, EDGE, lead));
        }
        // The dry lakes: salt in polygons, ridged at their cracks.
        let pan = self.pan_at(cx, cz);
        if pan > 0.0 {
            let y = row;
            let poly = tworley(x, y, 26.0, 26.0, seed ^ 77, 0.85);
            let ridge = smoothstep(0.08, 0.02, poly.f2 - poly.f1);
            let mut p = scale(s.salt, 0.94 + 0.05 * (tvalue(x, y, 1.7, seed ^ 1) - 0.5) - 0.04 * poly.f1);
            p = mix3(p, scale(s.salt, 1.06), ridge * 0.7);
            p = mix3(p, s.desert.flat, smoothstep(0.65, 0.9, tvalue(x, y, 30.0, seed ^ 78)) * 0.25);
            c = mix3(c, p, smoothstep(0.2, 0.7, pan));
        }
        // Snow on the peaks, its line wandering.
        let line = geo.snow_line + 10.0 * fbm(x / 140.0, z / 140.0, seed ^ 0x93, 3) - 18.0 * (warm - 0.5);
        let snowy = smoothstep(line, line + 10.0, h);
        if snowy > 0.0 {
            c = mix3(c, self.snow(&s.ice, x, z, h, row), snowy);
        }
        c
    }

    /// How much of a point lies in a dry lake (0 to 1, bilinear over its cells).
    fn pan_at(&self, cx: f32, cz: f32) -> f32 {
        let Some(geo) = self.land.geo.as_ref() else { return 0.0 };
        if geo.pans.is_empty() {
            return 0.0;
        }
        let (w, h) = (self.land.w, self.land.h);
        let (x0, z0) = (cx.floor().clamp(0.0, (w - 1) as f32) as usize, cz.floor().clamp(0.0, (h - 1) as f32) as usize);
        let (x1, z1) = ((x0 + 1).min(w - 1), (z0 + 1).min(h - 1));
        let (fx, fz) = ((cx - x0 as f32).clamp(0.0, 1.0), (cz - z0 as f32).clamp(0.0, 1.0));
        let at = |x: usize, z: usize| geo.pans[z * w + x] as u32 as f32;
        let top = at(x0, z0) + (at(x1, z0) - at(x0, z0)) * fx;
        let bottom = at(x0, z1) + (at(x1, z1) - at(x0, z1)) * fx;
        top + (bottom - top) * fz
    }
}
