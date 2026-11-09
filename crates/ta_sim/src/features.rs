//! Where the map's features are: their indices by 64 px square, for what is asked of a small
//! area (what a blast reaches, what a fire spreads to, what a passing builder reclaims). A big
//! map has tens of thousands of trees and rocks; looking at all of them for every explosion
//! cost more than the battle itself.
//!
//! Features never move and their indices only grow (a destroyed feature stays in the list,
//! dead), so the squares are kept as one sorted list (each square's indices in order) plus the
//! few added since it was sorted, which are sorted in once there are enough of them.

use crate::fixed::{Fx, FxVec3};
use crate::sim::Feature;
use crate::world::{CELL, World};

/// Pixels a side of a square.
const SQUARE: i64 = 64;
/// Features added before the squares are sorted out again.
const LOOSE_MAX: usize = 256;

#[derive(Clone, Default)]
pub struct FeatureGrid {
    width: i32,
    height: i32,
    /// Per square, where its features start in `items` (they end where the next square's
    /// start).
    starts: Vec<u32>,
    /// Feature indices, square by square, each square's in index order.
    items: Vec<u32>,
    /// Features added since: their square and index, in index order.
    loose: Vec<(u32, u32)>,
    /// Features there are (the next one's index).
    count: u32,
}

impl FeatureGrid {
    pub fn new(world: &World, features: &[Feature]) -> FeatureGrid {
        let width = (world.width as i64 * CELL / SQUARE + 1) as i32;
        let height = (world.height as i64 * CELL / SQUARE + 1) as i32;
        let mut grid = FeatureGrid { width, height, ..Default::default() };
        grid.sort(features);
        grid
    }

    fn square(&self, p: FxVec3) -> (i32, i32) {
        (((p.x.to_int() / SQUARE) as i32).clamp(0, self.width - 1), ((p.z.to_int() / SQUARE) as i32).clamp(0, self.height - 1))
    }

    fn square_index(&self, p: FxVec3) -> u32 {
        let (x, z) = self.square(p);
        (z * self.width + x) as u32
    }

    /// Sorts every feature still standing into its square.
    fn sort(&mut self, features: &[Feature]) {
        let squares = (self.width * self.height) as usize;
        self.starts.clear();
        self.starts.resize(squares + 1, 0);
        let alive = || features.iter().enumerate().filter(|(_, f)| f.alive);
        for (_, f) in alive() {
            let s = self.square_index(f.pos) as usize;
            self.starts[s + 1] += 1;
        }
        for s in 1..=squares {
            self.starts[s] += self.starts[s - 1];
        }
        let mut fill = self.starts.clone();
        self.items.clear();
        self.items.resize(self.starts[squares] as usize, 0);
        for (i, f) in alive() {
            let s = self.square_index(f.pos) as usize;
            self.items[fill[s] as usize] = i as u32;
            fill[s] += 1;
        }
        self.loose.clear();
        self.count = features.len() as u32;
    }

    /// Feature `index` was added (the last of `features`).
    pub fn add(&mut self, index: usize, features: &[Feature]) {
        let Some(f) = features.get(index) else { return };
        self.count = self.count.max(index as u32 + 1);
        self.loose.push((self.square_index(f.pos), index as u32));
        if self.loose.len() > LOOSE_MAX {
            self.sort(features);
        }
    }

    /// The features (standing or not: callers look) whose square overlaps the square of
    /// half-size `r` round `p`, in index order: every feature within `r` of `p` among them.
    pub fn near(&self, p: FxVec3, r: Fx, out: &mut Vec<u32>) {
        out.clear();
        let (x0, z0) = self.square(p - FxVec3::new(r, Fx::ZERO, r));
        let (x1, z1) = self.square(p + FxVec3::new(r, Fx::ZERO, r));
        if (x0, z0) == (0, 0) && (x1, z1) == (self.width - 1, self.height - 1) {
            // (All of them: in order already.)
            out.extend(0..self.count);
            return;
        }
        for z in z0..=z1 {
            let row = (z * self.width) as usize;
            let (a, b) = (self.starts[row + x0 as usize] as usize, self.starts[row + x1 as usize + 1] as usize);
            out.extend_from_slice(&self.items[a..b]);
        }
        for &(s, i) in &self.loose {
            let (x, z) = (s as i32 % self.width, s as i32 / self.width);
            if (x0..=x1).contains(&x) && (z0..=z1).contains(&z) {
                out.push(i);
            }
        }
        out.sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::tests::test_world;

    /// What the squares give is what a look at every feature finds, as features are added and
    /// destroyed.
    #[test]
    fn squares_find_what_a_look_at_every_feature_finds() {
        let world = test_world();
        let (w, h) = (world.width as i64 * CELL, world.height as i64 * CELL);
        let mut rng = crate::rng::Rng::new(5);
        let at = |rng: &mut crate::rng::Rng| FxVec3::from_ints(rng.range_i32(0, w as i32 - 1) as i64, 0, rng.range_i32(0, h as i32 - 1) as i64);
        let mut features: Vec<Feature> = (0..300).map(|_| Feature::new(0, (0, 0), at(&mut rng), 0, Fx::ONE)).collect();
        let mut grid = FeatureGrid::new(&world, &features);
        let mut out = Vec::new();
        for round in 0..1200 {
            // Some destroyed, some added (past the loose limit: sorted in again).
            let k = rng.range_i32(0, features.len() as i32 - 1) as usize;
            features[k].alive = false;
            features.push(Feature::new(0, (0, 0), at(&mut rng), 0, Fx::ONE));
            grid.add(features.len() - 1, &features);
            let (p, r) = (at(&mut rng), Fx::from_int(rng.range_i32(0, 200) as i64));
            grid.near(p, r, &mut out);
            let found: Vec<u32> = out.iter().copied().filter(|&i| features[i as usize].alive && features[i as usize].pos.within_xz(p, r)).collect();
            let want: Vec<u32> = (0..features.len() as u32).filter(|&i| features[i as usize].alive && features[i as usize].pos.within_xz(p, r)).collect();
            assert_eq!(found, want, "round {round}");
            assert!(out.windows(2).all(|w| w[0] < w[1]), "in index order");
        }
    }
}
