//! Grid A* pathfinding over footprint anchor cells.
//!
//! Scratch arrays are sized to the map once and invalidated with a search stamp, so each
//! search only touches the cells it expands. Ties are broken on cell index so results are
//! deterministic.
//!
//! Cells are numbered in a `u32` and costs kept in one: a map has at most 2^28 cells
//! (`ta_formats::tnt::MAX_CELLS`), and no route is longer than every cell at a diagonal's
//! cost (14 × 2^28 < 2^32). Estimates (cost so far plus the distance left) are summed in a
//! `u64`, and the sums saturate.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::world::{MoveSpec, World};

const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
/// How far (cells) a unit standing where a distance field doesn't reach looks for the way out
/// to where it does (see [`FlowField::route`]).
const EXIT_RINGS: i32 = 4;
/// How much further than twice the field's goal is (cells) a unit sent to a place of its own
/// looks for where to leave the field's way for it (see [`FlowField::leave_for`]).
const LEAVE_SLACK: i32 = 16;
/// Most cells of the way it tries leaving from.
const LEAVE_LOOKS: usize = 12;

pub struct Pathfinder {
    width: i32,
    height: i32,
    /// What the search knows of each cell, valid where it holds the search's stamp.
    nodes: Vec<Node>,
    current: u32,
    heap: BinaryHeap<Reverse<(u64, u32, u32)>>,
}

/// A search's knowledge of a cell, all together (one fetch from memory, not six): its cost so
/// far and where from (valid where `stamp` is the search's), whether it is done (`closed`),
/// and whether it is open (valid where `seen` is: asked once a search, as every cell is a
/// neighbour of several others and a corner of more).
#[derive(Clone, Copy, Default)]
struct Node {
    stamp: u32,
    g: u32,
    parent: u32,
    closed: u32,
    seen: u32,
    open: bool,
}

impl Clone for Pathfinder {
    fn clone(&self) -> Pathfinder {
        self.fresh()
    }
}

pub struct PathQuery {
    pub spec: MoveSpec,
    pub start: (i32, i32),
    pub goal: (i32, i32),
    /// Accept any cell within this Chebyshev distance of the goal.
    pub goal_radius: i32,
    /// Building slot to ignore (e.g. the factory a unit is leaving).
    pub ignore: Option<u32>,
    pub max_expansions: usize,
}

impl Pathfinder {
    pub fn new(width: u32, height: u32) -> Pathfinder {
        let n = width as usize * height as usize;
        assert!(n as u64 <= ta_formats::tnt::MAX_CELLS, "a map of {width} x {height} cells is more than routes can be found on");
        Pathfinder { width: width as i32, height: height as i32, nodes: vec![Node::default(); n], current: 0, heap: BinaryHeap::new() }
    }

    /// A copy: one with nothing searched yet, as a search takes nothing from the last (what it
    /// knows of the cells is only its scratch, 24 bytes a cell: not worth copying with the
    /// game).
    fn fresh(&self) -> Pathfinder {
        Pathfinder::new(self.width as u32, self.height as u32)
    }

    fn heuristic(a: (i32, i32), b: (i32, i32)) -> u64 {
        let dx = a.0.abs_diff(b.0) as u64;
        let dz = a.1.abs_diff(b.1) as u64;
        let (lo, hi) = if dx < dz { (dx, dz) } else { (dz, dx) };
        DIAGONAL as u64 * lo + STRAIGHT as u64 * (hi - lo)
    }

    /// Finds a path of anchor cells from `start` to the goal (or to the reachable cell
    /// closest to it). The start cell is not included. Also returns whether the goal was
    /// reached (false = partial path, e.g. the expansion budget ran out) and the number of
    /// nodes expanded.
    pub fn find(&mut self, world: &World, q: &PathQuery) -> (Vec<(i32, i32)>, bool, usize) {
        self.current = self.current.wrapping_add(1);
        if self.current == 0 {
            self.nodes.fill(Node::default());
            self.current = 1;
        }
        let cur = self.current;
        let w = self.width;
        let index = |c: (i32, i32)| c.1 as usize * w as usize + c.0 as usize;
        let in_bounds = |c: (i32, i32)| c.0 >= 0 && c.1 >= 0 && c.0 < self.width && c.1 < self.height;
        if !in_bounds(q.start) {
            return (Vec::new(), false, 0);
        }
        let goal = (q.goal.0.clamp(0, self.width - 1), q.goal.1.clamp(0, self.height - 1));
        let passable_at = world.passable_for(&q.spec);
        self.heap.clear();
        let s = index(q.start);
        self.nodes[s] = Node { stamp: cur, g: 0, parent: u32::MAX, ..self.nodes[s] };
        // Heap key: (f, deepest-first tie break, cell). Preferring larger g among equal f
        // follows one straight line on open ground instead of flooding a wedge.
        self.heap.push(Reverse((Self::heuristic(q.start, goal), u32::MAX, s as u32)));
        let mut best = (Self::heuristic(q.start, goal), s as u32);
        let mut expansions = 0;
        let mut found: Option<u32> = None;

        while let Some(Reverse((_, inv_g, node))) = self.heap.pop() {
            let g = u32::MAX - inv_g;
            let n = node as usize;
            if self.nodes[n].closed == cur || g > self.nodes[n].g {
                continue;
            }
            self.nodes[n].closed = cur;
            let c = ((node as i32) % w, (node as i32) / w);
            let h = Self::heuristic(c, goal);
            if h < best.0 {
                best = (h, node);
            }
            if (c.0 - goal.0).abs() <= q.goal_radius && (c.1 - goal.1).abs() <= q.goal_radius {
                found = Some(node);
                break;
            }
            expansions += 1;
            if expansions > q.max_expansions {
                break;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let nc = (c.0 + dx, c.1 + dz);
                if !in_bounds(nc) {
                    continue;
                }
                let ni = index(nc);
                if self.nodes[ni].closed == cur {
                    continue;
                }
                let diagonal = dx != 0 && dz != 0;
                let mut passable = |cell: (i32, i32)| {
                    let n = &mut self.nodes[index(cell)];
                    if n.seen != cur {
                        n.seen = cur;
                        n.open = passable_at(cell, q.ignore);
                    }
                    n.open
                };
                if !passable(nc) {
                    continue;
                }
                // No corner cutting past blocked orthogonal neighbours (both lie on the map
                // when the diagonal neighbour does).
                if diagonal && (!passable((c.0 + dx, c.1)) || !passable((c.0, c.1 + dz))) {
                    continue;
                }
                let ng = g.saturating_add(if diagonal { DIAGONAL } else { STRAIGHT });
                let next = &mut self.nodes[ni];
                if next.stamp != cur || ng < next.g {
                    next.stamp = cur;
                    next.g = ng;
                    next.parent = node;
                    self.heap.push(Reverse((ng as u64 + Self::heuristic(nc, goal), u32::MAX - ng, ni as u32)));
                }
            }
        }

        let end = found.unwrap_or(best.1);
        let mut path = Vec::new();
        let mut n = end;
        while n != u32::MAX && n as usize != s {
            path.push(((n as i32) % w, (n as i32) / w));
            n = self.nodes[n as usize].parent;
        }
        path.reverse();
        // (The lines are walked over the cells the search knows, and the others are asked
        // once: the same answers as asking the world for each.)
        let (width, height) = (self.width, self.height);
        let nodes = &mut self.nodes;
        let open = |c: (i32, i32)| {
            if c.0 < 0 || c.1 < 0 || c.0 >= width || c.1 >= height {
                return passable_at(c, q.ignore);
            }
            let n = &mut nodes[index(c)];
            if n.seen != cur {
                n.seen = cur;
                n.open = passable_at(c, q.ignore);
            }
            n.open
        };
        (smooth(q.start, path, open), found.is_some(), expansions)
    }
}

/// Removes waypoints that can be skipped with a straight line open as `open` says.
fn smooth(start: (i32, i32), path: Vec<(i32, i32)>, mut open: impl FnMut((i32, i32)) -> bool) -> Vec<(i32, i32)> {
    if path.len() <= 2 {
        return path;
    }
    let mut out = Vec::new();
    let mut from = start;
    let mut i = 0;
    while i < path.len() {
        // Furthest waypoint visible from `from`.
        let mut j = path.len() - 1;
        while j > i && !line_clear_by(from, path[j], &mut open) {
            j -= 1;
        }
        out.push(path[j]);
        from = path[j];
        i = j + 1;
    }
    out
}

/// Grid line walk (supercover-ish Bresenham) checking passability of every visited cell.
pub fn line_clear(world: &World, spec: &MoveSpec, a: (i32, i32), b: (i32, i32), ignore: Option<u32>) -> bool {
    line_clear_by(a, b, |c| world.passable(spec, c, ignore))
}

/// [`line_clear`] with passability as `open` says.
fn line_clear_by(a: (i32, i32), b: (i32, i32), open: impl FnMut((i32, i32)) -> bool) -> bool {
    line_walk(a, b, open, true)
}

/// Whether a unit steering straight from cell `a` to cell `b` gets there, sliding along what it
/// grazes: every cell the line passes over is open, and at a diagonal step at least one of the
/// two corners (the unit slides through that one; [`line_clear`] wants both). A corridor that
/// zigzags cell by cell, a narrow pass running diagonally across the grid, is a straight way
/// so, where routes (which never cut a blocked corner) turn at every cell.
pub fn line_slides(world: &World, spec: &MoveSpec, a: (i32, i32), b: (i32, i32), ignore: Option<u32>) -> bool {
    line_walk(a, b, |c| world.passable(spec, c, ignore), false)
}

/// Whether every cell the straight line from `a` to `b` passes over is open as `open` says, and
/// at its diagonal steps both corners (`both_corners`) or one of them.
fn line_walk(a: (i32, i32), b: (i32, i32), mut open: impl FnMut((i32, i32)) -> bool, both_corners: bool) -> bool {
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    let n = dx.abs().max(dz.abs());
    if n == 0 {
        return open(a);
    }
    // Units steer along the straight line from wherever they stand, not from cell to cell:
    // every cell the segment passes over must be open, both rows (or columns) where it runs
    // between two, and both corners of a diagonal step (no cutting past a blocked corner).
    let mut prev = a;
    for i in 0..=n {
        // Coordinates scaled by n: exact along the line.
        let (x, z) = (a.0 * n + dx * i, a.1 * n + dz * i);
        let (x0, x1) = (x.div_euclid(n), (x + n - 1).div_euclid(n));
        let (z0, z1) = (z.div_euclid(n), (z + n - 1).div_euclid(n));
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if !open((cx, cz)) {
                    return false;
                }
            }
        }
        let here = (x0, z0);
        if here.0 != prev.0 && here.1 != prev.1 {
            let (c1, c2) = ((prev.0, here.1), (here.0, prev.1));
            let through = if both_corners { open(c1) && open(c2) } else { open(c1) || open(c2) };
            if !through {
                return false;
            }
        }
        prev = here;
    }
    true
}

/// Open cells a goal's patch of ground must reach for it not to be a pocket (see
/// [`open_goal`]).
const POCKET_CELLS: usize = 256;
/// How far (cells) the way out of a pocket is looked for.
const POCKET_EXIT: i32 = 24;

/// The open anchor cells reachable from `start` (as routes go: 8-connected, no corner cutting),
/// when there are no more than `limit` of them: a pocket walled in by trees, rocks or buildings.
/// None when there are more.
fn pocket(world: &World, spec: &MoveSpec, start: (i32, i32), limit: usize) -> Option<rustc_hash::FxHashSet<(i32, i32)>> {
    let open = |c: (i32, i32)| world.passable(spec, c, None);
    let mut seen = rustc_hash::FxHashSet::default();
    let mut todo = vec![start];
    seen.insert(start);
    while let Some(c) = todo.pop() {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let n = (c.0 + dx, c.1 + dz);
            if seen.contains(&n) || !open(n) || dx != 0 && dz != 0 && !(open((c.0 + dx, c.1)) && open((c.0, c.1 + dz))) {
                continue;
            }
            if seen.len() >= limit {
                return None;
            }
            seen.insert(n);
            todo.push(n);
        }
    }
    Some(seen)
}

/// A goal a unit can get to in place of `goal`: an open cell walled in by trees, rocks or
/// buildings (a pocket of fewer than [`POCKET_CELLS`] cells, although the terrain round it is
/// open) is aimed at from outside, from the nearest open cell of the same region that is not in
/// a pocket itself ([`POCKET_EXIT`] cells off at most; nearest first, then by row and column).
/// Every unit's search or shared field would otherwise flood all it can reach looking for it.
pub(crate) fn open_goal(world: &World, spec: &MoveSpec, goal: (i32, i32)) -> (i32, i32) {
    if !world.passable(spec, goal, None) {
        return goal;
    }
    let Some(walled) = pocket(world, spec, goal, POCKET_CELLS) else { return goal };
    let region = world.component(spec, goal);
    for r in 1..=POCKET_EXIT {
        let mut ring: Vec<(i32, (i32, i32))> = (-r..=r)
            .flat_map(|dz| (-r..=r).map(move |dx| (dx, dz)))
            .filter(|&(dx, dz)| dx.abs().max(dz.abs()) == r)
            .map(|(dx, dz)| (dx * dx + dz * dz, (goal.0 + dx, goal.1 + dz)))
            .filter(|&(_, c)| !walled.contains(&c) && world.passable(spec, c, None) && world.component(spec, c) == region)
            .collect();
        ring.sort_unstable_by_key(|&(d, c)| (d, c.1, c.0));
        if let Some(&(_, c)) = ring.iter().find(|&&(_, c)| pocket(world, spec, c, POCKET_CELLS).is_none()) {
            return c;
        }
    }
    goal
}

/// A distance field from one goal over a movement class's passable cells, shared by every
/// unit heading there (army-wide orders): one linear-time Dijkstra pass instead of an A*
/// search per unit. Units read their route off the field by walking downhill.
#[derive(Clone)]
pub struct FlowField {
    pub spec: MoveSpec,
    pub goal: (i32, i32),
    /// The window of the map the field covers: `origin` up to `origin + (width, height)`.
    origin: (i32, i32),
    width: i32,
    height: i32,
    /// Path cost to the goal per anchor cell (`u32::MAX` = unreachable), on the window's grid
    /// padded with a one-cell border that is never reached.
    dist: Vec<u32>,
    /// Whether each cell of that grid was passable when the distances were found.
    open: Vec<bool>,
    /// `World::blocking_version` it is up to date with (built or repaired at).
    pub blocking_version: u64,
    /// `World::occupancy_version` it is up to date with.
    pub occupancy_version: u64,
    pub tick: u64,
    /// The tick it was built or last brought up to date.
    pub repaired: u64,
}

/// Cells by distance, nearest first, for bringing a field up to date: what is found as it goes
/// lies at most a diagonal step beyond the cell being looked at, so 16 buckets in a ring hold
/// it (as in [`FlowField::build_within`]); the cells it starts from, at any distance, are sorted
/// and taken in as their distance comes up. (A binary heap took several times as long.)
struct DistanceQueue {
    ring: [Vec<u32>; 16],
    in_ring: usize,
    seeds: Vec<(u32, u32)>,
    next: usize,
    current: u32,
}

impl DistanceQueue {
    fn new(mut seeds: Vec<(u32, u32)>) -> DistanceQueue {
        seeds.sort_unstable();
        seeds.dedup();
        let current = seeds.first().map_or(0, |s| s.0);
        DistanceQueue { ring: Default::default(), in_ring: 0, seeds, next: 0, current }
    }

    /// A cell at distance `d`, no further than a diagonal step beyond the last one taken.
    fn push(&mut self, d: u32, cell: u32) {
        debug_assert!(d >= self.current && d < self.current + 16);
        self.ring[(d % 16) as usize].push(cell);
        self.in_ring += 1;
    }

    /// The nearest cell left and its distance (those at one distance in any order).
    fn pop(&mut self) -> Option<(u32, u32)> {
        loop {
            if let Some(&(d, cell)) = self.seeds.get(self.next).filter(|s| s.0 == self.current) {
                self.next += 1;
                return Some((d, cell));
            }
            if let Some(cell) = self.ring[(self.current % 16) as usize].pop() {
                self.in_ring -= 1;
                return Some((self.current, cell));
            }
            if self.in_ring > 0 {
                self.current += 1;
            } else {
                self.current = self.seeds.get(self.next)?.0;
            }
        }
    }
}

/// The eight steps between cells of a padded grid `pw` wide: (offset, cost, the two
/// orthogonal steps a diagonal needs open).
fn grid_steps(pw: usize) -> [(isize, u32, isize, isize); 8] {
    let (e, s) = (1isize, pw as isize);
    [
        (e, STRAIGHT, 0, 0),
        (-e, STRAIGHT, 0, 0),
        (s, STRAIGHT, 0, 0),
        (-s, STRAIGHT, 0, 0),
        (e + s, DIAGONAL, e, s),
        (e - s, DIAGONAL, e, -s),
        (-e + s, DIAGONAL, -e, s),
        (-e - s, DIAGONAL, -e, -s),
    ]
}

impl FlowField {
    /// Dijkstra from `goal` with a bucket queue (step costs are 10 and 14, so 16 rotating
    /// buckets suffice). Diagonal steps need both orthogonal neighbours passable, like A*.
    /// Shortest distances are unique, so the visiting order never changes the field.
    pub fn build(world: &World, spec: &MoveSpec, goal: (i32, i32), tick: u64) -> FlowField {
        Self::build_within(world, spec, goal, tick, (0, 0), (world.width as i32, world.height as i32))
    }

    /// [`FlowField::build`] over a window of the map (`from` up to, not including, `to`): the
    /// distances are exact for routes that stay inside it, and cells outside are unreachable.
    /// Big groups rarely need the whole map, and building costs in proportion to the area.
    pub fn build_within(world: &World, spec: &MoveSpec, goal: (i32, i32), tick: u64, from: (i32, i32), to: (i32, i32)) -> FlowField {
        let (mw, mh) = (world.width as i32, world.height as i32);
        let (x0, z0) = (from.0.clamp(0, mw), from.1.clamp(0, mh));
        let (x1, z1) = (to.0.clamp(x0, mw), to.1.clamp(z0, mh));
        let (w, h) = (x1 - x0, z1 - z0);
        // Flat indices on a grid padded with closed cells: no bounds checks in the loop.
        let pw = (w + 2) as usize;
        let n = pw * (h + 2) as usize;
        let mut open = vec![false; n];
        for (i, ok) in world.passable_window(spec, (x0, z0), (x1, z1)).into_iter().enumerate() {
            open[(i / w as usize + 1) * pw + i % w as usize + 1] = ok;
        }
        let mut dist = vec![u32::MAX; n];
        let mut buckets: Vec<Vec<u32>> = (0..16).map(|_| Vec::new()).collect();
        let inside = goal.0 >= x0 && goal.1 >= z0 && goal.0 < x1 && goal.1 < z1;
        let g = (goal.1 - z0 + 1).max(0) as usize * pw + (goal.0 - x0 + 1).max(0) as usize;
        if inside && open[g] {
            dist[g] = 0;
            buckets[0].push(g as u32);
        }
        let steps = grid_steps(pw);
        let mut cost = 0u32;
        let mut pending = buckets[0].len();
        while pending > 0 {
            let slot = (cost % 16) as usize;
            let cells = std::mem::take(&mut buckets[slot]);
            pending -= cells.len();
            for &c in &cells {
                let c = c as usize;
                if dist[c] != cost {
                    continue; // stale entry
                }
                for &(off, step, a, b) in &steps {
                    let ni = c.wrapping_add_signed(off);
                    if !open[ni] || step == DIAGONAL && !(open[c.wrapping_add_signed(a)] && open[c.wrapping_add_signed(b)]) {
                        continue;
                    }
                    let nd = cost.saturating_add(step);
                    if nd < dist[ni] {
                        dist[ni] = nd;
                        buckets[(nd % 16) as usize].push(ni as u32);
                        pending += 1;
                    }
                }
            }
            // Hand the emptied bucket's allocation back.
            let mut cells = cells;
            cells.clear();
            if buckets[slot].is_empty() {
                buckets[slot] = cells;
            }
            cost += 1;
        }
        FlowField {
            spec: *spec,
            goal,
            origin: (x0, z0),
            width: w,
            height: h,
            dist,
            open,
            blocking_version: world.blocking_version,
            occupancy_version: world.occupancy_version,
            tick,
            repaired: tick,
        }
    }

    /// Brings the field up to date with the world's buildings and features without building it
    /// again: what was blocked since takes its distances with it (and those of the cells that
    /// were only reached through it), what opened up gives its own, and the cells that lost
    /// theirs are found again from those around them. The distances come out exactly as
    /// [`FlowField::build_within`] over the same window would find them now (shortest
    /// distances are unique). False when it can't be: the world no longer knows every change
    /// since (a field far behind), or the goal itself was blocked or opened.
    pub fn repair(&mut self, world: &World, tick: u64) -> bool {
        let Some(changes) = world.changes_since(self.occupancy_version) else { return false };
        let (w, h) = (self.width, self.height);
        let pw = (w + 2) as usize;
        let index = |x: i32, z: i32| (z - self.origin.1 + 1) as usize * pw + (x - self.origin.0 + 1) as usize;
        // (Everything is measured from the goal: a goal blocked or opened is a new field.)
        if self.covers(self.goal) && world.passable(&self.spec, self.goal, None) != self.open[index(self.goal.0, self.goal.1)] {
            return false;
        }
        // The anchor cells of the window whose footprint covers a changed cell.
        let (fx, fz) = (self.spec.footprint.0 as i32, self.spec.footprint.1 as i32);
        let mut touched: Vec<u32> = Vec::new();
        for c in changes {
            let (ax0, ax1) = ((c.from.0 + fx / 2 - fx + 1).max(self.origin.0), (c.to.0 + fx / 2).min(self.origin.0 + w));
            let (az0, az1) = ((c.from.1 + fz / 2 - fz + 1).max(self.origin.1), (c.to.1 + fz / 2).min(self.origin.1 + h));
            for z in az0..az1 {
                touched.extend((ax0..ax1).map(|x| index(x, z) as u32));
            }
        }
        self.occupancy_version = world.occupancy_version;
        self.repaired = tick;
        self.blocking_version = world.blocking_version;
        touched.sort_unstable();
        touched.dedup();
        let (mut closed, mut opened) = (Vec::new(), Vec::new());
        for i in touched {
            let i = i as usize;
            let c = ((i % pw) as i32 - 1 + self.origin.0, (i / pw) as i32 - 1 + self.origin.1);
            let now = world.passable(&self.spec, c, None);
            if now != self.open[i] {
                self.open[i] = now;
                if now { opened.push(i) } else { closed.push(i) }
            }
        }
        if closed.is_empty() && opened.is_empty() {
            return true;
        }
        let steps = grid_steps(pw);
        let (open, dist) = (&self.open, &mut self.dist);
        let step_ok = |c: usize, &(off, cost, a, b): &(isize, u32, isize, isize)| {
            let n = c.wrapping_add_signed(off);
            open[n] && (cost == STRAIGHT || open[c.wrapping_add_signed(a)] && open[c.wrapping_add_signed(b)])
        };
        // Lost: the blocked cells, then (nearest the goal first) every cell none of whose
        // neighbours still gives it its distance. A cell's givers are nearer the goal, so they
        // are settled before it is looked at; it only ever took its distance from them.
        let mut seeds: Vec<(u32, u32)> = Vec::new();
        for &k in &closed {
            dist[k] = u32::MAX;
            // (Its neighbours may have come through it, or past its corner.)
            for s in &steps {
                let n = k.wrapping_add_signed(s.0);
                if dist[n] != u32::MAX {
                    seeds.push((dist[n], n as u32));
                }
            }
        }
        let mut queue = DistanceQueue::new(seeds);
        let mut lost: Vec<usize> = Vec::new();
        while let Some((d, c)) = queue.pop() {
            let c = c as usize;
            // (Gone already, looked at already, or the goal itself.)
            if dist[c] != d || d == 0 {
                continue;
            }
            let given = steps.iter().any(|s| step_ok(c, s) && dist[c.wrapping_add_signed(s.0)].saturating_add(s.1) == d);
            if given {
                continue;
            }
            dist[c] = u32::MAX;
            lost.push(c);
            // Those that took their distance from it.
            for s in &steps {
                let n = c.wrapping_add_signed(s.0);
                if dist[n] == d + s.1 && step_ok(c, s) {
                    queue.push(dist[n], n as u32);
                }
            }
        }
        // Found again: Dijkstra from the cells around the lost and the opened ones, lowering
        // whatever a shorter way now reaches (opened cells, and the diagonals past them).
        let mut seeds: Vec<(u32, u32)> = Vec::new();
        for &c in lost.iter().chain(&opened) {
            for s in &steps {
                let n = c.wrapping_add_signed(s.0);
                if dist[n] != u32::MAX {
                    seeds.push((dist[n], n as u32));
                }
            }
        }
        let mut queue = DistanceQueue::new(seeds);
        while let Some((d, c)) = queue.pop() {
            let c = c as usize;
            if dist[c] != d {
                continue;
            }
            for s in &steps {
                if !step_ok(c, s) {
                    continue;
                }
                let n = c.wrapping_add_signed(s.0);
                let nd = d.saturating_add(s.1);
                if nd < dist[n] {
                    dist[n] = nd;
                    queue.push(nd, n as u32);
                }
            }
        }
        true
    }

    #[inline]
    fn at(&self, c: (i32, i32)) -> u32 {
        self.dist[self.index(c)]
    }

    /// Whether a cell lies in the field's window (reachable from there or not).
    pub fn covers(&self, c: (i32, i32)) -> bool {
        c.0 >= self.origin.0 && c.1 >= self.origin.1 && c.0 < self.origin.0 + self.width && c.1 < self.origin.1 + self.height
    }

    /// The window of the map the field covers (from, to).
    pub fn window(&self) -> ((i32, i32), (i32, i32)) {
        (self.origin, (self.origin.0 + self.width, self.origin.1 + self.height))
    }

    pub fn reaches(&self, c: (i32, i32)) -> bool {
        self.covers(c) && self.at(c) != u32::MAX
    }

    /// The route from `start` to the goal (see [`FlowField::route_to`]).
    pub fn route(&self, start: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        self.route_to(start, self.goal)
    }

    /// The route from `start` to `goal`: steepest descent, then only the corners, then
    /// skipping waypoints with a clear straight line (looking a few ahead) as the field has the
    /// ground (its record of which cells are open: what its distances were found with). The
    /// start cell is not included. Standing where the field doesn't reach (a wreck fell there,
    /// or the crowd pushed the unit into a building's corner), the way out is to the nearest
    /// cell it does reach, at most [`EXIT_RINGS`] cells off, and on from there.
    ///
    /// `goal` needn't be the field's own: a group sent to places round a point shares the field
    /// to one of them, and each leaves the way there for its own place (see
    /// [`FlowField::leave_for`]); a place the field doesn't reach (a clearing walled in by
    /// trees) is aimed at from the nearest cell it does, [`POCKET_EXIT`] cells off at most (the
    /// route's last cell). `None` when there is no way.
    pub fn route_to(&self, start: (i32, i32), goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        self.route_to_with(start, goal, false)
    }

    /// [`FlowField::route_to`]; `far`, a unit of a formation going its own way to its place,
    /// leaves the way downhill for its place as early as it can (looking all along it, not
    /// just near its place): routes from a crowd to places side by side run side by side, not
    /// down the one way downhill they all join.
    pub fn route_to_with(&self, start: (i32, i32), goal: (i32, i32), far: bool) -> Option<Vec<(i32, i32)>> {
        if !self.reaches(start) {
            let exit = (1..=EXIT_RINGS).find_map(|r| {
                (-r..=r)
                    .flat_map(|dz| (-r..=r).map(move |dx| (start.0 + dx, start.1 + dz)))
                    .filter(|&c| (c.0 - start.0).abs().max((c.1 - start.1).abs()) == r && self.reaches(c))
                    .min_by_key(|&c| (self.at(c), c.1, c.0))
            })?;
            let mut route = self.route_to_with(exit, goal, far)?;
            route.insert(0, exit);
            return Some(route);
        }
        let mut cells = self.descend(start);
        if goal != self.goal {
            let goal = if self.reaches(goal) { goal } else { self.reached_near(goal)? };
            self.leave_for(start, &mut cells, goal, far)?;
        }
        Some(self.smooth(start, &cells))
    }

    /// The nearest cell to `goal` the field reaches, [`POCKET_EXIT`] cells off at most (nearest
    /// first, then by row and column).
    fn reached_near(&self, goal: (i32, i32)) -> Option<(i32, i32)> {
        (1..=POCKET_EXIT).find_map(|r| {
            (-r..=r)
                .flat_map(|dz| (-r..=r).map(move |dx| (dx, dz)))
                .filter(|&(dx, dz)| dx.abs().max(dz.abs()) == r && self.reaches((goal.0 + dx, goal.1 + dz)))
                .min_by_key(|&(dx, dz)| (dx * dx + dz * dz, goal.1 + dz, goal.0 + dx))
                .map(|(dx, dz)| (goal.0 + dx, goal.1 + dz))
        })
    }

    /// The flat index of a cell of the window on the padded grid.
    #[inline]
    fn index(&self, c: (i32, i32)) -> usize {
        (c.1 - self.origin.1 + 1) as usize * (self.width + 2) as usize + (c.0 - self.origin.0 + 1) as usize
    }

    /// Whether the field had a cell of its window open.
    fn open_at(&self, c: (i32, i32)) -> bool {
        self.covers(c) && self.open[self.index(c)]
    }

    /// Whether the straight line from `a` to `b` is clear as the field had the ground: the
    /// cells [`line_clear`] looks at, walked on the padded grid (the line's place in a cell
    /// kept as a remainder rather than found by division at every step).
    fn line_open(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        if !(self.covers(a) && self.covers(b)) {
            return line_clear_by(a, b, |c| self.open_at(c));
        }
        let open = &self.open;
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let n = dx.abs().max(dz.abs());
        if n == 0 {
            return open[self.index(a)];
        }
        // (Every cell looked at lies between the two, inside the window.)
        let pw = (self.width + 2) as usize;
        let (mut x, mut z) = a;
        let (mut rx, mut rz) = (0, 0);
        let mut prev = a;
        for i in 0..=n {
            if i > 0 {
                rx += dx;
                if rx >= n {
                    rx -= n;
                    x += 1;
                } else if rx < 0 {
                    rx += n;
                    x -= 1;
                }
                rz += dz;
                if rz >= n {
                    rz -= n;
                    z += 1;
                } else if rz < 0 {
                    rz += n;
                    z -= 1;
                }
            }
            let c = self.index((x, z));
            if !open[c] || rx > 0 && !open[c + 1] || rz > 0 && (!open[c + pw] || rx > 0 && !open[c + pw + 1]) {
                return false;
            }
            if x != prev.0 && z != prev.1 && !(open[self.index((prev.0, z))] && open[self.index((x, prev.1))]) {
                return false;
            }
            prev = (x, z);
        }
        true
    }

    /// The cells downhill from `from` (a cell the field reaches; not included) to the goal. On
    /// the padded grid (its border never reached: no bounds to check): of the neighbours the
    /// nearest the goal, the first in this order of equals; no cutting past a blocked corner
    /// (`from` may be a cell the field's own steps never came through).
    fn descend(&self, from: (i32, i32)) -> Vec<(i32, i32)> {
        let pw = (self.width + 2) as isize;
        let cell = |i: usize| ((i as isize % pw) as i32 - 1 + self.origin.0, (i as isize / pw) as i32 - 1 + self.origin.1);
        let steps: [(isize, isize, isize); 8] = [(1, 0, 0), (-1, 0, 0), (pw, 0, 0), (-pw, 0, 0), (1 + pw, 1, pw), (1 - pw, 1, -pw), (pw - 1, -1, pw), (-1 - pw, -1, -pw)];
        let dist = &self.dist;
        let mut cells = Vec::new();
        let mut c = self.index(from);
        let mut guard = 0;
        while dist[c] > 0 && guard < 100_000 {
            guard += 1;
            let mut best = c;
            let mut best_d = dist[c];
            for &(off, a, b) in &steps {
                let n = c.wrapping_add_signed(off);
                if dist[n] < best_d && (a == 0 || dist[c.wrapping_add_signed(a)] != u32::MAX && dist[c.wrapping_add_signed(b)] != u32::MAX) {
                    best = n;
                    best_d = dist[n];
                }
            }
            if best == c {
                break;
            }
            cells.push(cell(best));
            c = best;
        }
        cells
    }

    /// Turns `cells`, the way downhill from `start` to the field's goal, into one to `goal`, a
    /// cell the field reaches: off it at the first cell from which `goal` is in a clear
    /// straight line (the earliest is the shortest way, as no step downhill brings a place
    /// nearer than going straight), looked for among the cells within reach of `goal` (twice
    /// as far as the field's goal is, and a little); failing that, along it to where the way
    /// downhill from `goal` joins it and back up that. `None` when the two never meet.
    fn leave_for(&self, start: (i32, i32), cells: &mut Vec<(i32, i32)>, goal: (i32, i32), far: bool) -> Option<()> {
        let cheb = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs().max((a.1 - b.1).abs());
        let reach = if far { i32::MAX } else { 2 * cheb(self.goal, goal) + LEAVE_SLACK };
        // (Index 0 is `start`, then the cells.)
        let at = |i: usize| if i == 0 { start } else { cells[i - 1] };
        let near: Vec<usize> = (0..=cells.len()).filter(|&i| cheb(at(i), goal) <= reach).collect();
        let every = near.len().div_ceil(LEAVE_LOOKS).max(1);
        if let Some(&i) = near.iter().step_by(every).find(|&&i| self.line_open(at(i), goal)) {
            cells.truncate(i);
            cells.push(goal);
            return Some(());
        }
        let back = self.descend(goal);
        let on_back: rustc_hash::FxHashSet<(i32, i32)> = back.iter().copied().chain(std::iter::once(goal)).collect();
        let join = (0..=cells.len()).find(|&i| on_back.contains(&at(i)))?;
        let joined = at(join);
        cells.truncate(join);
        if joined != goal {
            let k = back.iter().position(|&c| c == joined)?;
            cells.extend(back[..k].iter().rev());
            cells.push(goal);
        }
        Some(())
    }

    /// Waypoints along `cells`, a chain of cells from `start` in which each is in a clear line
    /// from the one before (neighbours, mostly): only the corners, skipping those with a clear
    /// straight line past them (looking a few ahead: bounded cost on long routes). Where not
    /// even the next corner is in a clear line (a straight run that grazes an obstacle from a
    /// cell off the chain), the chain's own steps lead there.
    fn smooth(&self, start: (i32, i32), cells: &[(i32, i32)]) -> Vec<(i32, i32)> {
        // Corners only: drop cells in the middle of straight runs (kept by their place in
        // the chain).
        let mut corners: Vec<usize> = Vec::new();
        let mut prev = start;
        for (i, &cell) in cells.iter().enumerate() {
            let next = cells.get(i + 1).copied();
            let dir = (cell.0 - prev.0, cell.1 - prev.1);
            if next.is_none_or(|n| (n.0 - cell.0, n.1 - cell.1) != dir) {
                corners.push(i);
            }
            prev = cell;
        }
        let mut out = Vec::with_capacity(corners.len());
        let mut from = start;
        let mut done = 0;
        let mut i = 0;
        while i < corners.len() {
            let mut j = (i + 8).min(corners.len() - 1);
            while j > i && !self.line_open(from, cells[corners[j]]) {
                j -= 1;
            }
            if j == i && !self.line_open(from, cells[corners[i]]) {
                out.extend_from_slice(&cells[done..corners[i]]);
            }
            out.push(cells[corners[j]]);
            from = cells[corners[j]];
            done = corners[j] + 1;
            i = j + 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::tests::test_world;

    /// The field as a plain Dijkstra over (x, z) cells computes it.
    fn reference(world: &World, spec: &MoveSpec, goal: (i32, i32)) -> Vec<u32> {
        let (w, h) = (world.width as i32, world.height as i32);
        let passable = |c: (i32, i32)| c.0 >= 0 && c.1 >= 0 && c.0 < w && c.1 < h && world.passable(spec, c, None);
        let mut dist = vec![u32::MAX; (w * h) as usize];
        let mut heap = BinaryHeap::new();
        if passable(goal) {
            dist[(goal.1 * w + goal.0) as usize] = 0;
            heap.push(Reverse((0u32, goal)));
        }
        while let Some(Reverse((d, c))) = heap.pop() {
            if d != dist[(c.1 * w + c.0) as usize] {
                continue;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let n = (c.0 + dx, c.1 + dz);
                let diagonal = dx != 0 && dz != 0;
                if !passable(n) || diagonal && !(passable((c.0 + dx, c.1)) && passable((c.0, c.1 + dz))) {
                    continue;
                }
                let nd = d + if diagonal { DIAGONAL } else { STRAIGHT };
                if nd < dist[(n.1 * w + n.0) as usize] {
                    dist[(n.1 * w + n.0) as usize] = nd;
                    heap.push(Reverse((nd, n)));
                }
            }
        }
        dist
    }

    #[test]
    fn flow_fields_hold_the_shortest_distances() {
        let mut world = test_world();
        for footprint in [(1, 1), (2, 2), (3, 3)] {
            let spec = MoveSpec { footprint, max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
            world.prepare(&spec);
            for goal in [(2, 2), (40, 30), (22, 12), (63, 47), (-5, 3)] {
                let field = FlowField::build(&world, &spec, goal, 0);
                let want = reference(&world, &spec, goal);
                let mut reached = 0;
                for z in 0..world.height as i32 {
                    for x in 0..world.width as i32 {
                        let d = want[(z as u32 * world.width + x as u32) as usize];
                        assert_eq!(field.at((x, z)), d, "{footprint:?} goal {goal:?} at ({x}, {z})");
                        reached += (d != u32::MAX) as usize;
                    }
                }
                if goal == (2, 2) {
                    assert!(reached > 1000, "most of the land is reachable ({reached} cells)");
                    let route = field.route((45, 10)).expect("a route round the ridge");
                    assert_eq!(route.last(), Some(&(2, 2)));
                }
            }
        }
    }

    /// A field brought up to date as buildings and features come and go holds what a field built
    /// afresh over the same window finds.
    #[test]
    fn repaired_fields_match_fields_built_again() {
        use crate::world::Occupant;
        let mut rng = crate::rng::Rng::new(9);
        for (footprint, goal, from, to) in [((1, 1), (4, 4), (0, 0), (40, 30)), ((2, 2), (22, 26), (6, 3), (60, 45)), ((3, 3), (45, 12), (20, 0), (64, 48)), ((2, 2), (70, 20), (0, 0), (64, 48))] {
            let mut world = test_world();
            let spec = MoveSpec { footprint, max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
            world.prepare(&spec);
            let mut field = FlowField::build_within(&world, &spec, goal, 0, from, to);
            // (Where each feature placed stands: origin, footprint, slot.)
            type Placed = ((i32, i32), (u32, u32), u32);
            let mut placed: Vec<Placed> = Vec::new();
            let (mut repaired, mut rebuilt) = (0, 0);
            for round in 0..400u32 {
                if placed.len() < 3 || rng.chance(3, 5) {
                    let fp = (rng.range_i32(1, 3) as u32, rng.range_i32(1, 3) as u32);
                    let origin = (rng.range_i32(-2, 64), rng.range_i32(-2, 48));
                    world.occupy(Occupant::Feature(1000 + round), origin, fp, &[]);
                    placed.push((origin, fp, 1000 + round));
                } else {
                    let (origin, fp, slot) = placed.swap_remove(rng.range_i32(0, placed.len() as i32 - 1) as usize);
                    world.release(Occupant::Feature(slot), origin, fp);
                }
                if round % 3 == 0 {
                    let fresh = FlowField::build_within(&world, &spec, goal, 0, from, to);
                    if field.repair(&world, round as u64) {
                        assert_eq!(field.open, fresh.open, "{footprint:?} round {round}: passable cells");
                        assert_eq!(field.dist, fresh.dist, "{footprint:?} round {round}: distances");
                        repaired += 1;
                    } else {
                        field = fresh;
                        rebuilt += 1;
                    }
                }
            }
            assert!(repaired > rebuilt * 4, "{footprint:?}: {repaired} repaired, {rebuilt} built again");
        }
    }

    /// Lines walked on a field's grid look at the cells a line walked on the ground does; routes
    /// to places other than the field's own goal end there, along open ground.
    #[test]
    fn lines_and_routes_off_a_field() {
        use crate::world::Occupant;
        let mut rng = crate::rng::Rng::new(5);
        let mut world = test_world();
        let spec = MoveSpec { footprint: (2, 2), max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
        world.prepare(&spec);
        for i in 0..40 {
            let origin = (rng.range_i32(0, 62), rng.range_i32(0, 46));
            world.occupy(Occupant::Feature(100 + i), origin, (1, 1), &[]);
        }
        let (from, to) = ((3, 2), (61, 44));
        let field = FlowField::build_within(&world, &spec, (14, 24), 0, from, to);
        for _ in 0..4000 {
            let a = (rng.range_i32(from.0, to.0 - 1), rng.range_i32(from.1, to.1 - 1));
            let b = (rng.range_i32(from.0, to.0 - 1), rng.range_i32(from.1, to.1 - 1));
            assert_eq!(field.line_open(a, b), line_clear_by(a, b, |c| field.open_at(c)), "{a:?} to {b:?}");
        }
        let mut routes = 0;
        for _ in 0..300 {
            let start = (rng.range_i32(from.0, to.0 - 1), rng.range_i32(from.1, to.1 - 1));
            let goal = (rng.range_i32(from.0, to.0 - 1), rng.range_i32(from.1, to.1 - 1));
            if !field.reaches(start) || !field.reaches(goal) {
                continue;
            }
            let route = field.route_to(start, goal).expect("a way between two cells the field reaches");
            assert_eq!(route.last().copied().unwrap_or(start), goal, "from {start:?}");
            let mut at = start;
            for &w in &route {
                assert!(line_clear(&world, &spec, at, w, None), "{at:?} to {w:?} on the way from {start:?} to {goal:?}");
                at = w;
            }
            routes += 1;
        }
        assert!(routes > 100, "{routes} routes");
    }

    /// A corridor one cell wide running diagonally across the grid (open cells in a zigzag, its
    /// corners blocked on one side) is no straight way for `line_clear`, which routes keep to
    /// (so they turn at every cell), but one a unit slides along (`line_slides`); a line past a
    /// corner blocked on both sides, or over a blocked cell, is neither.
    #[test]
    fn a_zigzag_corridor_is_a_way_to_slide_along() {
        use crate::world::Occupant;
        let mut world = test_world();
        let spec = MoveSpec { footprint: (1, 1), max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
        world.prepare(&spec);
        // (40, 36) (40, 37) (41, 37) (41, 38) (42, 38) ...: everything else round it blocked, but
        // for two cells corner to corner, both their other corners blocked.
        let zigzag: Vec<(i32, i32)> = (0..6).flat_map(|k| [(40 + k, 36 + k), (40 + k, 37 + k)]).collect();
        let pair = [(46, 35), (47, 36)];
        let mut slot = 500;
        for z in 34..45 {
            for x in 38..49 {
                if !zigzag.contains(&(x, z)) && !pair.contains(&(x, z)) {
                    world.occupy(Occupant::Feature(slot), (x, z), (1, 1), &[]);
                    slot += 1;
                }
            }
        }
        assert!(zigzag.iter().chain(&pair).all(|&c| world.passable(&spec, c, None)));
        let (a, b) = ((40, 36), (44, 40));
        assert!(!line_clear(&world, &spec, a, b, None), "a diagonal past corners blocked on one side is no clear line");
        assert!(line_slides(&world, &spec, a, b, None), "but a way to slide along");
        assert!(line_slides(&world, &spec, (40, 37), (43, 39), None), "at a slant too");
        assert!(!line_slides(&world, &spec, pair[0], pair[1], None), "not past a corner blocked on both sides");
        assert!(!line_slides(&world, &spec, a, (45, 38), None), "nor over blocked cells");
        // A route along it keeps to it, cell by cell.
        let mut pf = Pathfinder::new(world.width, world.height);
        let (route, reached, _) = pf.find(&world, &PathQuery { spec, start: a, goal: (45, 41), goal_radius: 0, ignore: None, max_expansions: 10_000 });
        assert!(reached, "the corridor leads there");
        assert!(route.len() >= 9, "turning at every cell: {route:?}");
    }

    /// A field over a window never promises a shorter way than the whole map's field, knows
    /// nothing outside, and matches it near the goal.
    #[test]
    fn windowed_fields_are_exact_inside_and_blind_outside() {
        let mut world = test_world();
        let spec = MoveSpec { footprint: (2, 2), max_slope: 12, max_depth: 12, min_depth: -10_000, hover: false };
        world.prepare(&spec);
        let goal = (2, 2);
        let full = FlowField::build(&world, &spec, goal, 0);
        let (from, to) = ((0, 0), (30, 25));
        let part = FlowField::build_within(&world, &spec, goal, 0, from, to);
        assert_eq!(part.window(), (from, to));
        for z in 0..world.height as i32 {
            for x in 0..world.width as i32 {
                let inside = x >= from.0 && z >= from.1 && x < to.0 && z < to.1;
                assert_eq!(part.covers((x, z)), inside);
                if !inside {
                    assert!(!part.reaches((x, z)));
                } else if part.reaches((x, z)) {
                    assert!(part.at((x, z)) >= full.at((x, z)), "({x}, {z}): no shortcut through the window's edge");
                }
            }
        }
        assert_eq!(part.at(goal), 0);
        for c in [(3, 2), (2, 5), (6, 6)] {
            if full.reaches(c) {
                assert_eq!(part.at(c), full.at(c), "{c:?} near the goal");
            }
        }
        let whole = FlowField::build_within(&world, &spec, goal, 0, (-3, -3), (1000, 1000));
        assert_eq!(whole.dist, full.dist, "a window over the whole map is the whole field");
    }
}
