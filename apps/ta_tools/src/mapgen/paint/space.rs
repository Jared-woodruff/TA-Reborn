//! Worlds in space: rocks of grey regolith floating in the void, pocked with craters (their
//! bowls in shade, their rims bright, ejecta strewn round them), boulders and gravel, the rims
//! falling away in faces of dark rock; gantries of plating across the void between them; at each
//! base a platform of plating on the rock; and the void itself, painted as the stars (the game,
//! in its perspective view, draws nothing there and the sky shows through: this is what the
//! classic view and the minimap show).

use super::super::noise::{PERIOD, fbm, mix3, perlin, smoothstep, tunit, tvalue, tworley};
use super::metal::Metal;
use super::{Ground, scale};

/// A world in space's colours (sRGB, 0 to 1).
#[derive(Clone)]
pub struct Space {
    pub dust: [f32; 3],
    pub dust_dark: [f32; 3],
    pub dust_pale: [f32; 3],
    pub rock: [f32; 3],
    pub rock_dark: [f32; 3],
    pub ejecta: [f32; 3],
    /// The void's black and its faint nebula tint.
    pub black: [f32; 3],
    pub nebula: [f32; 3],
    /// The gantries' and platforms' plating.
    pub plating: Metal,
    /// How far round each base its platform reaches (cells, 0 none).
    pub platform: f32,
    pub rock_from: f32,
    pub rock_full: f32,
}

impl Ground<'_> {
    pub fn space(&self, s: &Space, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        let y = row;
        let (w, hh) = (self.land.w, self.land.h);
        let cell = ((cz + 0.5).max(0.0) as usize).min(hh - 1) * w + ((cx + 0.5).max(0.0) as usize).min(w - 1);
        if !self.land.void.is_empty() && self.land.void[cell] {
            return self.starfield(s, x, y, z);
        }
        // Gantries and the bases' platforms: plating.
        let base = self.land.starts.iter().map(|st| ((cx - st.0).abs()).max((cz - st.1).abs())).fold(f32::MAX, f32::min);
        let on_gantry = !self.land.gantry.is_empty() && self.land.gantry[cell];
        if on_gantry || (s.platform > 0.0 && base < s.platform) {
            return self.metal(&s.plating, x, z, h, row);
        }
        let g1 = tvalue(x, y, 1.5, seed ^ 1) - 0.5;
        let g2 = tvalue(x, y, 4.6, seed ^ 2) - 0.5;
        let gp3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let slope = self.slope.sample(cx, cz);
        let wide = self.slope_wide.sample(cx, cz);
        let cav = self.cavity.sample(cx, cz);
        // Regolith in regions darker and paler (old lava plains and the highlands' dust).
        let wx = x + 150.0 * fbm(x / 600.0, z / 600.0, seed ^ 131, 2);
        let wz = z + 150.0 * fbm(x / 600.0 + 1.7, z / 600.0 - 2.1, seed ^ 132, 2);
        let region = fbm(wx / 1300.0, wz / 1300.0, seed ^ 133, 3) * 0.5 + 0.5;
        let patch = fbm(wx / 200.0, wz / 200.0, seed ^ 134, 3) * 0.5 + 0.5;
        let mut c = mix3(s.dust, s.dust_dark, smoothstep(0.6, 0.62, region + (patch - 0.5) * 0.08));
        c = mix3(c, s.dust_pale, smoothstep(0.34, 0.32, region + (patch - 0.5) * 0.08) * 0.8);
        // Fine dust, pitted by micro-craters, gravel.
        let pit = tworley(x, y, 5.0, 5.0, seed ^ 135, 0.95);
        c = scale(c, 1.0 + 0.06 * g1 + 0.04 * g2 - 0.08 * smoothstep(0.25, 0.08, pit.f1) * ((pit.id & 3) == 0) as i32 as f32);
        let stone = tworley(x, y, 3.0, 3.0, seed ^ 136, 0.9);
        if (stone.id & 7) == 0 && stone.f1 < 0.3 {
            c = scale(mix3(c, s.rock, 0.6), 0.85 + 0.35 * smoothstep(0.3, 0.0, stone.f1 + 0.3 * stone.dy));
        }
        // Craters: their bowls in shade (deep hollows), their rims bright, ejecta round them.
        let bowl = smoothstep(2.0, 6.0, cav);
        c = scale(c, 1.0 - 0.18 * bowl);
        let rim = smoothstep(-1.5, -4.0, cav) * smoothstep(9.0, 3.0, slope);
        c = mix3(c, s.ejecta, rim * 0.5);
        // Boulders strewn on the crater rims and the broken ground.
        let rough = smoothstep(3.0, 7.0, wide) * smoothstep(12.0, 6.0, slope);
        if rough > 0.0 {
            let boulder = tworley(x, y, 7.0, 7.0, seed ^ 137, 0.9);
            if (boulder.id & 3) == 0 && boulder.f1 < 0.32 {
                let lit = smoothstep(0.32, 0.0, boulder.f1 + 0.35 * boulder.dy + 0.2 * boulder.dx);
                c = mix3(c, scale(s.rock, 0.7 + 0.6 * lit), rough);
            }
        }
        // Rock faces: the rims and the crater walls, dark and fractured. (Kept whole.)
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 2.5 * gp3);
        if rocky > 0.0 {
            let band = 0.5 + 0.5 * ((h + 6.0 * perlin(x / 23.0, z / 23.0, seed ^ 138)) * std::f32::consts::TAU / 13.0).sin();
            let crack = smoothstep(0.92, 0.98, 1.0 - perlin(x / 9.0, z / 6.0, seed ^ 139).abs());
            let mut r = mix3(s.rock_dark, s.rock, 0.25 + 0.6 * band);
            r = scale(r, (0.85 + 0.25 * (g1 + 0.5) * 0.6 + 0.08 * g2) * (1.0 - 0.45 * crack));
            c = mix3(c, r, rocky);
        }
        // The roads graded over the rock to the gantries: dust packed smooth, darker, in ruts.
        if !self.land.road.is_empty() && self.land.road[cell] {
            let rut = (tvalue(x, y, 2.2, seed ^ 144) - 0.5) * 0.06;
            c = scale(mix3(c, s.dust_dark, 0.6), 0.95 + rut);
        }
        // Tracks of churned dust.
        let t = self.track(x, z);
        if t < 1.8 {
            let half = 0.5 + 0.08 * fbm(x / 90.0, z / 90.0, seed ^ 140, 2);
            let core = smoothstep(half + 0.12, half - 0.12, t) * (1.0 - smoothstep(9.0, 12.0, slope));
            let rut = smoothstep(0.09, 0.03, (t - 0.25).abs());
            c = mix3(c, scale(s.dust_dark, 0.9 - 0.15 * rut + 0.08 * g2), core * 0.8);
        }
        self.shade_bands(c, x, z)
    }

    /// The void as the stars: black, a faint nebula in broad bands, stars a few pixels apart, a
    /// few brighter, repeating as the rest of the art does.
    fn starfield(&self, s: &Space, x: f32, y: f32, z: f32) -> [f32; 3] {
        let seed = self.seed;
        let band = smoothstep(0.62, 0.66, fbm(x / 1400.0, z / 1400.0, seed ^ 141, 3) * 0.5 + 0.5);
        let mut c = mix3(s.black, s.nebula, band * 0.6);
        let star = tworley(x, y, 9.0, 9.0, seed ^ 142, 1.0);
        let bright = ((star.id >> 4) & 255) as f32 / 255.0;
        if star.f1 < 0.12 && bright > 0.55 {
            let tint = [1.0 - 0.1 * ((star.id >> 12) & 1) as f32, 1.0, 1.0 + 0.1 * ((star.id >> 13) & 1) as f32];
            let k = smoothstep(0.12, 0.0, star.f1) * (0.4 + 0.6 * bright);
            c = mix3(c, tint.map(|v| v * 0.95), k);
        }
        if tunit(x, y, seed ^ 143) > 0.9985 {
            c = mix3(c, [0.85, 0.85, 0.9], 0.6);
        }
        let _ = PERIOD;
        c
    }
}
