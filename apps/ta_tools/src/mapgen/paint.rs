//! The ground's art, painted as TA's maps are: in its oblique view, where the ground at height h
//! shows `h / 2` pixels further up the picture than it lies (`docs/FORMATS.md`, Coordinates), so
//! the game's 3D terrain finds each colour where its ground is. A cliff facing the viewer is
//! stretched over many rows, one facing away is hidden behind its crest (the game draws those as
//! rock of its own).
//!
//! Each pixel's colour comes from what the ground is there. The first maps' painter
//! ([`Scheme`]): water (painted, as TA's seas are, the renderer reads them so), beach, grass wet
//! or dry, dirt, forest floor, rock in strata on the slopes, with soft shading baked in (the
//! crevices darker, a light from the north-west) and a grain the renderer's detail materials
//! build on up close. The wild painter ([`wild`]) paints temperate land in more detail: meadows,
//! heath and moss in patches, tufts and wildflowers, leaf litter under the trees, outcrops of
//! bedrock, scree at the foot of cliffs, reeds by the water and worn tracks; the desert painter
//! ([`desert`]) sand rippled by the wind, dunes, desert pavement, mesas banded in strata, dry
//! river beds, salt pans and oases; the winter painter ([`snow`]) snow in powder and crust,
//! frozen lakes, roads of packed snow and the villages' cobbled squares.

pub mod desert;
pub mod earth;
pub mod metal;
pub mod snow;
pub mod space;
pub mod volcanic;
pub mod wild;

use super::field::{Field, distance};
use super::noise::{fbm, lerp, mix3, perlin, smoothstep, value};
use super::terrain::Land;
use super::tracks::Tracks;

/// How a map is painted.
#[derive(Clone)]
pub enum Paint {
    /// The first maps' painter.
    Classic(Scheme),
    Wild(wild::Wild),
    Desert(desert::Desert),
    Volcanic(volcanic::Volcanic),
    Metal(metal::Metal),
    Space(space::Space),
    Snow(snow::Snow),
    Earth(Box<earth::Earth>),
}

impl Paint {
    /// Whether the sea is molten (a lava world).
    pub fn molten(&self) -> bool {
        matches!(self, Paint::Classic(s) if s.molten) || matches!(self, Paint::Volcanic(_))
    }

    /// Whether the art's fine texture repeats every `noise::PERIOD` pixels (every painter's but
    /// the first maps').
    pub fn periodic(&self) -> bool {
        !matches!(self, Paint::Classic(_))
    }
}

/// A world's colours (sRGB, 0 to 1).
#[derive(Clone)]
pub struct Scheme {
    /// Broad meadows of richer grass.
    pub grass_lush: [f32; 3],
    pub grass_wet: [f32; 3],
    pub grass_dry: [f32; 3],
    pub dirt: [f32; 3],
    pub forest: [f32; 3],
    pub rock: [f32; 3],
    pub rock_dark: [f32; 3],
    pub peak: [f32; 3],
    pub sand: [f32; 3],
    pub sand_wet: [f32; 3],
    pub shallow: [f32; 3],
    pub deep: [f32; 3],
    /// How strongly the slopes turn to rock (height units a cell where it starts and is full).
    pub rock_from: f32,
    pub rock_full: f32,
    /// How much of the open ground is bare (dirt) in patches, 0 to 1.
    pub bare: f32,
    /// How far above the sea the shore is sand (height units), and how far from it (cells).
    pub beach: f32,
    pub beach_cells: f32,
    /// Lava worlds: the sea is molten and glows; the shores are scorched, not sandy.
    pub molten: bool,
}

pub fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// What the painter reads of the land, per cell.
pub struct Ground<'a> {
    pub land: &'a Land,
    pub height: &'a Field,
    pub sea: f32,
    pub slope: Field,
    /// How much lower than its surroundings a cell lies (positive in hollows).
    pub cavity: Field,
    /// The slope over a few cells round (high at the foot and the crest of a cliff too).
    pub slope_wide: Field,
    pub moisture: &'a Field,
    pub forest: &'a Field,
    pub water_dist: &'a Field,
    pub tracks: &'a Tracks,
    /// Distance in cells to the nearest vent (where the ground is hottest).
    pub hot: &'a Field,
    pub paint: Paint,
    pub seed: u32,
    /// A winter world's ice: how far inside it each cell lies (cells; 0 on land), if it has any.
    pub ice: Option<Field>,
    /// The villages' squares (cells: their middles and radii).
    pub squares: Vec<(f32, f32, f32)>,
}

impl<'a> Ground<'a> {
    pub fn new(land: &'a Land, forest: &'a Field, tracks: &'a Tracks, hot: &'a Field, paint: Paint, seed: u32) -> Ground<'a> {
        let height = &land.height;
        let slope = height.ta_slope();
        let blurred = height.blurred(2);
        let cavity = Field::from_fn(height.w, height.h, |x, z| blurred.at(x, z) - height.at(x, z));
        let slope_wide = if matches!(paint, Paint::Classic(_)) { Field::new(1, 1, 0.0) } else { slope.blurred(2) };
        let ice = land.ice.iter().any(|&i| i).then(|| distance(land.w, land.h, &land.ice.iter().map(|&i| !i).collect::<Vec<bool>>()));
        Ground { land, height, sea: land.sea, slope, cavity, slope_wide, moisture: &land.moisture, forest, water_dist: &land.water_dist, tracks, hot, paint, seed, ice, squares: Vec::new() }
    }

    /// The ground's height at a point (pixels), as the game interpolates it.
    #[inline]
    pub fn h(&self, x: f32, z: f32) -> f32 {
        self.height.sample_tri(x / 16.0, z / 16.0)
    }

    /// How much of the ground at (x, z) (pixels), whose height is `h`, is painted as rock in
    /// strata (0..1): the slopes, as [`Ground::color`] paints them.
    pub fn rock(&self, x: f32, z: f32, h: f32) -> f32 {
        if self.sea - h > 0.0 {
            return 0.0;
        }
        let (from, full) = match &self.paint {
            Paint::Classic(s) => (s.rock_from, s.rock_full),
            Paint::Wild(s) => (s.rock_from, s.rock_full),
            Paint::Desert(s) => (s.rock_from, s.rock_full),
            Paint::Volcanic(s) => (s.rock_from, s.rock_full),
            Paint::Metal(s) => (s.rock_from, s.rock_full),
            Paint::Space(s) => (s.rock_from, s.rock_full),
            Paint::Snow(s) => (s.rock_from, s.rock_full),
            Paint::Earth(s) => (s.temperate.rock_from, s.temperate.rock_full),
        };
        let g3 = perlin(x / 19.0, z / 19.0, self.seed ^ 3);
        smoothstep(from, full, self.slope.sample(x / 16.0 - 0.5, z / 16.0 - 0.5) + 2.5 * g3)
    }

    /// The colour of the ground at (x, z) (pixels), whose height is `h`, seen at art row `row`
    /// (the fine texture is laid in the art's own rows, so it repeats tile for tile).
    pub fn color(&self, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        match &self.paint {
            Paint::Classic(s) => self.classic(s, x, z, h),
            Paint::Wild(s) => self.wild(s, x, z, h, row),
            Paint::Desert(s) => self.desert(s, x, z, h, row),
            Paint::Volcanic(s) => self.volcanic(s, x, z, h, row),
            Paint::Metal(s) => self.metal(s, x, z, h, row),
            Paint::Space(s) => self.space(s, x, z, h, row),
            Paint::Snow(s) => self.snow(s, x, z, h, row),
            Paint::Earth(s) => self.earth(s, x, z, h, row),
        }
    }

    /// The first maps' painter.
    fn classic(&self, s: &Scheme, x: f32, z: f32, h: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        // Grain: fine, finer, and patches.
        let g1 = value(x / 1.7, z / 1.7, seed ^ 1) - 0.5;
        let g2 = value(x / 4.3, z / 4.3, seed ^ 2) - 0.5;
        let g3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let patch = fbm(x / 140.0, z / 140.0, seed ^ 4, 3) * 0.5 + 0.5;
        let depth = self.sea - h;
        if depth > 0.0 {
            // Water as TA paints it: light over the shallows, deep blue further out, waves.
            let wave = value(x / 9.0, z / 3.2, seed ^ 5) - 0.5;
            let mut c = mix3(s.shallow, s.deep, smoothstep(0.0, 16.0, depth));
            if !s.molten {
                c = mix3(s.sand_wet, c, smoothstep(0.0, 2.5, depth));
            }
            let k = 1.0 + 0.10 * wave + 0.04 * g2;
            return c.map(|v| (v * k).clamp(0.0, 1.0));
        }
        let slope = self.slope.sample(cx, cz);
        let moist = (self.moisture.sample(cx, cz) + 0.08 * g3).clamp(0.0, 1.0);
        let forest = self.forest.sample(cx, cz);
        let water_dist = self.water_dist.sample(cx, cz);
        // (Everything between a few pixels and a few tiles across is soft: the map's tiles are
        // shared between alike places, and a hard edge there would show their squares.)
        let broad = fbm(x / 900.0, z / 900.0, seed ^ 9, 3) * 0.5 + 0.5;
        let meadow = fbm(x / 260.0, z / 260.0, seed ^ 10, 3) * 0.5 + 0.5;
        let mottle = fbm(x / 46.0, z / 46.0, seed ^ 11, 3);
        // Grass, wetter or drier over broad stretches, meadows of richer green, in tufts.
        let mut c = mix3(s.grass_dry, s.grass_wet, smoothstep(0.2, 0.85, moist * 0.7 + broad * 0.3));
        c = mix3(c, s.grass_lush, smoothstep(0.5, 0.85, meadow) * 0.6);
        let tuft = 1.0 + 0.03 * mottle + 0.11 * g1 + 0.06 * g2 + 0.02 * g3;
        c = c.map(|v| v * tuft);
        // Bare earth: worn, faintly, over dry stretches, and at the foot of the slopes.
        let bare = smoothstep(0.58, 0.82, patch + (0.5 - moist) * 0.3) * s.bare + smoothstep(4.5, 10.0, slope) * 0.45;
        let dirt = s.dirt.map(|v| v * (0.9 + 0.14 * g2 + 0.12 * g1 + 0.06 * g3));
        c = mix3(c, dirt, bare.clamp(0.0, 1.0));
        // Forest floor under the trees.
        let floor = s.forest.map(|v| v * (0.8 + 0.4 * (value(x / 6.0, z / 6.0, seed ^ 6))));
        c = mix3(c, floor, (forest * 1.6).clamp(0.0, 0.85));
        // Rock on the slopes, in strata following the contours, cracked.
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 2.5 * g3);
        if rocky > 0.0 {
            let band = 0.5 + 0.5 * ((h + 4.0 * perlin(x / 31.0, z / 31.0, seed ^ 7)) * std::f32::consts::TAU / 7.5).sin();
            let crack = smoothstep(0.92, 0.98, 1.0 - perlin(x / 13.0, z / 7.0, seed ^ 8).abs());
            let mut r = mix3(s.rock_dark, s.rock, 0.35 + 0.55 * band);
            r = r.map(|v| v * (0.86 + 0.3 * (g1 + 0.5) * 0.5 + 0.1 * g2) * (1.0 - 0.35 * crack));
            c = mix3(c, r, rocky);
        }
        // Pale rock on the highest peaks.
        let high = smoothstep(206.0, 246.0, h + 10.0 * g3);
        c = mix3(c, s.peak.map(|v| v * (0.88 + 0.18 * (g2 + 0.5) + 0.1 * g1)), high * 0.75);
        // The shore: sand, wet at the water's edge (scorched rock on a lava world).
        let shore = smoothstep(s.beach, s.beach * 0.25, -depth + 1.5 * g3) * smoothstep(s.beach_cells, s.beach_cells * 0.3, water_dist);
        if shore > 0.0 {
            let sand = mix3(s.sand_wet, s.sand, smoothstep(0.4, 2.2, -depth)).map(|v| v * (0.92 + 0.16 * (g1 + 0.5) * 0.5));
            c = mix3(c, sand, shore);
        }
        self.shade(c, x, z, 0.2)
    }

    /// Shading: hollows darker, crests lighter, and a light from the north-west, `light` of
    /// the way (the game lights the relief itself).
    pub fn shade(&self, c: [f32; 3], x: f32, z: f32, light: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let cav = self.cavity.sample(cx, cz);
        let ao = 1.0 - (cav * 0.035).clamp(0.0, 0.32) + (-cav * 0.02).clamp(0.0, 0.08);
        let (gx, gz) = self.gradient(x, z);
        let n = normalize([-gx, 1.0, -gz]);
        let l = normalize([-0.55, 0.62, -0.56]);
        let ndl = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
        let shade = lerp(1.0, 0.70 + 0.52 * ndl, light) * lerp(1.0, ao, 0.7);
        c.map(|v| (v * shade).clamp(0.0, 1.0))
    }

    /// Shading for art whose tiles share by their look ([`wild`], [`desert`]): the light from
    /// the north-west in three bands, lit, level and in shade, with short edges between, and the
    /// deepest hollows darker; never a gradient across many tiles, which the shared tiles would
    /// cut into squares. (The game lights the relief itself; this is what the classic camera has.)
    pub fn shade_bands(&self, c: [f32; 3], x: f32, z: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let (gx, gz) = self.gradient(x, z);
        // How much the ground faces the light, against level ground's.
        let toward = -(gx * -0.55 + gz * -0.56);
        let lit = smoothstep(0.06, 0.1, toward) * 0.07 - smoothstep(-0.06, -0.1, toward) * 0.13 - smoothstep(-0.22, -0.28, toward) * 0.08;
        let hollow = smoothstep(3.0, 4.0, self.cavity.sample(cx, cz)) * 0.12;
        let k = 1.0 + lit - hollow;
        c.map(|v| (v * k).clamp(0.0, 1.0))
    }

    /// The ground's rise per pixel along x and z at a point.
    pub fn gradient(&self, x: f32, z: f32) -> (f32, f32) {
        let e = 6.0;
        ((self.h(x + e, z) - self.h(x - e, z)) / (2.0 * e), (self.h(x, z + e) - self.h(x, z - e)) / (2.0 * e))
    }

    /// The distance (cells) from a pixel to the nearest track, exactly near one.
    #[inline]
    pub fn track(&self, x: f32, z: f32) -> f32 {
        self.tracks.exact(x / 16.0, z / 16.0)
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// `c` scaled by `k`, each channel.
#[inline]
pub fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// For each art row of the columns `x0..x0 + width` (pixels), the ground seen there in TA's
/// view: its z and height ([`ta_mapart::slant::columns`]). Rows `0..rows`.
pub fn view_columns(g: &Ground, x0: usize, width: usize, rows: usize) -> Vec<Vec<(f32, f32)>> {
    ta_mapart::slant::columns(|x, z| g.h(x, z), x0, width, rows)
}
