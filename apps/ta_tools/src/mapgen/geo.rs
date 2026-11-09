//! TA Reborn's maps of real places (Australia, the USA, the world), grown from rasters drawn by
//! `tools/geo/outlines.py` from Natural Earth's public-domain outlines (`apps/ta_tools/geo/`):
//! the land and its lakes, its relief (the ranges, plateaus and basins listed there) and its
//! biomes (forest, grassland, desert and cold, and how warm). Here the relief is cut into TA's
//! tiers along its own contours (cliffs, or long ramps where the ramp field says), hills and
//! ridged peaks laid over it; the coasts slope down to the sea, shelves fall away under it; dry
//! lakes are salt pans; the starts are the given places, on the nearest level ground.

use rayon::prelude::*;

use super::field::{Field, distance};
use super::noise::{fbm, lerp, ridged, smoothstep};
use super::terrain::{Land, Shape, Sym, connect, harbours, level};

/// A map of a real place: its rasters (PNG, the map's size in cells) and what the recipe says
/// of it.
#[derive(Clone)]
pub struct Geo {
    pub land: &'static [u8],
    pub lakes: &'static [u8],
    pub relief: &'static [u8],
    /// Forest, grassland, desert and cold (RGBA).
    pub biome: &'static [u8],
    /// How warm: 0 polar to 1 tropical.
    pub warmth: &'static [u8],
    /// The starts, map fractions (x east, z south), in their order.
    pub starts: Vec<(f32, f32)>,
    /// The lakes are salt pans, dry ground (Australia's).
    pub dry_lakes: bool,
    /// The height (units) above which the peaks are snow.
    pub snow_line: f32,
}

/// What a real place's land carries for the painter and the placer, cell by cell.
#[derive(Clone)]
pub struct GeoLand {
    /// Forest, grassland, desert and cold, 0 to 1 each (none in the water).
    pub biome: Vec<[f32; 4]>,
    pub warmth: Field,
    /// Dry lakes (salt pans).
    pub pans: Vec<bool>,
    pub snow_line: f32,
}

impl GeoLand {
    /// The biomes at a point (fractional cells), bilinear.
    pub fn biome_at(&self, w: usize, h: usize, cx: f32, cz: f32) -> [f32; 4] {
        let (x0, z0) = (cx.floor().clamp(0.0, (w - 1) as f32) as usize, cz.floor().clamp(0.0, (h - 1) as f32) as usize);
        let (x1, z1) = ((x0 + 1).min(w - 1), (z0 + 1).min(h - 1));
        let (fx, fz) = ((cx - x0 as f32).clamp(0.0, 1.0), (cz - z0 as f32).clamp(0.0, 1.0));
        let at = |x: usize, z: usize| self.biome[z * w + x];
        let (a, b, c, d) = (at(x0, z0), at(x1, z0), at(x0, z1), at(x1, z1));
        [0, 1, 2, 3].map(|k| lerp(lerp(a[k], b[k], fx), lerp(c[k], d[k], fx), fz))
    }
}

fn luma(png: &[u8], w: usize, h: usize, what: &str) -> Vec<f32> {
    let img = image::load_from_memory(png).unwrap_or_else(|e| panic!("geo {what}: {e}")).to_luma8();
    assert_eq!((img.width() as usize, img.height() as usize), (w, h), "geo {what}: not the map's size");
    img.into_raw().into_iter().map(|v| v as f32 / 255.0).collect()
}

fn rgba(png: &[u8], w: usize, h: usize, what: &str) -> Vec<[f32; 4]> {
    let img = image::load_from_memory(png).unwrap_or_else(|e| panic!("geo {what}: {e}")).to_rgba8();
    assert_eq!((img.width() as usize, img.height() as usize), (w, h), "geo {what}: not the map's size");
    img.pixels().map(|p| p.0.map(|v| v as f32 / 255.0)).collect()
}

/// The land of a real place (see the module).
pub fn build(s: &Shape, g: &Geo, sym: &Sym) -> Land {
    let (w, h) = (s.w, s.h);
    let seed = s.seed;
    let land_cov = luma(g.land, w, h, "land");
    let lakes = luma(g.lakes, w, h, "lakes");
    let relief = Field { w, h, data: luma(g.relief, w, h, "relief") };
    let biome = rgba(g.biome, w, h, "biome");
    let warmth = Field { w, h, data: luma(g.warmth, w, h, "warmth") };
    let lake = |i: usize| land_cov[i] >= 0.5 && lakes[i] >= 0.5;
    let water: Vec<bool> = (0..w * h).map(|i| land_cov[i] < 0.5 || (!g.dry_lakes && lake(i))).collect();
    let pans: Vec<bool> = (0..w * h).map(|i| g.dry_lakes && lake(i)).collect();
    let dry: Vec<bool> = water.iter().map(|&v| !v).collect();
    let water_in = distance(w, h, &dry);
    let water_dist = distance(w, h, &water);
    // How fast the relief changes, per cell: its tiers' cliffs are a cell or two across in cells,
    // whatever the relief's slope.
    let grad = relief.blurred(2).gradient();
    let ramp = Field::from_fn(w, h, |x, z| smoothstep(0.15, 0.55, fbm(x as f32 / 90.0, z as f32 / 90.0, seed ^ 0x44, 3) * 0.5 + 0.5 + s.ramps - 0.5));
    let sea = s.sea;
    let mut height = Field::new(w, h, 0.0);
    height.data.par_iter_mut().enumerate().for_each(|(i, ht)| {
        let (x, z) = (i % w, i / w);
        let (xf, zf) = (x as f32, z as f32);
        if water[i] {
            // The sea: a shelf off the coast, falling away to the deep; a lake shallower.
            let d = water_in.data[i];
            let deep = if land_cov[i] < 0.5 { (2.5 + 2.2 * d).min(46.0) } else { (2.0 + 1.6 * d).min(18.0) };
            // (At the coast its depth by how much of the cell is water: the waterline between
            // the cells, where the outline has it.)
            *ht = if d <= 1.0 { sea - (0.5 - land_cov[i].min(0.5)) * 6.0 - 0.6 } else { sea - deep };
            return;
        }
        // The relief, its contours wandering a little, cut into the tiers: each step a cliff, or
        // where the ramp field is high, a long slope.
        let rel = relief.data[i] + 0.022 * fbm(xf / 34.0, zf / 34.0, seed ^ 0x11, 3);
        let gr = grad.data[i].max(1e-5);
        let half = lerp(1.0, 9.0, ramp.data[i]);
        let mut v = s.levels[0];
        for (k, &step) in s.steps.iter().enumerate() {
            v += (s.levels[k + 1] - s.levels[k]) * smoothstep(-half, half, (rel - step) / gr);
        }
        v += 14.0 * rel;
        let hills = (fbm(xf / s.hill_scale, zf / s.hill_scale, seed ^ 0x55, 4) * 0.5 + 0.5) * s.hills * (0.45 + rel);
        let ripple = fbm(xf / 9.0, zf / 9.0, seed ^ 0x66, 2) * 1.6;
        let peaks = ridged(xf / 60.0, zf / 60.0, seed ^ 0x77, 5).powf(1.6) * s.peaks * smoothstep(0.45, 0.75, rel);
        v += hills + ripple + peaks;
        // The coast: the land sloping down to the sea over its first cells, its waterline where
        // the outline has it.
        let d = water_dist.data[i];
        let coast = if d <= 1.0 { sea + (land_cov[i].max(0.5) - 0.5) * 6.0 + 0.6 } else { sea + 1.5 + 2.6 * (d - 1.0) };
        *ht = v.min(coast).clamp(0.0, 255.0);
    });
    // Salt pans: each dry lake level, a little below the lowest of its shore.
    if g.dry_lakes {
        let mut seen = vec![false; w * h];
        for start in 0..w * h {
            if !pans[start] || seen[start] {
                continue;
            }
            let mut cells = vec![start];
            seen[start] = true;
            let mut k = 0;
            let mut low = f32::MAX;
            while k < cells.len() {
                let i = cells[k];
                k += 1;
                let (x, z) = (i % w, i / w);
                for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                    if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                        continue;
                    }
                    let j = nz as usize * w + nx as usize;
                    if pans[j] {
                        if !seen[j] {
                            seen[j] = true;
                            cells.push(j);
                        }
                    } else if !water[j] {
                        low = low.min(height.data[j]);
                    }
                }
            }
            let floor = (low - 2.0).max(sea + 2.0);
            for &i in &cells {
                height.data[i] = floor;
            }
        }
    }
    // The starts: the places given, each on the nearest level ground well in from the water.
    let slope = height.ta_slope();
    let starts: Vec<(f32, f32)> = g
        .starts
        .iter()
        .take(s.players)
        .map(|&(fx, fz)| {
            let (px, pz) = (fx * w as f32, fz * h as f32);
            let mut best = (f32::MAX, px, pz);
            for dz in -40i64..=40 {
                for dx in -40i64..=40 {
                    let (x, z) = (px as i64 + dx, pz as i64 + dz);
                    if x < 20 || z < 20 || x >= w as i64 - 20 || z >= h as i64 - 20 {
                        continue;
                    }
                    let i = z as usize * w + x as usize;
                    if water[i] || pans[i] || water_dist.data[i] < 8.0 {
                        continue;
                    }
                    // (Level round it, and as near the place as can be.)
                    let mut rough = 0.0f32;
                    for (ox, oz) in [(-6i64, -6i64), (6, -6), (-6, 6), (6, 6), (0, 0)] {
                        let j = ((z + oz).clamp(0, h as i64 - 1) as usize) * w + (x + ox).clamp(0, w as i64 - 1) as usize;
                        rough += slope.data[j];
                    }
                    let score = rough + 0.08 * ((dx * dx + dz * dz) as f32).sqrt();
                    if score < best.0 {
                        best = (score, x as f32, z as f32);
                    }
                }
            }
            (best.1, best.2)
        })
        .collect();
    for &(sx, sz) in &starts {
        level(&mut height, sx, sz, 16.0, 30.0);
    }
    if s.connect {
        let cut = connect(&mut height, sea, &starts, sym, s.causeway, s.whole);
        if cut > 0 {
            eprintln!("  cut {cut} ramp(s) so every start can be driven to");
        }
    }
    // Ports: deep water by the bases on the coasts.
    if s.harbours > 0.0 {
        let dredged = harbours(&mut height, sea, &starts, sym, s.harbours);
        eprintln!("  dredged {dredged} harbour(s)");
    }
    let moisture = Field::from_fn(w, h, |x, z| {
        let b = biome[z * w + x];
        (b[0] * 0.8 + b[1] * 0.35 + 0.12 * fbm(x as f32 / 160.0, z as f32 / 160.0, seed ^ 0xBB, 3)).clamp(0.0, 1.0)
    });
    let geo = GeoLand { biome, warmth, pans, snow_line: g.snow_line };
    Land {
        w,
        h,
        sea,
        height,
        water_dist,
        moisture,
        starts,
        bed: Field::new(w, h, 1e6),
        dune_mask: Field::new(w, h, 0.0),
        fords: Vec::new(),
        built: None,
        void: Vec::new(),
        gantry: Vec::new(),
        road: Vec::new(),
        ice: Vec::new(),
        geo: Some(Box::new(geo)),
        sym: *sym,
        halves_apart: false,
    }
}

/// How much of the forest grows in each cell's biome: all of it in forest, a little in
/// grassland, next to none in desert or the cold.
pub fn forest_share(land: &Land) -> Option<Field> {
    let geo = land.geo.as_ref()?;
    Some(Field::from_fn(land.w, land.h, |x, z| {
        let b = geo.biome[z * land.w + x];
        let total = (b[0] + b[1] + b[2] + b[3]).max(1e-4);
        ((b[0] + b[1] * 0.12 + b[2] * 0.01 + b[3] * 0.04) / total).clamp(0.0, 1.0)
    }))
}
