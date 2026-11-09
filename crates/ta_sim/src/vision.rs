//! Per-player line of sight, radar and sonar coverage, jamming, and explored ground.
//!
//! Coverage is rebuilt every few ticks on a coarse grid: each sensor stamps a disc of
//! precomputed scanline spans. With TA's "true" line of sight, hills hide what is behind
//! them: line of sight is traced instead, out from the eye over the ground's heights.
//! Everything is integer maths, so targeting and the AI may depend on it without breaking
//! lockstep determinism.

use rustc_hash::FxHashMap;

use crate::fixed::FxVec3;

/// Pixels per vision cell.
pub const VISION_CELL: i64 = 32;
/// Ticks between coverage rebuilds.
pub const VISION_INTERVAL: u64 = 6;

pub const LOS: u8 = 1;
pub const RADAR: u8 = 2;
pub const SONAR: u8 = 4;
pub const RADAR_JAM: u8 = 8;
pub const SONAR_JAM: u8 = 16;
/// Ever been in line of sight (sticky).
pub const EXPLORED: u8 = 32;

/// How a unit appears to a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Hidden,
    /// A radar/sonar contact: position known, identity not.
    Radar,
    Visible,
}

/// One sensor footprint to stamp.
#[derive(Clone, Copy, Debug)]
pub struct Sensor {
    pub player: u8,
    pub pos: FxVec3,
    /// Radius in pixels.
    pub radius: i64,
    pub flag: u8,
    /// The eye's height in pixels (line of sight over hills).
    pub eye: i64,
}

/// How far above the ground a cell is looked at (the top of a unit standing there).
const TARGET_HEIGHT: i64 = 16;
/// No horizon yet: lower than any ground (a slope, height over distance).
const OPEN: (i64, i64) = (-1 << 40, 1);

/// A cell of a disc traced for line of sight over hills (see [`disc_rings`]).
#[derive(Clone, Copy, Debug)]
struct RingCell {
    dx: i64,
    dz: i64,
    /// The cell before it on the line from the centre (its index; none next to the centre).
    before: u32,
    /// From the centre, in sixteenths of a cell.
    dist: i64,
}

#[derive(Clone)]
pub struct Vision {
    pub width: usize,
    pub height: usize,
    /// Per player, flag bits per cell.
    cells: Vec<Vec<u8>>,
    /// Bumped on every rebuild (clients re-upload fog textures when it changes).
    pub version: u64,
    /// Radius in cells → half-width of the disc for each row offset 0..=r.
    spans: FxHashMap<i64, Vec<i64>>,
    /// The ground's height at each cell's centre (the water's surface over the sea) when
    /// hills hide what is behind them; None: line of sight is a plain disc.
    terrain: Option<Vec<i32>>,
    /// Radius in cells → the disc's cells, nearest first, for tracing over hills.
    rings: FxHashMap<i64, Vec<RingCell>>,
    /// The steepest rise met on the way out to each cell of the disc being traced.
    horizon: Vec<(i64, i64)>,
}

impl Vision {
    pub fn new(width_px: i64, height_px: i64, players: usize) -> Vision {
        let width = ((width_px + VISION_CELL - 1) / VISION_CELL).max(1) as usize;
        let height = ((height_px + VISION_CELL - 1) / VISION_CELL).max(1) as usize;
        Vision { width, height, cells: vec![vec![0; width * height]; players], version: 0, spans: FxHashMap::default(), terrain: None, rings: FxHashMap::default(), horizon: Vec::new() }
    }

    /// Hills hide what is behind them from now on (TA's "true" line of sight): `heights`
    /// is the ground at each cell's centre, row-major. None: line of sight is a disc again.
    pub fn set_terrain(&mut self, heights: Option<Vec<i32>>) {
        self.terrain = heights.filter(|h| h.len() == self.width * self.height);
    }

    /// Cell containing a world position (clamped to the grid).
    pub fn cell_of(&self, pos: FxVec3) -> (usize, usize) {
        let x = (pos.x.to_int() / VISION_CELL).clamp(0, self.width as i64 - 1) as usize;
        let z = (pos.z.to_int() / VISION_CELL).clamp(0, self.height as i64 - 1) as usize;
        (x, z)
    }

    /// Flag bits for `player` at a world position.
    pub fn flags_at(&self, player: u8, pos: FxVec3) -> u8 {
        let (x, z) = self.cell_of(pos);
        self.cells.get(player as usize).map_or(0, |c| c[z * self.width + x])
    }

    /// Raw per-cell flags of one player, row-major `width × height` (for fog textures).
    pub fn player_cells(&self, player: u8) -> &[u8] {
        self.cells.get(player as usize).map_or(&[], |c| c.as_slice())
    }

    /// Sets flag bits on every cell of a player's grid.
    pub fn set_all(&mut self, player: u8, bits: u8) {
        if let Some(c) = self.cells.get_mut(player as usize) {
            for v in c.iter_mut() {
                *v |= bits;
            }
            self.version += 1;
        }
    }

    /// Rebuilds coverage from the given sensors (explored bits persist).
    pub fn rebuild(&mut self, sensors: &[Sensor]) {
        for c in &mut self.cells {
            for v in c.iter_mut() {
                *v &= EXPLORED;
            }
        }
        let (w, h) = (self.width as i64, self.height as i64);
        for s in sensors {
            let Some(grid) = self.cells.get_mut(s.player as usize) else { continue };
            // (Never past the map: a disc as wide as it covers every cell of it.)
            let r = ((s.radius + VISION_CELL / 2) / VISION_CELL).min(w + h);
            if r <= 0 {
                continue;
            }
            let spans = self.spans.entry(r).or_insert_with(|| disc_spans(r));
            let cx = s.pos.x.to_int().div_euclid(VISION_CELL);
            let cz = s.pos.z.to_int().div_euclid(VISION_CELL);
            let bits = if s.flag == LOS { LOS | EXPLORED } else { s.flag };
            if s.flag == LOS
                && let Some(terrain) = &self.terrain
                && (0..w).contains(&cx)
                && (0..h).contains(&cz)
            {
                let rings = self.rings.entry(r).or_insert_with(|| disc_rings(r));
                stamp_over_hills(grid, terrain, rings, &mut self.horizon, (w, h), (cx, cz), s.eye, bits);
                continue;
            }
            for dz in -r..=r {
                let z = cz + dz;
                if z < 0 || z >= h {
                    continue;
                }
                let hw = spans[dz.unsigned_abs() as usize];
                let (x0, x1) = ((cx - hw).max(0), (cx + hw).min(w - 1));
                if x0 > x1 {
                    continue;
                }
                let row = (z * w) as usize;
                for v in &mut grid[row + x0 as usize..=row + x1 as usize] {
                    *v |= bits;
                }
            }
        }
        self.version += 1;
    }
}

/// Line of sight over hills, a disc round `centre` (inside the grid), ring by ring out from
/// the eye: each cell's horizon is the steepest rise on the line back to the eye (its own
/// ground's, or that of the cell before it), and a cell is seen when its top clears the
/// horizon before it. Each cell once, without divisions.
#[allow(clippy::too_many_arguments)]
fn stamp_over_hills(grid: &mut [u8], terrain: &[i32], rings: &[RingCell], horizon: &mut Vec<(i64, i64)>, (w, h): (i64, i64), (cx, cz): (i64, i64), eye: i64, bits: u8) {
    horizon.clear();
    horizon.resize(rings.len(), OPEN);
    for (j, c) in rings.iter().enumerate() {
        let (x, z) = (cx + c.dx, cz + c.dz);
        if !(0..w).contains(&x) || !(0..h).contains(&z) {
            continue;
        }
        let cell = (z * w + x) as usize;
        if c.dist == 0 {
            grid[cell] |= bits;
            continue;
        }
        let rise = terrain[cell] as i64 - eye;
        let (top, at) = if c.before == u32::MAX { OPEN } else { horizon[c.before as usize] };
        if (rise + TARGET_HEIGHT) * at >= top * c.dist {
            grid[cell] |= bits;
        }
        horizon[j] = if rise * at > top * c.dist { (rise, c.dist) } else { (top, at) };
    }
}

/// A disc of `r` cells, nearest ring first (a cell's ring: its larger offset), each cell
/// with the cell before it on the line from the centre: the nearest to that line in the
/// ring inside.
fn disc_rings(r: i64) -> Vec<RingCell> {
    let mut offsets: Vec<(i64, i64)> = (-r..=r).flat_map(|dz| (-r..=r).map(move |dx| (dx, dz))).filter(|&(dx, dz)| dx * dx + dz * dz <= r * r).collect();
    offsets.sort_by_key(|&(dx, dz)| (dx.abs().max(dz.abs()), dz, dx));
    let index: FxHashMap<(i64, i64), u32> = offsets.iter().enumerate().map(|(i, &o)| (o, i as u32)).collect();
    // `a × (k - 1) / k` to the nearest, the same either side of the centre.
    let inward = |a: i64, k: i64| a.signum() * ((2 * a.abs() * (k - 1) + k) / (2 * k));
    offsets
        .iter()
        .map(|&(dx, dz)| {
            let k = dx.abs().max(dz.abs());
            let before = if k < 2 { u32::MAX } else { index.get(&(inward(dx, k), inward(dz, k))).copied().unwrap_or(u32::MAX) };
            let dist = crate::fixed::isqrt_u128((256 * (dx * dx + dz * dz)) as u128) as i64;
            RingCell { dx, dz, before, dist }
        })
        .collect()
}

/// Half-widths of a disc of radius `r` cells for each row offset.
fn disc_spans(r: i64) -> Vec<i64> {
    (0..=r)
        .map(|dz| {
            let rem = (r * r - dz * dz).max(0) as u128;
            crate::fixed::isqrt_u128(rem) as i64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::Fx;

    #[test]
    fn stamping_and_explored() {
        let mut v = Vision::new(640, 640, 2);
        let at = FxVec3::new(Fx::from_int(320), Fx::ZERO, Fx::from_int(320));
        v.rebuild(&[Sensor { player: 0, pos: at, radius: 96, flag: LOS, eye: 16 }]);
        assert_eq!(v.flags_at(0, at) & LOS, LOS);
        assert_eq!(v.flags_at(1, at), 0);
        let edge = FxVec3::new(Fx::from_int(320 + 90), Fx::ZERO, Fx::from_int(320));
        assert_eq!(v.flags_at(0, edge) & LOS, LOS);
        let outside = FxVec3::new(Fx::from_int(320 + 200), Fx::ZERO, Fx::from_int(320));
        assert_eq!(v.flags_at(0, outside), 0);
        // Sight moves on: the old spot stays explored but leaves line of sight.
        v.rebuild(&[]);
        assert_eq!(v.flags_at(0, at), EXPLORED);
    }

    #[test]
    fn a_sensor_past_the_map_sees_the_whole_map_and_no_further() {
        let mut v = Vision::new(640, 640, 1);
        let at = FxVec3::new(Fx::from_int(320), Fx::ZERO, Fx::from_int(320));
        // (A mod's "unlimited" sight: stamped as wide as the map, at once.)
        let started = std::time::Instant::now();
        v.rebuild(&[Sensor { player: 0, pos: at, radius: i64::MAX / 4, flag: LOS, eye: 16 }]);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        for corner in [(0, 0), (639, 0), (0, 639), (639, 639)] {
            let p = FxVec3::new(Fx::from_int(corner.0), Fx::ZERO, Fx::from_int(corner.1));
            assert_eq!(v.flags_at(0, p) & LOS, LOS);
        }
    }

    #[test]
    fn hills_hide_what_is_behind_them() {
        // A 20 × 20 cell plain with a wall two cells east of the middle.
        let mut v = Vision::new(640, 640, 1);
        let mut heights = vec![0; 400];
        for z in 0..20 {
            heights[z * 20 + 12] = 120;
        }
        let px = |cx: i64, cz: i64| FxVec3::new(Fx::from_int(cx * 32 + 16), Fx::ZERO, Fx::from_int(cz * 32 + 16));
        let seen = |v: &Vision, cx: i64, cz: i64| v.flags_at(0, px(cx, cz)) & LOS != 0;
        let eye = Sensor { player: 0, pos: px(10, 10), radius: 8 * 32, flag: LOS, eye: 20 };
        // A circle without the ground's heights.
        v.rebuild(&[eye]);
        assert!(seen(&v, 16, 10) && seen(&v, 4, 10));
        v.set_terrain(Some(heights.clone()));
        v.rebuild(&[eye]);
        assert!(seen(&v, 11, 10) && seen(&v, 12, 10), "up to the wall and its top");
        assert!(!seen(&v, 14, 10) && !seen(&v, 16, 10) && !seen(&v, 15, 12), "behind it");
        assert!(seen(&v, 4, 10) && seen(&v, 10, 4) && seen(&v, 10, 16), "the open sides");
        assert!(!seen(&v, 19, 10), "past the radius");
        // From up high the wall hides less.
        v.rebuild(&[Sensor { eye: 400, ..eye }]);
        assert!(seen(&v, 16, 10));
        // Explored ground stays explored.
        v.rebuild(&[]);
        assert_eq!(v.flags_at(0, px(11, 10)), EXPLORED);
    }
}
