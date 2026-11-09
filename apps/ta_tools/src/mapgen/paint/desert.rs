//! Desert: sand in regions paler and redder, each its own colour through (the map's tiles share
//! well between places alike, and a gradient across many tiles would be cut into squares by
//! them), rippled across the wind; dunes with sharp crests, their slip faces in shade; hard flats
//! between them strewn with gravel; desert pavement on the plateaus, varnished stones on pale
//! grit, cracked; mesas and buttes banded in strata, a hard caprock on their rims, varnish
//! streaking their faces, talus at their feet; dry river beds of pale silt braided by the last
//! flood, scrub along their banks; salt pans cracked in polygons in the lowest basins; oases: a
//! ring of green and reeds round the water.
//!
//! What lies where follows the ground (x, z); the fine texture within (ripples, stones, scrub,
//! grain) is laid in the art's own rows and repeats every `noise::PERIOD` pixels, so a stretch
//! of one ground is made of a few tiles that meet seamlessly.

use super::super::noise::{PERIOD, fbm, mix3, perlin, smoothstep, tfbm, tperlin, tvalue, tvalue2, tworley, twave, value};
use super::super::terrain::dune;
use super::{Ground, scale};

/// A desert world's colours (sRGB, 0 to 1) and its winds.
#[derive(Clone)]
pub struct Desert {
    pub sand: [f32; 3],
    pub sand_pale: [f32; 3],
    pub sand_red: [f32; 3],
    pub flat: [f32; 3],
    pub gravel: [f32; 3],
    /// A mesa's strata, top to bottom repeating, and its caprock.
    pub strata: [[f32; 3]; 4],
    pub cap: [f32; 3],
    pub scrub: [f32; 3],
    pub silt: [f32; 3],
    pub salt: [f32; 3],
    pub green: [f32; 3],
    pub reeds: [f32; 3],
    pub shallow: [f32; 3],
    pub deep: [f32; 3],
    pub track: [f32; 3],
    /// The dunes' spacing (cells) and the direction the wind blows from (radians), as the land
    /// was shaped with ([`super::super::terrain::Shape::dunes`]).
    pub dune_gap: f32,
    pub wind: f32,
    /// How much of the lowest flat ground is salt pan, how much scrub grows (0 to 1).
    pub salt_share: f32,
    pub scrub_share: f32,
    /// How thick the strata are (height units) and where the slopes turn to rock.
    pub band: f32,
    pub rock_from: f32,
    pub rock_full: f32,
    /// A ring of green and reeds round the water, as round an oasis (the desert worlds' pools);
    /// none on a real place, whose deserts run down to the sea as they are.
    pub oases: bool,
}

/// The ripples' spacing (pixels).
const RIPPLE: f32 = 6.2;

impl Ground<'_> {
    pub fn desert(&self, s: &Desert, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        let y = row;
        let g1 = tvalue(x, y, 1.7, seed ^ 1) - 0.5;
        let g2 = tvalue(x, y, 4.3, seed ^ 2) - 0.5;
        let gp3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let depth = self.sea - h;
        let water_dist = self.water_dist.sample(cx, cz);
        if depth > 0.0 {
            let wave = tvalue2(x, y, 9.0, 3.2, seed ^ 5) - 0.5;
            let mut c = mix3(s.shallow, s.deep, smoothstep(0.0, 9.0, depth));
            c = mix3(s.silt, c, smoothstep(0.0, 2.0, depth));
            return scale(c, 1.0 + 0.09 * wave + 0.04 * g2).map(|v| v.clamp(0.0, 1.0));
        }
        let slope = self.slope.sample(cx, cz);
        let wide = self.slope_wide.sample(cx, cz);
        let cav = self.cavity.sample(cx, cz);
        let moist = self.moisture.sample(cx, cz);
        let dunes = self.land.dune_mask.sample(cx, cz);
        let bed = self.land.bed.sample(cx, cz);
        // Regions of paler and redder sand, their edges short and wandering.
        let wx = x + 160.0 * fbm(x / 700.0, z / 700.0, seed ^ 61, 2);
        let wz = z + 160.0 * fbm(x / 700.0 - 2.3, z / 700.0 + 4.1, seed ^ 62, 2);
        let region = fbm(wx / 1600.0, wz / 1600.0, seed ^ 63, 3) * 0.5 + 0.5;
        let patch = fbm(wx / 240.0, wz / 240.0, seed ^ 64, 4) * 0.5 + 0.5;
        let wobble = (patch - 0.5) * 0.08;
        let mut sand = mix3(s.sand, s.sand_red, smoothstep(0.6, 0.62, region + wobble));
        sand = mix3(sand, s.sand_pale, smoothstep(0.38, 0.36, region + wobble) * 0.8 + smoothstep(0.71, 0.73, patch) * 0.35);
        // Ripples across the wind, a few pixels apart, bent; gone on the slip faces. (Their
        // crests run whole numbers of times across the repeat, so they repeat with it.)
        let k = PERIOD / RIPPLE;
        let (ra, rb) = ((k * s.wind.cos()).round(), (k * s.wind.sin()).round());
        let bend = tfbm(x, y, 110.0, seed ^ 65, 3) * 2.6 + tfbm(x, y, 26.0, seed ^ 66, 2) * 0.45;
        let ripple = ((twave(x, y, ra, rb) + bend) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let (wc, ws) = (s.wind.cos(), s.wind.sin());
        let (gx, gz) = self.gradient(x, z);
        // Facing down the wind: the lee, where the slip faces fall.
        let lee = (gx * wc + gz * ws).max(0.0);
        let windward = (-(gx * wc + gz * ws)).max(0.0);
        let slip = smoothstep(0.12, 0.16, lee);
        let rip = (1.0 - slip) * (0.6 + 0.4 * smoothstep(0.0, 0.15, windward));
        let mut c = scale(sand, 1.0 + 0.07 * (ripple - 0.5) * 2.0 * rip + 0.05 * g1 + 0.03 * g2);
        // The dunes: the windward slopes in the sun, the slip faces in shade, the crests sharp
        // and bright between.
        if dunes > 0.0 {
            let d = self.land.sym.eval(x / 16.0, z / 16.0, |x, z| dune(x, z, s.dune_gap, s.wind, seed));
            let crest = smoothstep(0.8, 0.86, d) * smoothstep(0.04, 0.08, lee + windward);
            c = scale(c, 1.0 + (0.1 * crest - 0.13 * slip) * dunes);
            c = mix3(c, s.sand_pale, crest * dunes * 0.35);
        }
        // Between the dunes and off them: hard flats strewn with gravel.
        let flat = smoothstep(0.35, 0.05, dunes) * smoothstep(3.0, 1.0, slope);
        let stony = smoothstep(0.56, 0.6, patch + (1.0 - dunes) * 0.15 + gp3 * 0.05) * flat;
        if stony > 0.0 {
            let stone = tworley(x, y, 3.4, 3.4, seed ^ 67, 0.95);
            let tone = ((stone.id >> 9) & 255) as f32 / 255.0;
            let mut f = scale(s.flat, 0.92 + 0.12 * g1 + 0.08 * g2);
            let pebble = scale(mix3(s.gravel, s.flat, tone * 0.5), 0.8 + 0.3 * smoothstep(0.4, 0.0, stone.f1 + 0.3 * stone.dy));
            f = mix3(f, pebble, smoothstep(0.4, 0.26, stone.f1) * (0.4 + 0.5 * stony));
            c = mix3(c, f, stony);
        }
        // Desert pavement on the plateaus: varnished stones packed on pale grit, a few cracks,
        // sand drifted into the hollows.
        let plateau = smoothstep(0.3, 0.0, dunes) * smoothstep(self.sea + 34.0, self.sea + 70.0, h) * smoothstep(5.0, 2.0, slope);
        if plateau > 0.0 {
            let stone = tworley(x, y, 2.8, 2.8, seed ^ 68, 1.0);
            let tile = tworley(x, y, 46.0, 36.0, seed ^ 69, 0.8);
            let crack = smoothstep(0.045, 0.012, tile.f2 - tile.f1) * smoothstep(0.62, 0.66, value(x / 70.0, z / 70.0, seed ^ 90));
            let tone = ((stone.id >> 7) & 255) as f32 / 255.0;
            let stones = smoothstep(0.36, 0.22, stone.f1) * (0.35 + 0.5 * smoothstep(0.55, 0.6, patch));
            // Patches of darker, varnished gravel, a few tiles across.
            let varnish = smoothstep(0.58, 0.62, fbm(wx / 160.0, wz / 160.0, seed ^ 91, 4) * 0.5 + 0.5);
            let mut p = scale(mix3(s.flat, s.sand, 0.3), 0.95 + 0.1 * g1 + 0.06 * g2);
            p = mix3(p, scale(mix3(s.gravel, s.strata[0], 0.3), 1.15), varnish * 0.45);
            p = mix3(p, scale(mix3(s.gravel, s.flat, tone * 0.6), 0.85 + 0.3 * smoothstep(0.3, 0.0, stone.f1 + 0.3 * stone.dy)), stones);
            p = scale(p, 1.0 - 0.28 * crack);
            let drift = smoothstep(1.8, 2.4, cav) * 0.8;
            p = mix3(p, sand, drift);
            c = mix3(c, p, plateau);
        }
        // Scrub: low grey-green bushes, thicker where the ground holds water.
        let scrub = s.scrub_share * (0.25 + moist * 0.9 + smoothstep(14.0, 3.0, bed) * 0.8) * smoothstep(0.5, 0.1, dunes);
        if scrub > 0.0 {
            let bush = tworley(x, y, 7.0, 7.0, seed ^ 70, 0.9);
            let keep = ((bush.id >> 3) & 1023) as f32 / 1023.0 < scrub * 0.6;
            if keep && bush.f1 < 0.3 {
                let b = scale(s.scrub, 0.8 + 0.4 * smoothstep(0.3, 0.0, bush.f1 + bush.dy * 0.5));
                c = mix3(c, b, 0.85);
            } else if keep && bush.f1 < 0.42 && bush.dy < 0.0 {
                // Its shadow, to the south-east.
                c = scale(c, 0.82);
            }
        }
        // A dry river's bed: pale silt braided by the last flood, its banks sharp.
        if bed < 9.0 {
            let half = 4.6 + 1.2 * fbm(x / 120.0, z / 120.0, seed ^ 72, 2);
            let k = smoothstep(half + 0.8, half - 0.8, bed);
            if k > 0.0 {
                let braid = 1.0 - tperlin(x, y, 46.0, seed ^ 73).abs();
                let mut b = scale(mix3(s.silt, sand, 0.35), 0.96 + 0.08 * g1 + 0.06 * g2);
                b = mix3(b, scale(s.silt, 0.86), smoothstep(0.78, 0.9, braid) * 0.5);
                let stone = tworley(x, y, 3.2, 3.2, seed ^ 74, 0.9);
                b = mix3(b, scale(mix3(s.gravel, s.flat, 0.5), 1.05), smoothstep(0.26, 0.16, stone.f1) * smoothstep(0.72, 0.86, braid) * 0.45);
                // Mud cracked where the last pools dried.
                let mud = tworley(x, y, 9.0, 9.0, seed ^ 75, 0.9);
                b = scale(b, 1.0 - 0.18 * smoothstep(0.05, 0.01, mud.f2 - mud.f1) * smoothstep(0.8, 0.95, braid));
                c = mix3(c, b, k);
            }
        }
        // Salt pans in the lowest, flattest ground.
        if s.salt_share > 0.0 {
            let low = smoothstep(self.sea + 24.0, self.sea + 9.0, h) * smoothstep(1.6, 0.6, wide) * smoothstep(0.2, 0.0, dunes);
            let pan = smoothstep(0.52, 0.55, fbm(wx / 300.0, wz / 300.0, seed ^ 76, 3) * 0.5 + 0.5 + low * 0.3) * low * s.salt_share;
            if pan > 0.0 {
                let poly = tworley(x, y, 26.0, 26.0, seed ^ 77, 0.85);
                let ridge = smoothstep(0.08, 0.02, poly.f2 - poly.f1);
                let mut p = scale(s.salt, 0.94 + 0.06 * g1 + 0.05 * g2 - 0.04 * poly.f1);
                p = mix3(p, scale(s.salt, 1.06), ridge * 0.7);
                p = mix3(p, s.flat, smoothstep(0.65, 0.9, tvalue(x, y, 30.0, seed ^ 78)) * 0.25);
                c = mix3(c, p, pan.clamp(0.0, 1.0));
            }
        }
        // Mesas and buttes: talus at their feet; strata banded across their faces, varnish
        // streaking down them, the caprock dark along their rims.
        let foot = smoothstep(5.0, 9.0, wide) * smoothstep(10.0, 4.0, slope);
        if foot > 0.0 {
            let stone = tworley(x, y, 4.4, 4.4, seed ^ 79, 0.9);
            let tone = ((stone.id >> 5) & 255) as f32 / 255.0;
            let rubble = scale(mix3(s.strata[(stone.id % 4) as usize], s.flat, 0.45), 0.85 + 0.35 * smoothstep(0.4, 0.0, stone.f1 + 0.3 * stone.dy) + 0.08 * tone);
            let grit = scale(mix3(s.flat, sand, 0.5), 0.97 + 0.08 * g1);
            let t = mix3(grit, rubble, smoothstep(0.34, 0.22, stone.f1));
            c = mix3(c, t, (foot * 0.9).clamp(0.0, 0.7));
        }
        // (The rock is kept whole by the tiles, so it is painted in the ground's own place.)
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 2.5 * gp3);
        if rocky > 0.0 {
            let lift = h + 3.0 * perlin(x / 47.0, z / 47.0, seed ^ 80) + 1.5 * perlin(x / 13.0, z / 13.0, seed ^ 81);
            let b = lift / s.band;
            let k = b.floor();
            let f = b - k;
            let layer = s.strata[(k.rem_euclid(4.0)) as usize];
            let next = s.strata[((k + 1.0).rem_euclid(4.0)) as usize];
            let mut r = mix3(layer, next, smoothstep(0.85, 1.0, f));
            // The joints between beds, and finer bedding within them.
            r = scale(r, 1.0 - 0.25 * smoothstep(0.06, 0.0, f.min(1.0 - f)) + 0.05 * (b * 5.0 * std::f32::consts::TAU).sin());
            // Varnish streaking down the faces.
            let streak = value(x / 2.5, lift / 30.0, seed ^ 82) * 0.6 + value(x / 9.0, lift / 60.0, seed ^ 83) * 0.4;
            r = scale(r, 1.0 - 0.22 * smoothstep(0.55, 0.85, streak));
            // The caprock: the top of the cliff, convex.
            let rim = smoothstep(-0.3, -2.0, cav) * smoothstep(0.4, 0.8, rocky);
            r = mix3(r, s.cap, rim * 0.6);
            r = scale(r, 0.9 + 0.14 * (g1 + 0.5) + 0.06 * g2);
            c = mix3(c, r, rocky);
        }
        // Oases: a ring of green and reeds round the water, damp sand beyond.
        let ring = if s.oases { smoothstep(7.5, 6.0, water_dist + 1.5 * fbm(x / 50.0, z / 50.0, seed ^ 92, 2)) } else { 0.0 };
        if ring > 0.0 {
            let lushness = smoothstep(0.55, 0.6, fbm(x / 40.0, z / 40.0, seed ^ 84, 3) * 0.5 + 0.5 + smoothstep(6.0, 2.0, water_dist) * 0.4);
            let grass = scale(s.green, 0.85 + 0.25 * (g1 + 0.5) + 0.1 * g2);
            let damp = scale(c, 0.8);
            c = mix3(c, damp, ring * 0.5);
            c = mix3(c, grass, ring * lushness);
            let reeds = smoothstep(2.6, 1.0, water_dist) * smoothstep(0.5, 0.7, tvalue(x, y, 6.0, seed ^ 85));
            c = mix3(c, scale(s.reeds, 0.75 + 0.45 * tvalue(x, y, 1.3, seed ^ 86)), reeds * 0.8);
            c = mix3(c, s.silt, smoothstep(1.0, 0.0, water_dist) * 0.6);
        }
        // Tracks: sand packed pale by wheels, two ruts.
        let t = self.track(x, z);
        if t < 1.8 {
            let half = 0.5 + 0.08 * fbm(x / 90.0, z / 90.0, seed ^ 87, 2);
            let core = smoothstep(half + 0.12, half - 0.12, t) * (1.0 - smoothstep(9.0, 12.0, slope));
            let rut = smoothstep(0.09, 0.03, (t - 0.25).abs());
            let mut tc = scale(s.track, 0.92 + 0.12 * (g2 + 0.5) + 0.08 * g1);
            tc = scale(tc, 1.0 - 0.18 * rut);
            let spill = smoothstep(half + 0.7, half, t) * 0.35;
            c = mix3(c, mix3(c, s.track, 0.5), spill * (1.0 - core));
            c = mix3(c, tc, core);
        }
        self.shade_bands(c, x, z)
    }
}
