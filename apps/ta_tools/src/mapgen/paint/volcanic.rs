//! Lava worlds: a molten sea, brightest where it laps the shore, deep red further out (the game
//! draws its crust and cracks over it, and reads it as lava by its colour); scorched rock along
//! the shores, veined with glowing cracks that die
//! away from the melt; basalt plains jointed in polygons, ash drifted over them in pale fields,
//! red cinder round the vents and on the cones, flows of younger lava, ropy and glossy, winding
//! down the slopes; sulphur crusted yellow round the vents; cliffs banded in basalt and scoria;
//! tracks of packed ash.
//!
//! What lies where follows the ground (x, z); the fine texture within (joints, grit, ropes,
//! crust) is laid in the art's own rows and repeats every `noise::PERIOD` pixels, so a stretch
//! of one ground is made of a few tiles that meet seamlessly.

use super::super::noise::{fbm, mix3, perlin, ridged, smoothstep, tfbm, tperlin, tvalue, tvalue2, tworley, value};
use super::{Ground, scale};

/// A lava world's colours (sRGB, 0 to 1).
#[derive(Clone)]
pub struct Volcanic {
    pub basalt: [f32; 3],
    pub basalt_dark: [f32; 3],
    pub ash: [f32; 3],
    pub cinder: [f32; 3],
    pub flow: [f32; 3],
    pub scorch: [f32; 3],
    pub sulphur: [f32; 3],
    /// The melt: where it is thin and brightest, and where it is deep.
    pub glow: [f32; 3],
    pub melt: [f32; 3],
    pub track: [f32; 3],
    /// How much of the open ground is ash, cinder and young flows (0 to 1).
    pub ash_share: f32,
    pub cinder_share: f32,
    pub flow_share: f32,
    pub rock_from: f32,
    pub rock_full: f32,
}

impl Ground<'_> {
    pub fn volcanic(&self, s: &Volcanic, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        let y = row;
        let g1 = tvalue(x, y, 1.7, seed ^ 1) - 0.5;
        let g2 = tvalue(x, y, 4.3, seed ^ 2) - 0.5;
        let gp3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let depth = self.sea - h;
        let water_dist = self.water_dist.sample(cx, cz);
        if depth > 0.0 {
            // The melt: bright at the shore, deep red further out, in slow swirls.
            let swirl = tvalue2(x, y, 14.0, 6.0, seed ^ 102) - 0.5;
            let c = mix3(s.glow, s.melt, smoothstep(0.0, 9.0, depth));
            return scale(c, 1.0 + 0.08 * swirl + 0.04 * g2).map(|v| v.clamp(0.0, 1.0));
        }
        let slope = self.slope.sample(cx, cz);
        let wide = self.slope_wide.sample(cx, cz);
        let cav = self.cavity.sample(cx, cz);
        // Regions, their edges short and wandering.
        let wx = x + 150.0 * fbm(x / 650.0, z / 650.0, seed ^ 105, 2);
        let wz = z + 150.0 * fbm(x / 650.0 + 2.7, z / 650.0 - 1.9, seed ^ 106, 2);
        let fields = fbm(wx / 420.0, wz / 420.0, seed ^ 107, 4) * 0.5 + 0.5;
        let patch = fbm(wx / 110.0, wz / 110.0, seed ^ 108, 3) * 0.5 + 0.5;
        // Basalt jointed in polygons: dark seams, each plate its own shade.
        let joint = tworley(x, y, 14.0, 14.0, seed ^ 109, 0.8);
        let tone = ((joint.id >> 10) & 255) as f32 / 255.0;
        let mut c = mix3(s.basalt_dark, s.basalt, 0.4 + 0.45 * tone);
        c = scale(c, (1.0 - 0.35 * smoothstep(0.07, 0.02, joint.f2 - joint.f1)) * (0.92 + 0.12 * g1 + 0.08 * g2));
        // Ash drifted in fields and into the hollows, finer and smoother.
        let ash = smoothstep(0.6, 0.63, fields * 0.5 + patch * 0.5 + smoothstep(1.0, 3.0, cav) * 0.12) * s.ash_share;
        if ash > 0.0 {
            let grit = tvalue(x, y, 2.2, seed ^ 110);
            let dune = (tfbm(x, y, 60.0, seed ^ 111, 2) * 0.5 + 0.5) * 0.08;
            c = mix3(c, scale(s.ash, 0.9 + 0.12 * grit + dune), ash);
        }
        // Cinder: red-brown scoria round the vents and on the cones' flanks.
        let cinder = smoothstep(0.66, 0.69, patch * 0.6 + (1.0 - fields) * 0.3 + smoothstep(3.0, 8.0, slope) * 0.2) * s.cinder_share;
        if cinder > 0.0 {
            let clast = tworley(x, y, 3.0, 3.0, seed ^ 112, 0.95);
            let cc = scale(s.cinder, 0.8 + 0.4 * smoothstep(0.45, 0.0, clast.f1 + 0.3 * clast.dy) + 0.06 * g2);
            c = mix3(c, cc, cinder);
        }
        // Young flows: ropy, glossy lava winding down the slopes in long tongues.
        if s.flow_share > 0.0 {
            // (A few broad tongues, where the ridges of a coarse noise run.)
            let tongue = ridged(wx / 700.0, wz / 1100.0, seed ^ 113, 2);
            let flow = smoothstep(0.8, 0.83, tongue + (s.flow_share - 0.5) * 0.2) * smoothstep(9.0, 4.0, slope);
            if flow > 0.0 {
                let rope = (tperlin(x, y, 9.0, seed ^ 114) * 2.0 + tvalue2(x, y, 3.0, 9.0, seed ^ 115) * 0.6).sin() * 0.5 + 0.5;
                let mut fc = scale(s.flow, 0.82 + 0.3 * rope + 0.06 * g1);
                // A sheen on the ropes' tops.
                fc = mix3(fc, scale(s.basalt, 1.3), smoothstep(0.8, 0.95, rope) * 0.3);
                c = mix3(c, fc, flow);
            }
        }
        // Scree at the foot of the cliffs.
        let foot = smoothstep(5.0, 9.0, wide) * smoothstep(10.0, 4.0, slope);
        if foot > 0.0 {
            let stone = tworley(x, y, 4.2, 4.2, seed ^ 116, 0.9);
            let rubble = scale(mix3(s.basalt_dark, s.cinder, ((stone.id >> 6) & 1) as f32 * 0.6), 0.8 + 0.4 * smoothstep(0.4, 0.0, stone.f1 + 0.3 * stone.dy));
            c = mix3(c, mix3(scale(s.ash, 0.8), rubble, smoothstep(0.38, 0.25, stone.f1)), (foot * 0.9).clamp(0.0, 0.7));
        }
        // Cliffs: banded basalt and scoria, cracked. (Kept whole by the tiles.)
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 2.5 * gp3);
        if rocky > 0.0 {
            let band = 0.5 + 0.5 * ((h + 5.0 * perlin(x / 37.0, z / 37.0, seed ^ 117)) * std::f32::consts::TAU / 11.0).sin();
            let crack = smoothstep(0.92, 0.98, 1.0 - perlin(x / 11.0, z / 6.0, seed ^ 118).abs());
            let mut r = mix3(s.basalt_dark, mix3(s.basalt, s.cinder, 0.35), 0.3 + 0.6 * band);
            r = scale(r, (0.88 + 0.24 * (g1 + 0.5) * 0.5 + 0.08 * g2) * (1.0 - 0.4 * crack));
            let streak = value(x / 3.0, h / 20.0, seed ^ 119);
            r = scale(r, 1.0 - 0.15 * smoothstep(0.6, 0.85, streak));
            c = mix3(c, r, rocky);
        }
        // Scorched shores, veined with glowing cracks that die away from the melt.
        let shore = smoothstep(5.0, 0.5, water_dist + 1.2 * gp3);
        if shore > 0.0 {
            let vein = tworley(x, y, 11.0, 9.0, seed ^ 120, 0.9);
            let crack = smoothstep(0.06, 0.015, vein.f2 - vein.f1) * smoothstep(3.0, 0.5, water_dist);
            let mut sc = scale(s.scorch, 0.8 + 0.3 * tvalue(x, y, 2.5, seed ^ 121));
            sc = mix3(sc, s.glow, crack * 0.85);
            c = mix3(c, sc, shore);
        }
        // Round the vents: red cinder, and sulphur crusted yellow nearest them.
        let hot = self.hot.sample(cx, cz) + 1.5 * (patch - 0.5);
        if hot < 9.0 {
            let clast = tworley(x, y, 3.0, 3.0, seed ^ 124, 0.95);
            let cc = scale(s.cinder, 0.8 + 0.4 * smoothstep(0.45, 0.0, clast.f1 + 0.3 * clast.dy));
            c = mix3(c, cc, smoothstep(9.0, 6.5, hot) * 0.85);
            let crystals = tvalue(x, y, 1.6, seed ^ 122);
            c = mix3(c, scale(s.sulphur, 0.85 + 0.3 * crystals), smoothstep(4.0, 2.5, hot) * 0.85);
        }
        // Tracks of packed ash.
        let t = self.track(x, z);
        if t < 1.8 {
            let half = 0.55 + 0.08 * fbm(x / 90.0, z / 90.0, seed ^ 123, 2);
            let core = smoothstep(half + 0.12, half - 0.12, t) * (1.0 - smoothstep(9.0, 12.0, slope));
            let rut = smoothstep(0.09, 0.03, (t - 0.27).abs());
            let mut tc = scale(s.track, 0.92 + 0.12 * (g2 + 0.5) + 0.08 * g1);
            tc = scale(tc, 1.0 - 0.2 * rut);
            c = mix3(c, mix3(c, s.track, 0.4), smoothstep(half + 0.7, half, t) * 0.3 * (1.0 - core));
            c = mix3(c, tc, core);
        }
        self.shade_bands(c, x, z)
    }
}
