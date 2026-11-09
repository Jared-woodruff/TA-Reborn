//! The ground levelled under buildings, as it is drawn.
//!
//! The simulation stands a building on the highest ground under its footprint
//! (`ta_sim::world::World::footprint_top`): TA drew its buildings over a flat picture of the map,
//! so their bases never met the ground. On the drawn ground (a hillside, a dune) a flat base set
//! there hung over the ground downhill, with a gap under it, which TA's camera high above never
//! showed and a camera at the height of a man does.
//!
//! So each building levels the ground it is drawn on, as a bulldozer would: its footprint's
//! vertices are drawn at its base (a pad; the building is drawn where the simulation has it, so
//! its guns' flashes, its nano sprays and the units it builds stay on it), and the ground round
//! it takes up the difference over a short apron, wider the more there is to take up. What the
//! pad raised or cut at each edge vertex is carried out from there and eased away (to nothing at
//! the apron's end, added to the ground's own shape: a cliff beside a pad stays a cliff), so the
//! pad stands on a bank of its own earth rather than on a terrace. Where the aprons of several
//! pads meet, the ground takes a mean of theirs by how near each is, so between a pad high on a
//! slope and one lower down it runs from the one to the other. Where buildings stand side by
//! side, the ground their footprints share is at the higher one's base (the lower one's edge is
//! dug in a little rather than the higher one's left hanging); a pad laid over one left by a
//! building gone since is drawn over it, what is left of the old one round it staying. Pads stay
//! after their buildings are destroyed (their wrecks and rubble lie on them). The sea's shore
//! stays where it is: what of a footprint lies under the sea stays there (a building on the
//! shore stands in the water on that side), and so does what of an apron does. A building forced
//! onto a cliff's top lays none (see [`MAX_FILL`]). The simulation keeps TA's heights; the pads
//! are the drawn ground's alone (see `terrain_surface.rs`), and everything drawn on the ground
//! follows them: the terrain and its shading, what stands and walks on it
//! (`ta_game::scene::drawn_ground`), decals, shadows, the water's edge.
//!
//! Games have thousands of buildings: pads are kept by the blocks of the map they reach, and a
//! new one changes only the ground it reaches (and any vertex comes out the same whatever
//! rectangle it is drawn in, so laying them one by one draws what laying them all at once does).

/// A building's pad: the cells of its footprint and the height its base stands at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundPad {
    /// The footprint's cells: `x0..x1` × `z0..z1` (its vertices `x0..=x1` × `z0..=z1`).
    pub cells: [i32; 4],
    /// The height (world units) its base stands at, and the pad is drawn at.
    pub height: f32,
    /// The game tick it was laid at (see `Renderer::forget_ground_pads_after`).
    pub tick: u64,
}

impl GroundPad {
    /// Whether vertex (`x`, `z`) is one of its footprint's.
    #[inline]
    fn covers(&self, x: i32, z: i32) -> bool {
        let [x0, z0, x1, z1] = self.cells;
        x >= x0 && x <= x1 && z >= z0 && z <= z1
    }

    /// Whether its footprint and `other`'s share a cell (not just an edge).
    #[inline]
    fn overlaps(&self, other: &GroundPad) -> bool {
        let ([ax0, az0, ax1, az1], [bx0, bz0, bx1, bz1]) = (self.cells, other.cells);
        ax0 < bx1 && bx0 < ax1 && az0 < bz1 && bz0 < az1
    }
}

/// The apron's reach (cells): at least, and at most (round a pad far above or below the ground
/// about it); in between, a cell for every this many units the pad's edge differs from the ground
/// there, so that a bank is no steeper than about 25° more than the ground it stands on.
const REACH_MIN: f32 = 2.0;
const REACH_MAX: f32 = 6.0;
const REACH_PER: f32 = 5.0;
/// Vertices round a pad its apron may reach (`REACH_MAX`, whole).
const REACH_CELLS: i32 = 6;
/// The most (world units) a pad raises the ground under its footprint. A building where one may
/// be built (no cell steeper than its `MaxSlope`, 10 for most) needs less than 30 at the
/// steepest; one whose footprint reaches further down stands at the top of a cliff (as
/// missions and staged scenes may put one), overhanging it as TA showed it, and lays no pad (the
/// cliff filled in under it was a tower of earth).
const MAX_FILL: f32 = 40.0;
/// The map is indexed in blocks of this many vertices a side.
const BLOCK: i32 = 16;
/// How far (units) the apron keeps ground that lies under the sea below it, and ground above it
/// above it (`terrain_surface`'s coast margin).
const COAST_MARGIN: f32 = 0.25;

/// How far (cells) the apron round a pad takes up a difference of `edge` units at its edge.
fn reach(edge: f32) -> f32 {
    (edge.abs() / REACH_PER).clamp(REACH_MIN, REACH_MAX)
}

/// How much of a pad's difference at its edge the ground `d` cells out keeps, the apron reaching
/// `r` cells: all of it at the edge and none at `r` and beyond; falling away as a bank does,
/// steadily, gentler at its top and easing out at its foot.
fn falloff(d: f32, r: f32) -> f32 {
    let t = (d / r).clamp(0.0, 1.0);
    (1.0 - t) * (1.0 - t) * (1.0 + t)
}

/// The pads laid on a map of `width` × `height` vertices, and the drawn ground without them.
pub(crate) struct Pads {
    width: i32,
    height: i32,
    sea: f32,
    /// In the order they were laid.
    list: Vec<GroundPad>,
    /// For every block of `BLOCK` × `BLOCK` vertices (row-major), the pads whose aprons reach
    /// it, in the order they were laid.
    blocks: Vec<Vec<u32>>,
    blocks_x: i32,
    /// The drawn heights without the pads (empty while there are none).
    natural: Vec<f32>,
}

impl Pads {
    pub(crate) fn new(width: u32, height: u32, sea: f32) -> Pads {
        let (w, h) = (width as i32, height as i32);
        let blocks_x = (w + BLOCK - 1) / BLOCK;
        let blocks_z = (h + BLOCK - 1) / BLOCK;
        Pads { width: w, height: h, sea, list: Vec::new(), blocks: vec![Vec::new(); (blocks_x * blocks_z).max(0) as usize], blocks_x, natural: Vec::new() }
    }

    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }

    /// The drawn heights without the pads, to change in place (`None` while there are no pads:
    /// the drawn heights are those).
    pub(crate) fn natural_mut(&mut self) -> Option<&mut Vec<f32>> {
        (!self.natural.is_empty()).then_some(&mut self.natural)
    }

    /// The vertices whose drawn height `pad` may change (`[x0, z0, x1, z1)`), on the map.
    fn reach_of(&self, pad: &GroundPad) -> [i32; 4] {
        let [x0, z0, x1, z1] = pad.cells;
        [(x0 - REACH_CELLS).max(0), (z0 - REACH_CELLS).max(0), (x1 + REACH_CELLS + 1).min(self.width), (z1 + REACH_CELLS + 1).min(self.height)]
    }

    /// The blocks a rectangle of vertices (`[x0, z0, x1, z1)`) lies in.
    fn blocks_of(&self, [x0, z0, x1, z1]: [i32; 4]) -> impl Iterator<Item = usize> + '_ {
        let (bx0, bz0, bx1, bz1) = (x0 / BLOCK, z0 / BLOCK, (x1 - 1) / BLOCK, (z1 - 1) / BLOCK);
        (bz0..=bz1).flat_map(move |bz| (bx0..=bx1).map(move |bx| (bz * self.blocks_x + bx) as usize))
    }

    /// Lays `pad` (the drawn heights being `display`); the vertices it may change, unless it lies
    /// off the map, is one laid already or stands at a cliff's top (see [`MAX_FILL`]).
    pub(crate) fn add(&mut self, display: &[f32], pad: GroundPad) -> Option<[i32; 4]> {
        let [x0, z0, x1, z1] = pad.cells;
        if x1 <= x0 || z1 <= z0 || x1 < 0 || z1 < 0 || x0 >= self.width || z0 >= self.height || !pad.height.is_finite() {
            return None;
        }
        let reach = self.reach_of(&pad);
        // (A building the game shows again, after going back in it, is laid once.)
        if self.blocks_of(reach).next().and_then(|b| self.blocks.get(b)).is_some_and(|b| b.iter().any(|&i| self.list[i as usize].cells == pad.cells && self.list[i as usize].height == pad.height)) {
            return None;
        }
        // (The ground under it, but for what lies under the sea, no further below it than a pad
        // ever fills.)
        let natural = if self.natural.is_empty() { display } else { &self.natural };
        let (w, h, sea) = (self.width, self.height, self.sea);
        let below = (z0.max(0)..=z1.min(h - 1))
            .flat_map(|z| (x0.max(0)..=x1.min(w - 1)).map(move |x| (z * w + x) as usize))
            .map(|i| natural[i])
            .filter(|&n| sea <= 0.0 || n >= sea)
            .fold(f32::MIN, |deepest, n| deepest.max(pad.height - n));
        if below > MAX_FILL {
            return None;
        }
        if self.natural.is_empty() {
            self.natural = display.to_vec();
        }
        let index = self.list.len() as u32;
        self.list.push(pad);
        for b in self.blocks_of(reach).collect::<Vec<_>>() {
            self.blocks[b].push(index);
        }
        Some(reach)
    }

    /// Keeps the pads laid at or before `tick` (the game went back to it); the vertices whose
    /// drawn heights change, as rectangles.
    pub(crate) fn retain_until(&mut self, tick: u64) -> Vec<[i32; 4]> {
        let gone: Vec<[i32; 4]> = self.list.iter().filter(|p| p.tick > tick).map(|p| self.reach_of(p)).collect();
        if gone.is_empty() {
            return gone;
        }
        let kept: Vec<GroundPad> = self.list.iter().copied().filter(|p| p.tick <= tick).collect();
        self.list.clear();
        for b in &mut self.blocks {
            b.clear();
        }
        for pad in kept {
            let index = self.list.len() as u32;
            let reach = self.reach_of(&pad);
            self.list.push(pad);
            for b in self.blocks_of(reach).collect::<Vec<_>>() {
                self.blocks[b].push(index);
            }
        }
        gone
    }

    /// The pads whose aprons may reach a rectangle of vertices, in the order they were laid.
    fn near(&self, rect: [i32; 4]) -> Vec<u32> {
        let mut near: Vec<u32> = self.blocks_of(rect).flat_map(|b| self.blocks[b].iter().copied()).collect();
        near.sort_unstable();
        near.dedup();
        near
    }

    /// The height vertex (`x`, `z`) is drawn at if a footprint among `near` covers it: the base
    /// of the highest building whose footprint covers it, but for one laid over since by a
    /// building whose footprint overlaps its own.
    fn covered(&self, x: i32, z: i32, near: &[u32]) -> Option<f32> {
        let mut top: Option<f32> = None;
        for (k, &i) in near.iter().enumerate() {
            let pad = &self.list[i as usize];
            if !pad.covers(x, z) {
                continue;
            }
            let over = near[k + 1..].iter().map(|&j| &self.list[j as usize]).any(|later| later.covers(x, z) && later.overlaps(pad));
            if !over {
                top = Some(top.map_or(pad.height, |t| t.max(pad.height)));
            }
        }
        top
    }

    /// Draws the ground of the vertices of `rect` (`[x0, z0, x1, z1)`) with the pads into
    /// `display` (the map's drawn heights); the most any changed by. (Block by block: each looks
    /// only at the pads near it. A large rectangle on all cores.)
    pub(crate) fn apply(&self, rect: [i32; 4], display: &mut [f32]) -> f32 {
        use rayon::prelude::*;
        let [x0, z0, x1, z1] = [rect[0].max(0), rect[1].max(0), rect[2].min(self.width), rect[3].min(self.height)];
        if x0 >= x1 || z0 >= z1 || self.natural.len() < (self.width * self.height) as usize || display.len() < self.natural.len() {
            return 0.0;
        }
        let parts: Vec<[i32; 4]> = (z0 / BLOCK..=(z1 - 1) / BLOCK)
            .flat_map(|bz| (x0 / BLOCK..=(x1 - 1) / BLOCK).map(move |bx| [(bx * BLOCK).max(x0), (bz * BLOCK).max(z0), ((bx + 1) * BLOCK).min(x1), ((bz + 1) * BLOCK).min(z1)]))
            .collect();
        let drawn: Vec<([i32; 4], Vec<f32>)> = if parts.len() > 4 {
            parts.par_iter().map(|&part| (part, self.drawn(part))).collect()
        } else {
            parts.iter().map(|&part| (part, self.drawn(part))).collect()
        };
        let mut most = 0.0f32;
        for ([px0, pz0, px1, pz1], heights) in drawn {
            let pw = (px1 - px0) as usize;
            for z in pz0..pz1 {
                let row = (z * self.width + px0) as usize;
                for (old, new) in display[row..row + pw].iter_mut().zip(&heights[(z - pz0) as usize * pw..][..pw]) {
                    most = most.max((new - *old).abs());
                    *old = *new;
                }
            }
        }
        most
    }

    /// The drawn heights of the vertices of `rect` (on the map, within a block), row by row.
    fn drawn(&self, rect: [i32; 4]) -> Vec<f32> {
        let [x0, z0, x1, z1] = rect;
        let w = self.width;
        // The footprints' heights over the rectangle and as far round it as an apron reaches (the
        // edges of the pads whose aprons reach into it): whatever rectangle a vertex is drawn in,
        // it comes out the same.
        let ext = [(x0 - REACH_CELLS).max(0), (z0 - REACH_CELLS).max(0), (x1 + REACH_CELLS).min(self.width), (z1 + REACH_CELLS).min(self.height)];
        let near = self.near(ext);
        let mut out = Vec::with_capacity(((x1 - x0) * (z1 - z0)) as usize);
        if near.is_empty() {
            for z in z0..z1 {
                out.extend_from_slice(&self.natural[(z * w + x0) as usize..(z * w + x1) as usize]);
            }
            return out;
        }
        let ew = ext[2] - ext[0];
        let mut covered = vec![f32::NAN; (ew * (ext[3] - ext[1])) as usize];
        for z in ext[1]..ext[3] {
            for x in ext[0]..ext[2] {
                // (What of a footprint lies under the sea stays there: a building on the shore
                // stands in the water on that side, and the shore stays where it is.)
                if self.sea > 0.0 && self.natural[(z * w + x) as usize] < self.sea {
                    continue;
                }
                if let Some(h) = self.covered(x, z, &near) {
                    covered[((z - ext[1]) * ew + x - ext[0]) as usize] = h;
                }
            }
        }
        let inside = |x: i32, z: i32| x >= ext[0] && x < ext[2] && z >= ext[1] && z < ext[3];
        let cover = |x: i32, z: i32| covered[((z - ext[1]) * ew + x - ext[0]) as usize];
        for z in z0..z1 {
            for x in x0..x1 {
                let natural = self.natural[(z * w + x) as usize];
                let mut drawn = cover(x, z);
                if drawn.is_nan() {
                    // The apron: what each pad raised (or cut) at its edge nearest, eased away with
                    // the distance from it; where several reach, a mean of theirs by how near each
                    // is (by distance alone, so that right by a pad its own counts most whatever
                    // the others' reach: between a pad high on a slope and one lower down the
                    // ground runs from the one to the other).
                    let (mut sum, mut weight) = (0.0f32, 0.0f32);
                    for &p in &near {
                        let [px0, pz0, px1, pz1] = self.list[p as usize].cells;
                        let (ex, ez) = (x.clamp(px0.max(0), px1.min(self.width - 1)), z.clamp(pz0.max(0), pz1.min(self.height - 1)));
                        let d = (((x - ex) * (x - ex) + (z - ez) * (z - ez)) as f32).sqrt();
                        if d >= REACH_MAX || !inside(ex, ez) {
                            continue;
                        }
                        let edge = cover(ex, ez) - self.natural[(ez * w + ex) as usize];
                        if edge.is_nan() {
                            continue;
                        }
                        // (Its say: the inverse square of the distance, eased to none at the
                        // furthest an apron reaches.)
                        let say = (1.0 - d / REACH_MAX).powi(2) / (d * d);
                        sum += say * edge * falloff(d, reach(edge));
                        weight += say;
                    }
                    if weight > 0.0 {
                        drawn = natural + sum / weight;
                    } else {
                        drawn = natural;
                    }
                    // (The shore stays where it is.)
                    if self.sea > 0.0 {
                        drawn = if natural < self.sea { drawn.min(natural.max(self.sea - COAST_MARGIN)) } else { drawn.max(natural.min(self.sea + COAST_MARGIN)) };
                    }
                }
                out.push(drawn);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map's drawn heights: `f(x, z)` at every vertex.
    fn ground(w: i32, h: i32, f: impl Fn(i32, i32) -> f32) -> Vec<f32> {
        (0..h).flat_map(|z| (0..w).map(move |x| (x, z))).map(|(x, z)| f(x, z)).collect()
    }

    fn pad(cells: [i32; 4], height: f32, tick: u64) -> GroundPad {
        GroundPad { cells, height, tick }
    }

    /// Lays `pads` on `natural`, the drawn heights as they come out.
    fn laid(w: i32, h: i32, sea: f32, natural: &[f32], pads: &[GroundPad]) -> (Pads, Vec<f32>) {
        let mut p = Pads::new(w as u32, h as u32, sea);
        let mut display = natural.to_vec();
        for &pad in pads {
            if let Some(r) = p.add(&display, pad) {
                p.apply(r, &mut display);
            }
        }
        (p, display)
    }

    #[test]
    fn a_building_on_a_slope_stands_on_a_level_pad_that_banks_away_to_the_ground() {
        // A slope rising 3 units a cell westwards; a 4 x 4 building at its highest corner's height.
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, _| 150.0 - 3.0 * x as f32);
        let base = 150.0 - 3.0 * 18.0;
        let (_, drawn) = laid(w, h, 0.0, &natural, &[pad([18, 18, 22, 22], base, 0)]);
        let at = |x: i32, z: i32| drawn[(z * w + x) as usize];
        for z in 18..=22 {
            for x in 18..=22 {
                assert_eq!(at(x, z), base, "the footprint's ({x}, {z}) is at the base");
            }
        }
        // Downhill the bank falls away steadily to the ground, never below it nor steeper than
        // a cell's height in a cell; uphill the ground is hardly cut (the pad meets it there).
        let mut last = base;
        for x in 23..30 {
            let (d, n) = (at(x, 20), natural[(20 * w + x) as usize]);
            assert!(d <= last && d >= n - 1e-4 && last - d < 16.0, "x {x}: {d} after {last} over ground at {n}");
            last = d;
        }
        assert_eq!(at(30, 20), natural[(20 * w + 30) as usize], "the apron ends");
        for x in 13..18 {
            assert!((at(x, 20) - natural[(20 * w + x) as usize]).abs() < 0.01, "uphill at {x}: {}", at(x, 20));
        }
        // Far away, nothing changes.
        assert_eq!(at(5, 5), natural[(5 * w + 5) as usize]);
    }

    #[test]
    fn the_apron_keeps_the_ground_s_own_shape() {
        // A pad 20 above a plateau beside a cliff 40 units down: the cliff is lifted by what the
        // pad raised at its edge, eased away, and stays a cliff.
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, _| if x <= 24 { 100.0 } else { 60.0 });
        let (_, drawn) = laid(w, h, 0.0, &natural, &[pad([18, 18, 22, 22], 120.0, 0)]);
        let at = |x: i32| drawn[(20 * w + x) as usize];
        assert!(at(24) > 100.0 && at(24) < 120.0 && at(25) < 66.0 && at(24) - at(25) > 38.0, "the cliff's lip {} and foot {}", at(24), at(25));
    }

    #[test]
    fn a_building_at_a_cliff_s_top_leaves_the_cliff_be() {
        // A footprint straddling a cliff 60 units high (as a mission may put one) lays no pad;
        // one on a slope as steep as any that may be built on does.
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, z| if x <= 20 { 100.0 + (z % 2) as f32 } else { 40.0 });
        let (pads, drawn) = laid(w, h, 0.0, &natural, &[pad([17, 17, 22, 22], 101.0, 0)]);
        assert_eq!((pads.len(), drawn), (0, natural));
        let slope = ground(w, h, |x, _| 200.0 - 7.0 * x as f32);
        let (pads, _) = laid(w, h, 0.0, &slope, &[pad([17, 17, 22, 22], 200.0 - 7.0 * 17.0, 0)]);
        assert_eq!(pads.len(), 1);
    }

    #[test]
    fn a_bank_between_a_high_pad_and_a_low_one_runs_from_one_to_the_other() {
        // A pad far up a slope and one just below it, two cells apart: the ground between them
        // falls from the one to the other with no wall at the lower one's edge.
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, _| 150.0 - 5.0 * x as f32);
        let (_, drawn) = laid(w, h, 0.0, &natural, &[pad([20, 10, 24, 14], 50.0, 0), pad([10, 10, 18, 14], 100.0, 1)]);
        let at = |x: i32| drawn[(12 * w + x) as usize];
        assert_eq!((at(18), at(20)), (100.0, 50.0));
        let drops = [at(18) - at(19), at(19) - at(20)];
        assert!(drops.iter().all(|&d| d > 15.0 && d < 35.0), "the bank falls {drops:?} a cell");
    }

    #[test]
    fn pads_laid_one_by_one_draw_the_ground_as_all_at_once() {
        // Thirty pads of all sizes laid one after another, some over others, each changing only
        // the ground it reaches: the same ground as all of them drawn over the whole map.
        let (w, h) = (90, 70);
        let natural = ground(w, h, |x, z| 80.0 + 30.0 * ((x as f32 * 0.21).sin() + (z as f32 * 0.17).cos()) + ((x * 7 + z * 13) % 5) as f32);
        let mut seed = 7u32;
        let mut next = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            ((seed >> 16) % n) as i32
        };
        let pads: Vec<GroundPad> = (0..30)
            .map(|i| {
                let (x, z, fx, fz) = (next(w as u32 + 4) - 2, next(h as u32 + 4) - 2, 1 + next(8), 1 + next(8));
                pad([x, z, x + fx, z + fz], 60.0 + next(80) as f32, i)
            })
            .collect();
        let (p, one_by_one) = laid(w, h, 0.0, &natural, &pads);
        let mut all_at_once = natural.clone();
        p.apply([0, 0, w, h], &mut all_at_once);
        assert_eq!(one_by_one, all_at_once);
    }

    #[test]
    fn neighbours_share_the_higher_edge_and_keep_their_own_footprints() {
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, _| 150.0 - 2.0 * x as f32);
        // Side by side along the slope, sharing the edge x = 14; then one apart further down.
        let (a, b, c) = (pad([10, 10, 14, 14], 130.0, 0), pad([14, 10, 18, 14], 122.0, 1), pad([19, 10, 23, 14], 112.0, 2));
        let (_, drawn) = laid(w, h, 0.0, &natural, &[a, b, c]);
        let at = |x: i32, z: i32| drawn[(z * w + x) as usize];
        assert_eq!(at(14, 12), 130.0, "the shared edge at the higher base");
        assert_eq!(at(10, 12), 130.0);
        assert_eq!(at(15, 12), 122.0, "the lower one's footprint keeps its base");
        assert_eq!(at(18, 12), 122.0, "and its other edge too, the next one's apron notwithstanding");
        assert_eq!(at(19, 12), 112.0);
    }

    #[test]
    fn a_building_laid_over_an_old_pad_stands_on_its_own() {
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, z| 100.0 - x as f32 - z as f32);
        // A building gone since, then another overlapping where it stood, lower.
        let (_, drawn) = laid(w, h, 0.0, &natural, &[pad([10, 10, 16, 16], 80.0, 0), pad([13, 13, 17, 17], 74.0, 5)]);
        let at = |x: i32, z: i32| drawn[(z * w + x) as usize];
        for (x, z) in [(13, 13), (16, 16), (13, 17), (17, 13), (15, 14)] {
            assert_eq!(at(x, z), 74.0, "({x}, {z}) under the new one");
        }
        assert_eq!(at(10, 10), 80.0, "the old pad round it stays");
        assert_eq!(at(12, 15), 80.0);
    }

    #[test]
    fn the_sea_keeps_its_shore() {
        // Land falling into a sea at 50 east of x = 25: a pad by the shore raises the bank, not
        // the sea bed above the sea; nor one whose footprint reaches into the sea the bed under
        // it (it stands in the water on that side).
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, _| 70.0 - 2.0 * (x as f32 - 15.0));
        for cells in [[20, 16, 24, 20], [22, 16, 27, 20]] {
            let (_, drawn) = laid(w, h, 50.0, &natural, &[pad(cells, natural[cells[0] as usize], 0)]);
            let mut raised = false;
            for x in 0..w {
                let (d, n) = (drawn[(18 * w + x) as usize], natural[(18 * w + x) as usize]);
                assert_eq!(d < 50.0, n < 50.0, "x {x}: {d} drawn over {n}");
                raised |= x > 22 && x < 26 && d > n;
            }
            assert!(raised, "the bank is raised");
        }
    }

    #[test]
    fn going_back_forgets_the_pads_laid_since() {
        let (w, h) = (40, 40);
        let natural = ground(w, h, |x, _| 150.0 - 3.0 * x as f32);
        let (mut pads, mut drawn) = laid(w, h, 0.0, &natural, &[pad([4, 4, 8, 8], 138.0, 3), pad([20, 20, 24, 24], 90.0, 9)]);
        let (_, only_first) = laid(w, h, 0.0, &natural, &[pad([4, 4, 8, 8], 138.0, 3)]);
        for r in pads.retain_until(5) {
            pads.apply(r, &mut drawn);
        }
        assert_eq!(pads.len(), 1);
        assert_eq!(drawn, only_first);
        // Laid again: the same as before.
        let mut p = Pads::new(w as u32, h as u32, 0.0);
        let mut d = natural.clone();
        let r = p.add(&d, pad([4, 4, 8, 8], 138.0, 3)).expect("laid");
        p.apply(r, &mut d);
        assert!(p.add(&d, pad([4, 4, 8, 8], 138.0, 4)).is_none(), "laid once");
    }

    #[test]
    fn pads_at_the_map_s_edges_stay_on_it() {
        let (w, h) = (20, 20);
        let natural = ground(w, h, |x, z| 60.0 + x as f32 + z as f32);
        let (_, drawn) = laid(w, h, 0.0, &natural, &[pad([-2, -2, 2, 2], 66.0, 0), pad([17, 17, 21, 21], 100.0, 1)]);
        assert_eq!(drawn[0], 66.0);
        assert_eq!(drawn[(19 * w + 19) as usize], 100.0);
    }
}
