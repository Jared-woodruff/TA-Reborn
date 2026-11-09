//! What lies round the map, for the ground beyond it (perspective camera, see sky.rs): next to
//! the map that ground is the map's own mirror image, so that whatever crosses an edge carries on
//! without a seam; further out it is land and sea of its own, made after the map's own ground
//! near each stretch of its edges (how much of it is sea, how high its land stands and how much
//! it rises and falls, the colours its dry ground is painted in), its coasts and hills shaped by
//! noise (`far_ground` in terrain_common.wgsl), never copies of the map's.

use bytemuck::{Pod, Zeroable};

/// Points spaced evenly round the map's edges, each measuring the ground near it.
pub(crate) const AROUND: usize = 128;

/// How far into the map, and either way along its edges, each point measures the ground (world
/// units).
const REACH: f32 = 768.0;

/// The map's ground near each of [`AROUND`] points spaced evenly round its edges, clockwise from
/// its north-west corner (mirrors `Surroundings` in terrain_common.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Surroundings {
    /// x = the fraction of the ground under the sea, y = the land's mean height, z = the sea
    /// bed's, w = how much the land's heights vary (their standard deviation).
    shape: [[f32; 4]; AROUND],
    /// The dry ground's colours (linear RGB, w unused): its lighter half's and its darker half's.
    light: [[f32; 4]; AROUND],
    dark: [[f32; 4]; AROUND],
    /// The sea's colour as painted (linear RGB, w unused).
    sea: [[f32; 4]; AROUND],
}

/// The ground near one point.
#[derive(Clone, Copy, Default)]
struct Measure {
    cells: u32,
    wet: u32,
    land: Option<Land>,
    sea: Option<Sea>,
}

#[derive(Clone, Copy)]
struct Sea {
    bed: f32,
    color: [f32; 3],
}

#[derive(Clone, Copy)]
struct Land {
    height: f32,
    relief: f32,
    light: [f32; 3],
    dark: [f32; 3],
}

impl Surroundings {
    /// Measured on a map `width` x `height` cells: its drawn heights `drawn` (a cell's each)
    /// against the sea level `sea`, and the mean colours of the tiles shown, `tiles` (linear RGB,
    /// a tile two cells a side, `width / 2` of them a row).
    pub(crate) fn measure(width: u32, height: u32, drawn: &[f32], sea: f32, tiles: &[[f32; 3]]) -> Surroundings {
        let (w, h) = (width as f32, height as f32);
        let perimeter = 2.0 * (w + h);
        let reach = (REACH / 16.0) as i32;
        // (Every other cell is plenty for a mean.)
        let step = 2usize;
        let tiles_x = (width / 2).max(1) as usize;
        let luminance = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let mut measures = [Measure::default(); AROUND];
        let mut dry = Vec::new();
        for (i, m) in measures.iter_mut().enumerate() {
            let (px, pz) = round_point((i as f32 + 0.5) / AROUND as f32 * perimeter, w, h);
            let (px, pz) = (px.round() as i32, pz.round() as i32);
            let (x0, x1) = ((px - reach).max(0) as usize, ((px + reach).min(width as i32 - 1)).max(0) as usize);
            let (z0, z1) = ((pz - reach).max(0) as usize, ((pz + reach).min(height as i32 - 1)).max(0) as usize);
            dry.clear();
            let (mut bed_sum, mut sea_color) = (0.0f32, [0.0f32; 3]);
            for z in (z0..=z1).step_by(step) {
                for x in (x0..=x1).step_by(step) {
                    let Some(&d) = drawn.get(z * width as usize + x) else { continue };
                    m.cells += 1;
                    let c = tiles.get((z / 2) * tiles_x + x / 2).copied().unwrap_or([0.0; 3]);
                    if d < sea {
                        m.wet += 1;
                        bed_sum += d;
                        sea_color = [sea_color[0] + c[0], sea_color[1] + c[1], sea_color[2] + c[2]];
                    } else {
                        dry.push((d, c, luminance(c)));
                    }
                }
            }
            if m.wet > 0 {
                let n = m.wet as f32;
                m.sea = Some(Sea { bed: bed_sum / n, color: sea_color.map(|c| c / n) });
            }
            if !dry.is_empty() {
                let n = dry.len() as f32;
                let height = dry.iter().map(|d| d.0).sum::<f32>() / n;
                let relief = (dry.iter().map(|d| (d.0 - height) * (d.0 - height)).sum::<f32>() / n).sqrt();
                let middle = dry.iter().map(|d| d.2).sum::<f32>() / n;
                // (Its lighter and darker halves: on a map of one colour, both that.)
                let mean = |lighter: bool| {
                    let (mut sum, mut count) = ([0.0f32; 3], 0u32);
                    for &(_, c, _) in dry.iter().filter(|d| (d.2 >= middle) == lighter) {
                        sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
                        count += 1;
                    }
                    (count > 0).then(|| sum.map(|s| s / count as f32))
                };
                let (light, dark) = match (mean(true), mean(false)) {
                    (Some(l), Some(d)) => (l, d),
                    (Some(c), None) | (None, Some(c)) => (c, c),
                    (None, None) => unreachable!("some dry ground was measured"),
                };
                m.land = Some(Land { height, relief, light, dark });
            }
        }
        // Points with no land near them (out at sea) take the nearest land's, round the edges
        // either way; with none anywhere, a plain earth (no far land shows then).
        let nearest = |i: usize, has: &dyn Fn(&Measure) -> bool| {
            (1..AROUND / 2 + 1).find_map(|k| [(i + k) % AROUND, (i + AROUND - k) % AROUND].into_iter().find(|&j| has(&measures[j])))
        };
        let earth = Land { height: sea.max(0.0) + 8.0, relief: 0.0, light: [0.3, 0.26, 0.2], dark: [0.22, 0.19, 0.15] };
        let dry_sea = Sea { bed: (sea - 16.0).max(0.0), color: [0.0; 3] };
        let mut out = Surroundings::zeroed();
        for (i, m) in measures.iter().enumerate() {
            let land = m.land.or_else(|| nearest(i, &|m| m.land.is_some()).and_then(|j| measures[j].land)).unwrap_or(earth);
            let water = m.sea.or_else(|| nearest(i, &|m| m.sea.is_some()).and_then(|j| measures[j].sea)).unwrap_or(dry_sea);
            let wet = if m.cells > 0 { m.wet as f32 / m.cells as f32 } else { 0.0 };
            out.shape[i] = [wet, land.height, water.bed, land.relief];
            out.light[i] = [land.light[0], land.light[1], land.light[2], 0.0];
            out.dark[i] = [land.dark[0], land.dark[1], land.dark[2], 0.0];
            out.sea[i] = [water.color[0], water.color[1], water.color[2], 0.0];
        }
        out
    }
}

/// The point (cells) `s` cells round the edges of a map `w` x `h` cells, clockwise from its
/// north-west corner: along the north edge, down the east, back along the south, up the west
/// (`round_the_edge` in terrain_common.wgsl finds a point's `s`).
fn round_point(s: f32, w: f32, h: f32) -> (f32, f32) {
    if s < w {
        (s, 0.0)
    } else if s < w + h {
        (w, s - w)
    } else if s < 2.0 * w + h {
        (w - (s - w - h), h)
    } else {
        (0.0, h - (s - 2.0 * w - h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map 128 x 64 cells: its western half sea (its bed at 10), its eastern half land at 60,
    /// painted green, with a stripe of brown.
    fn half_sea() -> (u32, u32, Vec<f32>, Vec<[f32; 3]>) {
        let (w, h) = (128u32, 64u32);
        let drawn = (0..w * h).map(|i| if i % w < w / 2 { 10.0 } else { 60.0 }).collect();
        let tiles = (0..(w / 2) * (h / 2)).map(|i| if (i % (w / 2)) % 8 == 0 { [0.3, 0.2, 0.1] } else { [0.1, 0.4, 0.1] }).collect();
        (w, h, drawn, tiles)
    }

    /// The point measured nearest `s` cells round the edges of a map with that perimeter.
    fn at(s: f32, perimeter: f32) -> usize {
        ((s / perimeter * AROUND as f32) as usize).min(AROUND - 1)
    }

    #[test]
    fn the_points_go_round_the_edges_clockwise() {
        assert_eq!(round_point(10.0, 100.0, 50.0), (10.0, 0.0));
        assert_eq!(round_point(120.0, 100.0, 50.0), (100.0, 20.0));
        assert_eq!(round_point(160.0, 100.0, 50.0), (90.0, 50.0));
        assert_eq!(round_point(290.0, 100.0, 50.0), (0.0, 10.0));
    }

    #[test]
    fn each_stretch_of_edge_is_measured_on_the_ground_near_it() {
        let (w, h, drawn, tiles) = half_sea();
        let s = Surroundings::measure(w, h, &drawn, 30.0, &tiles);
        let perimeter = 2.0 * (w + h) as f32;
        // The east edge's middle: all land, at 60, level; green and brown.
        let east = s.shape[at(w as f32 + h as f32 / 2.0, perimeter)];
        assert_eq!(east[0], 0.0);
        assert!((east[1] - 60.0).abs() < 1e-3 && east[3] < 1e-3, "{east:?}");
        let (light, dark) = (s.light[at(w as f32 + h as f32 / 2.0, perimeter)], s.dark[at(w as f32 + h as f32 / 2.0, perimeter)]);
        assert!(light[1] > dark[1], "the lighter half is the green: {light:?} {dark:?}");
        // The west edge's middle: all sea, its bed at 10, its land the nearest land's (at 60);
        // the sea as painted there; the east edge's sea the nearest sea's.
        let west = s.shape[at(2.0 * w as f32 + 1.5 * h as f32, perimeter)];
        assert_eq!(west[0], 1.0);
        assert!((west[2] - 10.0).abs() < 1e-3 && (west[1] - 60.0).abs() < 1e-3, "{west:?}");
        assert!(s.sea[at(w as f32 + h as f32 / 2.0, perimeter)][1] > 0.1);
        // The north edge's middle, where the coast meets it: part land, part sea.
        let north = s.shape[at(w as f32 / 2.0, perimeter)];
        assert!(north[0] > 0.2 && north[0] < 0.8, "{north:?}");
    }

    #[test]
    fn a_map_without_land_or_sea_still_has_surroundings() {
        let (w, h) = (64u32, 64u32);
        let tiles = vec![[0.2, 0.2, 0.2]; 32 * 32];
        let all_sea = Surroundings::measure(w, h, &vec![5.0; 64 * 64], 30.0, &tiles);
        assert!(all_sea.shape.iter().all(|p| p[0] == 1.0 && p[1] > 30.0));
        let all_land = Surroundings::measure(w, h, &vec![50.0; 64 * 64], 0.0, &tiles);
        assert!(all_land.shape.iter().all(|p| p[0] == 0.0 && p[1] == 50.0 && p[2] == 0.0));
    }
}
