//! Metal worlds: decks of plating laid in panels, a panel a tile: plain plate riveted at its
//! corners, tread plate, grates over dark shafts, hatches, round vents, conduits and lamps, each
//! deck its own shade of steel; seams dark and bevels lit along the panels' edges; hazard stripes
//! along the decks' lips and chevrons up the ramps; grime in the corners and rust streaking from
//! the seams; bunkers armoured darker; walls ribbed and riveted down to the coolant channels;
//! landing pads ringed in hazard yellow at the bases.
//!
//! The panels are a patchwork of plates 128, 64 and 32 pixels across, laid over the repeat of
//! `noise::PERIOD` on a grid 16 pixels off the tiles' (so their seams run through the tiles, at
//! many spacings, not along every tile's edge as a grid), each plate's kind chosen by its place
//! in the repeat: a stretch of one deck is made of a few dozen tiles that meet as TA's own metal
//! maps' do. What changes from deck to deck (its shade, its grime) changes at the decks' edges,
//! which are walls.

use super::super::noise::{PERIOD, fbm, hash, mix3, perlin, smoothstep, tvalue, value};
use super::{Ground, scale};

/// A metal world's colours (sRGB, 0 to 1).
#[derive(Clone)]
pub struct Metal {
    /// Steel: the decks' shades (one a deck), and the plating's darker and lighter tones.
    pub decks: [[f32; 3]; 4],
    pub dark: [f32; 3],
    pub light: [f32; 3],
    pub grate: [f32; 3],
    pub rust: [f32; 3],
    pub grime: [f32; 3],
    pub hazard: [f32; 3],
    pub hazard_dark: [f32; 3],
    pub lamp: [f32; 3],
    pub wall: [f32; 3],
    pub wall_dark: [f32; 3],
    pub bunker: [f32; 3],
    pub coolant: [f32; 3],
    pub coolant_deep: [f32; 3],
    /// How much rust and grime (0 to 1).
    pub rust_share: f32,
    pub grime_share: f32,
    pub rock_from: f32,
    pub rock_full: f32,
}

/// A panel's kinds (see [`Ground::metal`]).
const PLAIN: u32 = 0;
const TREAD: u32 = 1;
const GRATE: u32 = 2;
const HATCH: u32 = 3;
const VENT: u32 = 4;
const CONDUIT: u32 = 5;
const LAMP: u32 = 6;
const SCUFFED: u32 = 7;

impl Ground<'_> {
    pub fn metal(&self, s: &Metal, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        let y = row;
        let g1 = tvalue(x, y, 1.6, seed ^ 1) - 0.5;
        let g2 = tvalue(x, y, 5.0, seed ^ 2) - 0.5;
        let gp3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let depth = self.sea - h;
        if depth > 0.0 {
            // Coolant: dark, faintly lit by the decks' lamps along the channel walls.
            let ripple = tvalue(x, y, 7.0, seed ^ 5) - 0.5;
            let c = mix3(s.coolant, s.coolant_deep, smoothstep(0.0, 12.0, depth));
            return scale(c, 1.0 + 0.06 * ripple + 0.03 * g2).map(|v| v.clamp(0.0, 1.0));
        }
        let slope = self.slope.sample(cx, cz);
        let wide = self.slope_wide.sample(cx, cz);
        let cav = self.cavity.sample(cx, cz);
        let built = self.land.built.as_ref();
        let cell = ((cz + 0.5).max(0.0) as usize).min(self.land.h - 1) * self.land.w + ((cx + 0.5).max(0.0) as usize).min(self.land.w - 1);
        let block = built.map_or(0, |b| b.block_of[cell]);
        // The deck's own shade of steel, the lower decks a little darker (in the upper ones'
        // shadow, and grimier).
        let shade_of = hash(block as i32, 3, seed ^ 0x5E7);
        let mut steel = s.decks[(shade_of % 4) as usize];
        let level = built.map_or(0, |b| b.block_level[block as usize].max(0));
        steel = scale(steel, 0.9 + 0.12 * ((shade_of >> 8) & 255) as f32 / 255.0 + 0.03 * level as f32);
        // The plate holding this pixel: the repeat cut into plates of 128, split into four of 64
        // a third of the time, and those into four of 32 half the time.
        let p = PERIOD as i32;
        let (qx, qy) = ((x.floor() as i32 + 16).rem_euclid(p), (y.floor() as i32 + 16).rem_euclid(p));
        let mut size = 128;
        let (mut ox, mut oy) = ((qx / 128) * 128, (qy / 128) * 128);
        loop {
            let split = hash(ox / 32, oy / 32 + size * 7, seed ^ 0x9A1) % 6;
            if size == 32 || (size == 128 && split >= 2) || (size == 64 && split >= 3) {
                break;
            }
            size /= 2;
            ox += ((qx - ox) / size) * size;
            oy += ((qy - oy) / size) * size;
        }
        let sz = size as f32;
        let (ux, uy) = ((qx - ox) as f32 + 0.5, (qy - oy) as f32 + 0.5);
        // (The kinds' drawings are made for a plate of 32 and scaled to the plate.)
        let k = sz / 32.0;
        let kind_hash = hash(ox / 32, oy / 32, seed ^ 0x9A3 ^ size as u32);
        let kind = match kind_hash % 24 {
            0..=8 => PLAIN,
            9..=11 => TREAD,
            12 => GRATE,
            13..=15 => HATCH,
            16 => VENT,
            17..=18 => CONDUIT,
            19 => LAMP,
            _ => SCUFFED,
        };
        // Distance from the plate's edge (pixels), its seam and its lit bevel (north-west).
        let edge = ux.min(sz - ux).min(uy).min(sz - uy);
        let (cxp, cyp) = (ux - sz * 0.5, uy - sz * 0.5);
        let mut c = scale(steel, 0.96 + 0.05 * g2 + 0.03 * g1);
        match kind {
            TREAD => {
                // Diamond tread: raised bars at right angles in a checker.
                let (a, b) = ((ux / 4.0).floor() as i32, (uy / 4.0).floor() as i32);
                let (fx, fy) = ((ux / 4.0).fract() - 0.5, (uy / 4.0).fract() - 0.5);
                let bar = if (a + b) % 2 == 0 { (fx + fy).abs() < 0.22 } else { (fx - fy).abs() < 0.22 };
                if bar {
                    c = scale(c, 1.12);
                }
            }
            GRATE => {
                // Bars over a dark shaft.
                let bar = (ux % 4.0) < 1.6 || (uy % 8.0) < 1.4;
                c = if bar { scale(steel, 0.82) } else { scale(s.grate, 0.85 + 0.2 * g2) };
            }
            HATCH => {
                // An inset square, its rim raised, bolts at its corners.
                let inner = cxp.abs().max(cyp.abs());
                let half = sz * 0.5 - 6.0 * k.min(2.0);
                if inner < half {
                    c = scale(steel, 0.9);
                    if inner > half - 1.5 {
                        c = scale(steel, 1.15);
                    }
                }
                let b = half - 3.0;
                let bolt = [(-b, -b), (b, -b), (-b, b), (b, b)].iter().any(|&(bx, by)| (cxp - bx).hypot(cyp - by) < 1.3);
                if bolt {
                    c = scale(s.light, 0.95);
                }
            }
            VENT => {
                // A round grille, its slots dark, a ring round it.
                let r = cxp.hypot(cyp);
                let outer = sz * 0.5 - 4.0;
                if r < outer {
                    let a = cyp.atan2(cxp);
                    let slot = ((a * (12.0 * k).round() / std::f32::consts::TAU).fract() < 0.5) && r > outer * 0.25;
                    c = if slot { scale(s.grate, 0.8) } else { scale(steel, 0.88) };
                    if r > outer - 1.5 {
                        c = scale(steel, 1.12);
                    }
                }
            }
            CONDUIT => {
                // Pipes across the plate, lit on top, shadowed below.
                let along_x = (kind_hash >> 12) & 1 == 0;
                let v = if along_x { uy } else { ux };
                let pipes = (k as usize).max(1) * 2;
                for n in 0..pipes {
                    let c0 = sz * (n as f32 + 0.5) / pipes as f32;
                    let d = v - c0;
                    if d.abs() < 3.0 {
                        let lit = 1.0 - (d + 1.0) / 4.0;
                        c = scale(mix3(steel, s.dark, 0.3), 0.8 + 0.5 * lit.clamp(0.0, 1.0));
                    } else if (3.0..4.5).contains(&d) {
                        c = scale(c, 0.75);
                    }
                }
            }
            LAMP => {
                // A lamp in a housing.
                let r = cxp.hypot(cyp);
                if r < 4.0 {
                    c = scale(s.dark, 0.9);
                }
                if r < 2.4 {
                    c = s.lamp;
                } else if r < 7.0 {
                    c = mix3(c, s.lamp, smoothstep(7.0, 2.4, r) * 0.25);
                }
            }
            SCUFFED => {
                // Scratches across the plate.
                let sc = (tvalue(x * 0.3 + y, y * 0.3 - x * 0.2, 2.0, seed ^ 0x5C) - 0.5).abs();
                if sc < 0.05 {
                    c = scale(c, 1.15);
                }
            }
            _ => {
                // Plain plate: rivets along its edges, every 16 pixels.
                let near = edge > 3.0 && edge < 5.0;
                let along = if ux.min(sz - ux) < uy.min(sz - uy) { uy } else { ux };
                if near && (along % 16.0 - 8.0).abs() < 1.2 {
                    c = scale(s.light, 0.9);
                }
            }
        }
        // The seams between plates: dark, a lit bevel on the north-west side, shade on the other.
        if edge < 1.0 {
            c = scale(s.dark, 0.9);
        } else if edge < 2.2 {
            c = if ux < 3.0 || uy < 3.0 { scale(c, 1.12) } else { scale(c, 0.85) };
        }
        // Grime: in the corners and along the walls' feet, in bands.
        let corner = smoothstep(1.5, 2.6, cav) + smoothstep(4.0, 7.0, wide) * smoothstep(9.0, 5.0, slope) * 0.6;
        let grime = (smoothstep(0.45, 0.55, corner + (fbm(x / 120.0, z / 120.0, seed ^ 0x61, 3) * 0.5 + 0.5 - 0.5) * 0.4) * s.grime_share).clamp(0.0, 0.8);
        if grime > 0.0 {
            c = mix3(c, scale(s.grime, 0.9 + 0.2 * tvalue(x, y, 6.0, seed ^ 0x62)), grime * 0.7);
        }
        // Rust: in patches, streaking from the seams.
        if s.rust_share > 0.0 {
            let patch = fbm(x / 90.0, z / 90.0, seed ^ 0x63, 3) * 0.5 + 0.5;
            let rust = smoothstep(0.66, 0.7, patch + (hash(block as i32, 9, seed) & 255) as f32 / 255.0 * 0.1 - 0.05) * s.rust_share;
            if rust > 0.0 {
                let streak = tvalue(x * 2.0, y * 0.25, 3.0, seed ^ 0x64);
                c = mix3(c, scale(s.rust, 0.8 + 0.4 * streak), rust * (0.4 + 0.4 * smoothstep(0.4, 0.8, streak)));
            }
        }
        // The decks' lips: a band of hazard stripes where the deck drops away.
        let lip = smoothstep(4.0, 6.0, wide) * smoothstep(8.0, 4.0, slope);
        if lip > 0.0 {
            let k = 2.0 * PERIOD / 256.0 * 16.0;
            let stripe = (((x + y) / PERIOD * k).fract() < 0.5) as i32 as f32;
            let band = mix3(s.hazard_dark, s.hazard, stripe);
            c = mix3(c, scale(band, 0.9 + 0.15 * g2), lip * 0.9);
        }
        // Ramps: tread plate for the grip, the colour of the decks it joins (its sides get the
        // lips' stripes where the walls drop beside it).
        if built.is_some_and(|b| b.ramp[cell]) {
            let (a, b) = ((x / 4.0).floor() as i32, (y / 4.0).floor() as i32);
            let (fx, fy) = ((x / 4.0).fract() - 0.5, (y / 4.0).fract() - 0.5);
            let bar = if (a + b).rem_euclid(2) == 0 { (fx + fy).abs() < 0.22 } else { (fx - fy).abs() < 0.22 };
            c = scale(steel, if bar { 1.05 } else { 0.86 } + 0.05 * g2);
        }
        // Landing pads at the bases: a ring of hazard yellow, the pad's plate darker.
        let base = self.land.starts.iter().map(|st| ((cx - st.0).abs()).max((cz - st.1).abs())).fold(f32::MAX, f32::min);
        if base < 13.0 {
            let ring = smoothstep(11.0, 11.6, base) * smoothstep(13.0, 12.4, base);
            if base < 11.0 {
                c = scale(mix3(c, s.dark, 0.25), 0.95);
            }
            let stripe = ((((x - y) / 16.0).fract()) < 0.5) as i32 as f32;
            c = mix3(c, mix3(s.hazard_dark, s.hazard, stripe), ring);
        }
        // Bunkers: armour plate, darker, its edges lit.
        if let Some(Some((x0, z0, x1, z1))) = built.map(|b| b.bunkers[block as usize])
            && cx >= x0 as f32 - 0.5
            && cx <= x1 as f32 + 0.5
            && cz >= z0 as f32 - 0.5
            && cz <= z1 as f32 + 0.5
        {
            let rim = (cx - x0 as f32).min(x1 as f32 - cx).min(cz - z0 as f32).min(z1 as f32 - cz);
            let mut b = scale(s.bunker, 0.92 + 0.1 * g2);
            if rim < 2.5 {
                b = scale(b, 0.85 + 0.25 * smoothstep(0.0, 2.5, rim));
            }
            c = mix3(c, b, 0.9);
        }
        // Walls: ribbed plate, riveted, dark between the ribs. (Kept whole by the tiles.)
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 1.0 * gp3);
        if rocky > 0.0 {
            let rib = (h / 6.0).fract();
            let seam = (x / 32.0).fract() < 0.05;
            let mut w = mix3(s.wall_dark, s.wall, smoothstep(0.1, 0.3, rib) * smoothstep(0.95, 0.75, rib));
            if seam {
                w = scale(w, 0.7);
            }
            let rivet = (rib - 0.5).abs() < 0.08 && (x % 8.0) < 1.5;
            if rivet {
                w = scale(s.light, 0.8);
            }
            w = scale(w, 0.92 + 0.12 * value(x / 4.0, h / 4.0, seed ^ 0x65));
            c = mix3(c, w, rocky);
        }
        self.shade_bands(c, x, z)
    }
}
