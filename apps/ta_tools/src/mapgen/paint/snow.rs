//! Winter (TA Reborn's seasonal maps): snow over everything, white in the sun and blue in its
//! hollows, in regions of fresh powder and of crust the wind has packed, rippled across the wind
//! and glinting; rock breaking through the snow on the steep slopes, snow on its ledges; the
//! forest floor in the firs' shade, needles and twigs fallen on it; frozen lakes and rivers of
//! ice, dark and clear where the wind has swept them, cracked, snow drifted across them and
//! heaped at their shores; roads of packed snow, two ruts of slush and grit and a ridge between;
//! the villages' squares cobbled, snow in the joints; the ground round a vent bare and wet.
//!
//! What lies where follows the ground (x, z); the fine texture within (ripples, glints, cobbles,
//! cracks) is laid in the art's own rows and repeats every `noise::PERIOD` pixels, so a stretch
//! of one ground is made of a few tiles that meet seamlessly, as the other painters' is.

use super::super::noise::{fbm, mix3, perlin, smoothstep, tfbm, tperlin, tunit, tvalue, tvalue2, tworley, twave};
use super::{Ground, scale};

/// A winter world's colours (sRGB, 0 to 1).
#[derive(Clone)]
pub struct Snow {
    pub snow: [f32; 3],
    /// Snow in the shade and in the hollows (blue), and the crust the wind packed (greyer).
    pub shade: [f32; 3],
    pub packed: [f32; 3],
    pub rock: [f32; 3],
    pub rock_dark: [f32; 3],
    pub ice: [f32; 3],
    pub ice_dark: [f32; 3],
    /// The roads: packed snow, and the slush and grit in their ruts.
    pub road: [f32; 3],
    pub slush: [f32; 3],
    /// The squares' cobbles, the bare ground round a vent, the needles fallen under the firs.
    pub cobble: [f32; 3],
    pub soil: [f32; 3],
    pub needles: [f32; 3],
    /// The wind's way (radians; its ripples lie across it, its streaks over the ice along it), and
    /// how much of the snow it has packed (0 to 1).
    pub wind: f32,
    pub crust: f32,
    /// Where the slopes turn to rock (height units a cell where it starts and is full).
    pub rock_from: f32,
    pub rock_full: f32,
}

impl Ground<'_> {
    pub fn snow(&self, s: &Snow, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        let y = row;
        let g1 = tvalue(x, y, 1.7, seed ^ 1) - 0.5;
        let g2 = tvalue(x, y, 4.3, seed ^ 2) - 0.5;
        let g3 = tperlin(x, y, 19.0, seed ^ 3);
        let gp3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let slope = self.slope.sample(cx, cz);
        let cav = self.cavity.sample(cx, cz);
        let forest = self.forest.sample(cx, cz);
        let wx = x + 140.0 * fbm(x / 640.0, z / 640.0, seed ^ 31, 2);
        let wz = z + 140.0 * fbm(x / 640.0 + 3.1, z / 640.0 - 1.7, seed ^ 32, 2);
        let region = fbm(wx / 1400.0, wz / 1400.0, seed ^ 33, 3) * 0.5 + 0.5;
        let patch = fbm(wx / 110.0, wz / 110.0, seed ^ 35, 3) * 0.5 + 0.5;
        // Powder or crust, their edges short and their colour the same (the tiles share between
        // alike places: a stretch of snow is a few tiles, its texture laid in the art's rows).
        let crust = smoothstep(0.585, 0.6, region * 0.6 + patch * 0.4 + (s.crust - 0.5) * 0.4);
        let mut c = s.snow;
        // Ripples across the wind (the crust's sharper), glints in the powder.
        let k = 3.0;
        let (ra, rb) = ((k * s.wind.cos()).round(), (k * s.wind.sin()).round());
        let bend = tfbm(x, y, 110.0, seed ^ 65, 3) * 2.2 + tfbm(x, y, 26.0, seed ^ 66, 2) * 0.4;
        let ripple = ((twave(x, y, ra * 12.0, rb * 12.0) + bend) * std::f32::consts::TAU).sin();
        c = scale(c, 1.0 + ripple * (0.012 + 0.02 * crust) + 0.018 * g1 + 0.012 * g2 + 0.01 * g3);
        if crust < 0.5 && tunit(x, y, seed ^ 67) > 0.993 {
            c = mix3(c, [1.0, 1.0, 1.0], 0.6);
        }
        // (Blue in the hollows: the shading's, in its bands, see `shade_snow`.)
        let _ = cav;
        // Rock where the slopes are too steep for the snow to lie, in strata, snow on its ledges.
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 2.5 * gp3);
        if rocky > 0.0 {
            let band = 0.5 + 0.5 * ((h + 4.0 * perlin(x / 31.0, z / 31.0, seed ^ 7)) * std::f32::consts::TAU / 7.5).sin();
            let crack = smoothstep(0.92, 0.98, 1.0 - perlin(x / 13.0, z / 7.0, seed ^ 8).abs());
            let mut r = mix3(s.rock_dark, s.rock, 0.35 + 0.55 * band);
            r = scale(r, (0.88 + 0.26 * (g1 + 0.5) * 0.5 + 0.08 * g2) * (1.0 - 0.3 * crack));
            // Snow clinging to it in patches, the more on its ledges.
            let cling = smoothstep(0.5, 0.62, tfbm(x, y, 22.0, seed ^ 50, 3) * 0.5 + 0.5 + (band - 0.5) * 0.35);
            r = mix3(r, scale(s.snow, 0.96), cling * 0.8);
            c = mix3(c, r, rocky);
        }
        // Under the firs: needles fallen on the snow (its colour the open snow's: the firs' own
        // shadows say where the forest is, and an edge of another colour round every wood would
        // cost a tile at every step of it).
        if forest > 0.0 {
            let fall = tworley(x, y, 2.6, 2.6, seed ^ 43, 1.0);
            let speck = smoothstep(0.24, 0.12, fall.f1) * ((fall.id & 7) == 0) as u32 as f32;
            c = mix3(c, s.needles, speck * 0.6 * smoothstep(0.08, 0.14, forest + 0.04 * gp3));
        }
        // The ice: a lake or river frozen.
        let ice = self.ice_in(cx, cz);
        if ice > 0.0 {
            c = mix3(c, self.snow_ice(s, x, y, z, ice, g1, g2), smoothstep(0.0, 0.6, ice));
        }
        // The ground round a vent, bare and wet.
        let melt = smoothstep(3.5, 1.2, self.hot.sample(cx, cz) + 0.6 * gp3);
        if melt > 0.0 {
            let wet = scale(s.soil, 0.85 + 0.25 * (g1 + 0.5) + 0.1 * g2);
            c = mix3(c, mix3(wet, scale(s.rock_dark, 0.9), smoothstep(0.4, 0.6, tvalue(x, y, 6.0, seed ^ 68)) * 0.4), melt);
        }
        // The roads: packed snow, two ruts of slush and grit and a ridge between them.
        let t = self.track(x, z);
        if t < 1.8 {
            let half = 0.6 + 0.08 * fbm(x / 90.0, z / 90.0, seed ^ 56, 2);
            let core = smoothstep(half + 0.1, half - 0.1, t);
            let verge = smoothstep(half + 0.8, half, t) * 0.5;
            let rut = smoothstep(0.11, 0.04, (t - 0.27).abs());
            let mut rc = scale(s.road, 0.93 + 0.1 * (g2 + 0.5) + 0.06 * g1);
            rc = mix3(rc, scale(s.slush, 0.9 + 0.2 * (g1 + 0.5)), rut * 0.8);
            let trodden = scale(mix3(c, s.packed, 0.6), 0.97);
            c = mix3(c, trodden, verge * (1.0 - core));
            c = mix3(c, rc, core * (1.0 - smoothstep(9.0, 12.0, slope)));
        }
        // A village's square: cobbles, snow in their joints and drifted over the edges.
        let square = self.squares.iter().map(|&(sx, sz, r)| r - (cx - sx).hypot(cz - sz)).fold(f32::MIN, f32::max);
        if square > -1.0 {
            let stone = tworley(x, y, 5.5, 4.2, seed ^ 69, 0.75);
            let tone = ((stone.id >> 9) & 255) as f32 / 255.0;
            let mut sq = scale(s.cobble, 0.82 + 0.3 * tone + 0.08 * g1);
            sq = scale(sq, 1.0 + 0.12 * smoothstep(0.4, 0.0, stone.f1 + 0.3 * stone.dy));
            let joint = smoothstep(0.1, 0.03, stone.f2 - stone.f1);
            sq = mix3(sq, s.snow, joint * 0.85);
            let dusted = smoothstep(0.6, 0.75, tfbm(x, y, 30.0, seed ^ 70, 2) * 0.5 + 0.5);
            sq = mix3(sq, scale(s.snow, 0.97), dusted * 0.55);
            c = mix3(c, sq, smoothstep(-1.0, 0.5, square));
        }
        self.shade_snow(s, c, x, z)
    }

    /// How far inside the ice a cell (fractional cells) lies: 0 on land, rising across a cell
    /// at its shore.
    fn ice_in(&self, cx: f32, cz: f32) -> f32 {
        match &self.ice {
            Some(f) => f.sample(cx, cz),
            None => 0.0,
        }
    }

    /// The ice: pale over most of it, dark and clear where the wind swept it, white cracks, snow
    /// streaked over it along the wind, bubbles frozen in, snow heaped at its shore (`inside`:
    /// cells in from it).
    #[allow(clippy::too_many_arguments)]
    fn snow_ice(&self, s: &Snow, x: f32, y: f32, z: f32, inside: f32, g1: f32, g2: f32) -> [f32; 3] {
        let seed = self.seed;
        let swept = smoothstep(0.58, 0.6, fbm(x / 700.0, z / 700.0, seed ^ 71, 3) * 0.5 + 0.5);
        let mut c = mix3(s.ice, s.ice_dark, swept * 0.85);
        c = scale(c, 1.0 + 0.03 * g1 + 0.02 * g2 + 0.04 * (tvalue(x, y, 40.0, seed ^ 72) - 0.5));
        let big = tworley(x, y, 48.0, 40.0, seed ^ 73, 0.9);
        let fine = tworley(x, y, 15.0, 13.0, seed ^ 74, 1.0);
        let crack = smoothstep(0.035, 0.0, big.f2 - big.f1) * 0.8 + smoothstep(0.03, 0.0, fine.f2 - fine.f1) * 0.35 * smoothstep(0.5, 0.65, tvalue(x, y, 64.0, seed ^ 75));
        c = mix3(c, [0.9, 0.95, 0.98], crack.min(0.85));
        // Snow streaked along the wind (streaks along the art's x or y, whichever is nearer it).
        let along_x = s.wind.cos().abs() >= s.wind.sin().abs();
        let streak = if along_x { tvalue2(x, y, 64.0, 5.0, seed ^ 76) } else { tvalue2(x, y, 5.0, 64.0, seed ^ 76) };
        let drift = smoothstep(0.6, 0.78, streak * 0.7 + tvalue(x, y, 32.0, seed ^ 77) * 0.3) * (1.0 - swept * 0.6);
        c = mix3(c, scale(s.snow, 0.96), drift * 0.75);
        if tunit(x, y, seed ^ 78) > 0.995 {
            c = mix3(c, [0.95, 0.97, 1.0], 0.5);
        }
        // Heaped at the shore.
        mix3(c, scale(s.snow, 0.98 + 0.03 * g1), smoothstep(2.2, 0.6, inside + 0.5 * tperlin(x, y, 18.0, seed ^ 79)))
    }

    /// Snow's shading: as [`Ground::shade_bands`], lit, level and in shade, the shade blue (the
    /// sky's light on snow).
    fn shade_snow(&self, s: &Snow, c: [f32; 3], x: f32, z: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let (gx, gz) = self.gradient(x, z);
        let toward = -(gx * -0.55 + gz * -0.56);
        let lit = smoothstep(0.06, 0.1, toward) * 0.04 - smoothstep(-0.06, -0.1, toward) * 0.07 - smoothstep(-0.22, -0.28, toward) * 0.05;
        let hollow = smoothstep(3.0, 4.0, self.cavity.sample(cx, cz)) * 0.08;
        let k = 1.0 + lit - hollow;
        let shade = (1.0 - k).max(0.0);
        let cool = [s.shade[0] / s.snow[0], s.shade[1] / s.snow[1], s.shade[2] / s.snow[2]];
        [0, 1, 2].map(|i| (c[i] * k * (1.0 + (cool[i] - 1.0) * (shade * 2.2).min(1.0))).clamp(0.0, 1.0))
    }
}
