//! The land: which part of the map becomes the TA map and at what scale ([`Frame`]), its
//! heights pressed into TA's 0-255, the sea's level, and the cliffs Supreme Commander's units
//! could not climb ([`Land`]).

use crate::scmap::ScMap;

/// TA pixels per world unit at most: a T1 tank and a mass extractor (about one world unit each)
/// come out as big beside the land as TA's own units are.
pub const SCALE: f64 = 16.0;
/// The most cells a side (16,384 pixels: TA Reborn's largest maps, 32 × 32 in the menus).
pub const MAX_CELLS: usize = 1024;
/// A Supreme Commander cell whose corners differ by more than this (world units) is too steep
/// for its land units: the game's own rule.
pub const CLIFF: f32 = 0.75;
/// The water deep ships need: what the game calls deep water is at least this deep in TA.
const SHIP_DEPTH: f64 = 20.0;

/// The part of the map made into the TA map, and the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The area's corner (world units).
    pub x0: f64,
    pub z0: f64,
    /// TA pixels per world unit.
    pub s: f64,
    /// The TA map's size in cells (even).
    pub w: usize,
    pub h: usize,
}

impl Frame {
    /// The frame of `area` (x0, z0, x1, z1: world units) at [`SCALE`], or smaller to keep its
    /// longer side within [`MAX_CELLS`] cells.
    pub fn of(area: [f64; 4]) -> Frame {
        Frame::within(area, MAX_CELLS)
    }

    /// [`Frame::of`] with at most `max_cells` cells a side.
    pub fn within(area: [f64; 4], max_cells: usize) -> Frame {
        let (aw, ah) = ((area[2] - area[0]).max(16.0), (area[3] - area[1]).max(16.0));
        let s = SCALE.min(max_cells.max(64) as f64 * 16.0 / aw.max(ah));
        let cells = |side: f64| (((side * s / 16.0).round() as usize).max(32) + 1) & !1;
        Frame { x0: area[0], z0: area[1], s, w: cells(aw), h: cells(ah) }
    }

    /// World units per cell.
    pub fn cell(&self) -> f64 {
        16.0 / self.s
    }

    /// The world position of TA pixel (x, z).
    #[inline]
    pub fn world(&self, x: f64, z: f64) -> (f64, f64) {
        (self.x0 + x / self.s, self.z0 + z / self.s)
    }

    /// TA pixels of world position (u, v).
    #[inline]
    pub fn pixels(&self, u: f64, v: f64) -> (f64, f64) {
        ((u - self.x0) * self.s, (v - self.z0) * self.s)
    }
}

/// The land as TA holds it.
pub struct Land {
    pub frame: Frame,
    /// TA's height at each cell's top-left corner (whole units), `w × h`.
    pub heights: Vec<u8>,
    /// The sea's level in TA's units (0: no sea).
    pub sea: i32,
    /// The cells too steep for Supreme Commander's land units.
    pub cliff: Vec<bool>,
    /// The game's water: its surface's elevation (world units), if the map has water.
    pub water: Option<f32>,
}

impl Land {
    /// TA's slope of a cell: its highest corner less its lowest (what movement classes limit).
    pub fn slope(&self, x: usize, z: usize) -> u8 {
        let (w, h) = (self.frame.w, self.frame.h);
        let at = |x: usize, z: usize| self.heights[z.min(h - 1) * w + x.min(w - 1)];
        let c = [at(x, z), at(x + 1, z), at(x, z + 1), at(x + 1, z + 1)];
        c.iter().max().unwrap() - c.iter().min().unwrap()
    }

    /// How deep the water is at a cell (its top-left corner; 0 or less on land).
    pub fn depth(&self, x: usize, z: usize) -> i32 {
        self.sea - self.heights[z * self.frame.w + x] as i32
    }
}

/// The height (world units) at world position (u, v), between the heightmap's vertices.
pub fn height(m: &ScMap, u: f64, v: f64) -> f64 {
    let (fx, fz) = (u.floor(), v.floor());
    let (tx, tz) = (u - fx, v - fz);
    let (x, z) = (fx as i64, fz as i64);
    let h = |x: i64, z: i64| m.raw(x, z) as f64 * m.height_scale as f64;
    let a = h(x, z) + (h(x + 1, z) - h(x, z)) * tx;
    let b = h(x, z + 1) + (h(x + 1, z + 1) - h(x, z + 1)) * tx;
    a + (b - a) * tz
}

/// The height at world position (u, v) as a TA cell `k` world units across sees it: the
/// heightmap averaged over the cell round it (a tent over `k` units each way), so a map made
/// smaller keeps its hills' shapes rather than their noise.
fn height_over(m: &ScMap, u: f64, v: f64, k: f64) -> f64 {
    if k <= 1.0 {
        return height(m, u, v);
    }
    let n = k.ceil() as i64;
    let (mut sum, mut weight) = (0.0, 0.0);
    for j in -n..=n {
        for i in -n..=n {
            let w = (1.0 - (i as f64).abs() / (n as f64 + 1.0)) * (1.0 - (j as f64).abs() / (n as f64 + 1.0));
            sum += height(m, u + i as f64 * k / n as f64 * 0.5, v + j as f64 * k / n as f64 * 0.5) * w;
            weight += w;
        }
    }
    sum / weight
}

/// The land of `m` in `frame`.
pub fn build(m: &ScMap, frame: Frame) -> Land {
    let (w, h) = (frame.w, frame.h);
    let k = frame.cell();
    // The game's heights at TA's corners.
    let raw: Vec<f64> = (0..w * h).map(|i| {
        let (cx, cz) = ((i % w) as f64, (i / w) as f64);
        let (u, v) = frame.world(cx * 16.0, cz * 16.0);
        height_over(m, u, v, k)
    }).collect();
    let low = raw.iter().copied().fold(f64::INFINITY, f64::min);
    let high = raw.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let water = m.water.as_ref().filter(|wt| (wt.surface as f64) > low).map(|wt| wt.surface);
    let (heights, sea) = match (water, m.water.as_ref()) {
        (Some(e), Some(wt)) => {
            let e = e as f64;
            let depth = e - low;
            let land = (high - e).max(0.0);
            // One scale for land and sea where it leaves the game's deep water deep enough for
            // ships; else the sea is deepened and the land given what height is left.
            let mut k_land = frame.s.min(254.0 / (land + depth));
            let deep = (e - wt.deep as f64).max(0.5);
            let mut k_sea = k_land;
            if deep * k_sea < SHIP_DEPTH {
                k_sea = (SHIP_DEPTH / deep).min(200.0 / depth.max(1e-6));
                k_land = frame.s.min((254.0 - depth * k_sea) / land.max(1e-6));
                if k_land < 1.0 {
                    k_land = 1.0f64.min(frame.s);
                    k_sea = ((254.0 - land * k_land) / depth.max(1e-6)).max(k_land);
                }
            }
            let sea = (depth * k_sea).round().clamp(1.0, 254.0);
            let heights = raw.iter().map(|&r| {
                let t = if r >= e { sea + (r - e) * k_land } else { sea - (e - r) * k_sea };
                t.round().clamp(0.0, 255.0) as u8
            }).collect();
            (heights, sea as i32)
        }
        _ => {
            let k = frame.s.min(254.0 / (high - low).max(1e-6));
            (raw.iter().map(|&r| ((r - low) * k).round().clamp(0.0, 255.0) as u8).collect(), 0)
        }
    };
    let cliff = cliffs(m, frame);
    Land { frame, heights, sea, cliff, water }
}

/// The TA cells too steep for Supreme Commander's land units: a cell is a cliff where most of
/// the game's own cells under it are ([`CLIFF`]).
fn cliffs(m: &ScMap, frame: Frame) -> Vec<bool> {
    let (w, h) = (frame.w, frame.h);
    let steep = |x: i64, z: i64| -> bool {
        let c = [m.raw(x, z), m.raw(x + 1, z), m.raw(x, z + 1), m.raw(x + 1, z + 1)];
        let range = (c.iter().max().unwrap() - c.iter().min().unwrap()) as f32 * m.height_scale;
        range > CLIFF
    };
    let k = frame.cell();
    // The game's cells under a TA cell: one, or a square of them (sampled at their middles).
    let n = (k.round() as i64).max(1);
    (0..w * h).map(|i| {
        let (cx, cz) = ((i % w) as f64, (i / w) as f64);
        let mut count = 0;
        for j in 0..n {
            for i in 0..n {
                let (u, v) = frame.world((cx + (i as f64 + 0.5) / n as f64) * 16.0, (cz + (j as f64 + 0.5) / n as f64) * 16.0);
                if steep(u.floor() as i64, v.floor() as i64) {
                    count += 1;
                }
            }
        }
        2 * count > n * n
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scmap::{ScMap, Water};

    /// A map of `w × h` world units, its raw heights `f(x, z)`.
    pub(crate) fn map(w: u32, h: u32, f: impl Fn(u32, u32) -> u16, water: Option<f32>) -> ScMap {
        ScMap {
            version: 60,
            width: w,
            height: h,
            height_scale: 1.0 / 128.0,
            heights: (0..(h + 1)).flat_map(|z| (0..(w + 1)).map(move |x| (x, z))).map(|(x, z)| f(x, z)).collect(),
            water: water.map(|e| Water { surface: e, deep: e - 2.5, abyss: e - 15.0, ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn frames_keep_the_game_scale_up_to_the_largest_map() {
        let f = Frame::of([0.0, 0.0, 512.0, 512.0]);
        assert_eq!((f.s, f.w, f.h), (16.0, 512, 512));
        let f = Frame::of([0.0, 0.0, 4096.0, 4096.0]);
        assert_eq!((f.s, f.w, f.h), (4.0, 1024, 1024));
        let f = Frame::of([4.0, 56.0, 252.0, 200.0]);
        assert_eq!((f.s, f.w, f.h, f.x0, f.z0), (16.0, 248, 144, 4.0, 56.0));
        assert_eq!(f.world(16.0, 32.0), (5.0, 58.0));
        assert_eq!(f.pixels(5.0, 58.0), (16.0, 32.0));
        let f = Frame::of([0.0, 0.0, 2048.0, 1024.0]);
        assert_eq!((f.s, f.w, f.h), (8.0, 1024, 512));
    }

    /// A plateau 20 world units up over a sea 15 deep: the sea is deep enough for TA's warships,
    /// the land fits under 255, the shore stays where it was, and the plateau's edge (steeper
    /// than the game's units climb) is a cliff; the gentle ramp is not.
    #[test]
    fn heights_fit_ships_and_land_and_cliffs_are_marked() {
        let raw = |x: u32, _z: u32| -> u16 {
            let h = match x {
                0..=9 => 2.5,                                // the sea bed
                10..=49 => 2.5 + (x - 10) as f32 * 0.5,      // a slope up out of the sea (the shore at 40)
                50..=51 => 22.0 + (x - 49) as f32 * 10.0,    // a cliff
                _ => 42.5,
            };
            (h * 128.0) as u16
        };
        let m = map(64, 32, raw, Some(17.5));
        let land = build(&m, Frame::of([0.0, 0.0, 64.0, 32.0]));
        assert!(land.sea > 0);
        assert!(land.depth(5, 4) >= 20, "deep water: {}", land.depth(5, 4));
        assert!(land.depth(45, 4) < 0 && land.depth(55, 4) < 0);
        assert!(*land.heights.iter().max().unwrap() > 200, "the land given the height there is");
        // The shore: at x 40 the slope meets the water.
        assert!(land.depth(37, 4) > 0 && land.depth(43, 4) <= 0, "{} {}", land.depth(37, 4), land.depth(43, 4));
        let cliff = |x: usize| land.cliff[4 * land.frame.w + x];
        assert!(cliff(49) && cliff(50));
        assert!(!cliff(15) && !cliff(45) && !cliff(51) && !cliff(55));
        // TA's own slopes on the gentle parts are what a tank climbs.
        assert!((10..48).all(|x| land.slope(x, 4) <= 15), "{:?}", (10..48).map(|x| land.slope(x, 4)).collect::<Vec<_>>());
        // A map without water: no sea, its lowest point 0.
        let dry = build(&map(64, 32, raw, None), Frame::of([0.0, 0.0, 64.0, 32.0]));
        assert_eq!(dry.sea, 0);
        assert_eq!(*dry.heights.iter().min().unwrap(), 0);
    }
}
