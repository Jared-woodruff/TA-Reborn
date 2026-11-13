//! From painted ground to TA's tiles: a map holds its art as 32-pixel tiles, at most 65,536 of
//! them (a 16-bit index) and, to keep the game's tile atlas within reason, far fewer. The tiles of
//! a big map are many times that, so they are grouped: each tile is described by its colours at
//! 8-pixel blocks and along its edges (perceptual OKLab), tiles that look alike share one (the
//! member nearest the group's middle), and the threshold of "alike" is raised until the groups
//! fit most of the budget. Uniform ground shares widely; shores, cliffs and edges keep tiles of
//! their own. The rest of the budget goes to second versions of the biggest groups, so that wide
//! uniform ground is not one tile repeated in a grid. Shared tiles each carry a slightly
//! different tint, and placed blindly their steps at every tile edge drew the 32-pixel grid
//! over the ground: so every place then picks, among the tiles kept that look like it, the one
//! that meets its neighbours' as the painting does ([`stitch`]), each tile's edges settle to
//! the middle of the places showing it ([`settle`], [`shift`]), and again, a few times over.
//! Each tile kept is then dithered into TA's palette.

use rayon::prelude::*;
use rustc_hash::FxHashMap;

/// How colours go to OKLab and back. [`Exact`] uses the platform's own powers and roots (TA
/// Reborn's own maps were made with it); [`Portable`] only arithmetic whose results IEEE fixes
/// to the bit, so a map made from the same picture is the same, byte for byte, on every
/// platform (players compare their maps' bytes before a battle).
pub trait Colour: Copy + Send + Sync + 'static {
    fn oklab(c: [f32; 3]) -> [f32; 3];
    fn srgb(lab: [f32; 3]) -> [f32; 3];
}

/// [`oklab`] and [`srgb`] as they are.
#[derive(Clone, Copy, Debug, Default)]
pub struct Exact;

impl Colour for Exact {
    fn oklab(c: [f32; 3]) -> [f32; 3] {
        oklab(c)
    }
    fn srgb(lab: [f32; 3]) -> [f32; 3] {
        srgb(lab)
    }
}

/// [`oklab`] and [`srgb`] with every power and root taken by Newton's method, in f64, a fixed
/// number of steps: within a unit in the last place of the exact values, and the same on every
/// platform.
#[derive(Clone, Copy, Debug, Default)]
pub struct Portable;

impl Colour for Portable {
    fn oklab(c: [f32; 3]) -> [f32; 3] {
        let lin = |v: f32| {
            let v = v as f64;
            if v <= 0.04045 {
                v / 12.92
            } else {
                // a^2.4 = a² · (a²)^(1/5)
                let a = (v + 0.055) / 1.055;
                let a2 = a * a;
                a2 * root(a2, 5)
            }
        };
        let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
        let l = root(0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b, 3);
        let m = root(0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b, 3);
        let s = root(0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b, 3);
        [
            (0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s) as f32,
            (1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s) as f32,
            (0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s) as f32,
        ]
    }

    fn srgb(lab: [f32; 3]) -> [f32; 3] {
        let (a, b, c) = (lab[0] as f64, lab[1] as f64, lab[2] as f64);
        let l = a + 0.396_337_777_4 * b + 0.215_803_757_3 * c;
        let m = a - 0.105_561_345_8 * b - 0.063_854_172_8 * c;
        let s = a - 0.089_484_177_5 * b - 1.291_485_548_0 * c;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        let lin = [
            4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
        ];
        lin.map(|v| {
            let v = v.clamp(0.0, 1.0);
            // v^(1/2.4) = (v^(1/12))^5, and v^(1/12) = √√∛v.
            let out = if v <= 0.003_130_8 { v * 12.92 } else {
                let t = root(v, 3).sqrt().sqrt();
                1.055 * (t * t * t * t * t) - 0.055
            };
            out as f32
        })
    }
}

/// `x`'s `n`th root (n 3 or 5; a negative `x`'s is negative) by Newton's method from a guess its
/// exponent gives: the same bits on every platform.
fn root(x: f64, n: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return if x.is_finite() { 0.0 } else { x };
    }
    let (sign, a) = if x < 0.0 { (-1.0, -x) } else { (1.0, x) };
    // a = f·2^e with f in [1, 2): the guess 2^(e/n) is within a factor of 2.
    let e = ((a.to_bits() >> 52) & 0x7ff) as i64 - 1023;
    let mut y = f64::from_bits(((e.div_euclid(n as i64) + 1023) as u64) << 52);
    let k = n as f64;
    for _ in 0..12 {
        let mut p = 1.0;
        for _ in 0..n - 1 {
            p *= y;
        }
        y = ((k - 1.0) * y + a / p) / k;
    }
    sign * y
}

/// sRGB (0 to 1) to OKLab.
pub fn oklab(c: [f32; 3]) -> [f32; 3] {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s]
}

/// A tile's description: its mean colour (OKLab); its layout, the colour of 4 x 4 blocks of
/// 8 x 8 pixels less that mean (the grain averaged out); its edges, the colour of the 4 pixels
/// along each side in four stretches of 8 (sides north, south, west, east: entry
/// `(side * 4 + stretch) * 3 + channel`); and its phase: where it lies in the repeat of the
/// art's fine texture (`noise::PERIOD`; 0 for art that does not repeat). Only tiles of one
/// phase are ever alike: their grain carries on across their edges as the painting's does.
#[derive(Clone, Copy)]
pub struct Desc {
    pub mean: [f32; 3],
    pub layout: [f32; 48],
    pub edge: [f32; 48],
    pub phase: u16,
}

/// The sides of a tile in [`Desc::edge`].
const NORTH: usize = 0;
const SOUTH: usize = 1;
const WEST: usize = 2;
const EAST: usize = 3;

/// A tile's description from its 32 x 32 colours (sRGB, row by row).
pub fn describe(pixels: &[[f32; 3]]) -> Desc {
    describe_in::<Exact>(pixels)
}

/// [`describe`] with the colours taken to OKLab as `C` takes them.
pub fn describe_in<C: Colour>(pixels: &[[f32; 3]]) -> Desc {
    let lab: Vec<[f32; 3]> = pixels.iter().map(|&c| C::oklab(c)).collect();
    let mut mean = [0.0f32; 3];
    for c in &lab {
        for k in 0..3 {
            mean[k] += c[k] / 1024.0;
        }
    }
    let mut layout = [0.0f32; 48];
    for by in 0..4 {
        for bx in 0..4 {
            let mut acc = [0.0f32; 3];
            for y in 0..8 {
                for x in 0..8 {
                    let c = lab[(by * 8 + y) * 32 + bx * 8 + x];
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            for k in 0..3 {
                layout[(by * 4 + bx) * 3 + k] = acc[k] / 64.0 - mean[k];
            }
        }
    }
    let mut edge = [0.0f32; 48];
    for stretch in 0..4 {
        for (side, at) in [(NORTH, 0usize), (SOUTH, 28), (WEST, 0), (EAST, 28)] {
            let mut acc = [0.0f32; 3];
            for along in 0..8 {
                for deep in 0..4 {
                    let (x, y) = if side == NORTH || side == SOUTH { (stretch * 8 + along, at + deep) } else { (at + deep, stretch * 8 + along) };
                    let c = lab[y * 32 + x];
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            for k in 0..3 {
                edge[(side * 4 + stretch) * 3 + k] = acc[k] / 32.0;
            }
        }
    }
    Desc { mean, layout, edge, phase: 0 }
}

/// How alike two tiles must be, at scale `k`: their means within a step (lightness, colour), their
/// layouts within a mean squared difference.
#[derive(Clone, Copy)]
struct Tolerance {
    light: f32,
    color: f32,
    layout: f32,
}

impl Tolerance {
    fn at(k: f32) -> Tolerance {
        Tolerance { light: 0.0022 * k, color: 0.0016 * k, layout: 0.00002 * k * k * 48.0 }
    }

    fn alike(&self, a: &Desc, b: &Desc) -> Option<f32> {
        if a.phase != b.phase {
            return None;
        }
        let dl = (a.mean[0] - b.mean[0]).abs();
        let da = (a.mean[1] - b.mean[1]).abs();
        let db = (a.mean[2] - b.mean[2]).abs();
        if dl > self.light || da > self.color || db > self.color {
            return None;
        }
        let mut s = 0.0;
        for (x, y) in a.layout.iter().zip(b.layout.iter()) {
            s += (x - y) * (x - y);
            if s > self.layout {
                return None;
            }
        }
        Some(s / self.layout + (dl / self.light).powi(2) + ((da + db) / self.color).powi(2))
    }

    fn key(&self, d: &Desc) -> (u16, i32, i32, i32) {
        let m = d.mean;
        (d.phase, (m[0] / self.light).floor() as i32, (m[1] / self.color).floor() as i32, (m[2] / self.color).floor() as i32)
    }
}

/// How much rock in strata a tile must show to be kept whole, and the share of the budget such
/// tiles may take at most (the rockiest first) unless a map says otherwise.
const WHOLE_ROCK: f32 = 0.5;
pub const WHOLE_SHARE: f32 = 0.45;

/// Groups the tiles (`descs`; `rock`, how much of each is rock in strata) into at most `budget`
/// alike groups. Tiles of rock are kept whole, each its own (the rockiest, up to `whole_share` of
/// the budget): the description averages their strata and cracks away, so a striated cliff tile
/// looked like flat ground of its mean colour, and bands of cliff became a patchwork of blank
/// tiles. Returns each tile's group, each group's representative tile (indices into `descs`),
/// and the scale of the tolerance the groups settled on (see [`stitch`]).
pub fn group(descs: &[Desc], rock: &[f32], budget: usize, whole_share: f32) -> (Vec<u32>, Vec<u32>, f32) {
    let mut whole: Vec<u32> = (0..descs.len() as u32).filter(|&i| rock[i as usize] >= WHOLE_ROCK).collect();
    whole.sort_by(|&a, &b| rock[b as usize].total_cmp(&rock[a as usize]).then(a.cmp(&b)));
    whole.truncate((budget as f32 * whole_share) as usize);
    whole.sort_unstable();
    if whole.is_empty() {
        return group_alike(descs, budget);
    }
    let mut is_whole = vec![false; descs.len()];
    for &i in &whole {
        is_whole[i as usize] = true;
    }
    let rest: Vec<u32> = (0..descs.len() as u32).filter(|&i| !is_whole[i as usize]).collect();
    let rest_descs: Vec<Desc> = rest.iter().map(|&i| descs[i as usize]).collect();
    let (rest_assign, rest_reps, k) = group_alike(&rest_descs, budget - whole.len());
    let mut assign = vec![0u32; descs.len()];
    let mut reps: Vec<u32> = rest_reps.iter().map(|&r| rest[r as usize]).collect();
    for (j, &i) in rest.iter().enumerate() {
        assign[i as usize] = rest_assign[j];
    }
    for &i in &whole {
        assign[i as usize] = reps.len() as u32;
        reps.push(i);
    }
    eprintln!("  tiles of rock kept whole: {}", whole.len());
    (assign, reps, k)
}

/// Groups the tiles (`descs`) into at most `budget` alike groups (see [`group`]).
fn group_alike(descs: &[Desc], budget: usize) -> (Vec<u32>, Vec<u32>, f32) {
    // Three fifths of the budget for the groups (the tightest tolerance that fits), the rest for
    // versions of the biggest ones.
    let groups = (budget * 3 / 5).max(budget.min(1));
    let (mut lo, mut hi) = (0.5f32, 64.0f32);
    let mut best = None;
    for _ in 0..10 {
        let k = (lo * hi).sqrt();
        let (assign, reps) = greedy(descs, Tolerance::at(k), groups + 1);
        if reps.len() <= groups {
            best = Some((assign, reps, k));
            hi = k;
        } else {
            lo = k;
        }
    }
    let (assign, reps, k) = best.unwrap_or_else(|| {
        let k = hi * 4.0;
        let (a, r) = greedy(descs, Tolerance::at(k), usize::MAX);
        (a, r, k)
    });
    let tol = Tolerance::at(k);
    eprintln!("  tiles: {} -> {} (means within {:.4} light, {:.4} colour)", descs.len(), reps.len(), tol.light, tol.color);
    // Each group's representative: its member nearest the group's mean.
    let mut sums = vec![([0.0f32; 3], [0.0f32; 48]); reps.len()];
    let mut counts = vec![0u32; reps.len()];
    for (i, &g) in assign.iter().enumerate() {
        let s = &mut sums[g as usize];
        counts[g as usize] += 1;
        for k in 0..3 {
            s.0[k] += descs[i].mean[k];
        }
        for k in 0..48 {
            s.1[k] += descs[i].layout[k];
        }
    }
    let centres: Vec<Desc> = sums
        .iter()
        .zip(&counts)
        .map(|(s, &c)| {
            let c = c.max(1) as f32;
            Desc { mean: s.0.map(|v| v / c), layout: s.1.map(|v| v / c), edge: [0.0; 48], phase: 0 }
        })
        .collect();
    let centres: Vec<Desc> = centres.into_iter().zip(&reps).map(|(c, &r)| Desc { phase: descs[r as usize].phase, ..c }).collect();
    let wide = Tolerance::at(k * 4.0);
    let mut chosen: Vec<(f32, u32)> = vec![(f32::MAX, u32::MAX); reps.len()];
    for (i, &g) in assign.iter().enumerate() {
        let d = wide.alike(&descs[i], &centres[g as usize]).unwrap_or(f32::MAX / 2.0);
        if d < chosen[g as usize].0 {
            chosen[g as usize] = (d, i as u32);
        }
    }
    let reps: Vec<u32> = chosen.into_iter().zip(reps).map(|((_, i), first)| if i == u32::MAX { first } else { i }).collect();
    let (assign, reps) = versions(descs, assign, reps, budget);
    eprintln!("  tiles kept: {} (with versions of the biggest groups)", reps.len());
    (assign, reps, k)
}

/// How much a tile's edges may differ from the painting's, in steps across them (OKLab; about
/// the painting's own change over 4 pixels), and the weights of L, a and b (a step in colour
/// shows at a third of the size of one in lightness).
const SEAM_SIGMA: f32 = 0.004;
const SEAM_WEIGHT: [f32; 3] = [1.0, 3.0, 3.0];
/// The cost of showing the same tile as a neighbour (one tile repeated shows as a grid).
const REPEAT: f32 = 0.3;
/// The tiles a place may choose from (the most alike within three times the grouping's
/// tolerance), and how many times every place chooses again.
const CANDIDATES: usize = 64;
const WIDER: f32 = 3.0;
const SWEEPS: usize = 6;
/// How much more rock in strata than its own painting a place's tile may show (see [`stitch`]).
const STRATA_MARGIN: f32 = 0.25;

/// Picks the tile every place shows (`assign`, indices into `shown`, the tiles kept as they
/// will be drawn; the places, `descs`, a map `width` tiles wide, row by row) so that the tiles
/// meet as the painting does. A place's cost for a tile is how unlike its own painting the tile
/// is (as the grouping judges it, at `k`, its tolerance's scale, widened: the candidates are the
/// most alike within that), plus how far the steps in colour across its four edges, to the
/// tiles its neighbours show, are from the painting's steps there (so a step the painting has,
/// a shore, a cliff's crest, a forest's edge, stays, and one it has not, a tile's tint against
/// its neighbour's, goes), plus a little for repeating a neighbour's tile. Lowered place by
/// place on the two colours of a checkerboard in turn, each half reading only the other (the
/// outcome does not depend on the threads), a few times over. A place takes no tile showing
/// much more rock in strata than its painting (`rock`, per place, and `rock_shown`, per tile
/// shown): the description averages strata away, so a cliff's tile kept whole looked like
/// plain ground of its colour, and stood as a striped square on the flat. Returns how many
/// places changed their tile.
pub fn stitch(descs: &[Desc], shown: &[Desc], rock: &[f32], rock_shown: &[f32], assign: &mut [u32], width: usize, k: f32) -> usize {
    let width = width.max(1);
    let tol = Tolerance::at(k * WIDER);
    let mut buckets: FxHashMap<(u16, i32, i32, i32), Vec<u32>> = FxHashMap::default();
    for (g, d) in shown.iter().enumerate() {
        buckets.entry(tol.key(d)).or_default().push(g as u32);
    }
    // Each place's candidates (its present tile always among them), with their likeness.
    let candidates: Vec<Vec<(u32, f32)>> = (0..descs.len())
        .into_par_iter()
        .map(|i| {
            let d = &descs[i];
            let key = tol.key(d);
            let mut list: Vec<(u32, f32)> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(gs) = buckets.get(&(key.0, key.1 + dx, key.2 + dy, key.3 + dz)) {
                            let strata = |g: u32| rock_shown[g as usize] <= rock[i] + STRATA_MARGIN;
                            list.extend(gs.iter().filter(|&&g| strata(g)).filter_map(|&g| tol.alike(d, &shown[g as usize]).map(|e| (g, e))));
                        }
                    }
                }
            }
            list.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            list.truncate(CANDIDATES);
            let own = assign[i];
            if !list.iter().any(|c| c.0 == own) {
                let e = tol.alike(d, &shown[own as usize]).unwrap_or(4.0);
                if list.len() == CANDIDATES {
                    list.pop();
                }
                list.push((own, e));
            }
            list
        })
        .collect();
    let before = assign.to_vec();
    let rows = descs.len() / width;
    // The cost of the seams and repeats of tile `g` at place `i`, given the others' tiles.
    let seams = |i: usize, g: u32, assign: &[u32]| -> f32 {
        let (x, z) = ((i % width) as i64, (i / width) as i64);
        let (tile, painted) = (&shown[g as usize], &descs[i]);
        let mut cost = 0.0;
        for (side, opp, nx, nz) in [(NORTH, SOUTH, x, z - 1), (SOUTH, NORTH, x, z + 1), (WEST, EAST, x - 1, z), (EAST, WEST, x + 1, z)] {
            if nx < 0 || nz < 0 || nx >= width as i64 || nz >= rows as i64 {
                continue;
            }
            let j = nz as usize * width + nx as usize;
            if assign[j] == g {
                cost += REPEAT;
            }
            let (tile_n, painted_n) = (&shown[assign[j] as usize], &descs[j]);
            let mut e = 0.0;
            for stretch in 0..4 {
                for (ch, w) in SEAM_WEIGHT.iter().enumerate() {
                    let (a, b) = ((side * 4 + stretch) * 3 + ch, (opp * 4 + stretch) * 3 + ch);
                    let step = (tile.edge[a] - tile_n.edge[b]) - (painted.edge[a] - painted_n.edge[b]);
                    e += w * step * step;
                }
            }
            cost += e / (12.0 * SEAM_SIGMA * SEAM_SIGMA);
        }
        cost
    };
    for _ in 0..SWEEPS {
        for parity in 0..2 {
            let picks: Vec<(usize, u32)> = (0..descs.len())
                .into_par_iter()
                .filter(|&i| (i % width + i / width) % 2 == parity)
                .map(|i| {
                    let best = candidates[i].iter().map(|&(g, e)| (e + seams(i, g, assign), g)).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                    (i, best.map_or(assign[i], |b| b.1))
                })
                .collect();
            for (i, g) in picks {
                assign[i] = g;
            }
        }
    }
    assign.iter().zip(&before).filter(|(a, b)| a != b).count()
}

/// For every tile kept (`shown`), how far its edges are from those of the places showing it
/// (`descs`, `assign`): the mean of theirs less its own. A tile shifted by it ([`shift`]) meets
/// its neighbours with the steps the places it stands for have on average. Its middle, grain
/// and broad colour stay its own: pulled to the places' average, the middle of a tile shared
/// by unlike places would lose its contrast.
pub fn settle(descs: &[Desc], shown: &[Desc], assign: &[u32]) -> Vec<[f32; 48]> {
    let mut sums = vec![[0.0f64; 48]; shown.len()];
    let mut counts = vec![0.0f64; shown.len()];
    for (d, &g) in descs.iter().zip(assign) {
        for (s, v) in sums[g as usize].iter_mut().zip(&d.edge) {
            *s += *v as f64;
        }
        counts[g as usize] += 1.0;
    }
    sums.iter()
        .zip(&counts)
        .zip(shown)
        .map(|((s, &c), own)| if c == 0.0 { [0.0; 48] } else { std::array::from_fn(|k| (s[k] / c) as f32 - own.edge[k]) })
        .collect()
}

/// Shifts a tile's colours (sRGB, 0 to 1, 32 x 32) so its edges change by `delta` (OKLab, as
/// [`Desc::edge`] and [`settle`] give it): in full over the 4 pixels along each side, fading
/// out 8 pixels in, and from the middle of each stretch along it to the next.
pub fn shift(pixels: &mut [[f32; 3]], delta: &[f32; 48]) {
    shift_in::<Exact>(pixels, delta)
}

/// [`shift`] with the colours taken to OKLab and back as `C` takes them.
pub fn shift_in<C: Colour>(pixels: &mut [[f32; 3]], delta: &[f32; 48]) {
    // The change near `side`, `a` pixels along it and `d` in from it.
    let edge = |side: usize, a: f32, d: f32| -> [f32; 3] {
        let w = 1.0 - ((d - 4.0) / 4.0).clamp(0.0, 1.0);
        let s = ((a - 4.0) / 8.0).clamp(0.0, 3.0);
        let s0 = (s.floor() as usize).min(2);
        let f = s - s0 as f32;
        std::array::from_fn(|k| {
            let (p, q) = (delta[(side * 4 + s0) * 3 + k], delta[(side * 4 + s0 + 1) * 3 + k]);
            (p + (q - p) * f) * w
        })
    };
    for y in 0..32 {
        for x in 0..32 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut lab = C::oklab(pixels[y * 32 + x]);
            for c in [edge(NORTH, px, py), edge(SOUTH, px, 32.0 - py), edge(WEST, py, px), edge(EAST, py, 32.0 - px)] {
                for k in 0..3 {
                    lab[k] += c[k];
                }
            }
            pixels[y * 32 + x] = C::srgb(lab);
        }
    }
}

/// OKLab to sRGB (0 to 1, clamped).
pub fn srgb(lab: [f32; 3]) -> [f32; 3] {
    let l = lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2];
    let m = lab[0] - 0.105_561_35 * lab[1] - 0.063_854_17 * lab[2];
    let s = lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    let lin = [4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s, -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s, -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s];
    lin.map(|v| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    })
}

/// Drops the tiles no place shows (of `count`), renumbering the places' choices (`assign`).
/// Returns the old numbers of the tiles left, in the order of the places first showing them.
pub fn compact(assign: &mut [u32], count: usize) -> Vec<u32> {
    let mut new = vec![u32::MAX; count];
    let mut kept = Vec::new();
    for a in assign.iter_mut() {
        let g = *a as usize;
        if new[g] == u32::MAX {
            new[g] = kept.len() as u32;
            kept.push(g as u32);
        }
        *a = new[g];
    }
    kept
}

/// How the art of a map (`tile_map`: the tile at each place, `width` places a row; `tiles`: the
/// tiles' palette indices, `colors` the palette's) shows its tile grid: per OKLab channel, the
/// mean step in colour across tile edges (between the 4 pixels either side) over the mean step
/// across the middle of a tile at the same spacing. About 1 where tiles meet as seamlessly as
/// the painting runs inside them (TA's own maps measure 0.92 to 1.12).
pub fn blockiness(tile_map: &[u16], tiles: &[Box<[u8; 1024]>], colors: &[[u8; 3]; 256], width: usize) -> [f32; 3] {
    let lab: Vec<[f32; 3]> = colors.iter().map(|c| oklab(c.map(|v| v as f32 / 255.0))).collect();
    // Per tile: the mean colour of its 4-pixel strips: columns 0-3, 12-15, 16-19, 28-31, then
    // the same rows.
    let strips: Vec<[[f32; 3]; 8]> = tiles
        .par_iter()
        .map(|t| {
            let mut s = [[0.0f32; 3]; 8];
            for (k, from) in [0usize, 12, 16, 28].into_iter().enumerate() {
                for a in from..from + 4 {
                    for b in 0..32 {
                        let (col, row) = (lab[t[b * 32 + a] as usize], lab[t[a * 32 + b] as usize]);
                        for ch in 0..3 {
                            s[k][ch] += col[ch] / 128.0;
                            s[4 + k][ch] += row[ch] / 128.0;
                        }
                    }
                }
            }
            s
        })
        .collect();
    let width = width.max(1);
    let rows = tile_map.len() / width;
    let (mut edge, mut within) = ([0.0f64; 3], [0.0f64; 3]);
    for z in 0..rows {
        for x in 0..width {
            let a = &strips[tile_map[z * width + x] as usize];
            // East, then south: the far strip against the neighbour's near one, and the two
            // middle strips of this tile.
            for (next, far, near, mid_a, mid_b) in [((x + 1 < width).then(|| z * width + x + 1), 3, 0, 1, 2), ((z + 1 < rows).then(|| (z + 1) * width + x), 7, 4, 5, 6)] {
                let Some(j) = next else { continue };
                let b = &strips[tile_map[j] as usize];
                for ch in 0..3 {
                    edge[ch] += (a[far][ch] - b[near][ch]).abs() as f64;
                    within[ch] += (a[mid_a][ch] - a[mid_b][ch]).abs() as f64;
                }
            }
        }
    }
    std::array::from_fn(|ch| (edge[ch] / within[ch].max(1e-12)) as f32)
}


/// How different two tiles look, for picking versions (layout and mean, squared).
fn apart(a: &Desc, b: &Desc) -> f32 {
    let mut s = 0.0;
    for (x, y) in a.layout.iter().zip(b.layout.iter()) {
        s += (x - y) * (x - y);
    }
    s + (0..3).map(|k| (a.mean[k] - b.mean[k]).powi(2) * 48.0).sum::<f32>()
}

/// The biggest groups get more representatives, up to `budget` in all: up to eight each, the
/// members farthest apart (each has its own grain), every member then taking the nearest of its
/// group's (near ties broken by where it is, so uniform ground mixes them rather than repeating one
/// tile in a grid).
fn versions(descs: &[Desc], assign: Vec<u32>, reps: Vec<u32>, budget: usize) -> (Vec<u32>, Vec<u32>) {
    const BIG: usize = 16;
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); reps.len()];
    for (i, &g) in assign.iter().enumerate() {
        members[g as usize].push(i as u32);
    }
    let spare = budget.saturating_sub(reps.len());
    let total: usize = members.iter().filter(|m| m.len() >= BIG).map(|m| m.len()).sum();
    if spare == 0 || total == 0 {
        return (assign, reps);
    }
    let (mut assign, mut reps) = (assign, reps);
    let jitter = |i: u32| {
        let mut h = i.wrapping_mul(0x9E37_79B9) ^ 0x7F4A_7C15;
        h ^= h >> 16;
        h = h.wrapping_mul(0x85EB_CA6B);
        (h >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut left = spare;
    for (g, list) in members.iter().enumerate() {
        if list.len() < BIG || left == 0 {
            continue;
        }
        let extra = ((spare as f64 * list.len() as f64 / total as f64) as usize).min(7).min(list.len() / 8).min(left);
        if extra == 0 {
            continue;
        }
        left -= extra;
        // Farthest apart, from the group's own representative.
        let mut chosen = vec![reps[g]];
        let mut far: Vec<f32> = list.iter().map(|&i| apart(&descs[i as usize], &descs[reps[g] as usize])).collect();
        for _ in 0..extra {
            let Some((k, _)) = far.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else { break };
            let pick = list[k];
            chosen.push(pick);
            for (j, &i) in list.iter().enumerate() {
                far[j] = far[j].min(apart(&descs[i as usize], &descs[pick as usize]));
            }
        }
        let ids: Vec<u32> = std::iter::once(g as u32)
            .chain(chosen[1..].iter().map(|&pick| {
                reps.push(pick);
                (reps.len() - 1) as u32
            }))
            .collect();
        // How far apart the versions are, so near ties are told apart by place alone.
        let scale = chosen.iter().map(|&a| apart(&descs[a as usize], &descs[chosen[0] as usize])).fold(1e-6f32, f32::max);
        for &i in list {
            let d = |c: u32| apart(&descs[i as usize], &descs[c as usize]) / scale;
            let best = (0..chosen.len()).min_by(|&a, &b| {
                let (da, db) = (d(chosen[a]) + 0.15 * jitter(i ^ (a as u32).wrapping_mul(0x632B_E5AB)), d(chosen[b]) + 0.15 * jitter(i ^ (b as u32).wrapping_mul(0x632B_E5AB)));
                da.total_cmp(&db)
            });
            assign[i as usize] = ids[best.unwrap_or(0)];
        }
    }
    (assign, reps)
}

/// Tiles in order, each joining the most alike group within `tol` of it (looked for among groups
/// of a similar mean colour), or starting one. Gives up past `cap` groups.
fn greedy(descs: &[Desc], tol: Tolerance, cap: usize) -> (Vec<u32>, Vec<u32>) {
    let mut buckets: FxHashMap<(u16, i32, i32, i32), Vec<u32>> = FxHashMap::default();
    let mut reps: Vec<u32> = Vec::new();
    let mut assign = vec![0u32; descs.len()];
    for (i, d) in descs.iter().enumerate() {
        let k = tol.key(d);
        let mut found = None;
        let mut best = f32::MAX;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = buckets.get(&(k.0, k.1 + dx, k.2 + dy, k.3 + dz)) {
                        for &g in list {
                            if let Some(e) = tol.alike(d, &descs[reps[g as usize] as usize])
                                && e < best
                            {
                                best = e;
                                found = Some(g);
                            }
                        }
                    }
                }
            }
        }
        assign[i] = match found {
            Some(g) => g,
            None => {
                let g = reps.len() as u32;
                reps.push(i as u32);
                buckets.entry(k).or_default().push(g);
                if reps.len() > cap {
                    return (assign, reps);
                }
                g
            }
        };
    }
    (assign, reps)
}

/// A 4 x 4 Bayer matrix: the order the pattern dithering takes its sixteen entries in.
pub const BAYER: [usize; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// The nearest palette entry (by OKLab) to every 6-bit-per-channel colour, using entries
/// `allowed` only.
pub struct PaletteLut {
    lut: Vec<u8>,
    pub colors: [[u8; 3]; 256],
}

impl PaletteLut {
    pub fn new(colors: [[u8; 3]; 256], allowed: impl Fn(usize) -> bool) -> PaletteLut {
        Self::new_in::<Exact>(colors, allowed)
    }

    /// [`PaletteLut::new`] with the colours taken to OKLab as `C` takes them.
    pub fn new_in<C: Colour>(colors: [[u8; 3]; 256], allowed: impl Fn(usize) -> bool) -> PaletteLut {
        let entries: Vec<(usize, [f32; 3])> = (0..256).filter(|&i| allowed(i)).map(|i| (i, C::oklab(colors[i].map(|v| v as f32 / 255.0)))).collect();
        let lut = (0..64 * 64 * 64usize)
            .into_par_iter()
            .map(|k| {
                let c = [(k >> 12) & 63, (k >> 6) & 63, k & 63].map(|v| (v as f32 * 4.0 + 2.0) / 255.0);
                let l = C::oklab(c);
                let mut best = (f32::MAX, 0usize);
                for &(i, e) in &entries {
                    let d = (l[0] - e[0]).powi(2) + (l[1] - e[1]).powi(2) * 1.4 + (l[2] - e[2]).powi(2) * 1.4;
                    if d < best.0 {
                        best = (d, i);
                    }
                }
                best.1 as u8
            })
            .collect();
        PaletteLut { lut, colors }
    }

    #[inline]
    pub fn nearest(&self, c: [f32; 3]) -> u8 {
        let q = c.map(|v| ((v * 255.0).clamp(0.0, 255.0) as usize) >> 2);
        self.lut[(q[0] << 12) | (q[1] << 6) | q[2]]
    }

    /// A tile's colours (sRGB, 0 to 1) dithered into the palette by pattern dithering (Knoll's):
    /// each colour becomes a mix of sixteen palette entries whose average is that colour, ordered
    /// by lightness, and each pixel takes the entry its place in a 4 x 4 Bayer matrix picks. Every
    /// pixel is chosen alone, so a tile keeps its colour exactly on average (error diffusion lost
    /// some of it, differently tile by tile, which showed as squares) and the pattern runs on
    /// across tile edges, whichever tiles meet there.
    pub fn dither(&self, pixels: &[[f32; 3]]) -> [u8; 32 * 32] {
        let mut out = [0u8; 32 * 32];
        for y in 0..32 {
            for x in 0..32 {
                out[y * 32 + x] = self.pattern(pixels[y * 32 + x], x, y);
            }
        }
        out
    }

    /// The palette entry the pattern dithering (see `dither`) gives colour `c` (sRGB, 0 to 1) at
    /// pixel (`x`, `y`): chosen by its colour and its place alone, so a pixel of a picture that
    /// stays as it was keeps its entry (a twinkling painting's frames).
    pub fn pattern(&self, c: [f32; 3], x: usize, y: usize) -> u8 {
        let lightness = |i: u8| {
            let c = self.colors[i as usize];
            0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
        };
        let mut mix = [0u8; 16];
        let mut error = [0.0f32; 3];
        for slot in &mut mix {
            let attempt = [c[0] + error[0] * 0.75, c[1] + error[1] * 0.75, c[2] + error[2] * 0.75];
            let idx = self.nearest(attempt);
            *slot = idx;
            let p = self.colors[idx as usize];
            for k in 0..3 {
                error[k] += c[k] - p[k] as f32 / 255.0;
            }
        }
        mix.sort_by(|&a, &b| lightness(a).total_cmp(&lightness(b)));
        mix[BAYER[(y % 4) * 4 + x % 4]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alike_tiles_share_and_different_ones_do_not() {
        let flat = |c: [f32; 3]| describe(&vec![c; 1024]);
        let descs = vec![flat([0.3, 0.5, 0.2]), flat([0.301, 0.5, 0.2]), flat([0.8, 0.2, 0.2]), flat([0.3, 0.5, 0.201])];
        let (assign, reps, _) = group(&descs, &[0.0; 4], 4, WHOLE_SHARE);
        assert_eq!(reps.len(), 2);
        assert_eq!(assign[0], assign[1]);
        assert_eq!(assign[0], assign[3]);
        assert_ne!(assign[0], assign[2]);
    }

    #[test]
    fn only_tiles_of_one_phase_share() {
        // Four plain tiles alike in every way but their phase: two groups, by phase.
        let d = describe(&vec![[0.4, 0.4, 0.4]; 1024]);
        let descs: Vec<Desc> = [1u16, 2, 1, 2].iter().map(|&phase| Desc { phase, ..d }).collect();
        let (assign, reps, _) = group(&descs, &[0.0; 4], 4, WHOLE_SHARE);
        assert_eq!(assign[0], assign[2]);
        assert_eq!(assign[1], assign[3]);
        assert_ne!(assign[0], assign[1]);
        assert!(reps.iter().all(|&r| descs[r as usize].phase == descs[assign.iter().position(|&a| reps[a as usize] == r).unwrap()].phase));
    }

    #[test]
    fn tiles_of_rock_are_kept_whole() {
        // Six tiles alike in their description; the two of rock keep their own.
        let descs: Vec<Desc> = (0..6).map(|i| describe(&vec![[0.4 + i as f32 * 1e-4, 0.4, 0.4]; 1024])).collect();
        let rock = [0.0, 0.5, 0.0, 0.3, 0.0, 0.0];
        let (assign, reps, _) = group(&descs, &rock, 6, WHOLE_SHARE);
        assert_eq!(reps[assign[1] as usize], 1);
        assert_eq!(reps[assign[3] as usize], 3);
        assert!([0, 2, 4, 5].iter().all(|&i| assign[i] == assign[0]), "{assign:?}");
    }

    #[test]
    fn a_big_uniform_group_gets_versions_mixed_through_it() {
        // 400 tiles alike but for their grain: one group, then versions sharing it out.
        let descs: Vec<Desc> = (0..400).map(|i| describe(&vec![[0.4 + (i % 7) as f32 * 1e-4, 0.4, 0.4]; 1024])).collect();
        let (assign, reps, _) = group(&descs, &vec![0.0; descs.len()], 10, WHOLE_SHARE);
        assert!(reps.len() > 1 && reps.len() <= 10, "{}", reps.len());
        // Neighbours (in order) are not all the same tile.
        let same = assign.windows(2).filter(|w| w[0] == w[1]).count();
        assert!(same < 300, "{same} of 399 neighbours repeat");
    }

    #[test]
    fn dithering_keeps_a_tiles_colour_on_average() {
        // A palette of greys 16 apart: a grey between two of them comes out as their mix.
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            let v = ((i / 16) * 16) as u8;
            *c = [v, v, v];
        }
        let lut = PaletteLut::new(colors, |_| true);
        for grey in [0.31f32, 0.47, 0.52, 0.71] {
            let tile = lut.dither(&vec![[grey; 3]; 1024]);
            let mean = tile.iter().map(|&i| colors[i as usize][0] as f32 / 255.0).sum::<f32>() / 1024.0;
            assert!((mean - grey).abs() < 0.012, "{grey}: {mean}");
        }
    }

    /// The mean step in lightness across the edges between the tiles `shown` at the places of a
    /// map `width` tiles wide (`assign`).
    fn edge_steps(shown: &[Desc], assign: &[u32], width: usize) -> f32 {
        let rows = assign.len() / width;
        let (mut sum, mut n) = (0.0f32, 0.0f32);
        for z in 0..rows {
            for x in 0..width {
                let a = &shown[assign[z * width + x] as usize];
                for (next, side, opp) in [((x + 1 < width).then(|| z * width + x + 1), EAST, WEST), ((z + 1 < rows).then(|| (z + 1) * width + x), SOUTH, NORTH)] {
                    let Some(j) = next else { continue };
                    let b = &shown[assign[j] as usize];
                    for s in 0..4 {
                        sum += (a.edge[(side * 4 + s) * 3] - b.edge[(opp * 4 + s) * 3]).abs();
                        n += 1.0;
                    }
                }
            }
        }
        sum / n.max(1.0)
    }

    #[test]
    fn stitching_hides_the_tile_grid() {
        // Ground whose colour drifts smoothly, with grain, 40 x 40 tiles shared down to 80.
        let width = 40;
        let paint = |t: usize| -> Vec<[f32; 3]> {
            (0..1024)
                .map(|i| {
                    let (x, z) = (((t % width) * 32 + i % 32) as f32, ((t / width) * 32 + i / 32) as f32);
                    let grain = ((x * 12.9898 + z * 78.233).sin() * 43_758.547).fract() * 0.03;
                    let drift = (x / 300.0).sin() * 0.06 + (z / 410.0).cos() * 0.05;
                    [0.42 + drift + grain, 0.36 + drift * 0.8 + grain, 0.28 + drift * 0.5 + grain]
                })
                .collect()
        };
        let painted: Vec<Vec<[f32; 3]>> = (0..width * width).map(paint).collect();
        let descs: Vec<Desc> = painted.iter().map(|p| describe(p)).collect();
        let (mut assign, reps, k) = group(&descs, &vec![0.0; descs.len()], 80, WHOLE_SHARE);
        let mut kept: Vec<Vec<[f32; 3]>> = reps.iter().map(|&r| painted[r as usize].clone()).collect();
        let own: Vec<u32> = (0..descs.len() as u32).collect();
        let painting = edge_steps(&descs, &own, width);
        let before = edge_steps(&kept.iter().map(|p| describe(p)).collect::<Vec<_>>(), &assign, width);
        for _ in 0..4 {
            let shown: Vec<Desc> = kept.iter().map(|p| describe(p)).collect();
            stitch(&descs, &shown, &vec![0.0; descs.len()], &vec![0.0; shown.len()], &mut assign, width, k);
            for (p, d) in kept.iter_mut().zip(settle(&descs, &shown, &assign)) {
                shift(p, &d);
            }
        }
        let after = edge_steps(&kept.iter().map(|p| describe(p)).collect::<Vec<_>>(), &assign, width);
        assert!(after < before * 0.75 && after < painting * 1.6, "painting {painting:.5}, shared {before:.5}, stitched {after:.5}");
        // Compacted: the tiles shown, each once, numbered from 0.
        let distinct = assign.iter().collect::<std::collections::BTreeSet<_>>().len();
        let order = compact(&mut assign, kept.len());
        assert_eq!(order.len(), distinct);
        assert!(assign.iter().all(|&g| (g as usize) < order.len()));
    }

    #[test]
    fn plain_ground_takes_no_tile_of_strata() {
        // Plain places, and two tiles alike in their description (a cliff's, kept whole, its
        // strata averaged away): repeating the plain one costs, so the places would take turns
        // with the cliff's (as the strata's own places, here none, still may).
        let plain = describe(&vec![[0.4, 0.4, 0.4]; 1024]);
        let descs = vec![plain; 16];
        let shown = [plain, plain];
        let mut assign = vec![0u32; 16];
        let mut free = vec![0u32; 16];
        stitch(&descs, &shown, &[0.0; 16], &[0.0, 0.0], &mut free, 4, 4.0);
        assert!(free.contains(&1), "{free:?}");
        stitch(&descs, &shown, &[0.0; 16], &[0.0, 0.8], &mut assign, 4, 4.0);
        assert!(assign.iter().all(|&g| g == 0), "{assign:?}");
    }

    #[test]
    fn shifting_moves_a_tiles_edges() {
        let pixels: Vec<[f32; 3]> = (0..1024).map(|i| [0.3 + (i % 32) as f32 * 0.004, 0.25 + (i / 32) as f32 * 0.003, 0.2]).collect();
        let before = describe(&pixels);
        // Lighter along the second stretch of the east side.
        let mut delta = [0.0f32; 48];
        delta[(EAST * 4 + 1) * 3] = 0.02;
        let mut shifted = pixels.clone();
        shift(&mut shifted, &delta);
        let after = describe(&shifted);
        let east = (EAST * 4 + 1) * 3;
        assert!(after.edge[east] - before.edge[east] > 0.012, "{} -> {}", before.edge[east], after.edge[east]);
        // The west side and the tile's middle stay as they were.
        let west = (WEST * 4 + 1) * 3;
        assert!((after.edge[west] - before.edge[west]).abs() < 1e-4);
        // (Block (1, 1): the second block of the second row.)
        let middle = 5 * 3;
        assert!((after.layout[middle] + after.mean[0] - before.layout[middle] - before.mean[0]).abs() < 1e-4);
        // OKLab there and back.
        for c in [[0.0f32, 0.0, 0.0], [1.0, 1.0, 1.0], [0.8, 0.3, 0.1], [0.05, 0.2, 0.6]] {
            let back = srgb(oklab(c));
            assert!((0..3).all(|k| (back[k] - c[k]).abs() < 1e-3), "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn oklab_of_white_is_one() {
        let w = oklab([1.0, 1.0, 1.0]);
        assert!((w[0] - 1.0).abs() < 1e-3 && w[1].abs() < 1e-3 && w[2].abs() < 1e-3);
    }

    /// The portable colour maths agrees with the platform's to within its own rounding, and goes
    /// there and back; its roots are exact on exact powers.
    #[test]
    fn portable_colours_agree_with_the_exact_ones() {
        for r in 0..=16 {
            for g in 0..=16 {
                for b in 0..=16 {
                    let c = [r as f32 / 16.0, g as f32 / 16.0, b as f32 / 16.0];
                    let (e, p) = (Exact::oklab(c), Portable::oklab(c));
                    assert!((0..3).all(|k| (e[k] - p[k]).abs() < 2e-6), "{c:?}: {e:?} against {p:?}");
                    let back = Portable::srgb(p);
                    assert!((0..3).all(|k| (back[k] - c[k]).abs() < 1e-4), "{c:?} -> {back:?}");
                    let es = Exact::srgb(e);
                    assert!((0..3).all(|k| (back[k] - es[k]).abs() < 2e-4), "{c:?}: {back:?} against {es:?}");
                }
            }
        }
        assert_eq!(root(27.0, 3), 3.0);
        assert_eq!(root(-8.0, 3), -2.0);
        assert_eq!(root(32.0, 5), 2.0);
        assert_eq!(root(0.0, 3), 0.0);
        assert!((root(1e-9, 3) - 1e-3).abs() < 1e-15);
    }
}
