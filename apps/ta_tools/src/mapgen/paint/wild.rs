//! Temperate land in detail (forests, meadows, islands): the grass in regions wetter and drier
//! and patches of meadow, straw and heath, each its own colour through and their edges short and
//! wandering (the map's tiles share well between places alike, and a gradient across many tiles
//! would be cut into squares by them), in tufts, wildflowers drifted over the meadows; moss in
//! the damp hollows; leaf litter and moss under the trees, a fringe of rank grass at the forest's
//! edge; bare earth worn on the dry knolls; outcrops of bedrock in slabs, cracked and spotted with
//! lichen; cliffs in strata; scree fanned at their feet; pebbles and sand on the shore, reeds in
//! the shallows of a marsh; tracks worn into the turf, two ruts and a strip of grass between.
//!
//! What lies where follows the ground (x, z); the fine texture within (tufts, stones, grain)
//! is laid in the art's own rows and repeats every `noise::PERIOD` pixels, so a stretch of one
//! ground is made of a few tiles that meet seamlessly.

use super::super::noise::{fbm, mix3, perlin, smoothstep, tfbm, tperlin, tunit, tvalue, tvalue2, tworley, value};
use super::{Ground, scale};

/// A temperate world's colours (sRGB, 0 to 1) and how much of each kind of ground it has.
#[derive(Clone)]
pub struct Wild {
    pub meadow: [f32; 3],
    pub grass: [f32; 3],
    pub dry: [f32; 3],
    pub heath: [f32; 3],
    pub moss: [f32; 3],
    pub soil: [f32; 3],
    pub litter: [f32; 3],
    pub rock: [f32; 3],
    pub rock_dark: [f32; 3],
    pub lichen: [f32; 3],
    pub sand: [f32; 3],
    pub sand_wet: [f32; 3],
    pub shallow: [f32; 3],
    pub deep: [f32; 3],
    pub track: [f32; 3],
    pub reeds: [f32; 3],
    /// The wildflowers' colours, and how thick they grow over the meadows (0 none).
    pub flowers: [[f32; 3]; 3],
    pub bloom: f32,
    /// How much heath the dry heights carry, how much of the open ground is worn bare, how
    /// much bedrock shows on the knolls (each 0 to 1), how wet the land is overall (-0.3 to 0.3).
    pub heath_share: f32,
    pub bare: f32,
    pub outcrops: f32,
    pub wetness: f32,
    /// Where the slopes turn to rock (height units a cell where it starts and is full).
    pub rock_from: f32,
    pub rock_full: f32,
    /// How far above the sea the shore is sand or shingle (height units), and from it (cells).
    pub beach: f32,
    pub beach_cells: f32,
    /// Marsh: how much of the low, flat ground by the water is reed beds (0 to 1).
    pub marsh: f32,
    /// Coral reefs in the shallows (0 none, 1 most of them), and their colours.
    pub reef: f32,
    pub coral: [[f32; 3]; 3],
    /// The forest floor's edge short, no fringe of rank grass (a real place's, where the woods
    /// stand on ground of another colour: their soft edges the tiles cut into squares); the first
    /// maps' forests as they were made.
    pub crisp: bool,
    /// The sea painted in bands by depth, so its depths read off the art (none: the shallows'
    /// colour running into the deep's over 18 units, as first made).
    pub bands: Option<Bands>,
}

/// The sea in bands by depth: past the shallows the shelf's colour, from `deep` units down the
/// deep's (the wild painter's own), from `abyss_from` the abyss's; each its own colour through,
/// the edges between short and wandering (a gradient across many tiles is cut into squares by
/// the shared tiles).
#[derive(Clone, Copy)]
pub struct Bands {
    pub shelf: [f32; 3],
    pub abyss: [f32; 3],
    pub deep: f32,
    pub abyss_from: f32,
}

impl Ground<'_> {
    pub fn wild(&self, s: &Wild, x: f32, z: f32, h: f32, row: f32) -> [f32; 3] {
        let (cx, cz) = (x / 16.0 - 0.5, z / 16.0 - 0.5);
        let seed = self.seed;
        // The fine texture, in the art's rows (`y`), repeating.
        let y = row;
        let g1 = tvalue(x, y, 1.7, seed ^ 1) - 0.5;
        let g2 = tvalue(x, y, 4.3, seed ^ 2) - 0.5;
        let g3 = tperlin(x, y, 19.0, seed ^ 3);
        let depth = self.sea - h;
        let water_dist = self.water_dist.sample(cx, cz);
        if depth > 0.0 {
            return self.wild_water(s, x, y, z, depth, g1, g2);
        }
        let slope = self.slope.sample(cx, cz);
        let wide = self.slope_wide.sample(cx, cz);
        // (Where the ground changes, the threshold wanders with the ground's own noise, as
        // `Ground::rock` measures it.)
        let gp3 = perlin(x / 19.0, z / 19.0, seed ^ 3);
        let moist = (self.moisture.sample(cx, cz) + 0.08 * gp3).clamp(0.0, 1.0);
        let forest = self.forest.sample(cx, cz);
        let cav = self.cavity.sample(cx, cz);
        // Regions and patches, their edges wandering (the noise read through a warp).
        let wx = x + 140.0 * fbm(x / 640.0, z / 640.0, seed ^ 31, 2);
        let wz = z + 140.0 * fbm(x / 640.0 + 3.1, z / 640.0 - 1.7, seed ^ 32, 2);
        let region = fbm(wx / 1500.0, wz / 1500.0, seed ^ 33, 3) * 0.5 + 0.5;
        let fields = fbm(wx / 360.0, wz / 360.0, seed ^ 34, 4) * 0.5 + 0.5;
        let patch = fbm(wx / 95.0, wz / 95.0, seed ^ 35, 3) * 0.5 + 0.5;
        // How lush the grass grows: wetter by the water and over wet regions, in patches.
        let wet = (moist * 0.5 + region * 0.35 + (fields - 0.5) * 0.6 + s.wetness + 0.08).clamp(0.0, 1.0);
        let lush = smoothstep(0.585, 0.6, wet + (patch - 0.5) * 0.3);
        let straw = smoothstep(0.3, 0.285, wet + (patch - 0.5) * 0.2);
        let mut c = mix3(s.grass, s.meadow, lush);
        c = mix3(c, s.dry, straw);
        // Tufts: clumps a few pixels across, lit on their tops, dark between.
        let tuft = tworley(x, y, 3.1, 2.6, seed ^ 36, 0.95);
        let blades = 1.0 + 0.12 * g1 + 0.07 * g2 + 0.09 * (0.45 - tuft.f1) + 0.03 * g3;
        c = scale(c, blades);
        // Clover and finer grass in the meadows.
        let clover = smoothstep(0.55, 0.75, tfbm(x, y, 23.0, seed ^ 37, 2) * 0.5 + 0.5) * lush * 0.35;
        c = mix3(c, scale(s.meadow, 1.12), clover);
        // Heath on the dry heights: heather and bracken, browner, in clumps.
        let high = smoothstep(self.sea + 40.0, self.sea + 120.0, h);
        let heath = smoothstep(0.62, 0.65, fields * 0.6 + high * 0.5 + (0.5 - wet) * 0.6 + (patch - 0.5) * 0.3) * s.heath_share;
        if heath > 0.0 {
            let clump = tworley(x, y, 6.5, 5.5, seed ^ 38, 0.9);
            let mut hc = mix3(s.heath, scale(s.heath, 0.72), smoothstep(0.2, 0.6, clump.f1));
            // Each clump heather, bracken or straw.
            let kind = ((clump.id >> 8) & 255) as f32 / 255.0;
            hc = mix3(hc, scale(mix3(s.grass, s.dry, 0.4), 0.85), smoothstep(0.6, 0.7, kind) * 0.7);
            hc = mix3(hc, s.dry, smoothstep(0.85, 0.95, kind) * 0.6);
            c = mix3(c, scale(hc, 1.0 + 0.1 * g1 + 0.06 * g2), heath);
        }
        // Moss in the damp hollows.
        let damp = smoothstep(2.6, 3.4, cav + (patch - 0.5) * 2.0) * smoothstep(0.45, 0.5, moist);
        c = mix3(c, scale(s.moss, 0.9 + 0.2 * (g2 + 0.5)), damp * 0.6);
        // Wildflowers drifted over the meadows: single blooms, thick in the drifts.
        if s.bloom > 0.0 {
            let drift = smoothstep(0.66, 0.7, fbm(x / 70.0, z / 70.0, seed ^ 39, 3) * 0.5 + 0.5) * (lush * 0.8 + 0.2) * s.bloom * (1.0 - heath) * (1.0 - forest);
            if drift > 0.0 && tunit(x, y, seed ^ 40) < drift * 0.3 {
                let k = (tunit(y, x, seed ^ 41) * 3.0) as usize;
                c = mix3(c, s.flowers[k.min(2)], 0.85);
            }
        }
        // Bare earth: worn on the dry knolls, and below the slopes.
        let knoll = smoothstep(-0.5, -2.5, cav);
        let bare = (smoothstep(0.75, 0.78, patch * 0.7 + (0.5 - wet) * 0.6 + knoll * 0.25) * s.bare + smoothstep(5.0, 10.0, slope) * 0.35).clamp(0.0, 0.9);
        if bare > 0.0 {
            let pebble = tworley(x, y, 3.5, 3.5, seed ^ 42, 0.9);
            let mut soil = scale(s.soil, 0.88 + 0.16 * g2 + 0.14 * g1 + 0.06 * g3);
            soil = mix3(soil, scale(s.rock, 0.95), smoothstep(0.32, 0.18, pebble.f1) * 0.5);
            c = mix3(c, soil, bare);
        }
        // Under the trees: leaf litter, moss in patches, twigs; rank grass at the forest's edge.
        if forest > 0.0 {
            if !s.crisp {
                let fringe = smoothstep(0.02, 0.12, forest) * smoothstep(0.45, 0.15, forest);
                c = mix3(c, scale(mix3(s.grass, s.moss, 0.4), 0.85 + 0.25 * (g1 + 0.5)), fringe * 0.5);
            }
            let leaf = tworley(x, y, 2.4, 2.4, seed ^ 43, 1.0);
            let mut floor = scale(s.litter, 0.78 + 0.34 * tvalue(x, y, 5.0, seed ^ 6) + 0.12 * (0.5 - leaf.f1));
            let mossy = smoothstep(0.45, 0.65, tfbm(x, y, 34.0, seed ^ 44, 3) * 0.5 + 0.5 + (moist - 0.5) * 0.4);
            floor = mix3(floor, scale(s.moss, 0.85 + 0.3 * (g2 + 0.5)), mossy * 0.7);
            // Fallen leaves, lighter, here and there.
            if (leaf.id & 15) == 0 && leaf.f1 < 0.3 {
                floor = mix3(floor, scale(s.dry, 0.9), 0.5);
            }
            let under = if s.crisp { smoothstep(0.08, 0.14, forest + 0.03 * gp3) * 0.9 } else { (forest * 1.6).clamp(0.0, 0.9) };
            c = mix3(c, floor, under);
        }
        // Bedrock in slabs on the knolls and the shoulders of the hills.
        if s.outcrops > 0.0 {
            let crop = smoothstep(0.7, 0.72, fbm(wx / 130.0, wz / 130.0, seed ^ 45, 4) * 0.5 + 0.5 + knoll * 0.12 + smoothstep(2.0, 7.0, slope) * 0.08 - forest * 0.3) * s.outcrops;
            if crop > 0.0 {
                let slab = tworley(x, y, 17.0, 11.0, seed ^ 46, 0.85);
                let tone = ((slab.id >> 12) & 255) as f32 / 255.0;
                let mut r = mix3(s.rock_dark, s.rock, 0.45 + 0.45 * tone);
                r = scale(r, 0.9 + 0.14 * (g1 + 0.5) + 0.06 * g2);
                let crack = smoothstep(0.09, 0.02, slab.f2 - slab.f1);
                r = scale(r, 1.0 - 0.45 * crack);
                let lichen = smoothstep(0.62, 0.75, tvalue(x, y, 4.0, seed ^ 47) * 0.6 + tvalue(x, y, 11.0, seed ^ 48) * 0.4);
                r = mix3(r, s.lichen, lichen * 0.45 * (1.0 - crack));
                // Grass in the joints.
                r = mix3(r, scale(s.grass, 0.8), crack * 0.35 * (1.0 - smoothstep(4.0, 9.0, slope)));
                c = mix3(c, r, crop);
            }
        }
        // Scree fanned out at the foot of the cliffs: stones on grit.
        let foot = smoothstep(4.5, 8.0, wide) * smoothstep(10.0, 5.0, slope) * (0.5 + 0.5 * smoothstep(-0.5, 1.5, cav));
        if foot > 0.0 {
            let stone = tworley(x, y, 4.6, 4.6, seed ^ 49, 0.9);
            let tone = ((stone.id >> 4) & 255) as f32 / 255.0;
            let grit = mix3(s.soil, s.rock, 0.55);
            let pebble = scale(mix3(s.rock_dark, s.rock, 0.3 + 0.6 * tone), 0.85 + 0.35 * smoothstep(0.4, 0.0, stone.f1 + stone.dx * 0.3 + stone.dy * 0.3));
            let sc = mix3(scale(grit, 0.9 + 0.12 * g1), pebble, smoothstep(0.42, 0.3, stone.f1));
            c = mix3(c, sc, (foot * 1.2).clamp(0.0, 0.85));
        }
        // Cliffs: rock in strata following the contours, cracked. (Kept whole by the tiles, so
        // in the ground's own place.)
        let rocky = smoothstep(s.rock_from, s.rock_full, slope + 2.5 * gp3);
        if rocky > 0.0 {
            let band = 0.5 + 0.5 * ((h + 4.0 * perlin(x / 31.0, z / 31.0, seed ^ 7)) * std::f32::consts::TAU / 7.5).sin();
            let crack = smoothstep(0.92, 0.98, 1.0 - perlin(x / 13.0, z / 7.0, seed ^ 8).abs());
            let ledge = smoothstep(0.75, 0.95, band) * smoothstep(0.55, 0.75, value(x / 9.0, h / 3.0, seed ^ 50));
            let mut r = mix3(s.rock_dark, s.rock, 0.35 + 0.55 * band);
            r = scale(r, (0.86 + 0.3 * (g1 + 0.5) * 0.5 + 0.1 * g2) * (1.0 - 0.35 * crack));
            // Grass and moss on the ledges.
            r = mix3(r, scale(mix3(s.grass, s.moss, 0.5), 0.8), ledge * 0.5);
            c = mix3(c, r, rocky);
        }
        // The shore: shingle and sand, wet at the water's edge; reed beds in the marshy flats.
        let shore = smoothstep(s.beach, s.beach * 0.25, -depth + 1.5 * gp3) * smoothstep(s.beach_cells, s.beach_cells * 0.3, water_dist);
        let flat_low = smoothstep(4.0, 1.0, slope) * smoothstep(s.beach * 2.5, 0.5, -depth);
        let marsh = (s.marsh * flat_low * smoothstep(0.52, 0.56, fbm(x / 80.0, z / 80.0, seed ^ 51, 3) * 0.5 + 0.5 + moist * 0.2)).clamp(0.0, 1.0);
        if shore > 0.0 {
            let stone = tworley(x, y, 3.2, 3.2, seed ^ 52, 0.9);
            let shingle = smoothstep(0.6, 0.64, fbm(x / 60.0, z / 60.0, seed ^ 53, 2) * 0.5 + 0.5);
            let mut sand = scale(mix3(s.sand_wet, s.sand, smoothstep(0.4, 2.2, -depth)), 0.92 + 0.16 * (g1 + 0.5) * 0.5);
            let pebbles = scale(mix3(s.rock, s.sand, 0.4), 0.8 + 0.35 * smoothstep(0.45, 0.0, stone.f1));
            sand = mix3(sand, pebbles, shingle * smoothstep(0.5, 0.3, stone.f1) * 0.8);
            c = mix3(c, sand, shore * (1.0 - marsh * 0.8));
        }
        if marsh > 0.0 {
            // Reeds seen from above: stalks in dense clumps, pools of dark water between.
            let stalk = tvalue(x, y, 1.3, seed ^ 54);
            let clump = tworley(x, y, 9.0, 9.0, seed ^ 55, 0.9);
            let mut r = scale(s.reeds, 0.75 + 0.45 * stalk);
            r = mix3(r, scale(s.deep, 0.7), smoothstep(0.55, 0.75, clump.f1) * 0.5);
            c = mix3(c, r, marsh);
        }
        // Tracks: two ruts and a strip of grass between, the turf worn either side.
        let t = self.track(x, z);
        if t < 1.8 {
            let half = 0.55 + 0.08 * fbm(x / 90.0, z / 90.0, seed ^ 56, 2);
            let core = smoothstep(half + 0.1, half - 0.1, t);
            let verge = smoothstep(half + 0.9, half, t) * 0.55;
            let rut = smoothstep(0.1, 0.03, (t - 0.27).abs());
            let mid = smoothstep(0.13, 0.05, t);
            let mut tc = scale(s.track, 0.9 + 0.16 * (g2 + 0.5) + 0.1 * g1);
            tc = scale(tc, 1.0 - 0.22 * rut);
            tc = mix3(tc, scale(mix3(s.grass, s.dry, 0.5), 0.9), mid * 0.5);
            let worn = mix3(c, mix3(s.dry, s.soil, 0.4), 0.45);
            c = mix3(c, worn, verge * (1.0 - core));
            c = mix3(c, tc, core * (1.0 - smoothstep(9.0, 12.0, slope)));
        }
        self.shade_bands(c, x, z)
    }

    /// Water: light over the shallows, where the bed shows (sand and stones), deep blue
    /// further out, waves.
    #[allow(clippy::too_many_arguments)]
    fn wild_water(&self, s: &Wild, x: f32, y: f32, z: f32, depth: f32, g1: f32, g2: f32) -> [f32; 3] {
        let seed = self.seed;
        let wave = tvalue2(x, y, 9.0, 3.2, seed ^ 5) - 0.5;
        let swell = 1.0 - tperlin(x, y, 40.0, seed ^ 57).abs();
        let mut c = match &s.bands {
            None => mix3(s.shallow, s.deep, smoothstep(0.0, 18.0, depth)),
            Some(b) => {
                let e1 = b.deep + 2.0 * perlin(x / 130.0, z / 130.0, seed ^ 63);
                let e2 = b.abyss_from + 4.0 * perlin(x / 170.0, z / 170.0, seed ^ 62);
                let c = mix3(s.shallow, b.shelf, smoothstep(1.5, 6.0, depth));
                let c = mix3(c, s.deep, smoothstep(e1 - 1.5, e1 + 1.5, depth));
                mix3(c, b.abyss, smoothstep(e2 - 2.0, e2 + 2.0, depth))
            }
        };
        // The bed in the shallows: sand, stones, weed.
        let bed = smoothstep(7.0, 0.5, depth);
        if bed > 0.0 {
            let stone = tworley(x, y, 6.0, 6.0, seed ^ 58, 0.9);
            let weed = smoothstep(0.55, 0.7, tfbm(x, y, 40.0, seed ^ 59, 3) * 0.5 + 0.5);
            let mut b = mix3(s.sand_wet, scale(s.rock_dark, 1.1), smoothstep(0.5, 0.3, stone.f1) * 0.4);
            b = mix3(b, scale(s.moss, 0.8), weed * 0.5);
            c = mix3(c, mix3(b, s.shallow, 0.45), bed * 0.6);
        }
        // Coral reefs: heads of coral in many colours on pale sand, in banks along the
        // shallows, fading into the deep.
        if s.reef > 0.0 && depth < 16.0 {
            let bank = smoothstep(0.55, 0.58, fbm(x / 160.0, z / 160.0, seed ^ 60, 3) * 0.5 + 0.5 + (s.reef - 0.5) * 0.3) * smoothstep(16.0, 6.0, depth) * smoothstep(0.5, 2.0, depth);
            if bank > 0.0 {
                let head = tworley(x, y, 7.0, 6.0, seed ^ 61, 0.9);
                let kind = (head.id >> 7) as usize % 3;
                let lit = smoothstep(0.42, 0.0, head.f1 + 0.25 * head.dy);
                let sand = mix3(s.sand, s.shallow, 0.35);
                let coral = mix3(scale(s.coral[kind], 0.7 + 0.4 * lit), s.shallow, smoothstep(4.0, 14.0, depth) * 0.6);
                let r = mix3(sand, coral, smoothstep(0.46, 0.36, head.f1));
                c = mix3(c, r, bank * 0.8);
            }
        }
        c = mix3(s.sand_wet, c, smoothstep(0.0, 2.5, depth));
        let k = 1.0 + 0.10 * wave + 0.04 * g2 + 0.02 * g1 + 0.05 * (swell - 0.5) * smoothstep(4.0, 20.0, depth);
        scale(c, k).map(|v| v.clamp(0.0, 1.0))
    }
}
