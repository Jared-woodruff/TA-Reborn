//! The lie of the land: heights over the map's cells, the water, and where the players start.
//! Every world is built from the same parts: a smooth elevation shaped into tiers joined by cliff
//! bands (ramps where a second field says so), hills and ridged peaks on top, rivers and lakes or
//! a sea carved in, and level ground for the starts on the land vehicles can reach. Deserts add
//! dunes, dry river beds and oases; duels and team maps are made point-symmetric, each half the
//! other turned round the middle, so no start is better placed than another.

use rayon::prelude::*;

use super::decks::{Built, Decks};
use super::field::{Field, distance};
use super::noise::{fbm, lerp, perlin, ridged, smoothstep, unit};

/// What the land is made of, for the painter and the placer.
pub struct Land {
    /// Cells across (west to east) and down (north to south).
    pub w: usize,
    pub h: usize,
    pub sea: f32,
    pub height: Field,
    /// Distance in cells to the nearest water (0 in it).
    pub water_dist: Field,
    /// 0 dry to 1 wet: the grass's colour, where forests grow.
    pub moisture: Field,
    /// Where the players start (cells): the first four nearest the middle (the first maps), or
    /// (symmetric maps) in pairs facing each other across the middle.
    pub starts: Vec<(f32, f32)>,
    /// Distance in cells to the middle of a dry river bed (a wadi), far where there is none.
    pub bed: Field,
    /// Where dunes lie (0 to 1: how much of their height the ground has).
    pub dune_mask: Field,
    /// Where the river is shallow enough to wade (cells).
    pub fords: Vec<(f32, f32)>,
    /// What was built, on a metal world.
    pub built: Option<Built>,
    /// Space (TA's void cells: nothing there, aircraft cross it), the gantries laid across it
    /// and the roads graded over the rock to them; empty but on a world in space.
    pub void: Vec<bool>,
    pub gantry: Vec<bool>,
    pub road: Vec<bool>,
    /// Frozen water (a winter world's lakes and rivers: level ground of ice where the water
    /// was); empty but on a winter world.
    pub ice: Vec<bool>,
    /// A real place's biomes, warmth and salt pans (`super::geo`); none for the rest.
    pub geo: Option<Box<super::geo::GeoLand>>,
    /// The symmetry the land was made with (the placer keeps to it).
    pub sym: Sym,
    /// The halves are apart (two coasts across an ocean, `super::coasts`): tracks join the bases
    /// of a half, never across the middle.
    pub halves_apart: bool,
}

/// How a world's land is laid out.
#[derive(Clone)]
pub struct Shape {
    /// Cells across and down.
    pub w: usize,
    pub h: usize,
    pub seed: u32,
    pub sea: f32,
    /// The tiers' heights, lowest first, and the share of the land below each step.
    pub levels: Vec<f32>,
    pub steps: Vec<f32>,
    /// Hills on the tiers (height units) and how far apart (cells).
    pub hills: f32,
    pub hill_scale: f32,
    /// Ridged peaks on the highest tier (height units).
    pub peaks: f32,
    /// A river across the map, with this many lakes along it.
    pub river: bool,
    pub lakes: usize,
    /// An ocean around and between islands: the share of the map under the sea (0 for none).
    pub ocean: f32,
    /// How many hills and valleys (or islands) across the map: the elevation's frequency.
    pub scale: f32,
    pub players: usize,
    /// Point symmetry about the middle (the halves split across `axis`, radians: the direction
    /// from the middle into the first half), or none.
    pub symmetric: bool,
    pub axis: f32,
    /// How much more of the tiers' steps are ramps rather than cliffs (added to the ramp field).
    pub ramps: f32,
    /// Bumps on the elevation before it is ranked, raising or sinking parts of the map: x and z
    /// as fractions of the map, the radius as a fraction of its longer side, the strength (the
    /// elevation runs about -1 to 1). Mirrored on a symmetric map.
    pub guide: Vec<(f32, f32, f32, f32)>,
    /// Dunes on the lowest tier: their height (units), the cells between crests, the direction
    /// the wind blows from (radians, 0 from the west).
    pub dunes: (f32, f32, f32),
    /// The river's bed dry (a wadi): carved shallow, sandy, no water in it.
    pub dry: bool,
    /// Pools in hollows of the low ground (oases, tarns): how many (pairs on a symmetric map) and
    /// their radius in cells.
    pub pools: (usize, f32),
    /// Where the players start: the distance between bases kept clear of water (cells), and
    /// how far from the middle the first pair stands (a fraction of the half-diagonal; 0 as far
    /// as can be).
    pub start_reach: f32,
    /// Cut ramps where a start's ground is walled off from the first start's, so a tank can
    /// drive from every base to every other (land maps; the first maps were made without).
    pub connect: bool,
    /// Rings on the elevation before it is ranked (a caldera's rim, an atoll's reef): x and z
    /// as fractions of the map, the radius and the ring's half width as fractions of its longer
    /// side, the strength. Mirrored on a symmetric map (one round the middle is its own twin).
    pub rings: Vec<(f32, f32, f32, f32, f32)>,
    /// Shoals and bridges: the sea bed raised along a line (x0, z0, x1, z1 as fractions of the
    /// map, the half width in cells) to `rise` units above the sea (below it, negative: a sand
    /// bar land units wade across where ships cannot; above it, a bridge of rock over a river of
    /// fire). Mirrored on a symmetric map.
    pub shoals: Vec<(f32, f32, f32, f32, f32, f32)>,
    /// Shallow fords along the river (the first maps had them; a river of fire is crossed by
    /// bridges instead).
    pub fords: bool,
    /// Where a start is cut off by water, [`connect`] may raise a causeway across it, to this
    /// many units above the sea (a bridge of basalt over a sea of fire) or below it (a sand bar
    /// a tank wades), or none (the water is crossed by sea and air only).
    pub causeway: Option<f32>,
    /// A built world (metal decks on a grid) in place of the grown land.
    pub decks: Option<Decks>,
    /// A world in space: the share of the map under `ocean` is void (rocks floating in it),
    /// the rocks' rims falling away to it in faces of rock; the shoals are gantries across it,
    /// and causeways too.
    pub space: bool,
    /// Impact craters: how many per million cells (pairs on a symmetric map), and the largest's
    /// radius (cells).
    pub craters: (f32, f32),
    /// Judge where a tank can drive on the heights as the map stores them, whole units (a slope
    /// of 9.6 may be 11 there): the first maps were made judging the ground as grown, and kept.
    pub whole: bool,
    /// A winter world: its water frozen, level ice at the water's height (the map has no sea),
    /// which every unit crosses.
    pub frozen: bool,
    /// A real place, grown from its rasters (`super::geo`): its levels and steps then the tiers'
    /// heights and the relief (0 to 1) each rises at.
    pub geo: Option<super::geo::Geo>,
    /// A map for fleets: a harbour by every base within this many cells of the sea ([`harbours`]),
    /// or none (0).
    pub harbours: f32,
    /// Two coasts facing each other across an ocean, laid out (`super::coasts`) in place of the
    /// grown land.
    pub coasts: Option<super::coasts::Coasts>,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape {
            w: 1024,
            h: 1024,
            seed: 1,
            sea: 40.0,
            levels: vec![64.0, 104.0, 146.0, 186.0],
            steps: vec![0.36, 0.68, 0.90],
            hills: 12.0,
            hill_scale: 70.0,
            peaks: 40.0,
            river: false,
            lakes: 0,
            ocean: 0.0,
            scale: 2.4,
            players: 2,
            symmetric: false,
            axis: std::f32::consts::FRAC_PI_4 * 5.0,
            ramps: 0.0,
            guide: Vec::new(),
            dunes: (0.0, 40.0, 0.0),
            dry: false,
            pools: (0, 10.0),
            start_reach: 0.0,
            connect: false,
            rings: Vec::new(),
            shoals: Vec::new(),
            fords: true,
            causeway: None,
            decks: None,
            space: false,
            craters: (0.0, 20.0),
            whole: false,
            frozen: false,
            geo: None,
            harbours: 0.0,
            coasts: None,
        }
    }
}

/// A map's symmetry: none, or each point the same as the point turned half round the middle.
/// Noise read through [`Sym::eval`] comes out symmetric: the first half reads it where it is, the
/// second where its twin is, and a band between the halves blends the two.
#[derive(Clone, Copy)]
pub struct Sym {
    pub on: bool,
    w: f32,
    h: f32,
    /// The direction into the first half, and the blend band's half width (cells).
    nx: f32,
    nz: f32,
    band: f32,
}

impl Sym {
    pub fn none(w: usize, h: usize) -> Sym {
        Sym { on: false, w: w as f32, h: h as f32, nx: 0.0, nz: 0.0, band: 1.0 }
    }

    pub fn halves(w: usize, h: usize, axis: f32) -> Sym {
        Sym { on: true, w: w as f32, h: h as f32, nx: axis.cos(), nz: axis.sin(), band: w.max(h) as f32 * 0.05 }
    }

    /// The cell turned half round the middle.
    #[inline]
    pub fn twin(&self, x: f32, z: f32) -> (f32, f32) {
        (self.w - 1.0 - x, self.h - 1.0 - z)
    }

    /// The cell index turned half round (cells `w` across).
    #[inline]
    pub fn twin_index(&self, i: usize) -> usize {
        let w = self.w as usize;
        let (x, z) = (i % w, i / w);
        (self.h as usize - 1 - z) * w + (w - 1 - x)
    }

    /// How far into the first half a cell lies (cells; negative in the second).
    #[inline]
    pub fn side(&self, x: f32, z: f32) -> f32 {
        (x - (self.w - 1.0) * 0.5) * self.nx + (z - (self.h - 1.0) * 0.5) * self.nz
    }

    /// Whether a cell is in the first half (always, without symmetry).
    #[inline]
    pub fn first(&self, x: f32, z: f32) -> bool {
        !self.on || self.side(x, z) > 0.0
    }

    /// `f` made symmetric (for fields of any mean: the two halves blended linearly).
    #[inline]
    pub fn eval(&self, x: f32, z: f32, f: impl Fn(f32, f32) -> f32) -> f32 {
        if !self.on {
            return f(x, z);
        }
        let s = smoothstep(self.band, -self.band, self.side(x, z));
        let (tx, tz) = self.twin(x, z);
        if s <= 0.0 {
            f(x, z)
        } else if s >= 1.0 {
            f(tx, tz)
        } else {
            lerp(f(x, z), f(tx, tz), s)
        }
    }

    /// `f`, a noise about 0, made symmetric keeping its spread across the band (two noises
    /// averaged are flatter than either).
    #[inline]
    pub fn noise(&self, x: f32, z: f32, f: impl Fn(f32, f32) -> f32) -> f32 {
        if !self.on {
            return f(x, z);
        }
        let s = smoothstep(self.band, -self.band, self.side(x, z));
        let (tx, tz) = self.twin(x, z);
        if s <= 0.0 {
            f(x, z)
        } else if s >= 1.0 {
            f(tx, tz)
        } else {
            (f(x, z) * (1.0 - s) + f(tx, tz) * s) / ((1.0 - s) * (1.0 - s) + s * s).sqrt()
        }
    }

    /// Makes a field exactly symmetric: each cell and its twin take their mean.
    pub fn settle(&self, f: &mut Field) {
        if !self.on {
            return;
        }
        for i in 0..f.data.len() {
            let j = self.twin_index(i);
            if j > i {
                let m = (f.data[i] + f.data[j]) * 0.5;
                f.data[i] = m;
                f.data[j] = m;
            }
        }
    }
}

pub fn build(s: &Shape) -> Land {
    let (w, h) = (s.w, s.h);
    let seed = s.seed;
    let sym = if s.symmetric { Sym::halves(w, h, s.axis) } else { Sym::none(w, h) };
    if let Some(d) = &s.decks {
        return build_decks(s, d, &sym);
    }
    if let Some(g) = &s.geo {
        return super::geo::build(s, g, &sym);
    }
    if let Some(c) = &s.coasts {
        return super::coasts::build(s, c, &sym);
    }
    // (The noise's scale is the longer side's, so a map twice as wide has twice the hills.)
    let nf = w.max(h) as f32;
    let guides: Vec<(f32, f32, f32, f32)> = s
        .guide
        .iter()
        .flat_map(|&(gx, gz, r, k)| {
            let (cx, cz) = (gx * (w as f32 - 1.0), gz * (h as f32 - 1.0));
            let mut v = vec![(cx, cz, r * nf, k)];
            if sym.on {
                let (tx, tz) = sym.twin(cx, cz);
                v.push((tx, tz, r * nf, k));
            }
            v
        })
        .collect();
    let rings: Vec<(f32, f32, f32, f32, f32)> = s
        .rings
        .iter()
        .flat_map(|&(gx, gz, r, half, k)| {
            let (cx, cz) = (gx * (w as f32 - 1.0), gz * (h as f32 - 1.0));
            let mut v = vec![(cx, cz, r * nf, half * nf, k)];
            if sym.on {
                let (tx, tz) = sym.twin(cx, cz);
                if (tx - cx).hypot(tz - cz) > 1.0 {
                    v.push((tx, tz, r * nf, half * nf, k));
                }
            }
            v
        })
        .collect();
    // 1. A smooth elevation, warped so its contours meander, ranked so the tiers take their shares.
    let raw = Field::from_fn(w, h, |x, z| {
        let e = sym.noise(x as f32, z as f32, |x, z| {
            let (u, v) = (x / nf, z / nf);
            let wx = u + 0.10 * fbm(u * 2.5, v * 2.5, seed ^ 0x11, 4);
            let wz = v + 0.10 * fbm(u * 2.5 + 5.2, v * 2.5 + 1.3, seed ^ 0x22, 4);
            fbm(wx * s.scale, wz * s.scale, seed ^ 0x33, 6)
        });
        let mut e = e;
        if s.ocean > 0.0 {
            // Islands: a little lower towards the edges, so the sea surrounds them.
            let edge = ((x as f32 / w as f32 - 0.5).abs().max((z as f32 / h as f32 - 0.5).abs()) * 2.0).powi(4);
            e -= edge * 0.55;
        }
        for &(gx, gz, r, k) in &guides {
            let d2 = ((x as f32 - gx).powi(2) + (z as f32 - gz).powi(2)) / (r * r);
            e += k * (-d2).exp();
        }
        for &(gx, gz, r, half, k) in &rings {
            let d = (x as f32 - gx).hypot(z as f32 - gz);
            e += k * (-((d - r) / half).powi(2)).exp();
        }
        e
    });
    let rank = ranks(&raw);
    // How fast the rank changes, per cell: distances in rank over it are distances in cells.
    let rank_slope = rank.gradient();
    // 2. Tiers, joined by cliffs; where the ramp field is high, by long ramps instead. Each step
    // rises over a band at least two cells wide (measured in cells, not in rank: a band
    // narrower than a cell put each cliff's lip on the grid's cells, a staircase).
    let ramp = Field::from_fn(w, h, |x, z| smoothstep(0.15, 0.55, sym.noise(x as f32, z as f32, |x, z| fbm(x / 90.0, z / 90.0, seed ^ 0x44, 3)) + s.ramps));
    let mut height = Field::from_fn(w, h, |x, z| {
        let t = rank.at(x, z);
        let r = ramp.at(x, z);
        let g = rank_slope.at(x, z).max(1e-7);
        let half = lerp(1.0, 11.0, r);
        let mut v = s.levels[0];
        for (k, &step) in s.steps.iter().enumerate() {
            let k1 = smoothstep(-half, half, (t - step) / g);
            v += (s.levels[k + 1] - s.levels[k]) * k1;
        }
        // Each tier rises a little across its width.
        v + 10.0 * t
    });
    // 3. Hills, finer ripples, and on the heights ridged peaks; dunes on the low ground.
    let top = *s.steps.last().unwrap_or(&1.0);
    let low = s.steps.first().copied().unwrap_or(1.0);
    let (dune_h, dune_gap, dune_wind) = s.dunes;
    let dune_mask = if dune_h > 0.0 { Field::from_fn(w, h, |x, z| smoothstep(low, low - 0.08, rank.at(x, z))) } else { Field::new(w, h, 0.0) };
    height.data.par_iter_mut().enumerate().for_each(|(i, ht)| {
        let (x, z) = ((i % w) as f32, (i / w) as f32);
        let t = rank.data[i];
        let hills = (sym.noise(x, z, |x, z| fbm(x / s.hill_scale, z / s.hill_scale, seed ^ 0x55, 4)) * 0.5 + 0.5) * s.hills;
        let ripple = sym.noise(x, z, |x, z| fbm(x / 9.0, z / 9.0, seed ^ 0x66, 2)) * 1.6;
        let peaks = sym.eval(x, z, |x, z| ridged(x / 70.0, z / 70.0, seed ^ 0x77, 5)).powf(1.6) * s.peaks * smoothstep(top, top + 0.06, t);
        let dunes = if dune_h > 0.0 { sym.eval(x, z, |x, z| dune(x, z, dune_gap, dune_wind, seed)) * dune_h * dune_mask.data[i] } else { 0.0 };
        *ht += hills + ripple + peaks + dunes;
    });
    // 4. Water: a river with lakes, or an ocean, or pools. How far in from its shore each cell
    // lies (`wet`, cells; negative on land): a continuous field, so the shore runs between the
    // cells where it is, not round them. A dry river's bed is kept apart (`bed_wet`).
    let mut fords = Vec::new();
    let mut wet = vec![-1e6f32; w * h];
    let mut bed_wet = vec![-1e6f32; w * h];
    if s.space {
        return space(s, &sym, rank, rank_slope, height);
    }
    if s.ocean > 0.0 {
        let cut = quantile(&rank, s.ocean);
        for (i, wt) in wet.iter_mut().enumerate() {
            *wt = (cut - rank.data[i]) / rank_slope.data[i].max(1e-7);
        }
    }
    let mut river_line = Vec::new();
    if s.river {
        river_line = if sym.on { river_symmetric(w, h, seed, &sym) } else { river(w, h, seed) };
        let channel = if s.dry { &mut bed_wet } else { &mut wet };
        // The channel, a few cells wide (the distance to its course, exactly, near it), and
        // its lakes.
        let mut course = vec![f32::MAX; w * h];
        for &(px, pz) in &river_line {
            let (x0, x1) = ((px - 12.0).floor().max(0.0) as usize, ((px + 12.0).ceil().max(0.0) as usize).min(w - 1));
            let (z0, z1) = ((pz - 12.0).floor().max(0.0) as usize, ((pz + 12.0).ceil().max(0.0) as usize).min(h - 1));
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let d = (x as f32 - px).hypot(z as f32 - pz);
                    let c = &mut course[z * w + x];
                    *c = c.min(d);
                }
            }
        }
        for (i, wt) in channel.iter_mut().enumerate() {
            let (x, z) = ((i % w) as f32, (i / w) as f32);
            let half = 4.5 + 2.5 * (sym.eval(x, z, |x, z| perlin(x / 40.0, z / 40.0, seed ^ 0x88)) * 0.5 + 0.5);
            if course[i] < f32::MAX {
                *wt = wt.max(half - course[i]);
            }
        }
        if !s.dry {
            for k in 0..s.lakes {
                let at = river_line[river_line.len() * (k + 1) / (s.lakes + 1)];
                // (Smaller on smaller maps.)
                let r = (55.0 + 30.0 * unit(k as i32, 3, seed)) * (nf / 1024.0).min(1.0);
                lake(&mut wet, w, h, at, r, k as f32 * 7.0, seed);
            }
            // Fords: four shallow crossings spread along the river (not in the lakes).
            if s.fords {
                for k in 0..4 {
                    let i = river_line.len() * (2 * k + 1) / 8;
                    fords.push(river_line[i]);
                }
            }
        }
    }
    if sym.on {
        for field in [&mut wet, &mut bed_wet] {
            for i in 0..w * h {
                let j = sym.twin_index(i);
                if j > i {
                    let m = field[i].max(field[j]);
                    field[i] = m;
                    field[j] = m;
                }
            }
        }
    }
    let water: Vec<bool> = wet.iter().map(|&wt| wt > 0.0).collect();
    let land: Vec<bool> = water.iter().map(|wt| !wt).collect();
    let water_in = distance(w, h, &land);
    let mut water_dist = distance(w, h, &water);
    let ford_dist = if fords.is_empty() {
        Field::new(w, h, 1e6)
    } else {
        let mut seed_cells = vec![false; w * h];
        for &(x, z) in &fords {
            seed_cells[(z as usize).min(h - 1) * w + (x as usize).min(w - 1)] = true;
        }
        distance(w, h, &seed_cells)
    };
    // Carve: beds under the water, deep in the middle; a valley floor rising gently from the
    // banks, then canyon walls up to the land (ramps by the fords). Across the shore the ground
    // passes through sea level, 2 units a cell, so the waterline (and the painted shore and
    // the foam, which follow it) runs where the shore is. (`shore`: cells from the shore,
    // positive on land; from `wet` near it, the distances to the cells beyond.)
    let shore = |i: usize| {
        let wt = wet[i];
        let far = if water[i] { 0.5 - water_in.data[i] } else { water_dist.data[i] - 0.5 };
        lerp(-wt, far, smoothstep(1.5, 3.0, wt.abs()))
    };
    // How far above or below the sea the ground lies `a` cells from the shore, running into a
    // beach or bed rising `k` a cell.
    let profile = |a: f32, k: f32| lerp(2.0 * a, 2.5 + k * (a + 0.5), smoothstep(0.0, 1.5, a));
    height.data.par_iter_mut().enumerate().for_each(|(i, ht)| {
        let (x, z) = ((i % w) as f32, (i / w) as f32);
        let ford = smoothstep(26.0, 8.0, ford_dist.data[i]);
        let a = shore(i);
        if water[i] {
            let deep = if s.ocean > 0.0 { 46.0 } else { 20.0 };
            // Fleet maps must admit the Core's deepest submarine (25 units of draft),
            // including the sills joining their seas. Keep the coast and land bridges.
            let bed_slope = if s.ocean <= 0.0 { 2.4 } else if s.harbours > 0.0 && !s.connect { 1.6 } else { 0.8 };
            let depth = profile(-a, bed_slope).min(deep);
            let depth = lerp(depth, depth.min(7.0), ford);
            *ht = s.sea - depth;
        } else {
            let d = water_dist.data[i];
            let valley = if s.ocean > 0.0 { 22.0 } else { 34.0 + 18.0 * (sym.eval(x, z, |x, z| perlin(x / 60.0, z / 60.0, seed ^ 0xAA)) * 0.5 + 0.5) };
            let floor = s.sea + profile(a, 0.8);
            let wall = lerp(9.0, 1.6, ford.max(ramp.at(i % w, i / w) * 0.6));
            let limit = floor + (d - valley).max(0.0) * wall;
            *ht = ht.min(limit).max(s.sea + profile(a, 0.8).min(2.5));
        }
    });
    // Shoals and bridges: the sea bed raised along their lines.
    for &(x0, z0, x1, z1, half, rise) in &s.shoals {
        let mut lines = vec![((x0 * (w as f32 - 1.0), z0 * (h as f32 - 1.0)), (x1 * (w as f32 - 1.0), z1 * (h as f32 - 1.0)))];
        if sym.on {
            let (a, b) = lines[0];
            lines.push((sym.twin(a.0, a.1), sym.twin(b.0, b.1)));
        }
        let top = s.sea + rise;
        for ((ax, az), (bx, bz)) in lines {
            let (dx, dz) = (bx - ax, bz - az);
            let len2 = (dx * dx + dz * dz).max(1e-6);
            height.data.par_iter_mut().enumerate().for_each(|(i, ht)| {
                let (x, z) = ((i % w) as f32, (i / w) as f32);
                let t = (((x - ax) * dx + (z - az) * dz) / len2).clamp(0.0, 1.0);
                let d = (x - ax - dx * t).hypot(z - az - dz * t);
                // (Wandering a little along its length, its sides sloping off.)
                let wide = half * (1.0 + 0.3 * perlin(t * 6.0, 0.7, seed ^ 0x5B));
                let k = smoothstep(wide + 6.0, wide, d);
                if k > 0.0 && *ht < top {
                    *ht = lerp(*ht, top, k);
                }
            });
        }
    }
    // (A bridge turns water to land: the distances to the water follow.)
    if s.shoals.iter().any(|sh| sh.5 > 0.0) {
        let now: Vec<bool> = height.data.iter().map(|&v| v < s.sea).collect();
        water_dist = distance(w, h, &now);
    }
    // A dry river: a flat bed sunk below the land round it, sloping banks.
    let bed = if s.dry && s.river {
        carve_bed(&mut height, &bed_wet, s.sea);
        let mut seeds = vec![false; w * h];
        for &(x, z) in &river_line {
            let (xi, zi) = (x.round() as i64, z.round() as i64);
            if xi >= 0 && zi >= 0 && (xi as usize) < w && (zi as usize) < h {
                seeds[zi as usize * w + xi as usize] = true;
            }
        }
        distance(w, h, &seeds)
    } else {
        Field::new(w, h, 1e6)
    };
    // 5. Start positions on level ground that vehicles can reach, then levelled for a base.
    let starts = if sym.on {
        place_starts_symmetric(&height, s.sea, &water_dist, s.players, s.start_reach, &sym)
    } else {
        place_starts(&height, s.sea, &water_dist, s.players, seed)
    };
    // No dunes where a base will be built (a factory needs level ground): round each start the
    // ground eased to its broad lie, the dunes fading out over the last stretch.
    let mut dune_mask = dune_mask;
    if dune_h > 0.0 {
        let broad = height.blurred(8);
        let calm = |x: f32, z: f32| starts.iter().map(|s| smoothstep(30.0, 50.0, (x - s.0).hypot(z - s.1))).fold(1.0f32, f32::min);
        height.data.par_iter_mut().zip(dune_mask.data.par_iter_mut()).enumerate().for_each(|(i, (ht, m))| {
            let k = calm((i % w) as f32, (i / w) as f32);
            if k < 1.0 {
                *ht = lerp(broad.data[i], *ht, k + (1.0 - k) * (1.0 - *m));
                *m *= k;
            }
        });
    }
    for &(sx, sz) in &starts {
        level(&mut height, sx, sz, 16.0, 30.0);
    }
    // Every start a tank can drive to: walled-off ground gets a ramp cut to it.
    if s.connect {
        let cut = connect(&mut height, s.sea, &starts, &sym, s.causeway, s.whole);
        if cut > 0 {
            eprintln!("  cut {cut} ramp(s) so every start can be driven to");
        }
    }
    // A map for fleets: deep water by every base.
    if s.harbours > 0.0 {
        let dredged = harbours(&mut height, s.sea, &starts, &sym, s.harbours);
        eprintln!("  dredged {dredged} harbour(s)");
    }
    // Pools in the hollows of the low ground, away from the bases.
    if s.pools.0 > 0 {
        let pools = pools(&height, s, &starts, &sym);
        if !pools.is_empty() {
            let mut pool_wet = vec![-1e6f32; w * h];
            for (k, &(px, pz)) in pools.iter().enumerate() {
                let r = s.pools.1 * (0.8 + 0.4 * unit(k as i32 / 2, 5, seed));
                lake(&mut pool_wet, w, h, (px, pz), r, (k / 2) as f32 * 5.0, seed ^ 0x5A);
            }
            if sym.on {
                for i in 0..w * h {
                    let j = sym.twin_index(i);
                    let m = pool_wet[i].max(pool_wet[j]);
                    pool_wet[i] = m;
                }
            }
            sink_pools(&mut height, &pool_wet, s.sea);
            let all: Vec<bool> = (0..w * h).map(|i| water[i] || pool_wet[i] > 0.0).collect();
            water_dist = distance(w, h, &all);
        }
    }
    sym.settle(&mut height);
    let moisture = Field::from_fn(w, h, |x, z| {
        let near_water = smoothstep(90.0, 4.0, water_dist.at(x, z));
        let m = sym.eval(x as f32, z as f32, |x, z| fbm(x / 160.0, z / 160.0, seed ^ 0xBB, 4)) * 0.5 + 0.5;
        (m * 0.75 + near_water * 0.35).clamp(0.0, 1.0)
    });
    // A winter world's water frozen: level ice where it was, a little above its height, and no
    // sea (the starts, the ramps and the causeways were laid round the water, as for a fleet).
    let (sea, ice) = if s.frozen { (0.0, freeze(&mut height, s.sea)) } else { (s.sea, Vec::new()) };
    Land { w, h, sea, height, water_dist, moisture, starts, bed, dune_mask, fords, built: None, void: Vec::new(), gantry: Vec::new(), road: Vec::new(), ice, geo: None, sym, halves_apart: false }
}

/// Freezes the water below `sea`: its cells levelled to the ice's height (a unit above the
/// sea's); returns which they are.
fn freeze(height: &mut Field, sea: f32) -> Vec<bool> {
    let ice: Vec<bool> = height.data.iter().map(|&v| v < sea + 1.0).collect();
    for (v, &frozen) in height.data.iter_mut().zip(&ice) {
        if frozen {
            *v = sea + 1.0;
        }
    }
    ice
}

/// How deep a rock's rim falls away to the void (units) over how many cells.
const RIM_DROP: f32 = 24.0;
const RIM_CELLS: f32 = 3.0;
/// How far either side of its line a gantry reaches (cells).
const GANTRY_HALF: f32 = 3.0;

/// A world in space: the ranked land below `ocean`'s share is void; craters pocked into the
/// rocks; the gantries (`shoals`) laid across the void between them; the starts on the rocks;
/// more gantries where a rock is cut off from the first start's; then the rocks' rims falling
/// away to the void.
fn space(s: &Shape, sym: &Sym, rank: Field, rank_slope: Field, mut base: Field) -> Land {
    let (w, h) = (s.w, s.h);
    let seed = s.seed;
    let cut = quantile(&rank, s.ocean);
    let mut void: Vec<bool> = (0..w * h).map(|i| (cut - rank.data[i]) / rank_slope.data[i].max(1e-7) > 0.0).collect();
    // Craters: a bowl and a raised rim, each in the first half and its twin.
    if s.craters.0 > 0.0 {
        let wanted = (s.craters.0 * (w * h) as f32 / 1e6 * if sym.on { 0.5 } else { 1.0 }) as usize;
        let mut list: Vec<(f32, f32, f32)> = Vec::new();
        for k in 0..wanted * 4 {
            if list.len() >= wanted * if sym.on { 2 } else { 1 } {
                break;
            }
            let (x, z) = (unit(k as i32, 1, seed ^ 0xC7A) * w as f32, unit(k as i32, 2, seed ^ 0xC7B) * h as f32);
            if !sym.first(x, z) || void[(z as usize).min(h - 1) * w + (x as usize).min(w - 1)] {
                continue;
            }
            // Many small, few large.
            let r = s.craters.1 * (0.15 + 0.85 * unit(k as i32, 3, seed ^ 0xC7C).powi(3));
            list.push((x, z, r));
            if sym.on {
                let (tx, tz) = sym.twin(x, z);
                list.push((tx, tz, r));
            }
        }
        for &(cx, cz, r) in &list {
            let reach = (r * 1.6).ceil() as i64;
            for dz in -reach..=reach {
                for dx in -reach..=reach {
                    let (x, z) = (cx as i64 + dx, cz as i64 + dz);
                    if x < 0 || z < 0 || x >= w as i64 || z >= h as i64 {
                        continue;
                    }
                    let d = ((x as f32 - cx).hypot(z as f32 - cz)) / r;
                    // The bowl a third of the radius deep, the rim at the radius, its ejecta
                    // falling off beyond.
                    let bowl = if d < 1.0 { -(1.0 - d * d) * 0.32 } else { 0.0 };
                    let rim = 0.12 * (-((d - 1.0) / 0.22).powi(2)).exp();
                    let i = z as usize * w + x as usize;
                    base.data[i] += (bowl + rim) * r * 1.6;
                }
            }
        }
    }
    // The recipe's gantries. (`keep`: the cells a gantry or its road over the rock was laid on,
    // which the rims leave level.)
    let mut gantry = vec![false; w * h];
    let mut keep = vec![false; w * h];
    for &(x0, z0, x1, z1, half, _) in &s.shoals {
        let (a, b) = ((x0 * (w as f32 - 1.0), z0 * (h as f32 - 1.0)), (x1 * (w as f32 - 1.0), z1 * (h as f32 - 1.0)));
        let line = |a: (f32, f32), b: (f32, f32)| -> Vec<(f32, f32)> {
            let n = (b.0 - a.0).hypot(b.1 - a.1).ceil().max(1.0) as usize;
            (0..=n).map(|k| (a.0 + (b.0 - a.0) * k as f32 / n as f32, a.1 + (b.1 - a.1) * k as f32 / n as f32)).collect()
        };
        lay_gantry(&mut base, &mut void, &mut gantry, &mut keep, &line(a, b), half);
        if sym.on {
            lay_gantry(&mut base, &mut void, &mut gantry, &mut keep, &line(sym.twin(a.0, a.1), sym.twin(b.0, b.1)), half);
        }
    }
    let mut height = rims(&base, &void, &keep, s.sea);
    let water_dist = distance(w, h, &void);
    let starts = if sym.on {
        place_starts_symmetric_on(&height, s.sea, &water_dist, s.players, s.start_reach, sym, Some(&void))
    } else {
        place_starts(&height, s.sea, &water_dist, s.players, seed)
    };
    for &(sx, sz) in &starts {
        level(&mut base, sx, sz, 16.0, 30.0);
    }
    height = rims(&base, &void, &keep, s.sea);
    if s.connect {
        let mut laid = 0;
        for _ in 0..s.players.max(2) * 3 {
            let judged = as_stored(&height, true);
            let reach = reachable(&judged, s.sea, &starts, Some(&void));
            let Some(k) = reach.iter().position(|&r| !r) else { break };
            let Some(path) = gantry_path(&judged, &void, starts[0], starts[k]) else { break };
            lay_gantry(&mut base, &mut void, &mut gantry, &mut keep, &path, GANTRY_HALF);
            if sym.on {
                let twin: Vec<(f32, f32)> = path.iter().map(|&(x, z)| sym.twin(x, z)).collect();
                lay_gantry(&mut base, &mut void, &mut gantry, &mut keep, &twin, GANTRY_HALF);
            }
            height = rims(&base, &void, &keep, s.sea);
            laid += 1;
        }
        if laid > 0 {
            eprintln!("  laid {laid} gantr(ies) so every start can be driven to");
        }
    }
    sym.settle(&mut height);
    let water_dist = distance(w, h, &void);
    let moisture = Field::from_fn(w, h, |x, z| sym.eval(x as f32, z as f32, |x, z| fbm(x / 160.0, z / 160.0, seed ^ 0xBB, 4)) * 0.5 + 0.5);
    let road: Vec<bool> = keep.iter().zip(&gantry).map(|(&k, &g)| k && !g).collect();
    Land { w, h, sea: s.sea, height, water_dist, moisture, starts, bed: Field::new(w, h, 1e6), dune_mask: Field::new(w, h, 0.0), fords: Vec::new(), built: None, void, gantry, road, ice: Vec::new(), geo: None, sym: *sym, halves_apart: false }
}

/// Lays a gantry along a way (cells, a cell apart), `half` cells either side of it: its cells out
/// of the void (`gantry`: plating), the way over the rock a road graded into it, all of it
/// (`keep`) a profile from the rock at its one end to the rock at its other (straight across the
/// void), eased so it rises at most [`RAMP_RISE`] a cell, the rock beside it blended in.
fn lay_gantry(base: &mut Field, void: &mut [bool], gantry: &mut [bool], keep: &mut [bool], pts: &[(f32, f32)], half: f32) {
    let (w, h) = (base.w, base.h);
    if pts.len() < 2 {
        return;
    }
    let cell = |p: (f32, f32)| (p.1.round().clamp(0.0, h as f32 - 1.0) as usize) * w + p.0.round().clamp(0.0, w as f32 - 1.0) as usize;
    // Along the way: the rock's heights where it is rock, a straight line across the void
    // between the rock either side.
    let mut along: Vec<f32> = pts.iter().map(|&p| base.data[cell(p)]).collect();
    let solid: Vec<bool> = pts.iter().map(|&p| !void[cell(p)] || gantry[cell(p)]).collect();
    let mut k = 0;
    while k < pts.len() {
        if solid[k] {
            k += 1;
            continue;
        }
        let start = k;
        while k < pts.len() && !solid[k] {
            k += 1;
        }
        let a = if start > 0 { along[start - 1] } else if k < pts.len() { along[k] } else { along[start] };
        let b = if k < pts.len() { along[k] } else { a };
        let n = (k - start + 1) as f32;
        for (j, v) in along[start..k].iter_mut().enumerate() {
            *v = a + (b - a) * (j as f32 + 1.0) / n;
        }
    }
    // Eased (no step steeper than the rise, cutting and filling about the middle), smoothed.
    let mut lo = along.clone();
    let mut hi = along.clone();
    for i in 1..lo.len() {
        lo[i] = lo[i].max(lo[i - 1] - RAMP_RISE);
        hi[i] = hi[i].min(hi[i - 1] + RAMP_RISE);
    }
    for i in (0..lo.len() - 1).rev() {
        lo[i] = lo[i].max(lo[i + 1] - RAMP_RISE);
        hi[i] = hi[i].min(hi[i + 1] + RAMP_RISE);
    }
    let mut p: Vec<f32> = lo.iter().zip(&hi).map(|(a, b)| (a + b) * 0.5).collect();
    for _ in 0..3 {
        let q = p.clone();
        for i in 1..p.len() - 1 {
            p[i] = (q[i - 1] + q[i] * 2.0 + q[i + 1]) * 0.25;
        }
    }
    let reach = half + 3.0;
    let mut best = vec![(f32::MAX, 0.0f32); w * h];
    for (k, &(px, pz)) in pts.iter().enumerate() {
        let (x0, x1) = ((px - reach).floor().max(0.0) as usize, ((px + reach).ceil() as usize).min(w - 1));
        let (z0, z1) = ((pz - reach).floor().max(0.0) as usize, ((pz + reach).ceil() as usize).min(h - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                let d = (x as f32 - px).hypot(z as f32 - pz);
                let i = z * w + x;
                if d < best[i].0 {
                    best[i] = (d, p[k]);
                }
            }
        }
    }
    for i in 0..w * h {
        let (d, v) = best[i];
        if d <= half {
            if void[i] {
                void[i] = false;
                gantry[i] = true;
            }
            keep[i] = true;
            base.data[i] = v;
        } else if d <= reach && !void[i] && !keep[i] {
            // The rock beside it blended in.
            base.data[i] = lerp(base.data[i], v, smoothstep(reach, half, d));
        }
    }
}

/// The heights drawn from the rocks' (`base`): the void at the sea, the rocks' last cells
/// falling away to it over [`RIM_CELLS`] ([`RIM_DROP`] at the edge), the gantries and their roads
/// (`keep`) level to their edges and the rock where they run not falling away (or there would be
/// a step there).
fn rims(base: &Field, void: &[bool], keep: &[bool], sea: f32) -> Field {
    let (w, h) = (base.w, base.h);
    let dist = distance(w, h, void);
    let near_gantry = if keep.iter().any(|&g| g) { distance(w, h, keep) } else { Field::new(w, h, 1e6) };
    let mut height = base.clone();
    for i in 0..w * h {
        if void[i] {
            height.data[i] = sea;
        } else if !keep[i] {
            let d = dist.data[i];
            if d <= RIM_CELLS + 0.5 {
                let landing = smoothstep(1.5, 6.0, near_gantry.data[i]);
                height.data[i] -= RIM_DROP * smoothstep(RIM_CELLS + 0.5, 0.5, d) * landing;
            }
        }
    }
    height
}

/// The cheapest way for a gantry from the rock at `a` to the rock at `b` (cells): across the void
/// dear, over level rock cheap, as points a cell apart.
fn gantry_path(height: &Field, void: &[bool], a: (f32, f32), b: (f32, f32)) -> Option<Vec<(f32, f32)>> {
    let (w, h) = (height.w, height.h);
    let slope = height.ta_slope();
    let cost: Vec<f32> = (0..w * h).map(|i| if void[i] { 6.0 } else { 1.0 + slope.data[i] * 0.3 }).collect();
    let from = (a.1 as usize).min(h - 1) * w + (a.0 as usize).min(w - 1);
    let goal = (b.1 as usize).min(h - 1) * w + (b.0 as usize).min(w - 1);
    let path = cheapest_to(&cost, w, h, from, |i| i == goal)?;
    Some(path.iter().map(|&i| ((i % w) as f32, (i / w) as f32)).collect())
}

/// A built world's land: its decks ([`super::decks`]), the starts on them (not levelled: the
/// decks are level), ramps cut where a start's deck is walled off.
fn build_decks(s: &Shape, d: &Decks, sym: &Sym) -> Land {
    let (w, h) = (s.w, s.h);
    let (mut height, built) = super::decks::build(d, w, h, s.sea, s.seed, sym);
    let water: Vec<bool> = height.data.iter().map(|&v| v < s.sea).collect();
    let water_dist = distance(w, h, &water);
    let starts = if sym.on { place_starts_symmetric(&height, s.sea, &water_dist, s.players, s.start_reach, sym) } else { place_starts(&height, s.sea, &water_dist, s.players, s.seed) };
    if s.connect {
        let cut = connect(&mut height, s.sea, &starts, sym, s.causeway, s.whole);
        if cut > 0 {
            eprintln!("  cut {cut} ramp(s) so every start can be driven to");
        }
    }
    sym.settle(&mut height);
    let moisture = Field::from_fn(w, h, |x, z| sym.eval(x as f32, z as f32, |x, z| fbm(x / 160.0, z / 160.0, s.seed ^ 0xBB, 4)) * 0.5 + 0.5);
    Land { w, h, sea: s.sea, height, water_dist, moisture, starts, bed: Field::new(w, h, 1e6), dune_mask: Field::new(w, h, 0.0), fords: Vec::new(), built: Some(built), void: Vec::new(), gantry: Vec::new(), road: Vec::new(), ice: Vec::new(), geo: None, sym: *sym, halves_apart: false }
}

/// A dune field's shape at (x, z) (cells), 0 to 1: crests across the wind `gap` cells apart, a
/// long slope up from the windward side and a short, steep one down the lee (the slip face),
/// the crests winding and rising and falling along their length, smaller ripples between.
pub fn dune(x: f32, z: f32, gap: f32, wind: f32, seed: u32) -> f32 {
    let (c, s) = (wind.cos(), wind.sin());
    // Along the wind (u) and across it (v).
    let (u, v) = (x * c + z * s, -x * s + z * c);
    // The crests bow downwind between the horns, crescents in a chain.
    let bow = (v / (gap * 2.4) * std::f32::consts::PI).sin().abs();
    let bend = fbm(v / (gap * 2.6), u / (gap * 4.0), seed ^ 0xD1, 3) * gap * 1.3 + bow * gap * 0.45;
    let t = (u + bend) / gap;
    let f = t - t.floor();
    // Up the windward slope over 80% of the gap, down the lee over the rest.
    let profile = if f < 0.8 { smoothstep(0.0, 0.8, f) } else { smoothstep(1.0, 0.8, f) };
    // Each crest's height rises and falls along it, and breaks between the crescents.
    let crest = 0.55 + 0.45 * fbm(v / (gap * 1.6), t * 0.7, seed ^ 0xD2, 3) - 0.35 * smoothstep(0.75, 0.98, 1.0 - bow);
    let small = 0.5 + 0.5 * ((u + bend * 0.5) / (gap * 0.23)).sin();
    (profile * crest.clamp(0.0, 1.0) * 0.85 + small * 0.15 * smoothstep(0.3, 0.0, profile)).clamp(0.0, 1.0)
}

/// Adds a lake (a ragged disc of radius `r` round `at`, squashed north to south) to `wet`.
fn lake(wet: &mut [f32], w: usize, h: usize, at: (f32, f32), r: f32, phase: f32, seed: u32) {
    let (x0, z0) = (((at.0 - r * 1.6).max(0.0)) as usize, ((at.1 - r * 1.6).max(0.0)) as usize);
    let (x1, z1) = (((at.0 + r * 1.6) as usize).min(w - 1), ((at.1 + r * 1.6) as usize).min(h - 1));
    for z in z0..=z1 {
        for x in x0..=x1 {
            let (dx, dz) = (x as f32 - at.0, (z as f32 - at.1) * 1.25);
            let a = dz.atan2(dx);
            let edge = r * (1.0 + 0.28 * fbm(a.cos() * 1.5 + phase, a.sin() * 1.5, seed ^ 0x99, 3));
            let wt = &mut wet[z * w + x];
            *wt = wt.max(edge - (dx * dx + dz * dz).sqrt());
        }
    }
}

/// Sinks a dry river's bed (`bed`: cells in from its banks, positive in it) into the land: a
/// flat floor following the land round it (blurred), 8 units down, banks sloping 3 units a cell,
/// never below the sea.
fn carve_bed(height: &mut Field, bed: &[f32], sea: f32) {
    let around = height.blurred(10);
    height.data.par_iter_mut().enumerate().for_each(|(i, ht)| {
        let b = bed[i];
        if b <= -4.0 {
            return;
        }
        let floor = (around.data[i] - 8.0).max(sea + 5.0);
        let k = smoothstep(-4.0, 1.5, b);
        *ht = lerp(*ht, ht.min(floor), k);
    });
}

/// Where pools lie: hollows of the lowest ground, level, away from the bases and each other (in
/// the first half, mirrored, on a symmetric map).
fn pools(height: &Field, s: &Shape, starts: &[(f32, f32)], sym: &Sym) -> Vec<(f32, f32)> {
    let (w, h) = (height.w, height.h);
    let slope = height.ta_slope().blurred(3);
    let around = height.blurred(16);
    let mut candidates: Vec<(f32, f32, f32)> = Vec::new();
    let margin = (s.pools.1 * 2.0) as usize + 4;
    for z in (margin..h.saturating_sub(margin)).step_by(3) {
        for x in (margin..w.saturating_sub(margin)).step_by(3) {
            let (xf, zf) = (x as f32, z as f32);
            if !sym.first(xf, zf) || (sym.on && sym.side(xf, zf) < s.pools.1 * 2.0) {
                continue;
            }
            let i = z * w + x;
            if slope.data[i] > 3.0 || height.data[i] < s.sea + 6.0 {
                continue;
            }
            if starts.iter().any(|p| (xf - p.0).hypot(zf - p.1) < 60.0) {
                continue;
            }
            // Lower than round it is better.
            candidates.push((xf, zf, height.data[i] - around.data[i] + slope.data[i] * 2.0 + unit(x as i32, z as i32, s.seed ^ 0x77) * 3.0));
        }
    }
    candidates.sort_by(|a, b| a.2.total_cmp(&b.2));
    let mut out: Vec<(f32, f32)> = Vec::new();
    let apart = (w.min(h) as f32 / (s.pools.0 as f32 + 1.0).sqrt() * 0.45).max(s.pools.1 * 5.0);
    for c in candidates {
        if out.len() >= s.pools.0 * if sym.on { 2 } else { 1 } {
            break;
        }
        let far = out.iter().all(|p| (c.0 - p.0).hypot(c.1 - p.1) > apart);
        if far {
            out.push((c.0, c.1));
            if sym.on {
                out.push(sym.twin(c.0, c.1));
            }
        }
    }
    out
}

/// Sinks pools (`wet`: cells in from their shores) into the land: water below the sea in the
/// middle, the ground passing through sea level at the shore and rising to the land round it.
fn sink_pools(height: &mut Field, wet: &[f32], sea: f32) {
    height.data.par_iter_mut().enumerate().for_each(|(i, ht)| {
        let a = wet[i];
        if a <= -6.0 {
            return;
        }
        // 2 units a cell through the shore, the bed falling to 10 below the sea.
        let target = if a > 0.0 { sea - (2.0 * a).min(10.0) } else { sea + (-a) * 2.0 + 0.5 };
        let k = smoothstep(-6.0, -1.0, a);
        *ht = lerp(*ht, ht.min(target), k);
    });
}

/// Each value's rank among all of them, 0 to 1.
fn ranks(f: &Field) -> Field {
    let mut order: Vec<u32> = (0..f.data.len() as u32).collect();
    order.par_sort_unstable_by(|&a, &b| f.data[a as usize].total_cmp(&f.data[b as usize]));
    let mut out = vec![0.0; f.data.len()];
    let k = 1.0 / (f.data.len() - 1).max(1) as f32;
    for (r, &i) in order.iter().enumerate() {
        out[i as usize] = r as f32 * k;
    }
    Field { w: f.w, h: f.h, data: out }
}

fn quantile(f: &Field, q: f32) -> f32 {
    let mut v = f.data.clone();
    let k = ((v.len() - 1) as f32 * q) as usize;
    v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    v[k]
}

/// A river's course across the map from west to east, meandering: points a cell apart.
fn river(w: usize, h: usize, seed: u32) -> Vec<(f32, f32)> {
    let (wf, hf) = (w as f32, h as f32);
    let k = 7;
    let mut ctrl: Vec<(f32, f32)> = (0..=k)
        .map(|i| {
            let t = i as f32 / k as f32;
            let x = lerp(-0.04, 1.04, t) * wf;
            let z = lerp(0.30, 0.68, t) * hf + perlin(t * 3.0, 0.5, seed ^ 0xCC) * 0.16 * hf;
            (x, z)
        })
        .collect();
    ctrl.insert(0, (ctrl[0].0 - wf * 0.1, ctrl[0].1));
    let last = *ctrl.last().unwrap();
    ctrl.push((last.0 + wf * 0.1, last.1));
    meander(&catmull_rom(&ctrl), seed, 22.0, 150.0, false)
}

/// A river through the middle, the same turned half round: from beyond the map's edge to the
/// middle along the line between the halves (dividing them), then on as its own twin. Its
/// meanders die away towards the middle, so the two halves meet there in one smooth course.
fn river_symmetric(w: usize, h: usize, seed: u32, sym: &Sym) -> Vec<(f32, f32)> {
    let (wf, hf) = (w as f32, h as f32);
    let (cx, cz) = ((wf - 1.0) * 0.5, (hf - 1.0) * 0.5);
    let (ax, az) = (-sym.nz, sym.nx);
    let reach = (wf * wf + hf * hf).sqrt() * 0.62;
    let k = 6;
    let pts: Vec<(f32, f32)> = (0..=k)
        .map(|i| {
            let t = i as f32 / k as f32;
            let along = (1.0 - t) * reach;
            let off = perlin(t * 3.1 + 0.4, 0.5, seed ^ 0xCC) * 0.2 * wf.min(hf) * (1.0 - t * t);
            (cx - ax * along + sym.nx * off, cz - az * along + sym.nz * off)
        })
        .collect();
    // (Steered in from beyond the first point, and on through the middle by the twin of the
    // point before it, so the course runs straight through.)
    let mut ctrl = vec![(pts[0].0 - ax * 0.1 * wf, pts[0].1 - az * 0.1 * hf)];
    ctrl.extend(&pts);
    ctrl.push(sym.twin(pts[k - 1].0, pts[k - 1].1));
    let side = wf.min(hf);
    let mut first = meander(&catmull_rom(&ctrl), seed, (side * 0.055).clamp(14.0, 40.0), side / 7.0, true);
    first.push((cx, cz));
    let mut all = first.clone();
    all.extend(first.iter().rev().skip(1).map(|&(x, z)| sym.twin(x, z)));
    all
}

/// Catmull-Rom through control points (the first and last only steering), densely.
pub(super) fn catmull_rom(ctrl: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut dense = Vec::new();
    for i in 1..ctrl.len() - 2 {
        let (p0, p1, p2, p3) = (ctrl[i - 1], ctrl[i], ctrl[i + 1], ctrl[i + 2]);
        for s in 0..200 {
            let t = s as f32 / 200.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3);
            dense.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    dense
}

/// Meanders across a course (`amp` cells, a bend every `bend` x pi cells; dying away over its
/// last 80 cells when `taper`), then resampled a cell apart.
pub(super) fn meander(dense: &[(f32, f32)], seed: u32, amp: f32, bend: f32, taper: bool) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut walked = 0.0f32;
    let length: f32 = dense.windows(2).map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1)).sum();
    for i in 0..dense.len() {
        let (a, b) = (dense[i.saturating_sub(1)], dense[(i + 1).min(dense.len() - 1)]);
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let l = (dx * dx + dz * dz).sqrt().max(1e-3);
        if i > 0 {
            let p = dense[i - 1];
            walked += ((dense[i].0 - p.0).powi(2) + (dense[i].1 - p.1).powi(2)).sqrt();
        }
        let mut off = amp * (walked / bend + 1.3).sin() + 12.0 * perlin(walked / 70.0, 2.5, seed ^ 0xDD);
        if taper {
            off *= smoothstep(0.0, 80.0, length - walked);
        }
        let p = (dense[i].0 - dz / l * off, dense[i].1 + dx / l * off);
        if out.last().is_none_or(|q: &(f32, f32)| (q.0 - p.0).hypot(q.1 - p.1) >= 1.0) {
            out.push(p);
        }
    }
    out
}

/// The ground a base can stand on: open (not steep, not deep), reachable, level, clear of the
/// water by at least the first of `shores` that leaves enough, `margin` cells in from the edges;
/// with `room`, most of the ground round it gentle and dry too (a base on a butte's top or among
/// broken ground has nowhere to build, and its AI starves).
fn start_candidates(height: &Field, sea: f32, water_dist: &Field, margin: (usize, usize), shores: &[f32], room: bool) -> Vec<(usize, usize, f32)> {
    start_candidates_on(height, sea, water_dist, margin, shores, room, None)
}

/// [`start_candidates`], none of them on (or reached across) `void`.
#[allow(clippy::too_many_arguments)]
fn start_candidates_on(height: &Field, sea: f32, water_dist: &Field, margin: (usize, usize), shores: &[f32], room: bool, void: Option<&[bool]>) -> Vec<(usize, usize, f32)> {
    let (w, h) = (height.w, height.h);
    let slope = height.ta_slope();
    let roomy = room.then(|| Field::from_fn(w, h, |x, z| if slope.at(x, z) <= 5.0 && height.at(x, z) > sea + 4.0 { 1.0 } else { 0.0 }).blurred(7));
    // The land a tank can cross, and the largest piece of it.
    let open: Vec<bool> = (0..w * h).map(|i| slope.data[i] <= 12.0 && height.data[i] > sea - 8.0 && !void.is_some_and(|v| v[i])).collect();
    // (On land maps the largest piece; among islands, any big enough for a base.)
    let main = regions_of(w, h, &open, w * h / 70);
    let calm = slope.blurred(6);
    let (margin_x, margin_z) = margin;
    let mut candidates: Vec<(usize, usize, f32)> = Vec::new();
    for &shore in shores {
        for z in (margin_z..h - margin_z).step_by(4) {
            for x in (margin_x..w - margin_x).step_by(4) {
                let i = z * w + x;
                if main[i] && calm.data[i] < 3.0 && height.data[i] > sea + 6.0 && water_dist.data[i] > shore && roomy.as_ref().is_none_or(|r| r.data[i] > 0.8) {
                    candidates.push((x, z, calm.data[i]));
                }
            }
        }
        if candidates.len() > 200 {
            break;
        }
    }
    if room && candidates.len() < 20 {
        // (Ground this broken has no room anywhere: the level ground will have to do.)
        return start_candidates_on(height, sea, water_dist, margin, shores, false, void);
    }
    assert!(!candidates.is_empty(), "no level ground for the starts");
    candidates
}

/// Start positions: level, reachable ground, as far apart as they can be; the first four on a
/// ring around the middle (the menu's battle uses those), the rest spread over the map.
fn place_starts(height: &Field, sea: f32, water_dist: &Field, players: usize, seed: u32) -> Vec<(f32, f32)> {
    let (w, h) = (height.w, height.h);
    let candidates = start_candidates(height, sea, water_dist, (w / 9, h / 9), &[40.0, 20.0, 8.0], false);
    let mut starts: Vec<(f32, f32)> = Vec::new();
    let (cx, cz) = (w as f32 / 2.0, h as f32 / 2.0);
    // Four round the middle, a quadrant each.
    for q in 0..4 {
        let a = std::f32::consts::FRAC_PI_4 + q as f32 * std::f32::consts::FRAC_PI_2 + (unit(q, 1, seed) - 0.5) * 0.4;
        let want = (cx + a.cos() * w as f32 * 0.21, cz + a.sin() * h as f32 * 0.21);
        let best = candidates
            .iter()
            .min_by(|p, q| {
                let dp = (p.0 as f32 - want.0).hypot(p.1 as f32 - want.1) + p.2 * 8.0;
                let dq = (q.0 as f32 - want.0).hypot(q.1 as f32 - want.1) + q.2 * 8.0;
                dp.total_cmp(&dq)
            })
            .unwrap();
        starts.push((best.0 as f32, best.1 as f32));
    }
    // The rest as far as can be from those taken.
    while starts.len() < players {
        let best = candidates
            .iter()
            .max_by(|p, q| {
                let far = |c: &&(usize, usize, f32)| starts.iter().map(|s| (c.0 as f32 - s.0).hypot(c.1 as f32 - s.1)).fold(f32::MAX, f32::min) - c.2 * 10.0;
                far(p).total_cmp(&far(q))
            })
            .unwrap();
        starts.push((best.0 as f32, best.1 as f32));
    }
    starts
}

/// Start positions on a symmetric map, in pairs: each in the first half and its twin, so the
/// first two face each other across the middle, and the next two, and so on. The first pair as
/// far apart as the ground allows (or `reach` of the way out from the middle), the rest as far
/// from those taken as can be.
fn place_starts_symmetric(height: &Field, sea: f32, water_dist: &Field, players: usize, reach: f32, sym: &Sym) -> Vec<(f32, f32)> {
    place_starts_symmetric_on(height, sea, water_dist, players, reach, sym, None)
}

/// [`place_starts_symmetric`], off the `void`.
fn place_starts_symmetric_on(height: &Field, sea: f32, water_dist: &Field, players: usize, reach: f32, sym: &Sym, void: Option<&[bool]>) -> Vec<(f32, f32)> {
    let (w, h) = (height.w, height.h);
    let margin = (w / 10, h / 10);
    let candidates: Vec<(usize, usize, f32)> = start_candidates_on(height, sea, water_dist, margin, &[40.0, 24.0, 12.0, 6.0], true, void)
        .into_iter()
        .filter(|c| sym.side(c.0 as f32, c.1 as f32) > 20.0)
        .collect();
    assert!(!candidates.is_empty(), "no level ground for the starts in the first half");
    let (cx, cz) = ((w as f32 - 1.0) * 0.5, (h as f32 - 1.0) * 0.5);
    let half_diag = (cx * cx + cz * cz).sqrt();
    let mut starts: Vec<(f32, f32)> = Vec::new();
    while starts.len() < players.max(2) {
        let score = |c: &(usize, usize, f32)| -> f32 {
            let (x, z) = (c.0 as f32, c.1 as f32);
            let calm = c.2 * 10.0;
            if starts.is_empty() {
                let r = (x - cx).hypot(z - cz);
                if reach > 0.0 { -(r - reach * half_diag).abs() - calm } else { r - calm }
            } else {
                let (tx, tz) = sym.twin(x, z);
                let near = starts.iter().map(|s| (x - s.0).hypot(z - s.1).min((tx - s.0).hypot(tz - s.1))).fold(f32::MAX, f32::min);
                near.min((x - tx).hypot(z - tz)) - calm
            }
        };
        let best = candidates.iter().max_by(|a, b| score(a).total_cmp(&score(b)).then((b.1, b.0).cmp(&(a.1, a.0)))).unwrap();
        let (x, z) = (best.0 as f32, best.1 as f32);
        starts.push((x, z));
        starts.push(sym.twin(x, z));
    }
    starts.truncate(players);
    starts
}

/// The 4-connected regions of `open` cells of at least `least` cells, or the largest one when
/// only it is.
fn regions_of(w: usize, h: usize, open: &[bool], least: usize) -> Vec<bool> {
    let (label, sizes) = label_regions(w, h, open);
    let big: Vec<bool> = sizes.iter().map(|&s| s >= least).collect();
    if big.iter().filter(|&&b| b).count() >= 2 {
        label.iter().map(|&l| l != u32::MAX && big[l as usize]).collect()
    } else {
        largest_region(w, h, open)
    }
}

/// The largest 4-connected region of `open` cells.
pub fn largest_region(w: usize, h: usize, open: &[bool]) -> Vec<bool> {
    let (label, sizes) = label_regions(w, h, open);
    let best = sizes.iter().enumerate().max_by_key(|(_, s)| **s).map_or(u32::MAX, |(i, _)| i as u32);
    label.iter().map(|&l| l == best).collect()
}

/// Each open cell's region, and the regions' sizes.
pub fn label_regions(w: usize, h: usize, open: &[bool]) -> (Vec<u32>, Vec<usize>) {
    let mut label = vec![u32::MAX; w * h];
    let mut sizes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !open[start] || label[start] != u32::MAX {
            continue;
        }
        let id = sizes.len() as u32;
        let mut size = 0usize;
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, z) = (i % w, i / w);
            let mut visit = |j: usize| {
                if open[j] && label[j] == u32::MAX {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if z > 0 {
                visit(i - w);
            }
            if z + 1 < h {
                visit(i + w);
            }
        }
        sizes.push(size);
    }
    (label, sizes)
}

/// Levels the ground round (x, z) to its mean within `inner` cells, blending back out to `outer`.
pub fn level(height: &mut Field, x: f32, z: f32, inner: f32, outer: f32) {
    let (w, h) = (height.w, height.h);
    let (mut sum, mut count) = (0.0, 0.0);
    let r = inner as i64;
    for dz in -r..=r {
        for dx in -r..=r {
            if ((dx * dx + dz * dz) as f32) <= inner * inner {
                sum += height.get(x as i64 + dx, z as i64 + dz);
                count += 1.0;
            }
        }
    }
    let mean = sum / count;
    let r = outer as i64;
    for dz in -r..=r {
        for dx in -r..=r {
            let (cx, cz) = (x as i64 + dx, z as i64 + dz);
            if cx < 0 || cz < 0 || cx as usize >= w || cz as usize >= h {
                continue;
            }
            let d = ((dx * dx + dz * dz) as f32).sqrt();
            let k = smoothstep(outer, inner, d);
            let v = height.at(cx as usize, cz as usize);
            height.set(cx as usize, cz as usize, lerp(v, mean, k));
        }
    }
}

/// The ground as a tank's way is judged on: as given, or (`whole`) as the map will store it.
pub fn as_stored(height: &Field, whole: bool) -> Field {
    if whole { height.map(|v| v.round().clamp(0.0, 255.0)) } else { height.clone() }
}

/// What a tank needs of the ground (TA's TANKSH3, kept a little under its limits so kbots pass
/// too): a 3 x 3 footprint, slopes of at most this, water at most this deep.
pub const DRIVE_SLOPE: f32 = 10.0;
pub const DRIVE_DEPTH: f32 = 10.0;
const DRIVE_FOOT: usize = 3;
/// How steep a ramp cut through a cliff rises along its way (units a cell).
const RAMP_RISE: f32 = 6.0;

/// Where a tank can stand (its footprint's top-left cell): every cell of its footprint level
/// enough and dry or shallow enough, none of them `blocked`.
pub fn drivable(height: &Field, sea: f32, blocked: Option<&[bool]>) -> Vec<bool> {
    let (w, h) = (height.w, height.h);
    let slope = height.ta_slope();
    let open: Vec<bool> = (0..w * h).map(|i| slope.data[i] <= DRIVE_SLOPE && sea - height.data[i] <= DRIVE_DEPTH && !blocked.is_some_and(|b| b[i])).collect();
    (0..w * h)
        .map(|i| {
            let (x, z) = (i % w, i / w);
            x + DRIVE_FOOT <= w && z + DRIVE_FOOT <= h && (0..DRIVE_FOOT).all(|dz| (0..DRIVE_FOOT).all(|dx| open[(z + dz) * w + x + dx]))
        })
        .collect()
}

/// The footprint position a start's tank stands at (its centre on the start).
fn foot_at(s: (f32, f32), w: usize, h: usize) -> usize {
    let x = (s.0 as usize).saturating_sub(1).min(w - DRIVE_FOOT);
    let z = (s.1 as usize).saturating_sub(1).min(h - DRIVE_FOOT);
    z * w + x
}

/// Which starts a tank can drive to from the first (all of them, on a land map that plays).
pub fn reachable(height: &Field, sea: f32, starts: &[(f32, f32)], blocked: Option<&[bool]>) -> Vec<bool> {
    let (w, h) = (height.w, height.h);
    let open = drivable(height, sea, blocked);
    let (label, _) = label_regions(w, h, &open);
    let home = label[foot_at(starts[0], w, h)];
    starts.iter().map(|&s| home != u32::MAX && label[foot_at(s, w, h)] == home).collect()
}

/// The way a tank drives from the first start to each of the others (over the ground alone):
/// the cells its footprint covers on the way, and a cell round them. Kept clear of features, it
/// leaves every base reachable whatever stands elsewhere.
pub fn corridor(height: &Field, sea: f32, starts: &[(f32, f32)], void: Option<&[bool]>) -> Vec<bool> {
    let (w, h) = (height.w, height.h);
    let open = drivable(height, sea, void);
    let cost: Vec<f32> = open.iter().map(|&o| if o { 1.0 } else { f32::INFINITY }).collect();
    let mut out = vec![false; w * h];
    let from = foot_at(starts[0], w, h);
    for &s in &starts[1..] {
        let to = foot_at(s, w, h);
        let Some(path) = cheapest_to_by(&cost, w, h, from, |i| i == to, false) else { continue };
        for &i in &path {
            let (x, z) = (i % w, i / w);
            for dz in 0..DRIVE_FOOT + 2 {
                for dx in 0..DRIVE_FOOT + 2 {
                    let (xx, zz) = ((x + dx).saturating_sub(1), (z + dz).saturating_sub(1));
                    if xx < w && zz < h {
                        out[zz * w + xx] = true;
                    }
                }
            }
        }
    }
    out
}

/// Makes sure a tank can drive from every start to the first: where a start's ground is walled
/// off, the cheapest way through (cliffs dear, deep water out of the question) is cut into a
/// ramp rising [`RAMP_RISE`] a cell, a few cells wide, blended into the ground beside it (and its
/// twin cut too, on a symmetric map). Returns how many ramps were cut.
pub fn connect(height: &mut Field, sea: f32, starts: &[(f32, f32)], sym: &Sym, causeway: Option<f32>, whole: bool) -> usize {
    let (w, h) = (height.w, height.h);
    let mut cut = 0;
    for _ in 0..8 {
        let judged = as_stored(height, whole);
        let reach = reachable(&judged, sea, starts, None);
        let Some(k) = reach.iter().position(|&r| !r) else { break };
        let open = drivable(&judged, sea, None);
        let (label, _) = label_regions(w, h, &open);
        let home = label[foot_at(starts[0], w, h)];
        // The cheapest way from the start walled off to the first start's ground.
        let slope = height.ta_slope();
        let cost: Vec<f32> = (0..w * h)
            .map(|i| {
                let deep = sea - height.data[i] - DRIVE_DEPTH;
                if deep > 0.0 {
                    // (Across water only with a causeway, and the shorter the better.)
                    if causeway.is_some() { 14.0 + deep * 0.3 } else { f32::INFINITY }
                } else if open[i] {
                    1.0
                } else {
                    4.0 + slope.data[i]
                }
            })
            .collect();
        let from = foot_at(starts[k], w, h);
        let Some(path) = cheapest_to(&cost, w, h, from, |i| label[i] == home && home != u32::MAX) else { break };
        let mut pts: Vec<(f32, f32)> = path.iter().map(|&i| ((i % w) as f32 + 1.5, (i / w) as f32 + 1.5)).collect();
        // (On past the foot of the cliff, straight on, as far as the drop needs to be spread.)
        let drop = pts.iter().map(|&(x, z)| height.sample(x - 0.5, z - 0.5)).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        let need = ((drop.1 - drop.0) / RAMP_RISE).ceil() as usize + 4;
        if pts.len() >= 2 {
            let (a, b) = (pts[pts.len().saturating_sub(6)], pts[pts.len() - 1]);
            let l = (b.0 - a.0).hypot(b.1 - a.1).max(1e-3);
            let (dx, dz) = ((b.0 - a.0) / l, (b.1 - a.1) / l);
            for k in 1..=need {
                let (x, z) = (b.0 + dx * k as f32, b.1 + dz * k as f32);
                if x < 1.0 || z < 1.0 || x > w as f32 - 2.0 || z > h as f32 - 2.0 {
                    break;
                }
                pts.push((x, z));
            }
        }
        ramp(height, &pts);
        if let Some(rise) = causeway {
            raise(height, &pts, sea + rise);
        }
        if sym.on {
            let twin: Vec<(f32, f32)> = pts
                .iter()
                .map(|&(x, z)| {
                    let (tx, tz) = sym.twin(x - 0.5, z - 0.5);
                    (tx + 0.5, tz + 0.5)
                })
                .collect();
            ramp(height, &twin);
            if let Some(rise) = causeway {
                raise(height, &twin, sea + rise);
            }
        }
        cut += 1;
    }
    cut
}

/// How deep a harbour's berth is dredged (a shipyard stands only where all of it, 8 x 8, is 30
/// deep) and how far round its middle it is that deep (cells); how deep and wide the channel on
/// from it to the open sea is (submarine drafts reach 25, the biggest ship is 6 across); over how many
/// cells their sides rise to the sea bed round them.
const BERTH_DEPTH: f32 = 34.0;
const BERTH_HALF: f32 = 8.0;
/// How far either way of its middle a berth runs from north to south (see `berth_reach`).
const BERTH_RUN: f32 = 8.0;
const CHANNEL_DEPTH: f32 = 30.0;
const CHANNEL_HALF: f32 = 8.0;
const HARBOUR_SIDE: f32 = 6.0;
/// How far from the shore the berth's middle lies (cells; it climbs out at most BERTH_STEPS cells
/// to it), and the longest channel dredged.
const BERTH_OUT: f32 = 6.0;
const BERTH_STEPS: usize = 12;
const MAX_CHANNEL: usize = 80;
/// A harbour's dock: the shore within this many cells of where the way meets the sea, as low as
/// this over it at the most (the beach, not the land behind), dug out with the berth.
const DOCK_REACH: f32 = 10.0;
const DOCK_RISE: f32 = 10.0;

/// How deep a harbour may be dredged `d` cells from the land: steeply, 30 deep two cells out (a
/// kbot on the beach builds a yard there).
fn shore(d: f32) -> f32 {
    6.0 + 14.0 * d
}

/// Harbours, on a map for fleets: deep water by every base within `reach` cells' walk of the sea.
/// At the shore nearest each base (as the walk to it and the way out to sea go) a berth is
/// dredged deep enough for a shipyard, its bed rising steeply to the shore so that a builder on
/// the beach can put a yard up in it, and from it a channel out to the open sea (the largest
/// stretch of it every ship can sail) deep and wide enough for every ship. (The sea shelves gently,
/// and was 30 deep only some 35 cells out: a yard 80 to 140 cells from its base, and the computer
/// players, who look within 56, built none.) Only water is dredged, and never the ways the tanks
/// drive between the bases (a sand bar, the shallows between two keys). On a symmetric map each
/// base's twin gets the twin of its harbour. Returns how many were dredged.
pub fn harbours(height: &mut Field, sea: f32, starts: &[(f32, f32)], sym: &Sym, reach: f32) -> usize {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let (w, h) = (height.w, height.h);
    let n = w * h;
    let depth: Vec<f32> = height.data.iter().map(|&v| sea - v).collect();
    // The open sea: the stretches of water 18 deep and more big enough for fleets, a quarter of
    // the biggest at the least (a real place's oceans are several; not a pocket by an island).
    let deep: Vec<bool> = depth.iter().map(|&d| d >= 18.0).collect();
    let (label, sizes) = label_regions(w, h, &deep);
    let biggest = sizes.iter().copied().max().unwrap_or(0);
    if biggest < 2000 {
        return 0;
    }
    let open: Vec<bool> = label.iter().map(|&l| l != u32::MAX && sizes[l as usize] * 4 >= biggest).collect();
    // What is kept as it is: the ways the tanks drive between the bases, with room either side
    // (both halves', on a symmetric map).
    let way = corridor(height, sea, starts, None);
    let mut near_way = distance(w, h, &way);
    if sym.on {
        for i in 0..n {
            let j = sym.twin_index(i);
            near_way.data[i] = near_way.data[i].min(near_way.data[j]);
        }
    }
    // (Nothing dredged within KEEP of them, the dredging easing in over KEEP_SOFT more: a hard
    // edge drew a straight cut through the shallows; no channel's line within KEEP_LINE.)
    const KEEP: f32 = 10.0;
    const KEEP_SOFT: f32 = 8.0;
    const KEEP_LINE: f32 = 15.0;
    let calm: Vec<f32> = near_way.data.iter().map(|&d| smoothstep(KEEP, KEEP + KEEP_SOFT, d)).collect();
    let keep: Vec<bool> = near_way.data.iter().map(|&d| d < KEEP_LINE).collect();
    let to_land = distance(w, h, &depth.iter().map(|&d| d <= 0.0).collect::<Vec<_>>());
    // First each base's way out to sea, and its dock: the low shore round where the way meets the
    // sea, dug out with the berth (a base 50 cells in from the beach had its yard just beyond the
    // computer players' search).
    let mut ways: Vec<Vec<usize>> = Vec::new();
    let mut dock = vec![false; n];
    for &(sx, sz) in starts {
        if sym.on && !sym.first(sx, sz) {
            continue;
        }
        // The cheapest way from the base to the open sea: over the land to the shore (no further
        // than `reach`), then over the water alone (never back onto the land), a cell of land
        // costing four of water: the walk to the yard short above all, the channel dredged short too.
        // Nodes: a cell on the land (0..n) or on the water (n..2n); costs in fifteenths of a cell.
        let mut best = vec![u32::MAX; 2 * n];
        let mut prev = vec![u32::MAX; 2 * n];
        let mut heap = BinaryHeap::new();
        let s0 = (sz as usize).min(h - 1) * w + (sx as usize).min(w - 1);
        best[s0] = 0;
        heap.push(Reverse((0u32, s0 as u32)));
        let mut goal = None;
        while let Some(Reverse((c, node))) = heap.pop() {
            let node = node as usize;
            if c > best[node] {
                continue;
            }
            let (wet, i) = (node >= n, node % n);
            if wet && open[i] && depth[i] >= 22.0 {
                goal = Some(node);
                break;
            }
            let (x, z) = ((i % w) as i64, (i / w) as i64);
            for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                    continue;
                }
                let j = nz as usize * w + nx as usize;
                let diagonal = dx != 0 && dz != 0;
                let next = if depth[j] > 0.0 {
                    if keep[j] {
                        continue;
                    }
                    // (Off the shore: a channel along it is cut in half by it.)
                    let hug = (6.0 - to_land.data[j]).max(0.0) as u32 * 4;
                    (n + j, if diagonal { 21 } else { 15 } + hug)
                } else {
                    // (Back onto the land from the water, or too far over it, no.)
                    if wet || c as f32 > reach * 60.0 {
                        continue;
                    }
                    (j, if diagonal { 85 } else { 60 })
                };
                let nc = c + next.1;
                if nc < best[next.0] {
                    best[next.0] = nc;
                    prev[next.0] = node as u32;
                    heap.push(Reverse((nc, next.0 as u32)));
                }
            }
        }
        let Some(goal) = goal else {
            eprintln!("  no harbour for the base at ({sx}, {sz}): the sea too far");
            continue;
        };
        // The channel: the way's cells on the water, from the shore out.
        let mut channel = Vec::new();
        let mut at = goal;
        while at != s0 && at != u32::MAX as usize {
            if at >= n {
                channel.push(at - n);
            }
            at = prev[at] as usize;
        }
        channel.reverse();
        // (Not across a sea of shallows: a trench dredged half across a gulf.)
        if channel.len() > MAX_CHANNEL {
            eprintln!("  no harbour for the base at ({sx}, {sz}): the deep sea {} cells out", channel.len());
            continue;
        }
        if let (Some(&a), Some(&b)) = (channel.first(), channel.last()) {
            eprintln!("  harbour for the base at ({sx}, {sz}): from the shore at ({}, {}), {} cells out to ({}, {})", a % w, a / w, channel.len(), b % w, b / w);
        }
        let (hx, hz) = ((channel[0] % w) as i64, (channel[0] / w) as i64);
        let r = DOCK_REACH.ceil() as i64;
        for z in (hz - r).max(0)..=(hz + r).min(h as i64 - 1) {
            for x in (hx - r).max(0)..=(hx + r).min(w as i64 - 1) {
                let i = z as usize * w + x as usize;
                if depth[i] <= 0.0 && depth[i] >= -DOCK_RISE && calm[i] >= 1.0 && ((x - hx) as f32).hypot((z - hz) as f32) <= DOCK_REACH {
                    dock[i] = true;
                    if sym.on {
                        dock[sym.twin_index(i)] = true;
                    }
                }
            }
        }
        ways.push(channel);
    }
    // The water once the docks are dug, and how far each place in it is from the land then.
    let wet: Vec<bool> = (0..n).map(|i| depth[i] > 0.0 || dock[i]).collect();
    let to_land = distance(w, h, &wet.iter().map(|&v| !v).collect::<Vec<_>>());
    let mut target = vec![0.0f32; n];
    let mut dredged = 0;
    // (Each harbour's berth and the end of its channel, for the ways on: see `ways_on`.)
    let mut ends: Vec<(usize, usize)> = Vec::new();
    for channel in ways {
        // The berth: a basin deep enough for a yard, straight out from where the way meets the
        // shore (a way out to sea often runs along the coast a while), its middle a little way
        // out; then the channel on from the shore, for the ships.
        let mut berth = channel[0];
        for _ in 0..BERTH_STEPS {
            if to_land.data[berth] >= BERTH_OUT {
                break;
            }
            let (x, z) = ((berth % w) as i64, (berth / w) as i64);
            let next = [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)]
                .iter()
                .map(|&(dx, dz)| (x + dx, z + dz))
                .filter(|&(nx, nz)| nx >= 0 && nz >= 0 && nx < w as i64 && nz < h as i64)
                .map(|(nx, nz)| nz as usize * w + nx as usize)
                .filter(|&j| wet[j] && !keep[j])
                .max_by(|&a, &b| to_land.data[a].total_cmp(&to_land.data[b]).then(b.cmp(&a)));
            match next {
                Some(j) if to_land.data[j] > to_land.data[berth] => berth = j,
                _ => break,
            }
        }
        // Whether a yard stands in a berth dredged round `c` as the game has it: an 8 x 8 30 deep
        // and more all over (31 here), 3 cells off the map's edge, and before its front a
        // battleship's way out (a factory's units leave by its south side: 8 across and 9 down,
        // 15 deep; 16 here), the bed as it is or as the berth would leave it.
        let after = |c: usize, x: i64, z: i64| {
            let i = z as usize * w + x as usize;
            let t = (BERTH_DEPTH * smoothstep(BERTH_HALF + HARBOUR_SIDE, BERTH_HALF, berth_reach(c, i, w)) * calm[i]).min(shore(to_land.data[i]));
            if wet[i] { depth[i].max(t) } else { depth[i] }
        };
        let yard_in = |c: usize| {
            let (cx, cz) = ((c % w) as i64, (c / w) as i64);
            let run = (BERTH_RUN + BERTH_HALF) as i64;
            (cz - run - 8..=cz + run).any(|oz| {
                (cx - 12..=cx + 4).any(|ox| {
                    ox >= 3 && oz >= 3 && ox + 11 <= w as i64 && oz + 17 <= h as i64 && oz + 11 <= h as i64 && (oz..oz + 8).all(|z| (ox..ox + 8).all(|x| after(c, x, z) >= 31.0)) && (oz + 8..oz + 17).all(|z| (ox..ox + 8).all(|x| after(c, x, z) >= 16.0))
                })
            })
        };
        // (Where it stands its twin does, turned half round: there too.)
        let both = |c: usize| yard_in(c) && (!sym.on || yard_in(sym.twin_index(c)));
        // (In a narrow bay or sound, or where the coast leaves no room before a yard's front, out
        // along the channel to the first place one stands.)
        let found = both(berth);
        if !found {
            berth = channel.iter().copied().find(|&c| both(c)).unwrap_or(berth);
        }
        eprintln!("  berth at ({}, {}){}", berth % w, berth / w, if found || both(berth) { "" } else { ": NO YARD STANDS IN IT" });
        ends.push((berth, *channel.last().unwrap()));
        let mut cut: Vec<(usize, f32, f32)> = vec![(berth, BERTH_DEPTH, BERTH_HALF)];
        cut.extend(channel.iter().map(|&c| (c, CHANNEL_DEPTH, CHANNEL_HALF)));
        for (k, &(c, full, half)) in cut.iter().enumerate() {
            let r = (half + HARBOUR_SIDE).ceil() as i64 + if k == 0 { BERTH_RUN as i64 } else { 0 };
            let (px, pz) = ((c % w) as i64, (c / w) as i64);
            for z in (pz - r).max(0)..=(pz + r).min(h as i64 - 1) {
                for x in (px - r).max(0)..=(px + r).min(w as i64 - 1) {
                    let i = z as usize * w + x as usize;
                    if !wet[i] || calm[i] <= 0.0 {
                        continue;
                    }
                    // (The berth long from north to south, the channel round its line.)
                    let d = if k == 0 { berth_reach(c, i, w) } else { ((x - px) as f32).hypot((z - pz) as f32) };
                    // (Full depth along it, its sides rising to the bed round it, and to the
                    // shore steeply: 30 deep two cells out, a builder on the beach in reach of a yard.)
                    let t = (full * smoothstep(half + HARBOUR_SIDE, half, d) * calm[i]).min(shore(to_land.data[i]));
                    target[i] = target[i].max(t);
                }
            }
        }
        dredged += 1 + sym.on as usize;
    }
    for i in 0..n {
        let t = if sym.on { target[i].max(target[sym.twin_index(i)]) } else { target[i] };
        if t > 0.0 && wet[i] {
            height.data[i] = height.data[i].min(sea - t);
        }
    }
    let open_sea: Vec<u32> = (0..n).map(|i| if open[i] { label[i] } else { u32::MAX }).collect();
    let on = ways_on(height, sea, &ends, sym, &wet, &keep, &calm, &open_sea);
    if on > 0 {
        eprintln!("  {on} harbour channel(s) run on to where submarines sail");
    }
    dredged
}

/// The submarines' footprints and drafts: CORSSUB's 3 x 3 at 25 (the deepest), and ARMACSUB's,
/// CORACSUB's and CORSUB's 4 x 4 at 20 (the biggest), as `MoveSpec`s judge them.
const SUBMARINES: [(usize, f32); 2] = [(3, 25.0), (4, 20.0)];
/// Water over the deepest draft by this much and more round a way out is left as it is.
const SUB_SPARE: f32 = 2.0;

/// Where a submarine of `side` x `side` cells drawing `draft` floats: the cells its footprint
/// from them (east and south) is that deep all over, as the game rounds the heights
/// (`World::cell_depth`).
fn sub_water(height: &Field, sea: f32, side: usize, draft: f32) -> Vec<bool> {
    let (w, h) = (height.w, height.h);
    let deep: Vec<bool> = height.data.iter().map(|&v| sea.round() - v.round().clamp(0.0, 255.0) >= draft).collect();
    let mut rows = vec![false; w * h];
    for z in 0..h {
        for x in 0..w.saturating_sub(side - 1) {
            rows[z * w + x] = (0..side).all(|k| deep[z * w + x + k]);
        }
    }
    let mut out = vec![false; w * h];
    for z in 0..h.saturating_sub(side - 1) {
        for x in 0..w {
            out[z * w + x] = (0..side).all(|k| rows[(z + k) * w + x]);
        }
    }
    out
}

/// Runs a harbour's channel on to the heart of the open sea it ends in (the largest stretch of
/// that sea each submarine sails, see [`sub_water`] and [`SUBMARINES`]) where its water does not
/// reach there. The open sea is judged 18 deep and more, and the channel ends where it is first
/// 22: enough for every ship but the submarines, whose drafts reach 25 (on Twin Reefs, whose sea
/// bed falls gently, the channels ended on a sill along the map's edge and the submarines never
/// left their harbours). From the harbour's water (the stretch of it the deepest-drawing
/// submarine sails round the berth, and its channel's end) the cheapest way over the water to
/// the heart, the shallower the dearer, never across the ways the tanks drive (`keep`), is
/// dredged as the channel is where the water round it is under a submarine; and its twin. A
/// harbour whose submarines already reach the heart is left as it is. `ends`: each harbour's
/// berth and its channel's last cell; `wet`: the water with the docks dug; `open`: the open
/// seas' labels (u32::MAX: none). Returns how many ways were laid.
#[allow(clippy::too_many_arguments)]
fn ways_on(height: &mut Field, sea: f32, ends: &[(usize, usize)], sym: &Sym, wet: &[bool], keep: &[bool], calm: &[f32], open: &[u32]) -> usize {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BinaryHeap};
    let (w, h) = (height.w, height.h);
    let n = w * h;
    // For each submarine, its water's stretches and the heart of each open sea: the stretch with
    // the most of that sea in it (ties to the first found).
    let subs: Vec<(Vec<u32>, BTreeMap<u32, u32>)> = SUBMARINES
        .iter()
        .map(|&(side, draft)| {
            let (region, _) = label_regions(w, h, &sub_water(height, sea, side, draft));
            let mut share: BTreeMap<(u32, u32), usize> = BTreeMap::new();
            for i in 0..n {
                if region[i] != u32::MAX && open[i] != u32::MAX {
                    *share.entry((open[i], region[i])).or_default() += 1;
                }
            }
            let mut best: BTreeMap<u32, (usize, u32)> = BTreeMap::new();
            for (&(sea_id, r), &count) in &share {
                let b = best.entry(sea_id).or_insert((0, u32::MAX));
                if count > b.0 {
                    *b = (count, r);
                }
            }
            (region, best.into_iter().map(|(k, (_, r))| (k, r)).collect())
        })
        .collect();
    let depth: Vec<f32> = height.data.iter().map(|&v| sea - v).collect();
    // (The shallowest water within 2 cells, on the map: under a submarine round there.)
    let shallowest = |c: usize| {
        let (cx, cz) = ((c % w) as i64, (c / w) as i64);
        let mut least = f32::MAX;
        for z in (cz - 2).max(0)..=(cz + 2).min(h as i64 - 1) {
            for x in (cx - 2).max(0)..=(cx + 2).min(w as i64 - 1) {
                least = least.min(depth[z as usize * w + x as usize]);
            }
        }
        least
    };
    let to_land = distance(w, h, &wet.iter().map(|&v| !v).collect::<Vec<_>>());
    let mut target = vec![0.0f32; n];
    let mut laid = 0;
    for &(berth, end) in ends {
        let Some(hearts) = subs.iter().map(|(_, best)| best.get(&open[end]).copied()).collect::<Option<Vec<u32>>>() else {
            eprintln!("  no way on from the harbour at ({}, {}): its channel ends in no open sea a submarine sails", berth % w, berth / w);
            continue;
        };
        let in_hearts = |c: usize| subs.iter().zip(&hearts).all(|((region, _), &heart)| region[c] == heart);
        if in_hearts(berth) {
            continue;
        }
        // (The harbour's water: where the deepest-drawing submarine sails round the berth.)
        let home = subs[0].0[berth];
        // (From all of the harbour's water at once: the sill is crossed where that is cheapest.)
        let mut cost = vec![u32::MAX; n];
        let mut prev = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        let mut sources: Vec<usize> = if home != u32::MAX { (0..n).filter(|&i| subs[0].0[i] == home).collect() } else { Vec::new() };
        sources.push(end);
        for i in sources {
            if cost[i] != 0 {
                cost[i] = 0;
                heap.push(Reverse((0u32, i as u32)));
            }
        }
        let mut goal = None;
        while let Some(Reverse((c, at))) = heap.pop() {
            let at = at as usize;
            if c > cost[at] {
                continue;
            }
            if in_hearts(at) {
                goal = Some(at);
                break;
            }
            let (x, z) = ((at % w) as i64, (at / w) as i64);
            for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                    continue;
                }
                let j = nz as usize * w + nx as usize;
                if depth[j] <= 0.0 || keep[j] {
                    continue;
                }
                // (In tenths of a cell; a cell under a submarine's draft the dearer, the shallower.)
                let step = if dx != 0 && dz != 0 { 14 } else { 10 };
                let nc = c + step * (1 + ((SUBMARINES[0].1 + SUB_SPARE + 1.0 - depth[j]).max(0.0) * 0.5) as u32);
                if nc < cost[j] {
                    cost[j] = nc;
                    prev[j] = at as u32;
                    heap.push(Reverse((nc, j as u32)));
                }
            }
        }
        let Some(goal) = goal else {
            eprintln!("  no way on from the harbour at ({}, {}): no way over the water to the open sea", berth % w, berth / w);
            continue;
        };
        let mut way = Vec::new();
        let mut at = goal;
        while cost[at] != 0 {
            way.push(at);
            at = prev[at] as usize;
        }
        way.reverse();
        // (Not a trench across a sea of shallows: as long a sill as a channel at the most.)
        let sill = way.iter().filter(|&&i| depth[i] < SUBMARINES[0].1).count();
        if sill > MAX_CHANNEL {
            eprintln!("  no way on from the harbour at ({}, {}): a sill {sill} cells long", berth % w, berth / w);
            continue;
        }
        if let (Some(&a), Some(&b)) = (way.first(), way.last()) {
            eprintln!("  the harbour at ({}, {}) runs on from ({}, {}) {} cells to ({}, {}) over a sill {sill} cells long", berth % w, berth / w, a % w, a / w, way.len(), b % w, b / w);
        }
        // (Only where the way crosses water a submarine does not sail already: the sills. The
        // deep water it runs through on the way is left as it is.)
        let r = (CHANNEL_HALF + HARBOUR_SIDE).ceil() as i64;
        for &c in way.iter().filter(|&&c| shallowest(c) < SUBMARINES[0].1 + SUB_SPARE) {
            let (px, pz) = ((c % w) as i64, (c / w) as i64);
            for z in (pz - r).max(0)..=(pz + r).min(h as i64 - 1) {
                for x in (px - r).max(0)..=(px + r).min(w as i64 - 1) {
                    let i = z as usize * w + x as usize;
                    if !wet[i] || calm[i] <= 0.0 {
                        continue;
                    }
                    let d = ((x - px) as f32).hypot((z - pz) as f32);
                    let t = (CHANNEL_DEPTH * smoothstep(CHANNEL_HALF + HARBOUR_SIDE, CHANNEL_HALF, d) * calm[i]).min(shore(to_land.data[i]));
                    target[i] = target[i].max(t);
                }
            }
        }
        laid += 1 + sym.on as usize;
    }
    for i in 0..n {
        let t = if sym.on { target[i].max(target[sym.twin_index(i)]) } else { target[i] };
        if t > 0.0 && wet[i] {
            height.data[i] = height.data[i].min(sea - t);
        }
    }
    laid
}

/// How far cell `i` lies from a berth round `c` (cells): from the line BERTH_RUN either way of it
/// from north to south (a yard and its ships' way out of its front fit in it, whichever way the
/// coast runs, and in its twin turned half round), its edge wandering a little (a straight side
/// drew a ruled line through the shallows).
fn berth_reach(c: usize, i: usize, w: usize) -> f32 {
    let (cx, cz) = ((c % w) as f32, (c / w) as f32);
    let (x, z) = ((i % w) as f32, (i / w) as f32);
    (x - cx).hypot(((z - cz).abs() - BERTH_RUN).max(0.0)) + 3.5 * perlin(x / 8.0, z / 8.0, 0xB3A7)
}

/// The cheapest way over cells (8 neighbours, `cost` per cell entered) from `from` to the first
/// cell `goal` takes, as cells.
fn cheapest_to(cost: &[f32], w: usize, h: usize, from: usize, goal: impl Fn(usize) -> bool) -> Option<Vec<usize>> {
    cheapest_to_by(cost, w, h, from, goal, true)
}

/// [`cheapest_to`], by the 4 neighbours only when not `diagonal` (as [`reachable`] connects
/// the ground: a squeeze only a diagonal step gets through is no way).
fn cheapest_to_by(cost: &[f32], w: usize, h: usize, from: usize, goal: impl Fn(usize) -> bool, diagonal: bool) -> Option<Vec<usize>> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;
    #[derive(PartialEq)]
    struct Node(f32, usize);
    impl Eq for Node {}
    impl Ord for Node {
        fn cmp(&self, o: &Self) -> Ordering {
            o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
        }
    }
    impl PartialOrd for Node {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    let mut best = vec![f32::INFINITY; w * h];
    let mut prev = vec![usize::MAX; w * h];
    let mut heap = BinaryHeap::new();
    best[from] = 0.0;
    heap.push(Node(0.0, from));
    while let Some(Node(c, at)) = heap.pop() {
        if c > best[at] {
            continue;
        }
        if goal(at) {
            let mut path = vec![at];
            let mut i = at;
            while i != from {
                i = prev[i];
                path.push(i);
            }
            path.reverse();
            return Some(path);
        }
        let (x, z) = ((at % w) as i64, (at / w) as i64);
        let steps: &[(i64, i64)] = if diagonal { &[(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] } else { &[(-1, 0), (1, 0), (0, -1), (0, 1)] };
        for &(dx, dz) in steps {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                continue;
            }
            let n = nz as usize * w + nx as usize;
            let step = if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            let nc = c + step * cost[n];
            if nc < best[n] {
                best[n] = nc;
                prev[n] = at;
                heap.push(Node(nc, n));
            }
        }
    }
    None
}

/// Raises a causeway along a way (cells): the ground under `top` within 3 cells of it lifted to
/// it, blended out over 3 more.
fn raise(height: &mut Field, pts: &[(f32, f32)], top: f32) {
    let (w, h) = (height.w, height.h);
    for &(px, pz) in pts {
        let (x0, x1) = ((px - 7.0).floor().max(0.0) as usize, ((px + 7.0).ceil() as usize).min(w - 1));
        let (z0, z1) = ((pz - 7.0).floor().max(0.0) as usize, ((pz + 7.0).ceil() as usize).min(h - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - px).hypot(z as f32 + 0.5 - pz);
                let k = smoothstep(6.0, 3.0, d);
                let v = height.at(x, z);
                if k > 0.0 && v < top {
                    height.set(x, z, lerp(v, top, k));
                }
            }
        }
    }
}

/// Cuts a ramp along a way (cells, a cell apart): the heights along it eased so they rise at most
/// [`RAMP_RISE`] a cell (cutting and filling about the middle of each step), laid over a band 3
/// cells either side and blended out over 3 more.
fn ramp(height: &mut Field, pts: &[(f32, f32)]) {
    if pts.len() < 2 {
        return;
    }
    let along: Vec<f32> = pts.iter().map(|&(x, z)| height.sample(x - 0.5, z - 0.5)).collect();
    // The highest profile no steeper than the rise that never rises above the ground, and the
    // lowest that never sinks below it; between them, the way.
    let mut lo = along.clone();
    let mut hi = along.clone();
    for i in 1..lo.len() {
        lo[i] = lo[i].max(lo[i - 1] - RAMP_RISE);
        hi[i] = hi[i].min(hi[i - 1] + RAMP_RISE);
    }
    for i in (0..lo.len() - 1).rev() {
        lo[i] = lo[i].max(lo[i + 1] - RAMP_RISE);
        hi[i] = hi[i].min(hi[i + 1] + RAMP_RISE);
    }
    let mut p: Vec<f32> = lo.iter().zip(&hi).map(|(a, b)| (a + b) * 0.5).collect();
    for _ in 0..3 {
        let q = p.clone();
        for i in 1..p.len() - 1 {
            p[i] = (q[i - 1] + q[i] * 2.0 + q[i + 1]) * 0.25;
        }
    }
    let (w, h) = (height.w, height.h);
    let mut weight = vec![0.0f32; w * h];
    let mut target = vec![0.0f32; w * h];
    for (k, &(px, pz)) in pts.iter().enumerate() {
        let (x0, x1) = ((px - 7.0).floor().max(0.0) as usize, ((px + 7.0).ceil() as usize).min(w - 1));
        let (z0, z1) = ((pz - 7.0).floor().max(0.0) as usize, ((pz + 7.0).ceil() as usize).min(h - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - px).hypot(z as f32 + 0.5 - pz);
                let wgt = smoothstep(6.0, 3.0, d);
                let i = z * w + x;
                if wgt > weight[i] {
                    weight[i] = wgt;
                    target[i] = p[k];
                }
            }
        }
    }
    for i in 0..w * h {
        if weight[i] > 0.0 {
            height.data[i] = lerp(height.data[i], target[i], weight[i]);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetry_turns_cells_half_round_the_middle() {
        let sym = Sym::halves(40, 30, 1.0);
        for i in [0usize, 7, 41, 599, 1199] {
            let j = sym.twin_index(i);
            assert_eq!(sym.twin_index(j), i);
            let (x, z) = ((i % 40) as f32, (i / 40) as f32);
            assert_eq!(sym.twin(x, z), ((j % 40) as f32, (j / 40) as f32));
            // Noise read through it is the same at a cell and its twin, and each cell is in
            // one half or the other.
            let f = |x: f32, z: f32| (x * 0.37).sin() + (z * 0.21).cos();
            let (tx, tz) = sym.twin(x, z);
            assert!((sym.noise(x, z, f) - sym.noise(tx, tz, f)).abs() < 1e-5);
            assert!((sym.eval(x, z, f) - sym.eval(tx, tz, f)).abs() < 1e-5);
            assert!(sym.first(x, z) != sym.first(tx, tz) || sym.side(x, z) == 0.0);
        }
    }

    #[test]
    fn a_walled_off_start_gets_a_ramp_cut_to_it() {
        // A plain at 40, and a plateau at 120 walled by a cliff, a start on each.
        let (w, h) = (120, 80);
        let mut height = Field::from_fn(w, h, |x, z| if (x as f32 - 90.0).hypot(z as f32 - 40.0) < 22.0 { 120.0 } else { 40.0 });
        let starts = [(25.0, 40.0), (90.0, 40.0)];
        assert_eq!(reachable(&height, 10.0, &starts, None), vec![true, false]);
        let cut = connect(&mut height, 10.0, &starts, &Sym::none(w, h), None, false);
        assert!(cut >= 1);
        assert_eq!(reachable(&height, 10.0, &starts, None), vec![true, true]);
        // The plateau's top stays where it was.
        assert!((height.at(90, 40) - 120.0).abs() < 1.0, "{}", height.at(90, 40));
    }

    #[test]
    fn a_start_across_a_sea_of_fire_gets_a_causeway() {
        // Two plains at 40 parted by a band of melt 20 deep (the sea at 30).
        let (w, h) = (120, 60);
        let mut height = Field::from_fn(w, h, |x, _| if (50..70).contains(&x) { 10.0 } else { 40.0 });
        let starts = [(20.0, 30.0), (100.0, 30.0)];
        assert_eq!(reachable(&height, 30.0, &starts, None), vec![true, false]);
        // Without a causeway the melt cannot be crossed; with one it is bridged above it.
        assert_eq!(connect(&mut height.clone(), 30.0, &starts, &Sym::none(w, h), None, false), 0);
        assert!(connect(&mut height, 30.0, &starts, &Sym::none(w, h), Some(4.0), false) >= 1);
        assert_eq!(reachable(&height, 30.0, &starts, None), vec![true, true]);
        assert!((0..h).any(|z| height.at(60, z) > 30.0), "no bridge over the melt");
    }
}
