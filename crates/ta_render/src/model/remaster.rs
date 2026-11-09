//! Reborn's paint: each of TA's unit textures redrawn at 128 texels a side from the player's own
//! install, when the game loads (nothing of it is stored or shipped).
//!
//! TA painted its textures small (most are 32 × 32) and dithered, and it painted their light in:
//! the same pattern comes lit and in shade (ArmCam3a to 3d, CorV04b to 04d), the faces turned
//! to the light given the bright one. Reborn lights its models itself, so a family's darker
//! variants are first brought up towards its brighter ones (`Source::gain`). Then the palette's
//! noise goes, its painted detail kept: flecks of other hues (pink, green and blue in a khaki or
//! a grey) take the hue round them, lone texels unlike all but one of their neighbours the
//! median round them (lines a texel wide, lamps and clusters of two texels and more stay), and
//! the dots of an ordered dither the mean of themselves and the shade round them. An
//! edge-keeping smoothing mixes the rest of the dither as the eye does, hues more than light,
//! and light more on neutral metal than on coloured paint (whose camouflage it would wash out).
//! Then the texture is enlarged by palette snapping: a smooth enlargement gives each new texel
//! its rough colour, and that colour is drawn towards the nearest of the colours round it.
//! Edges come out crisp, thin panel lines survive, and the colours stay the artist's. TA's
//! mottled paints (its camouflages and noise) are then refinished: their blotches calmed into a
//! factory paint, their lines and their broad shades kept (`refinished`). The relief of the
//! paint (its panel lines and recesses) becomes a normal map.

use ta_formats::image::{IndexedImage, Palette};

pub(super) const SIZE: u32 = 128;
/// TA painted its brightest metal with the light in it (chrome's sheen, white panels): lit by
/// Reborn as well, it would blow out to blank white. Above this luminance (linear) the paint's
/// light rises only by this share: TA's white comes out about as bright as sRGB 220.
const SHOULDER: (f32, f32) = (0.5, 0.42);

fn shouldered(c: [f32; 3]) -> [f32; 3] {
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    if l <= SHOULDER.0 {
        return c;
    }
    let k = (SHOULDER.0 + (l - SHOULDER.0) * SHOULDER.1) / l;
    c.map(|v| v * k)
}

/// How bright the players' colours are drawn (linear), of TA's paint: TA picked them for its own
/// light, near the top of the range; lit by Reborn's sun they would sit on the tone curve's
/// shoulder, every face the same flat electric blue. A little lower, the light shapes them.
const TEAM_GAIN: f32 = 0.78;
/// A tread's links (linear): the steel in their gaps, and on their worn tops.
const TREAD_STEEL: ([f32; 3], [f32; 3]) = ([0.012, 0.012, 0.011], [0.2, 0.19, 0.175]);
pub(super) const MIP_COUNT: u32 = 8;

/// A texture frame to remaster.
pub(super) struct Source<'a> {
    pub image: &'a IndexedImage,
    /// How much brighter to make it (linear light): towards its family's lit variants.
    pub gain: f32,
    pub roughness: f32,
    pub metallic: f32,
    /// Team colours, lights and animations: their paint kept exactly, flat, never smoothed.
    pub protected: bool,
    /// A player's colour: a little darker than TA painted it (`TEAM_GAIN`).
    pub team: bool,
    /// A track's tread (TA's `Tredside`): its links in the dark steel of Reborn's tracks, as on
    /// the sides the kit rebuilds (TA painted them a pale blue).
    pub tread: bool,
    /// One of TA's mottled paints (`mottled`): refinished (`refinished`).
    pub mottled: bool,
}

/// Whether a texture is one of TA's mottled paints, by its name: its camouflages (`ArmCam3a`,
/// `CoCam10a`, `Juncam01`, `descamo2`, `camob3`) and its noise (`graynoise3`, `noise6a`).
pub(super) fn mottled(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("cam") || name.contains("noise")
}

pub(super) struct SurfaceMaps {
    /// sRGB RGB, linear occlusion in alpha (1: the paint's own shadows are in its colour);
    /// largest mip first.
    pub color: Vec<Vec<u8>>,
    /// Linear: RG = tangent-space normal XY (128 flat), B = roughness, A = metallic.
    pub physical: Vec<Vec<u8>>,
}

type Lab = [f32; 3];

fn to_lab(c: [f32; 3]) -> Lab {
    let l = 0.412_221_47 * c[0] + 0.536_332_55 * c[1] + 0.051_445_99 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (l.max(0.0).cbrt(), m.max(0.0).cbrt(), s.max(0.0).cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_lab(c: Lab) -> [f32; 3] {
    let l = c[0] + 0.396_337_78 * c[1] + 0.215_803_76 * c[2];
    let m = c[0] - 0.105_561_346 * c[1] - 0.063_854_17 * c[2];
    let s = c[0] - 0.089_484_18 * c[1] - 1.291_485_5 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        (4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s).clamp(0.0, 1.0),
        (-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s).clamp(0.0, 1.0),
        (-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s).clamp(0.0, 1.0),
    ]
}

fn dist2(a: Lab, b: Lab) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// A grid of colours with its size; reads past the edge take the edge's (TA's textures are
/// stretched over one face each, never tiled).
struct Grid<T> {
    w: usize,
    h: usize,
    data: Vec<T>,
}

impl<T: Copy> Grid<T> {
    fn at(&self, x: isize, y: isize) -> T {
        self.data[y.clamp(0, self.h as isize - 1) as usize * self.w + x.clamp(0, self.w as isize - 1) as usize]
    }
}

/// A texel's eight neighbours.
fn around(grid: &Grid<Lab>, x: isize, y: isize) -> [Lab; 8] {
    let mut out = [[0.0f32; 3]; 8];
    let mut k = 0;
    for dy in -1..=1isize {
        for dx in -1..=1isize {
            if dx != 0 || dy != 0 {
                out[k] = grid.at(x + dx, y + dy);
                k += 1;
            }
        }
    }
    out
}

/// The median of each channel of `values`.
fn median<const N: usize>(values: [Lab; N]) -> Lab {
    std::array::from_fn(|i| {
        let mut v = values.map(|c| c[i]);
        v.sort_by(f32::total_cmp);
        if N % 2 == 1 { v[N / 2] } else { (v[N / 2 - 1] + v[N / 2]) * 0.5 }
    })
}

/// The palette's specks out of TA's paint, which enlarged would be blots, its painted details
/// kept. Flecks of other hues (pink, green and blue in a camouflage's khaki or a metal's grey):
/// a texel of a hue at most one of its neighbours shares, far from the hue round it, at about
/// the light round it and not vivid (not a lamp, which stands out brighter or in a strong
/// colour), takes the hue round it (its neighbours' median) and keeps its light. Then lone
/// texels, unlike all but one of their neighbours yet not far from them (noise, not a painted
/// light or rivet), take the median of the median across them, the median along the diagonals
/// and themselves (a hybrid median): the specks go, and lines a texel wide, straight or
/// stepped, stay, as do clusters of two texels and more.
fn despeckle(grid: &Grid<Lab>) -> Grid<Lab> {
    // How alike two hues are (chroma distance), how near the light round it a fleck is, and how
    // vivid a lamp is.
    const ALIKE: f32 = 0.05;
    const LIGHT: f32 = 0.1;
    const VIVID: f32 = 0.09;
    // How alike two colours are, and how far from the median round it a lone texel may be.
    const LONE: f32 = 0.06;
    const NOISE: f32 = 0.15;
    let hues = Grid {
        w: grid.w,
        h: grid.h,
        data: (0..grid.h)
            .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
            .map(|(x, y)| {
                let here = grid.at(x, y);
                let near = around(grid, x, y);
                let alike = near.iter().filter(|c| (c[1] - here[1]).hypot(c[2] - here[2]) < ALIKE).count();
                let m = median(near);
                let fleck = alike <= 1 && (m[1] - here[1]).hypot(m[2] - here[2]) > 2.0 * ALIKE && (here[0] - m[0]).abs() < LIGHT && here[1].hypot(here[2]) < VIVID;
                if fleck { [here[0], m[1], m[2]] } else { here }
            })
            .collect(),
    };
    let data = (0..hues.h)
        .flat_map(|y| (0..hues.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| {
            let here = hues.at(x, y);
            if around(&hues, x, y).iter().filter(|&&c| dist2(c, here) < LONE * LONE).count() > 1 {
                return here;
            }
            let across = median([here, hues.at(x - 1, y), hues.at(x + 1, y), hues.at(x, y - 1), hues.at(x, y + 1)]);
            let diagonal = median([here, hues.at(x - 1, y - 1), hues.at(x + 1, y - 1), hues.at(x - 1, y + 1), hues.at(x + 1, y + 1)]);
            let m = median([across, diagonal, here]);
            if dist2(m, here) < NOISE * NOISE { m } else { here }
        })
        .collect();
    Grid { w: hues.w, h: hues.h, data }
}

/// TA's ordered dither out (rows of dots through its metals' gradients, a checkerboard of two
/// shades): a texel unlike its two neighbours one way, which are alike, and not continued the
/// other way (a line a texel wide is) takes the mean of itself and them, as the eye mixes them.
fn dedither(grid: &Grid<Lab>) -> Grid<Lab> {
    // How alike two colours are, and how far a dot differs.
    const ALIKE: f32 = 0.05;
    const DIFFER: f32 = 0.02;
    let near = |a: Lab, b: Lab| dist2(a, b) < ALIKE * ALIKE;
    let apart = |a: Lab, b: Lab| dist2(a, b) > DIFFER * DIFFER;
    let mean = |values: &[Lab]| -> Lab { std::array::from_fn(|i| values.iter().map(|c| c[i]).sum::<f32>() / values.len() as f32) };
    let data = (0..grid.h)
        .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| {
            let here = grid.at(x, y);
            let (left, right, up, down) = (grid.at(x - 1, y), grid.at(x + 1, y), grid.at(x, y - 1), grid.at(x, y + 1));
            let across = near(left, right) && apart(left, here) && apart(right, here) && !near(up, here) && !near(down, here);
            let along = near(up, down) && apart(up, here) && apart(down, here) && !near(left, here) && !near(right, here);
            match (across, along) {
                (true, true) => mean(&[here, here, left, right, up, down]),
                (true, false) => mean(&[here, here, left, right]),
                (false, true) => mean(&[here, here, up, down]),
                (false, false) => here,
            }
        })
        .collect();
    Grid { w: grid.w, h: grid.h, data }
}

/// An edge-keeping smoothing (bilateral): each texel averages with its neighbours as far as
/// they are alike, so the dither goes and the edges between colours stay. Alike in their light
/// above all: the palette dithers hues together (a sandstone of khaki, pink and yellow texels),
/// and those it mixes as the eye does, while the light and shade TA painted (camouflage, panel
/// lines, mortar) stay. On neutral paint (TA's metals, their gradients dithered) it reaches
/// further in light than on coloured paint, whose camouflage it would wash out.
fn smooth(grid: &Grid<Lab>) -> Grid<Lab> {
    // The range of light (Oklab) on neutral paint and on coloured paint, and the chroma round a
    // texel from which it counts as coloured; and the range of hue.
    const NEUTRAL: f32 = 0.06;
    const COLOURED: f32 = 0.032;
    const CHROMA: (f32, f32) = (0.02, 0.05);
    const HUE: f32 = 0.11;
    let range: Vec<f32> = (0..grid.h)
        .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| {
            let chroma = |c: Lab| c[1].hypot(c[2]);
            let local = (around(grid, x, y).iter().map(|&c| chroma(c)).sum::<f32>() + chroma(grid.at(x, y))) / 9.0;
            let t = ((local - CHROMA.0) / (CHROMA.1 - CHROMA.0)).clamp(0.0, 1.0);
            NEUTRAL + (COLOURED - NEUTRAL) * t * t * (3.0 - 2.0 * t)
        })
        .collect();
    let mut cur = grid.data.clone();
    for _ in 0..2 {
        let src = Grid { w: grid.w, h: grid.h, data: cur };
        cur = (0..grid.h)
            .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
            .map(|(x, y)| {
                let here = src.at(x, y);
                let r = range[y as usize * grid.w + x as usize];
                let (mut acc, mut total) = ([0.0f32; 3], 0.0f32);
                for dy in -1..=1isize {
                    for dx in -1..=1isize {
                        let c = src.at(x + dx, y + dy);
                        let light = (c[0] - here[0]).powi(2) / (2.0 * r * r);
                        let hue = ((c[1] - here[1]).powi(2) + (c[2] - here[2]).powi(2)) / (2.0 * HUE * HUE);
                        let w = (-((dx * dx + dy * dy) as f32) * 0.5 - light - hue).exp();
                        for k in 0..3 {
                            acc[k] += c[k] * w;
                        }
                        total += w;
                    }
                }
                acc.map(|v| v / total)
            })
            .collect();
    }
    Grid { w: grid.w, h: grid.h, data: cur }
}

/// TA's metal ramps (a barrel's or a pod's chrome): bands of the palette's greys stepping one
/// way, each a few texels wide, which palette snapping would draw back into its steps, stripes
/// each with a ridge in the relief. Where the light round a texel only climbs or only falls
/// (along either way, nine texels), in steps no bigger than one or two of the palette's greys,
/// on both sides of it along one of them (a ramp goes on; the edge of a patch of paint is one
/// step, flat beyond), and the paint is neutral (not a camouflage's or a sign's colour), it is
/// a ramp: the bands are evened out (each texel the mean of the 5 x 5 round it) and the
/// enlargement keeps its smooth colour there. A line a texel wide (it falls and climbs back),
/// an edge (a big step) and flat paint are none. The grid evened out, and where it ramps.
fn deband(grid: &Grid<Lab>) -> (Grid<Lab>, Vec<f32>) {
    // Steps a ramp may take (Oklab lightness), a reversal it may not, how far it must go on
    // each side of a texel, and how neutral its paint is.
    const STEP: f32 = 0.07;
    const BACK: f32 = 0.004;
    const RISE: f32 = 0.012;
    const NEUTRAL: f32 = 0.05;
    let way = |x: isize, y: isize, dx: isize, dy: isize| -> (bool, bool) {
        let l: [f32; 9] = std::array::from_fn(|k| grid.at(x + dx * (k as isize - 4), y + dy * (k as isize - 4))[0]);
        let d: [f32; 8] = std::array::from_fn(|k| l[k + 1] - l[k]);
        let monotonic = d.iter().all(|&v| v >= -BACK) || d.iter().all(|&v| v <= BACK);
        let calm = monotonic && d.iter().all(|v| v.abs() <= STEP);
        (calm, calm && (l[4] - l[0]).abs() >= RISE && (l[8] - l[4]).abs() >= RISE)
    };
    // (Calm: a ramp may pass here; going on: it does, both sides. A band's first and last texels
    // see one side flat: calm texels near one where it goes on are the ramp too.)
    let (calm, going): (Vec<bool>, Vec<bool>) = (0..grid.h)
        .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| {
            let here = grid.at(x, y);
            let ((calm_x, rise_x), (calm_y, rise_y)) = (way(x, y, 1, 0), way(x, y, 0, 1));
            let calm = calm_x && calm_y && here[1].hypot(here[2]) < NEUTRAL;
            (calm, calm && (rise_x || rise_y))
        })
        .unzip();
    let going = Grid { w: grid.w, h: grid.h, data: going };
    let ramp: Vec<f32> = (0..grid.h)
        .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| (calm[y as usize * grid.w + x as usize] && (-2..=2isize).any(|dy| (-2..=2isize).any(|dx| going.at(x + dx, y + dy)))) as u8 as f32)
        .collect();
    let data = (0..grid.h)
        .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| {
            if ramp[y as usize * grid.w + x as usize] == 0.0 {
                return grid.at(x, y);
            }
            let mut acc = [0.0f32; 3];
            for dy in -2..=2isize {
                for dx in -2..=2isize {
                    let c = grid.at(x + dx, y + dy);
                    for k in 0..3 {
                        acc[k] += c[k] / 25.0;
                    }
                }
            }
            acc
        })
        .collect();
    (Grid { w: grid.w, h: grid.h, data }, ramp)
}

/// A feature's painting in TA's palette (a tree, a rock, a ruin) calmed as the units' paint is
/// before it is enlarged, its size kept: flecks of other hues take the hue round them (see
/// `despeckle`), and an edge-keeping smoothing mixes the rest of the dither as the eye does,
/// hues more than light (see `smooth`), so magnified (TA's camera zoomed in, a ruin before a
/// low camera) it is shading and colour, not static of palette colours. Its painted texels
/// (alpha 128 or more) only, among themselves. `rgba`: sRGB.
pub(crate) fn calm_sprite(w: usize, h: usize, rgba: &mut [u8]) {
    use rayon::prelude::*;
    // (As the units' paint, but further: a painting's dither is its noise, not a camouflage.)
    const ALIKE: f32 = 0.05;
    const LIGHT: f32 = 0.1;
    const VIVID: f32 = 0.09;
    const RANGE: (f32, f32) = (0.07, 0.05);
    const CHROMA: (f32, f32) = (0.02, 0.05);
    const HUE: f32 = 0.2;
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return;
    }
    let lut = crate::util::srgb_to_linear_lut();
    let painted: Vec<bool> = (0..w * h).map(|i| rgba[i * 4 + 3] >= 128).collect();
    let lab: Vec<Lab> = (0..w * h).map(|i| to_lab([0, 1, 2].map(|k| lut[rgba[i * 4 + k] as usize]))).collect();
    // A texel's painted neighbours (their indices, and how many), and itself last.
    let around = |i: usize| -> ([usize; 9], usize) {
        let (x, y) = (i % w, i / w);
        let mut out = [0usize; 9];
        let mut n = 0;
        for ny in y.saturating_sub(1)..(y + 2).min(h) {
            for nx in x.saturating_sub(1)..(x + 2).min(w) {
                let j = ny * w + nx;
                if j != i && painted[j] {
                    out[n] = j;
                    n += 1;
                }
            }
        }
        out[n] = i;
        (out, n)
    };
    // Flecks of other hues, and how far in light the smoothing reaches round each texel.
    let (lab, range): (Vec<Lab>, Vec<f32>) = (0..w * h)
        .into_par_iter()
        .with_min_len(256)
        .map(|i| {
            let here = lab[i];
            if !painted[i] {
                return (here, RANGE.0);
            }
            let (near, n) = around(i);
            let near = &near[..n];
            let chroma = (near.iter().map(|&j| lab[j][1].hypot(lab[j][2])).sum::<f32>() + here[1].hypot(here[2])) / (n + 1) as f32;
            let t = ((chroma - CHROMA.0) / (CHROMA.1 - CHROMA.0)).clamp(0.0, 1.0);
            let range = RANGE.0 + (RANGE.1 - RANGE.0) * t * t * (3.0 - 2.0 * t);
            if n < 5 {
                return (here, range);
            }
            let alike = near.iter().filter(|&&j| (lab[j][1] - here[1]).hypot(lab[j][2] - here[2]) < ALIKE).count();
            let middle = |k: usize| {
                let mut v = [0.0f32; 8];
                for (slot, &j) in v.iter_mut().zip(near) {
                    *slot = lab[j][k];
                }
                v[..n].sort_by(f32::total_cmp);
                v[n / 2]
            };
            let m = [middle(0), middle(1), middle(2)];
            let fleck = alike <= 2 && (m[1] - here[1]).hypot(m[2] - here[2]) > 2.0 * ALIKE && (here[0] - m[0]).abs() < LIGHT && here[1].hypot(here[2]) < VIVID;
            (if fleck { [here[0], m[1], m[2]] } else { here }, range)
        })
        .unzip();
    // The edge-keeping smoothing.
    let mut lab = lab;
    for _ in 0..3 {
        let src = lab;
        lab = (0..w * h)
            .into_par_iter()
            .with_min_len(256)
            .map(|i| {
                let here = src[i];
                if !painted[i] {
                    return here;
                }
                let (near, n) = around(i);
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                let (mut acc, mut total) = ([0.0f32; 3], 0.0f32);
                for &j in &near[..=n] {
                    let c = src[j];
                    let (dx, dy) = ((j % w) as isize - x, (j / w) as isize - y);
                    let light = (c[0] - here[0]).powi(2) / (2.0 * range[i] * range[i]);
                    let hue = ((c[1] - here[1]).powi(2) + (c[2] - here[2]).powi(2)) / (2.0 * HUE * HUE);
                    let weight = (-((dx * dx + dy * dy) as f32) * 0.5 - light - hue).exp();
                    for k in 0..3 {
                        acc[k] += c[k] * weight;
                    }
                    total += weight;
                }
                acc.map(|v| v / total)
            })
            .collect();
    }
    for i in (0..w * h).filter(|&i| painted[i]) {
        let c = from_lab(lab[i]);
        for k in 0..3 {
            rgba[i * 4 + k] = crate::util::linear_to_srgb(c[k]);
        }
    }
}

/// TA's grey metal, its paint lit as TA lit it: a palette with few true greys, whose light greys
/// lean lavender, and blotches of painted sheen (the Arm ships' hulls, light plates of it, read
/// as lavender marble or ice once Reborn's sun and sky light them too). Paint that is grey on
/// the whole with that cool cast (its mean chroma low, its mean hue towards blue; `n` texels a
/// side) loses it, its cool texels' colour by a quarter and its lighter ones' (above its mean
/// light) up to two thirds, and its soft painted sheen (what of it a blur over a twenty-fourth
/// of it rises above the mean) by half; its fine detail (rivets, lines, edges) stays.
/// Camouflage (olive, khaki: warm), signs and coloured metals keep their colours.
fn true_greys(colour: Vec<[f32; 3]>, n: usize) -> Vec<[f32; 3]> {
    const GREY: (f32, f32) = (0.04, 0.07);
    const COOL: f32 = 0.012;
    const SHEEN: f32 = 0.5;
    let lab: Vec<Lab> = colour.iter().map(|&c| to_lab(c)).collect();
    let count = lab.len().max(1) as f32;
    let chroma = lab.iter().map(|c| c[1].hypot(c[2])).sum::<f32>() / count;
    let cool = (-lab.iter().map(|c| c[2]).sum::<f32>() / count / COOL).clamp(0.0, 1.0);
    let grey = (1.0 - ((chroma - GREY.0) / (GREY.1 - GREY.0)).clamp(0.0, 1.0)) * cool;
    if grey <= 0.0 || lab.len() != n * n {
        return colour;
    }
    let middle = lab.iter().map(|c| c[0]).sum::<f32>() / lab.len() as f32;
    let light: Vec<f32> = lab.iter().map(|c| c[0]).collect();
    let soft = gaussian(&light, n, n as f32 / 24.0);
    lab.iter()
        .zip(soft)
        .map(|(&c, soft)| {
            let up = ((c[0] - middle) / 0.12).clamp(0.0, 1.0);
            let cast = (-c[2] / 0.015).clamp(0.0, 1.0);
            let keep = 1.0 - grey * cast * (0.25 + 0.4 * up * up * (3.0 - 2.0 * up));
            from_lab([c[0] - grey * SHEEN * (soft - middle).max(0.0), c[1] * keep, c[2] * keep])
        })
        .collect()
}

/// A separable Gaussian blur of an `n` x `n` grid, clamped at the edges.
fn gaussian(src: &[f32], n: usize, sigma: f32) -> Vec<f32> {
    let reach = (sigma * 2.5).ceil() as isize;
    let kernel: Vec<f32> = (-reach..=reach).map(|k| (-(k * k) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = kernel.iter().sum();
    let pass = |src: &[f32], dx: isize, dy: isize| -> Vec<f32> {
        (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as isize, (i / n) as isize);
                kernel.iter().enumerate().map(|(k, wt)| {
                    let o = k as isize - reach;
                    let (sx, sy) = ((x + o * dx).clamp(0, n as isize - 1), (y + o * dy).clamp(0, n as isize - 1));
                    src[sy as usize * n + sx as usize] * wt
                }).sum::<f32>() / sum
            })
            .collect()
    };
    pass(&pass(src, 1, 0), 0, 1)
}

/// How much of a mottled paint's blotches Reborn's refinish keeps: of their light, of their hue.
const REFINISH: [f32; 2] = [0.35, 0.15];

/// Reborn's refinish of TA's mottled paints (`mottled`; `n` texels a side): TA dithered its
/// camouflage and noise in blotches a few of its texels across, of other shades and hues, which
/// at 128 texels read as a soft, dirty smear. Split by size (in Oklab, by two Gaussians): what
/// spans much of the face (two tones, a gradient, a painted panel) and what is a texel or two
/// wide (panel lines, a grille, seams) stay; the blotches between keep `REFINISH` of their light
/// and less of their hue, a clean factory paint of the camouflage's own colours. TA's mottle
/// stays in its classic paint (Enhanced Classic, and liveries over TA's own paint: Heritage).
fn refinished(colour: Vec<[f32; 3]>, n: usize) -> Vec<[f32; 3]> {
    if colour.len() != n * n {
        return colour;
    }
    let lab: Vec<Lab> = colour.iter().map(|&c| to_lab(c)).collect();
    let channel = |c: usize| -> Vec<f32> { lab.iter().map(|t| t[c]).collect() };
    let bands: Vec<(Vec<f32>, Vec<f32>)> = (0..3)
        .map(|c| {
            let values = channel(c);
            (gaussian(&values, n, n as f32 / 48.0), gaussian(&values, n, n as f32 / 10.0))
        })
        .collect();
    (0..n * n)
        .map(|i| {
            let mut out = [0.0; 3];
            for (c, (fine, coarse)) in bands.iter().enumerate() {
                let keep = REFINISH[usize::from(c > 0)];
                out[c] = coarse[i] + (fine[i] - coarse[i]) * keep + (lab[i][c] - fine[i]);
            }
            from_lab(out)
        })
        .collect()
}

/// Catmull-Rom weights at `t` (0..1 between the middle two of four samples).
fn cubic(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

fn bicubic(grid: &Grid<Lab>, x: f32, y: f32) -> Lab {
    let (x0, y0) = (x.floor(), y.floor());
    let (wx, wy) = (cubic(x - x0), cubic(y - y0));
    let mut out = [0.0f32; 3];
    for (j, wyj) in wy.iter().enumerate() {
        for (i, wxi) in wx.iter().enumerate() {
            let c = grid.at(x0 as isize + i as isize - 1, y0 as isize + j as isize - 1);
            for k in 0..3 {
                out[k] += c[k] * wxi * wyj;
            }
        }
    }
    out
}

/// The way the paint's edges run round each texel: its light's and colour's gradients gathered
/// over the texels round it (a structure tensor: xx, xy, yy), for an enlargement along them.
fn edge_tensor(grid: &Grid<Lab>) -> Grid<[f32; 3]> {
    let raw: Vec<[f32; 3]> = (0..grid.h)
        .flat_map(|y| (0..grid.w).map(move |x| (x as isize, y as isize)))
        .map(|(x, y)| {
            let at = |dx: isize, dy: isize| grid.at(x + dx, y + dy);
            let mut t = [0.0f32; 3];
            for k in 0..3 {
                // (Sobel.)
                let gx = (at(1, -1)[k] + 2.0 * at(1, 0)[k] + at(1, 1)[k]) - (at(-1, -1)[k] + 2.0 * at(-1, 0)[k] + at(-1, 1)[k]);
                let gy = (at(-1, 1)[k] + 2.0 * at(0, 1)[k] + at(1, 1)[k]) - (at(-1, -1)[k] + 2.0 * at(0, -1)[k] + at(1, -1)[k]);
                t[0] += gx * gx;
                t[1] += gx * gy;
                t[2] += gy * gy;
            }
            t
        })
        .collect();
    // Gathered over the 5 x 5 round (two binomial passes each way).
    let mut cur = Grid { w: grid.w, h: grid.h, data: raw };
    for pass in 0..4 {
        let (dx, dy) = if pass % 2 == 0 { (1, 0) } else { (0, 1) };
        let data = (0..cur.h)
            .flat_map(|y| (0..cur.w).map(move |x| (x as isize, y as isize)))
            .map(|(x, y)| {
                let (a, b, c) = (cur.at(x - dx, y - dy), cur.at(x, y), cur.at(x + dx, y + dy));
                std::array::from_fn(|k| (a[k] + 2.0 * b[k] + c[k]) * 0.25)
            })
            .collect();
        cur = Grid { w: cur.w, h: cur.h, data };
    }
    cur
}

/// A texel's rough colour enlarged along the paint's edges (`tensor`, see `edge_tensor`): TA's
/// diagonal edges are staircases of texels, which an enlargement through them keeps (a team
/// emblem's diamond, a chevron, stepped two and three texels at a time); averaged along the
/// way an edge runs, each comes out a straight line. Where the edges run every way (a corner,
/// a dot) or there is none, as it was.
fn along_edges(grid: &Grid<Lab>, tensor: &Grid<[f32; 3]>, fx: f32, fy: f32) -> Lab {
    // The edge's strength (its larger eigenvalue) from which it is followed, and how far.
    const EDGE: (f32, f32) = (0.02, 0.15);
    const REACH: [(f32, f32); 4] = [(-2.0, 1.0), (-1.0, 2.0), (1.0, 2.0), (2.0, 1.0)];
    let base = bicubic(grid, fx, fy);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (ix, iy) = (x0 as isize, y0 as isize);
    let t: [f32; 3] = std::array::from_fn(|k| {
        let row = |y: isize| tensor.at(ix, y)[k] * (1.0 - tx) + tensor.at(ix + 1, y)[k] * tx;
        row(iy) * (1.0 - ty) + row(iy + 1) * ty
    });
    let [xx, xy, yy] = t;
    let middle = (xx + yy) * 0.5;
    let spread = (((xx - yy) * 0.5).powi(2) + xy * xy).sqrt();
    let (l1, l2) = (middle + spread, middle - spread);
    if l1 + l2 <= 1e-6 {
        return base;
    }
    // (How clearly they run one way: a staircase's 0.67, a corner's or a dot's near 0.)
    let coherence = ((((l1 - l2) / (l1 + l2)).powi(2) - 0.15) / 0.4).clamp(0.0, 1.0);
    let edge = ((l1 - EDGE.0) / (EDGE.1 - EDGE.0)).clamp(0.0, 1.0);
    let k = coherence * coherence * (3.0 - 2.0 * coherence) * edge * edge * (3.0 - 2.0 * edge);
    if k < 0.02 {
        return base;
    }
    // (The gradient's way, and the edge's across it.)
    let theta = 0.5 * (2.0 * xy).atan2(xx - yy);
    let (ex, ey) = (-theta.sin(), theta.cos());
    let (mut acc, mut total) = (base.map(|c| c * 3.0), 3.0);
    for (step, w) in REACH {
        let c = bicubic(grid, fx + ex * step, fy + ey * step);
        for i in 0..3 {
            acc[i] += c[i] * w;
        }
        total += w;
    }
    std::array::from_fn(|i| base[i] + (acc[i] / total - base[i]) * k)
}

/// The texture enlarged to SIZE × SIZE by palette snapping (linear RGB): each texel's rough
/// colour enlarged along the paint's edges (see `along_edges`); its ramps (see `deband`)
/// smoothly.
fn snapped(source: &Grid<Lab>, protected: bool) -> Vec<[f32; 3]> {
    let n = SIZE as usize;
    let (smoothed, ramp) = if protected {
        (Grid { w: source.w, h: source.h, data: source.data.clone() }, vec![0.0; source.w * source.h])
    } else {
        deband(&smooth(&dedither(&despeckle(source))))
    };
    // How sharply a texel is drawn to its nearest painted colour (in Oklab, squared).
    const SNAP: f32 = 9600.0;
    let (sw, sh) = (source.w as f32 / n as f32, source.h as f32 / n as f32);
    let tensor = edge_tensor(&smoothed);
    (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            let (fx, fy) = ((x as f32 + 0.5) * sw - 0.5, (y as f32 + 0.5) * sh - 0.5);
            let rough = along_edges(&smoothed, &tensor, fx, fy);
            let (cx, cy) = (((x as f32 + 0.5) * sw) as isize, ((y as f32 + 0.5) * sh) as isize);
            if ramp[cy.clamp(0, smoothed.h as isize - 1) as usize * smoothed.w + cx.clamp(0, smoothed.w as isize - 1) as usize] > 0.0 {
                return from_lab(rough);
            }
            let mut candidates = [[0.0f32; 3]; 9];
            let mut d = [0.0f32; 9];
            for k in 0..9 {
                candidates[k] = smoothed.at(cx + k as isize % 3 - 1, cy + k as isize / 3 - 1);
                d[k] = dist2(candidates[k], rough);
            }
            let nearest = d.iter().copied().fold(f32::MAX, f32::min);
            let (mut acc, mut total) = ([0.0f32; 3], 0.0f32);
            for k in 0..9 {
                let w = (-(d[k] - nearest) * SNAP).exp();
                for c in 0..3 {
                    acc[c] += candidates[k][c] * w;
                }
                total += w;
            }
            from_lab(acc.map(|v| v / total))
        })
        .collect()
}

/// A texture bigger than a layer: shrunk, each texel the average of the source's under it.
fn shrunk(source: &Grid<[f32; 3]>) -> Vec<[f32; 3]> {
    let n = SIZE as usize;
    (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            let (x0, x1) = (x * source.w / n, ((x + 1) * source.w).div_ceil(n).max(x * source.w / n + 1).min(source.w));
            let (y0, y1) = (y * source.h / n, ((y + 1) * source.h).div_ceil(n).max(y * source.h / n + 1).min(source.h));
            let mut acc = [0.0f32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let c = source.data[sy * source.w + sx];
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            let count = ((x1 - x0) * (y1 - y0)) as f32;
            acc.map(|v| v / count)
        })
        .collect()
}

pub(super) fn build(source: Source, palette: &Palette, lut: &[f32; 256]) -> SurfaceMaps {
    let n = SIZE as usize;
    let image = source.image;
    let (w, h) = (image.width.max(1) as usize, image.height.max(1) as usize);
    let gain = if source.team { TEAM_GAIN } else if source.protected { 1.0 } else { source.gain.clamp(0.5, 3.0) };
    let linear: Vec<[f32; 3]> = if image.pixels.len() == w * h {
        image
            .pixels
            .iter()
            .map(|&i| {
                let c = palette.rgb(i).map(|c| (lut[c as usize] * gain).min(1.0));
                if source.protected { c } else { shouldered(c) }
            })
            .collect()
    } else {
        vec![[1.0; 3]; w * h]
    };
    let colour: Vec<[f32; 3]> = if w > n || h > n {
        shrunk(&Grid { w, h, data: linear })
    } else {
        let lab = Grid { w, h, data: linear.iter().map(|&c| to_lab(c)).collect() };
        snapped(&lab, source.protected)
    };
    let colour = if source.protected || source.tread { colour } else { true_greys(colour, n) };
    let colour = if source.mottled && !source.protected && !source.tread { refinished(colour, n) } else { colour };
    let colour: Vec<[f32; 4]> = colour
        .iter()
        .map(|&c| {
            if !source.tread {
                return [c[0], c[1], c[2], 1.0];
            }
            // (Their light as dark steel's: from the gaps between the links to their worn tops.)
            let light = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            let t = light.sqrt().clamp(0.0, 1.0);
            let (dark, worn) = TREAD_STEEL;
            let steel = |k: usize| dark[k] + (worn[k] - dark[k]) * t;
            [steel(0), steel(1), steel(2), 1.0]
        })
        .collect();
    // The paint's relief: its lightness, a little blurred, as a height; darker is deeper.
    let lightness: Vec<f32> = colour.iter().map(|c| to_lab([c[0], c[1], c[2]])[0]).collect();
    let height: Vec<f32> = (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) as isize, (i / n) as isize);
            let mut acc = 0.0;
            for (dx, dy, w) in [(0, 0, 4.0), (1, 0, 2.0), (-1, 0, 2.0), (0, 1, 2.0), (0, -1, 2.0), (1, 1, 1.0), (-1, 1, 1.0), (1, -1, 1.0), (-1, -1, 1.0)] {
                let (sx, sy) = ((x + dx).clamp(0, n as isize - 1), (y + dy).clamp(0, n as isize - 1));
                acc += lightness[sy as usize * n + sx as usize] * w;
            }
            acc / 16.0
        })
        .collect();
    let relief = if source.protected { 0.0 } else { 3.0 };
    let finish: Vec<[f32; 4]> = (0..n * n)
        .map(|i| {
            let (x, y) = (i % n, i / n);
            let at = |dx: isize, dy: isize| height[((y as isize + dy).clamp(0, n as isize - 1) as usize) * n + (x as isize + dx).clamp(0, n as isize - 1) as usize];
            let (dx, dy) = ((at(1, 0) - at(-1, 0)) * relief, (at(0, 1) - at(0, -1)) * relief);
            let length = (dx * dx + dy * dy + 1.0).sqrt();
            // Recesses (darker than around them) hold dust: a little rougher.
            let recess = (at(-2, 0) + at(2, 0) + at(0, -2) + at(0, 2)) * 0.25 - at(0, 0);
            let rough = (source.roughness + recess.clamp(-0.1, 0.1) * 1.5).clamp(0.05, 1.0);
            [-dx / length, -dy / length, rough, source.metallic]
        })
        .collect();
    mipmaps(colour, finish)
}

fn byte(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn srgb(x: f32) -> u8 {
    byte(if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.max(0.0).powf(1.0 / 2.4) - 0.055 })
}

/// The mip chain: colours averaged in linear light, normals averaged (relief too fine to see
/// becomes roughness rather than sparkle).
fn mipmaps(mut colour: Vec<[f32; 4]>, mut finish: Vec<[f32; 4]>) -> SurfaceMaps {
    let mut maps = SurfaceMaps { color: Vec::new(), physical: Vec::new() };
    let mut size = SIZE as usize;
    for _ in 0..MIP_COUNT {
        maps.color.push(colour.iter().flat_map(|c| [srgb(c[0]), srgb(c[1]), srgb(c[2]), byte(c[3])]).collect());
        maps.physical.push(finish.iter().flat_map(|c| [byte(c[0] * 0.5 + 0.5), byte(c[1] * 0.5 + 0.5), byte(c[2]), byte(c[3])]).collect());
        if size == 1 {
            break;
        }
        let half = size / 2;
        let mut next_colour = vec![[0.0f32; 4]; half * half];
        let mut next_finish = vec![[0.0f32; 4]; half * half];
        for y in 0..half {
            for x in 0..half {
                let i = y * half + x;
                let mut spread = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let j = (y * 2 + dy) * size + x * 2 + dx;
                    for k in 0..4 {
                        next_colour[i][k] += colour[j][k] * 0.25;
                        next_finish[i][k] += finish[j][k] * 0.25;
                    }
                    spread += (finish[j][0].powi(2) + finish[j][1].powi(2)) * 0.25;
                }
                let variance = (spread - next_finish[i][0].powi(2) - next_finish[i][1].powi(2)).max(0.0);
                next_finish[i][2] = (next_finish[i][2] + variance.sqrt() * 0.5).min(1.0);
            }
        }
        colour = next_colour;
        finish = next_finish;
        size = half;
    }
    maps
}

/// A texture's family: its name without the letter TA puts after its number to tell the lit
/// and shaded variants apart (ArmCam3a, CorV04b, metal3c); other names are their own family.
pub(super) fn family(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    match bytes {
        [.., digit, b'a'..=b'd'] if digit.is_ascii_digit() => lower[..lower.len() - 1].to_string(),
        _ => lower,
    }
}

/// How much to brighten each of a family's variants (by their mean brightness): towards the
/// family's upper level (its brighter quarter), the darkest at most 2.5 times, the brightest
/// a little down.
pub(super) fn family_gains(levels: &[f32]) -> Vec<f32> {
    if levels.len() < 2 {
        return vec![1.0; levels.len()];
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let rank = 0.75 * (sorted.len() - 1) as f32;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    let target = sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f32);
    levels.iter().map(|&l| (target / l.max(1e-4)).clamp(0.8, 2.5)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8, i as u8, i as u8];
        }
        colors[200] = [40, 60, 220];
        Palette { colors }
    }

    fn lut() -> [f32; 256] {
        std::array::from_fn(|i| {
            let c = i as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        })
    }

    /// TA's mottled paints as Reborn draws them, without and with the refinish, side by side
    /// (`TA_REFINISH_OUT=<dir>`, `TA_REFINISH_NAMES=a,b`; on this computer only: TA's art).
    #[test]
    #[ignore = "requires the locally installed original game"]
    fn dump_refinish() {
        let root = std::env::var_os("TA_REBORN_GAME_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:/GOG Games/Total Annihilation".into());
        // (A tool: without a folder to write to, nothing to do, as the other dumps.)
        let Some(out) = std::env::var_os("TA_REFINISH_OUT").map(std::path::PathBuf::from) else {
            eprintln!("TA_REFINISH_OUT=<dir> writes the pictures; skipped");
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        let wanted: Vec<String> = std::env::var("TA_REFINISH_NAMES").unwrap_or_else(|_| "ArmCam3a,CorCam7d,camob3,descamo3,Juncam01,CoCam10a,graynoise3,Corcam5c".into())
            .split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let lut = std::array::from_fn(|i| crate::util::srgb_to_linear(i as f32 / 255.0));
        for path in vfs.list("textures", Some("gaf"), false) {
            let Some(gaf) = vfs.read_opt(&path).and_then(|b| ta_formats::gaf::Gaf::parse(&b).ok()) else { continue };
            for entry in gaf.entries.iter().filter(|e| wanted.contains(&e.name.to_ascii_lowercase())) {
                let image = &entry.frames[0].image;
                let side = |mottled| build(Source { image, gain: 1.0, roughness: 0.5, metallic: 0.1, protected: false, team: false, tread: false, mottled }, &palette, &lut).color.swap_remove(0);
                let (plain, refinished) = (side(false), side(true));
                let n = SIZE as usize;
                let mut both = vec![0u8; n * 2 * n * 4];
                for y in 0..n {
                    both[y * n * 8..y * n * 8 + n * 4].copy_from_slice(&plain[y * n * 4..(y + 1) * n * 4]);
                    both[y * n * 8 + n * 4..(y + 1) * n * 8].copy_from_slice(&refinished[y * n * 4..(y + 1) * n * 4]);
                }
                for px in both.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                image::save_buffer(out.join(format!("{}.png", entry.name)), &both, n as u32 * 2, n as u32, image::ColorType::Rgba8).unwrap();
            }
        }
    }

    /// Reborn's refinish: TA's mottle (blotches a few of its texels across) calmed, a broad
    /// two-tone and a line two texels wide kept, and only the mottled paints refinished.
    #[test]
    fn the_refinish_calms_blotches_and_keeps_lines_and_broad_shades() {
        let n = 128;
        let colour: Vec<[f32; 3]> = (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f32, (i / n) as f32);
                let broad = if x < 64.0 { 0.30 } else { 0.16 };
                let blotch = (x / 5.0).sin() * (y / 5.0).sin() * 0.08;
                let line = if (40.0..42.0).contains(&y) { -0.12 } else { 0.0 };
                let l = (broad + blotch + line).max(0.01);
                [l * 1.1, l, l * 0.6 + blotch * 0.6]
            })
            .collect();
        let out = refinished(colour.clone(), n);
        let light = |c: &[[f32; 3]], x: usize, y: usize| to_lab(c[y * n + x])[0];
        let hue = |c: &[[f32; 3]], x: usize, y: usize| to_lab(c[y * n + x])[2];
        // (Spread of light and hue over a patch of blotches, away from the line and the step.)
        let spread = |c: &[[f32; 3]], f: &dyn Fn(&[[f32; 3]], usize, usize) -> f32| {
            let values: Vec<f32> = (70..120).flat_map(|y| (12..52).map(move |x| (x, y))).map(|(x, y)| f(c, x, y)).collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
        };
        assert!(spread(&out, &light) < spread(&colour, &light) * 0.6, "the blotches' light calmed");
        assert!(spread(&out, &hue) < spread(&colour, &hue) * 0.45, "their hue more");
        let line = |c: &[[f32; 3]]| (36..44).map(|x| light(c, x, 37) - light(c, x, 40)).sum::<f32>();
        assert!(line(&out) > line(&colour) * 0.7, "a line two texels wide stays: {} of {}", line(&out), line(&colour));
        let step = |c: &[[f32; 3]]| (70..120).map(|y| light(c, 30, y) - light(c, 98, y)).sum::<f32>();
        assert!((step(&out) / step(&colour) - 1.0).abs() < 0.15, "the two tones stay: {} of {}", step(&out), step(&colour));
        assert!(mottled("ArmCam3a") && mottled("CoCam10a") && mottled("descamo3") && mottled("graynoise4"));
        assert!(!mottled("CorV04b") && !mottled("metal3b") && !mottled("Armpanel3") && !mottled("32xlogos"));
    }

    #[test]
    fn families_group_the_lit_and_shaded_variants() {
        assert_eq!(family("ArmCam3a"), family("ArmCam3d"));
        assert_eq!(family("CorV04b"), "corv04");
        assert_ne!(family("Tredside1"), family("Tredside2"));
        assert_eq!(family("Arm32Lt"), "arm32lt");
        let gains = family_gains(&[0.4, 0.2, 0.05, 0.42]);
        assert!(gains[1] > 1.5 && gains[2] == 2.5 && gains[3] < 1.0 && gains[0] > 0.95, "{gains:?}");
    }

    #[test]
    fn a_thin_line_survives_and_edges_come_out_crisp() {
        // 32 × 32 light grey with a one-texel dark line down column 10.
        let mut pixels = vec![180u8; 32 * 32];
        for y in 0..32 {
            pixels[y * 32 + 10] = 40;
        }
        let image = IndexedImage { width: 32, height: 32, pixels, ..Default::default() };
        let maps = build(Source { image: &image, gain: 1.0, roughness: 0.5, metallic: 0.1, protected: false, team: false, tread: false, mottled: false }, &palette(), &lut());
        let top = &maps.color[0];
        let row = 64usize;
        let at = |x: usize| top[(row * 128 + x) * 4] as f32;
        // The line, four texels wide now, is still dark; a few texels off it is the grey.
        let line = (40..44).map(at).fold(255.0, f32::min);
        assert!(line < 90.0, "line {line}");
        assert!(at(30) > 160.0 && at(60) > 160.0, "{} {}", at(30), at(60));
        // Crisp: from dark to light within about one source texel (four of these).
        let ramp = (36..50).filter(|&x| at(x) > 70.0 && at(x) < 150.0).count();
        assert!(ramp <= 4, "{ramp} texels between");
    }

    /// The palette's specks go (a pink fleck in a khaki takes its hue, a lone dark texel in it
    /// its colour); a painted lamp, vivid or bright, stays.
    #[test]
    fn specks_go_and_lamps_stay() {
        let lab = |rgb: [u8; 3]| to_lab(rgb.map(|c| lut()[c as usize]));
        let khaki = lab([170, 165, 120]);
        let mut grid = Grid { w: 9, h: 9, data: vec![khaki; 81] };
        let (fleck, dark, lamp) = (lab([200, 150, 170]), lab([140, 136, 98]), lab([230, 40, 30]));
        grid.data[2 * 9 + 2] = fleck;
        grid.data[2 * 9 + 6] = dark;
        grid.data[6 * 9 + 4] = lamp;
        let clean = despeckle(&grid);
        let hue = |c: Lab| c[1].hypot(c[2]);
        assert!(dist2(clean.data[2 * 9 + 2], khaki) < dist2(fleck, khaki) * 0.25, "the fleck takes the khaki's hue");
        assert!((clean.data[2 * 9 + 2][0] - fleck[0]).abs() < 1e-4, "and keeps its light");
        assert!(dist2(clean.data[2 * 9 + 6], khaki) < 1e-6, "the lone dark texel goes");
        assert!((hue(clean.data[6 * 9 + 4]) - hue(lamp)).abs() < 1e-4, "the lamp stays");
    }

    /// A row of dots through a metal's grey (TA's ordered dither) evens out; a line a texel wide
    /// stays.
    #[test]
    fn ordered_dither_evens_out_and_lines_stay() {
        let grey = [0.6f32, 0.0, 0.0];
        let mut grid = Grid { w: 16, h: 16, data: vec![grey; 256] };
        for x in (0..16).step_by(2) {
            grid.data[5 * 16 + x] = [0.66, 0.0, 0.0];
        }
        for y in 0..16 {
            grid.data[y * 16 + 11] = [0.3, 0.0, 0.0];
        }
        let out = dedither(&grid);
        let row: Vec<f32> = (2..9).map(|x| out.data[5 * 16 + x][0]).collect();
        let (lo, hi) = row.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
        assert!(hi - lo < 0.035, "the dots even out: {row:?}");
        assert!((0..16).all(|y| out.data[y * 16 + 11][0] == 0.3), "the line stays");
    }

    /// A diagonal edge stepped two texels at a time (TA's staircases) comes out a straight
    /// line: across the enlargement, where each row turns from one colour to the other lies
    /// on a line, not on steps.
    #[test]
    fn staircases_come_out_straight() {
        let mut pixels = vec![180u8; 32 * 32];
        for y in 0..32usize {
            // Steps two texels across for each two down: a 45-degree staircase in blocks.
            for x in 0..32usize {
                if x / 2 * 2 < y / 2 * 2 {
                    pixels[y * 32 + x] = 40;
                }
            }
        }
        let image = IndexedImage { width: 32, height: 32, pixels, ..Default::default() };
        let maps = build(Source { image: &image, gain: 1.0, roughness: 0.5, metallic: 0.1, protected: true, team: false, tread: false, mottled: false }, &palette(), &lut());
        let top = &maps.color[0];
        // Each row's turning point (the first texel light again), from rows 20 to 100.
        let turns: Vec<f32> = (20..100usize)
            .map(|row| (0..128usize).find(|&x| top[(row * 128 + x) * 4] > 110).unwrap_or(0) as f32)
            .collect();
        // Off a straight line through them by little (the staircase's own is two texels' worth,
        // eight here).
        let n = turns.len() as f32;
        let (mx, my) = ((0..turns.len()).map(|i| i as f32).sum::<f32>() / n, turns.iter().sum::<f32>() / n);
        let slope = (0..turns.len()).map(|i| (i as f32 - mx) * (turns[i] - my)).sum::<f32>() / (0..turns.len()).map(|i| (i as f32 - mx).powi(2)).sum::<f32>();
        let worst = (0..turns.len()).map(|i| (turns[i] - (my + slope * (i as f32 - mx))).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1.5, "off the line by {worst} texels: {turns:?}");
    }

    /// A metal's ramp in bands of the palette's greys comes out smooth; a panel line across it
    /// stays crisp.
    #[test]
    fn banded_ramps_come_out_smooth_and_lines_stay() {
        // 32 × 32: a ramp across, five texels a band, one grey a step; a dark line down column
        // 24.
        let mut grid = Grid { w: 32, h: 32, data: vec![[0.0f32; 3]; 32 * 32] };
        for y in 0..32 {
            for x in 0..32 {
                grid.data[y * 32 + x] = [0.4 + (x / 5) as f32 * 0.035, 0.0, 0.0];
            }
            grid.data[y * 32 + 24] = [0.2, 0.0, 0.0];
        }
        let (out, ramp) = deband(&grid);
        assert!(ramp[16 * 32 + 8] > 0.0 && ramp[16 * 32 + 24] == 0.0, "the bands ramp, the line does not");
        // Across the bands' steps (texels 4 to 11) the light now climbs in small steps.
        let row: Vec<f32> = (4..12).map(|x| out.data[16 * 32 + x][0]).collect();
        assert!(row.windows(2).all(|w| w[1] - w[0] < 0.02 && w[1] >= w[0] - 1e-5), "{row:?}");
        assert!((out.data[16 * 32 + 24][0] - 0.2).abs() < 1e-6, "the line stays");
    }

    /// A painting's palette static calms (a green fleck in a red rock takes the red's hue), its
    /// transparent texels stay so and its edge keeps its colour.
    #[test]
    fn a_paintings_static_calms() {
        let (w, h) = (9usize, 9usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 2..7 {
            for x in 2..7 {
                let i = (y * w + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[150, 60, 40, 255]);
            }
        }
        let fleck = (4 * w + 4) * 4;
        rgba[fleck..fleck + 3].copy_from_slice(&[80, 120, 50]);
        calm_sprite(w, h, &mut rgba);
        assert!(rgba[fleck] > rgba[fleck + 1], "the fleck takes the red's hue: {:?}", &rgba[fleck..fleck + 3]);
        assert_eq!(rgba[3], 0, "the transparent texels stay so");
        let corner = (2 * w + 2) * 4;
        assert!((rgba[corner] as i32 - 150).abs() < 12, "the edge keeps its colour: {:?}", &rgba[corner..corner + 3]);
    }

    /// A grey paint's lavender cast goes (its light parts' most), a coloured one keeps its hue.
    #[test]
    fn greys_lose_the_palettes_lavender() {
        let lab = |rgb: [u8; 3]| rgb.map(|c| lut()[c as usize]);
        let chroma = |c: [f32; 3]| {
            let l = to_lab(c);
            l[1].hypot(l[2])
        };
        let lavender = lab([200, 200, 225]);
        let grey: Vec<[f32; 3]> = (0..64).map(|k| if k % 2 == 0 { lavender } else { lab([90, 90, 98]) }).collect();
        let out = true_greys(grey, 8);
        assert!(chroma(out[0]) < chroma(lavender) * 0.5, "the light lavender greys");
        // Olive camouflage with grey in it (TA's ArmCam: as low in chroma on the whole) keeps its
        // colour.
        let olive = lab([110, 120, 70]);
        let camo: Vec<[f32; 3]> = (0..64).map(|k| if k % 3 == 0 { olive } else { lab([120, 120, 115]) }).collect();
        let out = true_greys(camo.clone(), 8);
        assert!((chroma(out[0]) - chroma(olive)).abs() < 1e-4, "olive camouflage keeps its hue");
    }

    #[test]
    fn team_colours_keep_their_paint_and_have_no_relief() {
        let mut pixels = vec![200u8; 32 * 32];
        pixels[5] = 255;
        let image = IndexedImage { width: 32, height: 32, pixels, ..Default::default() };
        let maps = build(Source { image: &image, gain: 2.0, roughness: 0.5, metallic: 0.1, protected: true, team: false, tread: false, mottled: false }, &palette(), &lut());
        let c = &maps.color[0];
        assert_eq!((c[64 * 4 * 128 + 64 * 4], c[64 * 4 * 128 + 64 * 4 + 2]), (40, 220));
        assert!(maps.physical[0].chunks(4).all(|p| p[0] == 128 && p[1] == 128));
        assert_eq!(maps.color.len(), MIP_COUNT as usize);
        assert_eq!(maps.color.last().unwrap().len(), 4);
    }
}
