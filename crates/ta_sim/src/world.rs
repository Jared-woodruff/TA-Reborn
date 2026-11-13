//! Terrain, passability, occupancy and resources on TA's 16-pixel cell grid.

use std::sync::Arc;

use rustc_hash::FxHashMap;
use ta_formats::tnt::{FEATURE_VOID, TntMap};

use crate::defs::{Locomotion, UnitType};
use crate::fixed::{Fx, FxVec3};

pub const CELL: i64 = 16;

/// A connected region of a movement class (see [`World::component`]); 0 is none. Maps have
/// at most 2^28 cells, so labels never run out.
pub type Region = u32;

/// Terrain constraints of a movement type; unit types sharing one share passability data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MoveSpec {
    pub footprint: (u32, u32),
    pub max_slope: i32,
    pub max_depth: i32,
    pub min_depth: i32,
    pub hover: bool,
}

impl MoveSpec {
    pub fn for_type(t: &UnitType, classes: &[ta_data::MoveClass]) -> MoveSpec {
        let (max_slope, max_depth, min_depth) = match t.move_class.and_then(|c| classes.get(c)) {
            Some(mc) => (mc.max_slope, mc.max_water_depth, mc.min_water_depth),
            None => (t.def.max_slope, t.def.max_water_depth, t.def.min_water_depth),
        };
        let hover = t.locomotion == Locomotion::Hover;
        let draft = Fx(t.waterline.0.saturating_add(Fx::ONE.0 - 1)).to_int().min(i32::MAX as i64) as i32;
        MoveSpec {
            footprint: t.footprint,
            max_slope: max_slope as i32,
            // Water-only units commonly omit MaxWaterDepth, as water buildings do.
            // Zero there means no maximum, rather than a contradictory dry-only limit.
            max_depth: if hover || t.locomotion == Locomotion::Naval && max_depth <= 0.0 { 10_000 } else { max_depth as i32 },
            min_depth: if t.locomotion == Locomotion::Naval { (min_depth as i32).max(draft).max(1) } else { -10_000 },
            hover,
        }
    }

    fn terrain_key(&self) -> (i32, i32, i32, bool) {
        (self.max_slope, self.max_depth, self.min_depth, self.hover)
    }
}

/// Reasons a building cannot be placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementError {
    OutOfBounds,
    Occupied,
    Feature,
    TooDeep,
    NeedsWater,
    TooSteep,
    /// Geothermal plants must stand on a vent.
    NeedsGeothermal,
    /// Extractors must cover a metal deposit (on a metal world, anywhere).
    NeedsMetal,
}

/// Plain ground this rich in metal (TA's metal worlds give 250 and more; ordinary ground 1 to
/// 5, lava 20 to 40) can be mined anywhere; elsewhere extractors go on deposits.
pub const METAL_WORLD: u32 = 100;

/// How far the ground under a landed aircraft may rise and fall, for each cell of its footprint's
/// longer side (px): 3 in 16, about 10 degrees (6 px under a fighter's 2 × 2 cells, 9 under a
/// bomber's 3 × 3). An aircraft sits level on its footprint's highest point (the renderer levels
/// no pad under it, as it does under a building), so over steeper ground it hung in the air on
/// the low side and cut into the hill on the high side. TA's own rule for aircraft is a
/// building's (every aircraft says `MaxSlope=10`: each cell's corners within 10 px, a 2 × 2
/// footprint up to 20 px across, 32 degrees), which its flat pictures never showed.
pub const LANDING_RISE_PER_CELL: i64 = 3;

/// Where an aircraft of a kind may set down (see `World::air_landing_height`;
/// `UnitType::landing` gives a kind's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landing {
    pub footprint: (u32, u32),
    /// How far its model reaches from its middle across the ground (px): its wings, nose and
    /// tail, under which the ground may not rise into them.
    pub reach: i64,
    /// Its `MaxSlope`: no cell under it steeper (each cell's corners within this, px).
    pub max_slope: i64,
    /// A seaplane (`Amphibious`) also sets down on water this deep at most (its `MaxWaterDepth`).
    pub water: Option<i64>,
}

impl Landing {
    /// How far the ground under it may rise and fall (px): [`LANDING_RISE_PER_CELL`] for each
    /// cell of its footprint's longer side.
    pub fn rise(&self) -> i64 {
        self.footprint.0.max(self.footprint.1) as i64 * LANDING_RISE_PER_CELL
    }
}

/// A change to the cells' occupants: the [`World::occupancy_version`] it made and the cells
/// it covered (`from` up to, not including, `to`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccupancyChange {
    pub version: u64,
    pub from: (i32, i32),
    pub to: (i32, i32),
}

/// Changes to the occupants kept (see [`World::changes_since`]): a distance field further
/// behind than this is built again rather than brought up to date.
const CHANGES_KEPT: usize = 4096;

/// What occupies a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occupant {
    Free,
    /// A building (unit slot).
    Unit(u32),
    /// A blocking feature (feature slot).
    Feature(u32),
}

/// The map's cells. What never changes after the map is set up (the terrain, the regions
/// and passability computed from it) is shared between copies (`Arc`), so a copy of the
/// game (the battle highlights keep a few) costs little more than its buildings and units.
#[derive(Clone)]
pub struct World {
    pub width: u32,
    pub height: u32,
    heights: Arc<Vec<u8>>,
    pub sea_level: Fx,
    void: Arc<Vec<bool>>,
    /// Per-cell slope: max minus min of the cell's four corner heights.
    slope: Arc<Vec<u8>>,
    /// Per-cell extractable metal value.
    pub metal: Vec<u32>,
    /// The map's plain ground metal (`SurfaceMetal`); richer cells are deposits.
    pub surface_metal: u32,
    occupancy: Vec<Occupant>,
    /// Building cells reserved for construction even while a factory yard is open.
    reservations: Vec<Option<u32>>,
    /// Cells over a geothermal vent.
    geo: Arc<Vec<bool>>,
    /// Incremented whenever buildings or blocking features change (path cache key).
    pub occupancy_version: u64,
    /// Incremented only when cells become blocked. Routes planned before stay valid when
    /// cells merely open up (a destroyed building, a reclaimed rock), so shared paths and
    /// flow fields key on this.
    pub blocking_version: u64,
    /// The latest changes to the occupants, oldest first (at most [`CHANGES_KEPT`]): what a
    /// shared route's distance field catches up on (`FlowField::repair`).
    changes: std::collections::VecDeque<OccupancyChange>,
    /// Cell passability bitmaps keyed by terrain constraints.
    terrain_ok: FxHashMap<(i32, i32, i32, bool), Arc<Vec<u64>>>,
    /// Connected regions of anchor cells per movement spec (0 = impassable), computed on
    /// static terrain. Lets path requests to unreachable goals resolve instantly.
    components: FxHashMap<MoveSpec, Arc<Vec<Region>>>,
    /// Footprint-expanded terrain passability per spec (bit per anchor cell), so a path
    /// step costs one bit test plus the dynamic occupancy check.
    anchor_ok: FxHashMap<MoveSpec, Arc<Vec<u64>>>,
}

impl World {
    pub fn new(map: &TntMap) -> World {
        let (w, h) = (map.width, map.height);
        let heights = map.heights.clone();
        let void: Vec<bool> = map.features.iter().map(|&f| f == FEATURE_VOID).collect();
        let cells = w as usize * h as usize;
        let mut slope = vec![0u8; cells];
        let hv = |x: u32, z: u32| heights[z.min(h - 1) as usize * w as usize + x.min(w - 1) as usize];
        for z in 0..h {
            for x in 0..w {
                let c = [hv(x, z), hv(x + 1, z), hv(x, z + 1), hv(x + 1, z + 1)];
                // A cliff the map marks (one converted from another game, its heights pressed
                // into TA's) is as steep as a slope can be: nothing that minds slopes crosses
                // it or builds on it.
                slope[z as usize * w as usize + x as usize] =
                    if map.flags_at(x, z) & ta_formats::tnt::CELL_CLIFF != 0 { u8::MAX } else { c.iter().max().unwrap() - c.iter().min().unwrap() };
            }
        }
        World {
            width: w,
            height: h,
            heights: Arc::new(heights),
            sea_level: Fx::from_int(map.sea_level as i64),
            void: Arc::new(void),
            slope: Arc::new(slope),
            metal: vec![0; cells],
            surface_metal: 0,
            occupancy: vec![Occupant::Free; cells],
            reservations: vec![None; cells],
            geo: Arc::new(vec![false; cells]),
            occupancy_version: 0,
            blocking_version: 0,
            changes: Default::default(),
            terrain_ok: FxHashMap::default(),
            components: FxHashMap::default(),
            anchor_ok: FxHashMap::default(),
        }
    }

    pub fn size_px(&self) -> (Fx, Fx) {
        (Fx::from_int(self.width as i64 * CELL), Fx::from_int(self.height as i64 * CELL))
    }

    #[inline]
    pub fn in_bounds(&self, cx: i32, cz: i32) -> bool {
        cx >= 0 && cz >= 0 && (cx as u32) < self.width && (cz as u32) < self.height
    }

    #[inline]
    fn idx(&self, cx: i32, cz: i32) -> usize {
        cz as usize * self.width as usize + cx as usize
    }

    #[inline]
    pub fn cell_of(pos: FxVec3) -> (i32, i32) {
        ((pos.x.0 >> 20) as i32, (pos.z.0 >> 20) as i32) // 16.16 / 16 px
    }

    /// Height of the grid vertex (cx, cz), clamped to the map (matches the render mesh).
    #[inline]
    pub fn vertex_height(&self, cx: i32, cz: i32) -> Fx {
        let x = cx.clamp(0, self.width as i32 - 1);
        let z = cz.clamp(0, self.height as i32 - 1);
        Fx::from_int(self.heights[self.idx(x, z)] as i64)
    }

    /// The highest ground under a footprint (its cells' corners). Buildings sit there: TA drew
    /// them over a flat picture of the map, so their pads never met the ground; on the 3D
    /// ground any bump under a flat pad would poke through it.
    pub fn footprint_top(&self, origin: (i32, i32), fp: (u32, u32)) -> Fx {
        let mut top = self.vertex_height(origin.0, origin.1);
        for z in origin.1..=origin.1 + fp.1 as i32 {
            for x in origin.0..=origin.0 + fp.0 as i32 {
                top = top.max(self.vertex_height(x, z));
            }
        }
        top
    }

    /// Where an aircraft of kind `l` may set down with its footprint about `pos` (flight and the
    /// crowd's separation of landed aircraft ask, and a scenario parking its aircraft at the
    /// start): the height it rests at, its footprint's highest point, or `None`. Its cells must
    /// be on the map, ground (not a space map's void) and free of buildings and features. Under it
    /// the ground must be dry, or for a seaplane water no deeper than it takes (it rests on the
    /// surface), varying by no more than [`Landing::rise`] and with no cell steeper than its
    /// `MaxSlope`; and round it, as far as its model reaches, no ground may stand more than that
    /// over where it rests: its wings, nose and tail would cut into the hillside, or the foot of
    /// the cliff, beside a flat place.
    pub fn air_landing_height(&self, pos: FxVec3, l: &Landing) -> Option<Fx> {
        let fp = l.footprint;
        let (x0, z0) = Self::footprint_origin(Self::pos_to_anchor(pos, fp), fp);
        let (x1, z1) = (x0 + fp.0 as i32, z0 + fp.1 as i32);
        for z in z0..z1 {
            for x in x0..x1 {
                if !self.in_bounds(x, z) || self.void[self.idx(x, z)] || self.occupancy[self.idx(x, z)] != Occupant::Free {
                    return None;
                }
            }
        }
        let sea = self.sea_level.to_int();
        // What it rests on at a corner: the ground, or a seaplane's water.
        let surface = |x: i32, z: i32| {
            let h = self.vertex_height(x, z).to_int();
            if h >= sea { Some(h) } else { l.water.filter(|&deep| sea - h <= deep).map(|_| sea) }
        };
        let (mut top, mut low) = (i64::MIN, i64::MAX);
        for z in z0..=z1 {
            for x in x0..=x1 {
                let s = surface(x, z)?;
                top = top.max(s);
                low = low.min(s);
            }
        }
        let rise = l.rise();
        if top - low > rise {
            return None;
        }
        // (A cell can be steeper than the whole only for a kind whose own limit is stricter.)
        if l.max_slope < rise {
            for z in z0..z1 {
                for x in x0..x1 {
                    let c = [surface(x, z)?, surface(x + 1, z)?, surface(x, z + 1)?, surface(x + 1, z + 1)?];
                    if c.iter().max()? - c.iter().min()? > l.max_slope {
                        return None;
                    }
                }
            }
        }
        // Under its wings, nose and tail (the corners within its reach of its middle, and half a
        // cell more: the ground between them is drawn from them).
        let (mx, mz) = (x0 as i64 * CELL + fp.0 as i64 * CELL / 2, z0 as i64 * CELL + fp.1 as i64 * CELL / 2);
        let near = l.reach + CELL / 2;
        let beyond = |side: u32| ((l.reach - side as i64 * CELL / 2).max(0) + CELL - 1) / CELL;
        let (bx, bz) = (beyond(fp.0) as i32, beyond(fp.1) as i32);
        for z in z0 - bz..=z1 + bz {
            for x in x0 - bx..=x1 + bx {
                let (dx, dz) = (x as i64 * CELL - mx, z as i64 * CELL - mz);
                if (x0..=x1).contains(&x) && (z0..=z1).contains(&z) || dx * dx + dz * dz > near * near {
                    continue;
                }
                if self.vertex_height(x, z).to_int() > top + rise {
                    return None;
                }
            }
        }
        Some(Fx::from_int(top))
    }

    /// Ground height at a world position, interpolated on the same triangles the renderer
    /// draws (quads split along the (0,0)-(1,1) diagonal).
    pub fn height_at(&self, x: Fx, z: Fx) -> Fx {
        let cx = (x.0 >> 20) as i32;
        let cz = (z.0 >> 20) as i32;
        let tx = Fx((x.0 & 0xF_FFFF) >> 4);
        let tz = Fx((z.0 & 0xF_FFFF) >> 4);
        let h00 = self.vertex_height(cx, cz);
        let h11 = self.vertex_height(cx + 1, cz + 1);
        if tx >= tz {
            let h10 = self.vertex_height(cx + 1, cz);
            h00 + (h10 - h00) * tx + (h11 - h10) * tz
        } else {
            let h01 = self.vertex_height(cx, cz + 1);
            h00 + (h11 - h01) * tx + (h01 - h00) * tz
        }
    }

    /// Water depth at a position (positive under water).
    pub fn depth_at(&self, x: Fx, z: Fx) -> Fx {
        self.sea_level - self.height_at(x, z)
    }

    pub fn cell_depth(&self, cx: i32, cz: i32) -> i32 {
        (self.sea_level.to_int() - self.vertex_height(cx, cz).to_int()) as i32
    }

    pub fn slope(&self, cx: i32, cz: i32) -> u8 {
        if self.in_bounds(cx, cz) { self.slope[self.idx(cx, cz)] } else { 255 }
    }

    pub fn is_void(&self, cx: i32, cz: i32) -> bool {
        !self.in_bounds(cx, cz) || self.void[self.idx(cx, cz)]
    }

    fn cell_terrain_ok(&self, spec: &MoveSpec, cx: i32, cz: i32) -> bool {
        if self.is_void(cx, cz) {
            return false;
        }
        let depth = self.cell_depth(cx, cz);
        if depth < spec.min_depth || depth > spec.max_depth {
            return false;
        }
        // Hovercraft ignore slope on water; everything else is limited on land and seabed.
        (spec.hover && depth > 0) || spec.min_depth > 0 || self.slope(cx, cz) as i32 <= spec.max_slope
    }

    /// Precomputes terrain passability and connected regions for a movement spec.
    pub fn prepare(&mut self, spec: &MoveSpec) {
        self.prepare_all(std::slice::from_ref(spec));
    }

    /// [`World::prepare`] for many specs at once, on all cores (every spec's regions are
    /// independent of the others'; the results are the same as preparing them one by one).
    pub fn prepare_all(&mut self, specs: &[MoveSpec]) {
        use rayon::prelude::*;
        let mut terrain: Vec<MoveSpec> = Vec::new();
        for s in specs {
            if !self.terrain_ok.contains_key(&s.terrain_key()) && !terrain.iter().any(|t| t.terrain_key() == s.terrain_key()) {
                terrain.push(*s);
            }
        }
        let bits: Vec<_> = terrain.par_iter().map(|s| (s.terrain_key(), Arc::new(self.terrain_bits(s)))).collect();
        self.terrain_ok.extend(bits);
        let mut todo: Vec<MoveSpec> = Vec::new();
        for s in specs {
            if !self.components.contains_key(s) && !todo.contains(s) {
                todo.push(*s);
            }
        }
        let regions: Vec<_> = todo.par_iter().map(|s| (*s, self.regions(s))).collect();
        for (s, (comp, anchor_bits)) in regions {
            self.anchor_ok.insert(s, Arc::new(anchor_bits));
            self.components.insert(s, Arc::new(comp));
        }
    }

    /// Anchor cells whose whole footprint is terrain-passable for `spec` (row by row), from
    /// two sliding-window counts of unsuitable cells; the map's outside counts as unsuitable.
    fn terrain_open(&self, spec: &MoveSpec) -> Vec<bool> {
        let (w, h) = (self.width as usize, self.height as usize);
        let Some(bits) = self.terrain_ok.get(&spec.terrain_key()) else { return vec![false; w * h] };
        let bad = |i: usize| bits[i >> 6] & (1 << (i & 63)) == 0;
        let (fx, fz) = (spec.footprint.0 as i64, spec.footprint.1 as i64);
        // Unsuitable cells in each anchor's footprint columns...
        let mut row_bad = vec![0u32; w * h];
        let mut prefix = vec![0u32; w + 1];
        for z in 0..h {
            for x in 0..w {
                prefix[x + 1] = prefix[x] + bad(z * w + x) as u32;
            }
            for x in 0..w {
                let x0 = x as i64 - fx / 2;
                let (a, b) = (x0.clamp(0, w as i64) as usize, (x0 + fx).clamp(0, w as i64) as usize);
                let outside = (x0 < 0) as u32 + (x0 + fx > w as i64) as u32;
                row_bad[z * w + x] = prefix[b] - prefix[a] + outside;
            }
        }
        // ...then over its rows.
        let mut column = vec![0u32; (h + 1) * w];
        for z in 0..h {
            for x in 0..w {
                column[(z + 1) * w + x] = column[z * w + x] + row_bad[z * w + x];
            }
        }
        let mut open = vec![false; w * h];
        for z in 0..h {
            let z0 = z as i64 - fz / 2;
            let (a, b) = (z0.clamp(0, h as i64) as usize, (z0 + fz).clamp(0, h as i64) as usize);
            let outside = z0 < 0 || z0 + fz > h as i64;
            for x in 0..w {
                open[z * w + x] = !outside && column[b * w + x] == column[a * w + x];
            }
        }
        open
    }

    /// Connected regions of anchor cells (8-connected, no corner cutting, like the
    /// pathfinder), and the bitmap of cells in any region.
    fn regions(&self, spec: &MoveSpec) -> (Vec<Region>, Vec<u64>) {
        let (w, h) = (self.width as i32, self.height as i32);
        let open = self.terrain_open(spec);
        let index = |c: (i32, i32)| c.1 as usize * w as usize + c.0 as usize;
        let passable = |c: (i32, i32)| c.0 >= 0 && c.1 >= 0 && c.0 < w && c.1 < h && open[index(c)];
        let mut comp: Vec<Region> = vec![0; w as usize * h as usize];
        let mut next: Region = 1;
        let mut stack = Vec::new();
        for start in 0..comp.len() {
            if comp[start] != 0 || !open[start] {
                continue;
            }
            // (Every region has a cell of its own: the labels never run out.)
            let id = next;
            next += 1;
            comp[start] = id;
            stack.push(((start % w as usize) as i32, (start / w as usize) as i32));
            while let Some((x, z)) = stack.pop() {
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                    let n = (x + dx, z + dz);
                    if !passable(n) {
                        continue;
                    }
                    let ni = index(n);
                    if comp[ni] != 0 {
                        continue;
                    }
                    if dx != 0 && dz != 0 && !(passable((x + dx, z)) && passable((x, z + dz))) {
                        continue;
                    }
                    comp[ni] = id;
                    stack.push(n);
                }
            }
        }
        let mut bits = vec![0u64; comp.len().div_ceil(64)];
        for (i, &c) in comp.iter().enumerate() {
            if c != 0 {
                bits[i >> 6] |= 1 << (i & 63);
            }
        }
        (comp, bits)
    }

    /// How many anchor cells each region of a spec has, by region id (`[0]`: those it cannot
    /// stand on); none for a spec without regions.
    pub fn region_sizes(&self, spec: &MoveSpec) -> Vec<usize> {
        let Some(comp) = self.components.get(spec) else { return Vec::new() };
        let mut sizes = vec![0; comp.iter().copied().max().unwrap_or(0) as usize + 1];
        for &region in comp.iter() {
            sizes[region as usize] += 1;
        }
        sizes
    }

    /// Region id of an anchor cell for a spec (0 = impassable or unknown).
    pub fn component(&self, spec: &MoveSpec, anchor: (i32, i32)) -> Region {
        if !self.in_bounds(anchor.0, anchor.1) {
            return 0;
        }
        self.components.get(spec).map_or(0, |c| c[self.idx(anchor.0, anchor.1)])
    }

    /// Whether a cell of `region` lies within `slack` cells (Chebyshev) of `anchor`: the anchor
    /// first, then ring by ring.
    pub fn region_within(&self, spec: &MoveSpec, region: Region, anchor: (i32, i32), slack: i32) -> bool {
        let Some(comp) = self.components.get(spec) else { return false };
        let at = |c: (i32, i32)| self.in_bounds(c.0, c.1) && comp[self.idx(c.0, c.1)] == region;
        (0..=slack).any(|ring| (-ring..=ring).any(|dz| (-ring..=ring).any(|dx| (dx.abs() == ring || dz.abs() == ring) && at((anchor.0 + dx, anchor.1 + dz)))))
    }

    /// Nearest anchor cell to `goal` inside region `region` (spiral search).
    pub fn nearest_in_component(&self, spec: &MoveSpec, region: Region, goal: (i32, i32), max_radius: i32) -> Option<(i32, i32)> {
        let comp = self.components.get(spec)?;
        for r in 0..=max_radius {
            let mut best: Option<(i32, (i32, i32))> = None;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let c = (goal.0 + dx, goal.1 + dz);
                    if self.in_bounds(c.0, c.1) && comp[self.idx(c.0, c.1)] == region {
                        let d = dx * dx + dz * dz;
                        if best.is_none_or(|(bd, bc)| d < bd || d == bd && c < bc) {
                            best = Some((d, c));
                        }
                    }
                }
            }
            if let Some((_, c)) = best {
                return Some(c);
            }
        }
        None
    }

    #[cfg(test)]
    fn prepare_terrain(&mut self, spec: &MoveSpec) {
        let key = spec.terrain_key();
        if !self.terrain_ok.contains_key(&key) {
            let bits = self.terrain_bits(spec);
            self.terrain_ok.insert(key, Arc::new(bits));
        }
    }

    /// Cells whose terrain (depth, slope, void) suits a spec.
    fn terrain_bits(&self, spec: &MoveSpec) -> Vec<u64> {
        let n = self.width as usize * self.height as usize;
        let mut bits = vec![0u64; n.div_ceil(64)];
        for cz in 0..self.height as i32 {
            for cx in 0..self.width as i32 {
                if self.cell_terrain_ok(spec, cx, cz) {
                    let i = self.idx(cx, cz);
                    bits[i >> 6] |= 1 << (i & 63);
                }
            }
        }
        bits
    }

    #[inline]
    fn terrain_bit(&self, bits: &[u64], cx: i32, cz: i32) -> bool {
        self.in_bounds(cx, cz) && {
            let i = self.idx(cx, cz);
            bits[i >> 6] & (1 << (i & 63)) != 0
        }
    }

    /// Top-left cell of a footprint centred at an anchor cell.
    #[inline]
    pub fn footprint_origin(anchor: (i32, i32), fp: (u32, u32)) -> (i32, i32) {
        (anchor.0 - fp.0 as i32 / 2, anchor.1 - fp.1 as i32 / 2)
    }

    /// Can a unit of this spec stand with its footprint anchored at `anchor`?
    /// `ignore` excludes one building (the unit itself / its factory).
    pub fn passable(&self, spec: &MoveSpec, anchor: (i32, i32), ignore: Option<u32>) -> bool {
        self.passable_with(self.anchor_ok.get(spec).map(|b| &b[..]), spec, anchor, ignore)
    }

    /// Check each anchor crossed by a physical step, in exact boundary order.
    /// Simultaneous diagonal crossings need an open orthogonal side, as sliding
    /// routes do. A unit escaping occupancy may cross its original obstacles,
    /// but never new obstacles, invalid terrain or map bounds.
    pub(crate) fn ground_segment_open(&self, spec: &MoveSpec, from: FxVec3, to: FxVec3, ignore: Option<u32>, escaping: bool) -> bool {
        let start = Self::pos_to_anchor(from, spec.footprint);
        let mut obstacles = Vec::new();
        if escaping {
            let (x0, z0) = Self::footprint_origin(start, spec.footprint);
            for z in z0..z0 + spec.footprint.1 as i32 { for x in x0..x0 + spec.footprint.0 as i32 {
                let who = self.occupant(x, z);
                if self.in_bounds(x, z) && who != Occupant::Free && !obstacles.contains(&who) { obstacles.push(who); }
            } }
        }
        let open = |c| {
            if self.passable(spec, c, ignore) { return true; }
            if !escaping || !self.terrain_passable(spec, c) { return false; }
            let (x0, z0) = Self::footprint_origin(c, spec.footprint);
            (z0..z0 + spec.footprint.1 as i32).all(|z| (x0..x0 + spec.footprint.0 as i32).all(|x| {
                let who = self.occupant(x, z);
                who == Occupant::Free || matches!(who, Occupant::Unit(id) if Some(id) == ignore) || obstacles.contains(&who)
            }))
        };
        let end = Self::pos_to_anchor(to, spec.footprint);
        if !open(end) { return false; }
        if start == end { return true; }
        let (dx, dz) = (to.x.0 as i128 - from.x.0 as i128, to.z.0 as i128 - from.z.0 as i128);
        let (sx, sz) = (dx.signum() as i32, dz.signum() as i32);
        let cell_raw = CELL as i128 * Fx::ONE.0 as i128;
        // pos_to_anchor rounds before dividing: its cell boundary is half a pixel
        // before the footprint's grid line. Compare crossing times as fractions,
        // without division or floating point (including negative-direction steps).
        let boundary = |anchor: i32, fp: u32, step: i32| {
            (anchor as i128 - fp as i128 / 2 + i128::from(step > 0)) * cell_raw
                + fp as i128 * cell_raw / 2 - Fx::ONE.0 as i128 / 2
        };
        let mut nx = (boundary(start.0, spec.footprint.0, sx) - from.x.0 as i128).abs();
        let mut nz = (boundary(start.1, spec.footprint.1, sz) - from.z.0 as i128).abs();
        let mut at = start;
        while at != end {
            let tx = if at.0 == end.0 { i128::MAX } else { nx * dz.abs() };
            let tz = if at.1 == end.1 { i128::MAX } else { nz * dx.abs() };
            let (cross_x, cross_z) = (tx <= tz, tz <= tx);
            if cross_x && cross_z && !open((at.0 + sx, at.1)) && !open((at.0, at.1 + sz)) { return false; }
            if cross_x { at.0 += sx; nx += cell_raw; }
            if cross_z { at.1 += sz; nz += cell_raw; }
            if !open(at) { return false; }
        }
        true
    }

    /// [`World::passable`] for many cells of one movement class (a search's): its tables
    /// looked up once.
    pub(crate) fn passable_for<'a>(&'a self, spec: &'a MoveSpec) -> impl Fn((i32, i32), Option<u32>) -> bool + 'a {
        let bits = self.anchor_ok.get(spec).map(|b| &b[..]);
        move |anchor, ignore| self.passable_with(bits, spec, anchor, ignore)
    }

    #[inline]
    fn passable_with(&self, bits: Option<&[u64]>, spec: &MoveSpec, anchor: (i32, i32), ignore: Option<u32>) -> bool {
        if let Some(bits) = bits {
            if !self.terrain_bit(bits, anchor.0, anchor.1) {
                return false;
            }
        } else if !self.terrain_passable(spec, anchor) {
            return false;
        }
        let (x0, z0) = Self::footprint_origin(anchor, spec.footprint);
        for z in z0..z0 + spec.footprint.1 as i32 {
            for x in x0..x0 + spec.footprint.0 as i32 {
                match self.occupancy[self.idx(x, z)] {
                    Occupant::Free => {}
                    Occupant::Unit(u) if Some(u) == ignore => {}
                    _ => return false,
                }
            }
        }
        true
    }

    /// `passable(spec, c, None)` for every anchor cell, row by row — the same answers at a
    /// fraction of the cost of asking cell by cell (flow fields need the whole map). The
    /// footprint test becomes two sliding-window sums over the occupied cells.
    pub fn passable_all(&self, spec: &MoveSpec) -> Vec<bool> {
        self.passable_window(spec, (0, 0), (self.width as i32, self.height as i32))
    }

    /// [`World::passable_all`] for the anchor cells of a window of the map, from `from` up to
    /// (not including) `to`, row by row; the sums cover the window and the footprints' reach
    /// around it.
    pub fn passable_window(&self, spec: &MoveSpec, from: (i32, i32), to: (i32, i32)) -> Vec<bool> {
        let (w, h) = (self.width as i32, self.height as i32);
        let (fx, fz) = (spec.footprint.0 as i32, spec.footprint.1 as i32);
        let (x0, z0) = (from.0.clamp(0, w), from.1.clamp(0, h));
        let (x1, z1) = (to.0.clamp(x0, w), to.1.clamp(z0, h));
        let (ww, wh) = ((x1 - x0) as usize, (z1 - z0) as usize);
        // The columns and rows the window's footprints reach.
        let (cx0, rz0) = ((x0 - fx / 2).max(0), (z0 - fz / 2).max(0));
        let cx1 = (x1 - 1 - fx / 2 + fx).min(w).max(cx0);
        let rz1 = (z1 - 1 - fz / 2 + fz).min(h).max(rz0);
        let (cols, rows) = ((cx1 - cx0) as usize, (rz1 - rz0) as usize);
        // Occupied cells within each anchor's footprint columns, per row...
        let mut row_hits = vec![0u32; rows * ww];
        let mut prefix = vec![0u32; cols + 1];
        for r in 0..rows {
            let line = (rz0 as usize + r) * w as usize + cx0 as usize;
            for i in 0..cols {
                prefix[i + 1] = prefix[i] + (self.occupancy[line + i] != Occupant::Free) as u32;
            }
            for (k, x) in (x0..x1).enumerate() {
                let s = x - fx / 2;
                let (a, b) = ((s.clamp(cx0, cx1) - cx0) as usize, ((s + fx).clamp(cx0, cx1) - cx0) as usize);
                row_hits[r * ww + k] = prefix[b] - prefix[a];
            }
        }
        // ...then summed over the footprint's rows (a running column prefix).
        let mut column = vec![0u32; (rows + 1) * ww];
        for r in 0..rows {
            for k in 0..ww {
                column[(r + 1) * ww + k] = column[r * ww + k] + row_hits[r * ww + k];
            }
        }
        let anchor_bits = self.anchor_ok.get(spec);
        let mut open = vec![false; ww * wh];
        for (j, z) in (z0..z1).enumerate() {
            let s = z - fz / 2;
            let (a, b) = ((s.clamp(rz0, rz1) - rz0) as usize, ((s + fz).clamp(rz0, rz1) - rz0) as usize);
            for (k, x) in (x0..x1).enumerate() {
                let terrain = match anchor_bits {
                    Some(bits) => {
                        let i = z as usize * w as usize + x as usize;
                        bits[i >> 6] & (1 << (i & 63)) != 0
                    }
                    None => self.terrain_passable(spec, (x, z)),
                };
                // Terrain-passable anchors have their whole footprint on the map, so the
                // clamped windows above are exact for them.
                open[j * ww + k] = terrain && column[b * ww + k] == column[a * ww + k];
            }
        }
        open
    }

    /// Terrain-only passability (ignores buildings), used to decide whether a goal is
    /// reachable at all.
    pub fn terrain_passable(&self, spec: &MoveSpec, anchor: (i32, i32)) -> bool {
        let Some(bits) = self.terrain_ok.get(&spec.terrain_key()) else { return false };
        let (x0, z0) = Self::footprint_origin(anchor, spec.footprint);
        (z0..z0 + spec.footprint.1 as i32).all(|z| (x0..x0 + spec.footprint.0 as i32).all(|x| self.terrain_bit(bits, x, z)))
    }

    pub fn occupant(&self, cx: i32, cz: i32) -> Occupant {
        if self.in_bounds(cx, cz) { self.occupancy[self.idx(cx, cz)] } else { Occupant::Unit(u32::MAX) }
    }

    /// Notes a change to the occupants of a footprint (see [`World::changes_since`]).
    fn note_change(&mut self, origin: (i32, i32), fp: (u32, u32)) {
        if self.changes.len() >= CHANGES_KEPT {
            self.changes.pop_front();
        }
        self.changes.push_back(OccupancyChange { version: self.occupancy_version, from: origin, to: (origin.0 + fp.0 as i32, origin.1 + fp.1 as i32) });
    }

    /// The changes to the occupants after `version` (oldest first), or None when they are no
    /// longer all known.
    pub fn changes_since(&self, version: u64) -> Option<impl Iterator<Item = &OccupancyChange>> {
        let first = self.changes.front().map_or(self.occupancy_version + 1, |c| c.version);
        if version + 1 < first {
            return None;
        }
        // (Every change made a version of its own, one after another.)
        let skip = (version + 1 - first) as usize;
        Some(self.changes.iter().skip(skip))
    }

    /// Forgets the changes made so far (a new world's buildings and features are where it
    /// starts from).
    pub fn forget_changes(&mut self) {
        self.changes.clear();
    }

    /// Marks a building's blocking cells.
    pub fn occupy(&mut self, who: Occupant, origin: (i32, i32), fp: (u32, u32), blocking: &[bool]) {
        self.occupancy_version += 1;
        self.blocking_version += 1;
        self.note_change(origin, fp);
        for dz in 0..fp.1 as i32 {
            for dx in 0..fp.0 as i32 {
                let (x, z) = (origin.0 + dx, origin.1 + dz);
                let solid = blocking.get((dz as u32 * fp.0 + dx as u32) as usize).copied().unwrap_or(true);
                if solid && self.in_bounds(x, z) {
                    let i = self.idx(x, z);
                    self.occupancy[i] = who;
                }
            }
        }
    }

    /// Apply a building's yard state without giving up its construction reservation.
    pub(crate) fn occupy_yard(&mut self, slot: u32, origin: (i32, i32), fp: (u32, u32), closed: &[bool], open: &[bool], opened: bool) {
        self.occupancy_version += 1;
        self.blocking_version += 1;
        self.note_change(origin, fp);
        let who = Occupant::Unit(slot);
        for dz in 0..fp.1 as i32 {
            for dx in 0..fp.0 as i32 {
                let (x, z) = (origin.0 + dx, origin.1 + dz);
                if !self.in_bounds(x, z) { continue; }
                let cell = (dz as u32 * fp.0 + dx as u32) as usize;
                let (closed, open) = (closed.get(cell).copied().unwrap_or(true), open.get(cell).copied().unwrap_or(true));
                let i = self.idx(x, z);
                if closed || open { self.reservations[i] = Some(slot); }
                if self.occupancy[i] == who || self.occupancy[i] == Occupant::Free {
                    let blocks = if opened { open } else { closed };
                    self.occupancy[i] = if blocks { who } else { Occupant::Free };
                }
            }
        }
    }

    pub fn release(&mut self, who: Occupant, origin: (i32, i32), fp: (u32, u32)) {
        self.occupancy_version += 1;
        self.note_change(origin, fp);
        for dz in 0..fp.1 as i32 {
            for dx in 0..fp.0 as i32 {
                let (x, z) = (origin.0 + dx, origin.1 + dz);
                if self.in_bounds(x, z) {
                    let i = self.idx(x, z);
                    if self.occupancy[i] == who {
                        self.occupancy[i] = Occupant::Free;
                    }
                    if matches!(who, Occupant::Unit(slot) if self.reservations[i] == Some(slot)) {
                        self.reservations[i] = None;
                    }
                }
            }
        }
    }

    /// Why a building of type `t` cannot be placed with its footprint's top-left at
    /// `origin`, or `None` if it fits.
    pub fn placement_error(&self, t: &UnitType, max_slope: i32, origin: (i32, i32)) -> Option<PlacementError> {
        let (fx, fz) = t.footprint;
        let d = &t.def;
        // Land buildings say `MaxWaterDepth=0` (dry land only); water buildings (shipyards,
        // tidal generators, underwater extractors, platforms) give a minimum depth and leave
        // the maximum out or at 0, which puts no limit on it.
        let max_depth = if d.min_water_depth > 0.0 && d.max_water_depth <= d.min_water_depth { i32::MAX } else { d.max_water_depth as i32 };
        for z in origin.1..origin.1 + fz as i32 {
            for x in origin.0..origin.0 + fx as i32 {
                if self.is_void(x, z) {
                    return Some(PlacementError::OutOfBounds);
                }
                if self.reservations[self.idx(x, z)].is_some() {
                    return Some(PlacementError::Occupied);
                }
                match self.occupancy[self.idx(x, z)] {
                    Occupant::Free => {}
                    Occupant::Unit(_) => return Some(PlacementError::Occupied),
                    Occupant::Feature(_) => return Some(PlacementError::Feature),
                }
                let depth = self.cell_depth(x, z);
                if depth > max_depth {
                    return Some(PlacementError::TooDeep);
                }
                if depth < d.min_water_depth as i32 {
                    return Some(PlacementError::NeedsWater);
                }
                if depth <= 0 && self.slope(x, z) as i32 > max_slope {
                    return Some(PlacementError::TooSteep);
                }
            }
        }
        if t.needs_geo && !self.geo_under(origin, t.footprint) {
            return Some(PlacementError::NeedsGeothermal);
        }
        if t.extracts_metal > Fx::ZERO && !t.needs_geo && !self.metal_world() && !self.ore_under(origin, t.footprint) {
            return Some(PlacementError::NeedsMetal);
        }
        None
    }

    /// Whether plain ground is worth mining (a metal world).
    pub fn metal_world(&self) -> bool {
        self.surface_metal >= METAL_WORLD
    }

    /// Whether a footprint covers any deposit cell (richer than plain ground).
    pub fn ore_under(&self, origin: (i32, i32), fp: (u32, u32)) -> bool {
        (origin.1..origin.1 + fp.1 as i32).any(|z| (origin.0..origin.0 + fp.0 as i32).any(|x| self.in_bounds(x, z) && self.metal[self.idx(x, z)] > self.surface_metal))
    }

    /// Marks a geothermal vent's cells.
    pub fn mark_geo(&mut self, origin: (i32, i32), fp: (u32, u32)) {
        for z in origin.1..origin.1 + fp.1 as i32 {
            for x in origin.0..origin.0 + fp.0 as i32 {
                if self.in_bounds(x, z) {
                    let i = self.idx(x, z);
                    Arc::make_mut(&mut self.geo)[i] = true;
                }
            }
        }
    }

    /// Whether any cell of a footprint lies over a geothermal vent.
    pub fn geo_under(&self, origin: (i32, i32), fp: (u32, u32)) -> bool {
        (origin.1..origin.1 + fp.1 as i32).any(|z| (origin.0..origin.0 + fp.0 as i32).any(|x| self.in_bounds(x, z) && self.geo[self.idx(x, z)]))
    }

    pub fn can_place(&self, t: &UnitType, max_slope: i32, origin: (i32, i32)) -> bool {
        self.placement_error(t, max_slope, origin).is_none()
    }

    /// Sum of metal values under a footprint.
    pub fn metal_under(&self, origin: (i32, i32), fp: (u32, u32)) -> i64 {
        let mut sum = 0i64;
        for z in origin.1..origin.1 + fp.1 as i32 {
            for x in origin.0..origin.0 + fp.0 as i32 {
                if self.in_bounds(x, z) {
                    sum += self.metal[self.idx(x, z)] as i64;
                }
            }
        }
        sum
    }

    pub fn set_metal(&mut self, cx: i32, cz: i32, value: u32) {
        if self.in_bounds(cx, cz) {
            let i = self.idx(cx, cz);
            self.metal[i] = self.metal[i].max(value);
        }
    }

    /// World position of a footprint anchored at `anchor` (centre of the footprint).
    pub fn anchor_to_pos(&self, anchor: (i32, i32), fp: (u32, u32)) -> FxVec3 {
        let (x0, z0) = Self::footprint_origin(anchor, fp);
        let x = Fx::from_int(x0 as i64 * CELL) + Fx::from_int(fp.0 as i64 * CELL) / 2;
        let z = Fx::from_int(z0 as i64 * CELL) + Fx::from_int(fp.1 as i64 * CELL) / 2;
        // Where a building of that footprint stands (see `footprint_top`).
        FxVec3::new(x, self.footprint_top((x0, z0), fp), z)
    }

    /// Anchor cell for a unit centred at `pos` with footprint `fp` (inverse of anchor_to_pos).
    pub fn pos_to_anchor(pos: FxVec3, fp: (u32, u32)) -> (i32, i32) {
        let x0 = ((pos.x - Fx::from_int(fp.0 as i64 * CELL) / 2).round_int()).div_euclid(CELL) as i32;
        let z0 = ((pos.z - Fx::from_int(fp.1 as i64 * CELL) / 2).round_int()).div_euclid(CELL) as i32;
        (x0 + fp.0 as i32 / 2, z0 + fp.1 as i32 / 2)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 64×48-cell test world: gentle slopes, a ridge, a sea along the east edge, void
    /// cells and a few buildings (one with a non-blocking yard cell).
    pub(crate) fn test_world() -> World {
        let (w, h) = (64u32, 48u32);
        // Gentle slopes, a steep ridge (x 36..38, z < 30) to walk around, sea from x 52.
        let heights = (0..h)
            .flat_map(|z| (0..w).map(move |x| if x >= 52 { 5 } else if (36..38).contains(&x) && z < 30 { 140 } else { (60 + (x / 3 + z / 4) % 4) as u8 }))
            .collect();
        let mut features = vec![ta_formats::tnt::FEATURE_NONE; (w * h) as usize];
        for z in 30..34 {
            features[(z * w + 10) as usize] = FEATURE_VOID;
        }
        let map = TntMap {
            width: w,
            height: h,
            sea_level: 40,
            tiles: Vec::new(),
            tile_map: Vec::new(),
            heights,
            features,
            feature_names: Vec::new(),
            minimap: Default::default(),
            flags: Vec::new(),
        };
        let mut world = World::new(&map);
        world.occupy(Occupant::Unit(1), (20, 10), (4, 4), &[]);
        world.occupy(Occupant::Unit(2), (30, 20), (3, 2), &[true, false, true, true, true, true]);
        world.occupy(Occupant::Feature(3), (5, 40), (2, 2), &[]);
        world
    }

    /// Regions as the original cell-by-cell flood fill found them.
    fn reference_regions(world: &World, spec: &MoveSpec) -> Vec<Region> {
        let (w, h) = (world.width as i32, world.height as i32);
        let mut comp = vec![0; (w * h) as usize];
        let mut next: Region = 1;
        let mut stack = Vec::new();
        for start in 0..(w * h) {
            if comp[start as usize] != 0 || !world.terrain_passable(spec, (start % w, start / w)) {
                continue;
            }
            let id = next;
            next += 1;
            comp[start as usize] = id;
            stack.push((start % w, start / w));
            while let Some((x, z)) = stack.pop() {
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                    let n = (x + dx, z + dz);
                    if n.0 < 0 || n.1 < 0 || n.0 >= w || n.1 >= h || comp[(n.1 * w + n.0) as usize] != 0 || !world.terrain_passable(spec, n) {
                        continue;
                    }
                    if dx != 0 && dz != 0 && (!world.terrain_passable(spec, (x + dx, z)) || !world.terrain_passable(spec, (x, z + dz))) {
                        continue;
                    }
                    comp[(n.1 * w + n.0) as usize] = id;
                    stack.push(n);
                }
            }
        }
        comp
    }

    #[test]
    fn regions_match_the_cell_by_cell_flood_fill() {
        let mut world = test_world();
        let specs: Vec<MoveSpec> = [(1, 1), (2, 2), (3, 3), (4, 4), (3, 2), (2, 5), (0, 0)]
            .into_iter()
            .flat_map(|footprint| {
                [
                    MoveSpec { footprint, max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false },
                    MoveSpec { footprint, max_slope: 12, max_depth: 255, min_depth: 20, hover: false },
                    MoveSpec { footprint, max_slope: 12, max_depth: 255, min_depth: -10_000, hover: true },
                ]
            })
            .collect();
        world.prepare_all(&specs);
        for spec in &specs {
            let want = reference_regions(&world, spec);
            assert_eq!(*world.components[spec], want, "{spec:?}");
            assert!(want.iter().any(|&c| c != 0) || spec.min_depth > 0, "{spec:?} reaches somewhere");
        }
    }

    /// A map of 67,600 islands (each a cell walled off by void): every island its own region,
    /// past the 65,535 a 16-bit label held (the last of which used to merge).
    #[test]
    fn regions_past_sixty_five_thousand_stay_apart() {
        let (w, h) = (520u32, 520u32);
        let island = |x: u32, z: u32| x.is_multiple_of(2) && z.is_multiple_of(2);
        let features = (0..h).flat_map(|z| (0..w).map(move |x| if island(x, z) { ta_formats::tnt::FEATURE_NONE } else { FEATURE_VOID })).collect();
        let map = TntMap { width: w, height: h, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; (w * h) as usize], features, feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let mut world = World::new(&map);
        let spec = MoveSpec { footprint: (1, 1), max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
        world.prepare(&spec);
        let mut seen = std::collections::BTreeSet::new();
        for z in (0..h as i32).step_by(2) {
            for x in (0..w as i32).step_by(2) {
                let r = world.component(&spec, (x, z));
                assert!(r != 0 && seen.insert(r), "island ({x}, {z}) shares region {r}");
            }
        }
        assert_eq!(seen.len(), 260 * 260);
        assert!(*seen.last().unwrap() > u16::MAX as Region);
        // The last island is found by its own region, and no other.
        let last = (w as i32 - 2, h as i32 - 2);
        let r = world.component(&spec, last);
        assert_eq!(world.nearest_in_component(&spec, r, (w as i32 - 1, h as i32 - 1), 4), Some(last));
        assert_ne!(world.component(&spec, (w as i32 - 4, h as i32 - 2)), r);
    }

    /// A wall of cells marked as cliffs across flat ground (a converted map's): a tank, and a
    /// kbot that climbs anything TA's do, keep to its own side and cannot build on it; a ship
    /// (on water, slopes are nothing to it) and a unit that minds no slope cross as before.
    #[test]
    fn a_maps_cliffs_are_too_steep_to_cross_or_build_on() {
        let (w, h) = (24u32, 12u32);
        let mut flags = vec![0u8; (w * h) as usize];
        for z in 0..h {
            flags[(z * w + 12) as usize] = ta_formats::tnt::CELL_CLIFF;
        }
        let map = TntMap { width: w, height: h, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; (w * h) as usize],
            features: vec![ta_formats::tnt::FEATURE_NONE; (w * h) as usize], feature_names: Vec::new(), minimap: Default::default(), flags };
        let mut world = World::new(&map);
        assert_eq!((world.slope(12, 5), world.slope(11, 5)), (u8::MAX, 0));
        let tank = MoveSpec { footprint: (2, 2), max_slope: 15, max_depth: 12, min_depth: -10_000, hover: false };
        let climber = MoveSpec { footprint: (2, 2), max_slope: 254, max_depth: 12, min_depth: -10_000, hover: false };
        let anything = MoveSpec { footprint: (2, 2), max_slope: 255, max_depth: 12, min_depth: -10_000, hover: false };
        world.prepare_all(&[tank, climber, anything]);
        for spec in [tank, climber] {
            assert_ne!(world.component(&spec, (4, 5)), world.component(&spec, (20, 5)), "{spec:?} crossed the cliff");
            assert!(!world.passable(&spec, (12, 5), None));
        }
        assert_eq!(world.component(&anything, (4, 5)), world.component(&anything, (20, 5)));
        assert_ne!(world.component(&anything, (4, 5)), 0);
        // (Building goes by the same slope: `placement_error` refuses dry cells steeper than
        // the builder's limit, and no limit of TA's is past 254.)
        assert!(world.slope(12, 0) as i32 > 254 && world.slope(10, 0) == 0);
    }

    #[test]
    fn passable_windows_match_the_whole_map() {
        let mut world = test_world();
        let (w, h) = (world.width as i32, world.height as i32);
        for footprint in [(1, 1), (2, 2), (3, 3), (4, 2)] {
            let spec = MoveSpec { footprint, max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
            world.prepare(&spec);
            let all = world.passable_all(&spec);
            for (from, to) in [((0, 0), (w, h)), ((5, 7), (30, 22)), ((0, 10), (12, h)), ((w - 9, 0), (w, 9)), ((20, 20), (20, 25)), ((-4, -4), (6, 6))] {
                let window = world.passable_window(&spec, from, to);
                let (x0, z0, x1, z1) = (from.0.clamp(0, w), from.1.clamp(0, h), to.0.clamp(0, w), to.1.clamp(0, h));
                assert_eq!(window.len(), ((x1 - x0) * (z1 - z0)) as usize);
                for z in z0..z1 {
                    for x in x0..x1 {
                        let got = window[((z - z0) * (x1 - x0) + (x - x0)) as usize];
                        assert_eq!(got, all[(z * w + x) as usize], "{footprint:?} window {from:?}..{to:?} at ({x}, {z})");
                    }
                }
            }
        }
    }

    #[test]
    fn passable_all_matches_passable_everywhere() {
        let mut world = test_world();
        let specs = [(1, 1), (2, 2), (3, 3), (4, 4), (3, 2), (2, 5)].map(|footprint| MoveSpec { footprint, max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false });
        for spec in &specs {
            // Before and after the spec's anchor bits are prepared (two code paths).
            world.prepare_terrain(spec);
            for _ in 0..2 {
                let all = world.passable_all(spec);
                assert!(all.iter().filter(|&&ok| ok).count() > 1500, "{spec:?}: most of the land is open");
                for z in 0..world.height as i32 {
                    for x in 0..world.width as i32 {
                        assert_eq!(all[(z as u32 * world.width + x as u32) as usize], world.passable(spec, (x, z), None), "{spec:?} at ({x}, {z})");
                    }
                }
                world.prepare(spec);
            }
        }
    }
}
