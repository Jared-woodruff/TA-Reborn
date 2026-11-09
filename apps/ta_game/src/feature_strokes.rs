//! The strokes of a painting: the centre lines of what TA painted from above as lines (a run of
//! pipes, a row of barriers), traced through its outline, how wide each is and where they meet;
//! and the patches it scatters (a ruin's blocks, broken pillars, rubble). The 3D stand-ins of
//! such features are laid along them or set where they lie (`features.rs`).
//!
//! The outline is thinned to its skeleton (Zhang and Suen), the skeleton read as a graph (its
//! ends, the places three or more lines meet, the lines between), the short spurs a painted bulge
//! leaves pruned, and each line smoothed and simplified; its half-width is the distance from its
//! middle to the outline's edge (a distance transform), taken where it runs on its own.

/// A centre line through a painting: its points (painting pixels from the anchor, x east and y
/// south), its half-width, and whether each end is free (the painted line ends there: the points
/// reach its painted end) or meets other lines (a junction's place).
#[derive(Clone, Debug)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub radius: f32,
    pub free: [bool; 2],
}

impl Stroke {
    /// How long it runs.
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| dist(w[0], w[1])).sum()
    }

    /// The point `s` along it (clamped to its ends).
    pub fn at(&self, s: f32) -> [f32; 2] {
        let mut left = s.max(0.0);
        for w in self.points.windows(2) {
            let d = dist(w[0], w[1]);
            if left <= d && d > 0.0 {
                let t = left / d;
                return [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
            }
            left -= d;
        }
        *self.points.last().unwrap_or(&[0.0; 2])
    }
}

/// Where lines meet: the middle of the place.
#[derive(Clone, Copy, Debug)]
pub struct Junction {
    pub at: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct Strokes {
    pub strokes: Vec<Stroke>,
    pub junctions: Vec<Junction>,
}

pub(crate) fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The strokes of an outline (`mask`, `w` x `h`, row-major), in painting pixels from `origin`
/// (the anchor's pixel).
pub fn trace(mask: &[bool], w: usize, h: usize, origin: [f32; 2]) -> Strokes {
    // (A border of empty pixels round it: the outline never touches the image's edge.)
    let (pw, ph) = (w + 2, h + 2);
    let mut solid = vec![false; pw * ph];
    for y in 0..h {
        for x in 0..w {
            solid[(y + 1) * pw + x + 1] = mask[y * w + x];
        }
    }
    let dt = distance_transform(&solid, pw, ph);
    let mut skel = solid.clone();
    thin(&mut skel, pw, ph);
    // The graph, pruned of spurs until none is left.
    let mut graph = Graph::of(&skel, pw, ph);
    for _ in 0..12 {
        let mut pruned = false;
        for e in &graph.edges {
            // (A spur: what a painted bulge or a square end's corners leave, reaching little
            // further from the junction than the line it leaves is wide.)
            let spur = match (e.ends[0], e.ends[1]) {
                (Node::End(p), Node::Junction(j)) | (Node::Junction(j), Node::End(p)) => {
                    let at = graph.junction_centre(j, pw);
                    let reach = dist(at, [(p % pw) as f32 + 0.5, (p / pw) as f32 + 0.5]) + dt[p];
                    reach <= graph.junction_radius(j, &dt) * 1.6 + 2.0
                }
                (Node::End(_), Node::End(_)) => e.pixels.len() < 3,
                // (A short loop back into its own junction: a knot of the thinning's.)
                (Node::Junction(a), Node::Junction(b)) => a == b && (e.pixels.len() as f32) < graph.junction_radius(a, &dt) * 2.0 + 4.0,
                _ => false,
            };
            if spur {
                // (Its pixels but the junction's own.)
                for &p in &e.pixels {
                    if !graph.junction_pixel[p] {
                        skel[p] = false;
                    }
                }
                pruned = true;
            }
        }
        if !pruned {
            break;
        }
        graph = Graph::of(&skel, pw, ph);
    }
    let origin = [origin[0] + 1.0, origin[1] + 1.0];
    let to_painting = |p: usize| [(p % pw) as f32 + 0.5 - origin[0], (p / pw) as f32 + 0.5 - origin[1]];
    let mut out = Strokes::default();
    let centres: Vec<[f32; 2]> = graph.junctions.iter().map(|px| {
        let n = px.len() as f32;
        let s = px.iter().map(|&p| to_painting(p)).fold([0.0; 2], |a, b| [a[0] + b[0], a[1] + b[1]]);
        [s[0] / n, s[1] / n]
    }).collect();
    out.junctions = centres.iter().map(|&at| Junction { at }).collect();
    for e in &graph.edges {
        let mut pts: Vec<[f32; 2]> = e.pixels.iter().map(|&p| to_painting(p)).collect();
        for (k, end) in e.ends.iter().enumerate() {
            if let Node::Junction(j) = end {
                let i = if k == 0 { 0 } else { pts.len() - 1 };
                pts[i] = centres[*j];
            }
        }
        // Its half-width where it runs on its own (clear of the junctions it meets).
        let clear: Vec<f32> = e.pixels.iter().enumerate().filter(|&(i, _)| {
            e.ends.iter().enumerate().all(|(k, end)| match end {
                Node::Junction(j) => {
                    let at = if k == 0 { 0 } else { e.pixels.len() - 1 };
                    i.abs_diff(at) as f32 > graph.junction_radius(*j, &dt) + 1.0
                }
                _ => true,
            })
        }).map(|(_, &p)| dt[p]).collect();
        let mut widths = if clear.len() >= 2 { clear } else { e.pixels.iter().map(|&p| dt[p]).collect() };
        widths.sort_by(f32::total_cmp);
        // (A pixel's centre in the middle of a line 2r wide lies r from the empty pixels' if r is
        // whole, r + 0.5 if not.)
        let radius = (widths[widths.len() / 2] - 0.25).max(0.5);
        smooth(&mut pts, e.closed);
        // (A wide line's middle wanders a little more with its painted highlights.)
        let mut pts = simplify(&pts, (radius * 0.2).max(0.7));
        let free = [matches!(e.ends[0], Node::End(_)), matches!(e.ends[1], Node::End(_))];
        // A free end reaches the painted end: on along its way by what of the outline is left.
        for (k, &f) in free.iter().enumerate() {
            if !f || pts.len() < 2 {
                continue;
            }
            let (a, b) = if k == 0 { (pts[1], pts[0]) } else { (pts[pts.len() - 2], pts[pts.len() - 1]) };
            let d = dist(a, b).max(1e-3);
            let px = if k == 0 { e.pixels[0] } else { e.pixels[e.pixels.len() - 1] };
            let more = (dt[px] - 0.5).max(0.0);
            let end = [b[0] + (b[0] - a[0]) / d * more, b[1] + (b[1] - a[1]) / d * more];
            let i = if k == 0 { 0 } else { pts.len() - 1 };
            pts[i] = end;
        }
        if pts.len() >= 2 {
            out.strokes.push(Stroke { points: pts, radius, free });
        }
    }
    even_through_junctions(&mut out);
    out
}

/// A line crossed or branched by another is traced as several, the stretches between the
/// junctions, and a short one's half-width is taken where the other's paint widens it: those
/// that run on straight through a junction, near enough as wide as each other, are one line, of
/// one half-width (the one most of its length has).
fn even_through_junctions(s: &mut Strokes) {
    let n = s.strokes.len();
    let mut group: Vec<usize> = (0..n).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for j in &s.junctions {
        // The lines ending here, and the way each leaves it.
        let ends: Vec<(usize, [f32; 2])> = s.strokes.iter().enumerate().flat_map(|(i, k)| {
            let m = k.points.len();
            [(0, 1), (m - 1, m.saturating_sub(2))].into_iter().filter(move |&(e, _)| m >= 2 && !k.free[usize::from(e != 0)] && dist(k.points[e], j.at) < 0.01).map(move |(e, next)| {
                let d = dist(k.points[e], k.points[next]).max(1e-6);
                (i, [(k.points[next][0] - k.points[e][0]) / d, (k.points[next][1] - k.points[e][1]) / d])
            })
        }).collect();
        for (x, &(a, da)) in ends.iter().enumerate() {
            for &(b, db) in &ends[x + 1..] {
                let (ra, rb) = (s.strokes[a].radius, s.strokes[b].radius);
                if a != b && da[0] * db[0] + da[1] * db[1] < -0.9 && ra.max(rb) < ra.min(rb) * 1.6 {
                    let (ga, gb) = (root(&mut group, a), root(&mut group, b));
                    group[ga] = gb;
                }
            }
        }
    }
    let mut members: rustc_hash::FxHashMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let g = root(&mut group, i);
        members.entry(g).or_default().push(i);
    }
    for list in members.values().filter(|l| l.len() > 1) {
        let mut by: Vec<(f32, f32)> = list.iter().map(|&i| (s.strokes[i].radius, s.strokes[i].length())).collect();
        by.sort_by(|a, b| a.0.total_cmp(&b.0));
        let half = by.iter().map(|b| b.1).sum::<f32>() * 0.5;
        let mut run = 0.0;
        let radius = by.iter().find(|b| {
            run += b.1;
            run >= half
        }).map_or(by[0].0, |b| b.0);
        for &i in list {
            s.strokes[i].radius = radius;
        }
    }
}

/// A patch of a painting standing apart from the rest (a broken pillar, a block of stone, a
/// lump of rubble), in painting pixels from the anchor: its pixels' middle, the middle of its
/// lowest row (its foot, as TA's view shows something standing: nearest the eye), how far it
/// reaches across (x) and down the painting (y), the way it runs longest (a unit vector) and how
/// far along that way and across, and how many pixels it has.
#[derive(Clone, Copy, Debug)]
pub struct Blob {
    pub centre: [f32; 2],
    pub foot: [f32; 2],
    pub across: f32,
    pub down: f32,
    pub axis: [f32; 2],
    pub length: f32,
    pub breadth: f32,
    pub pixels: usize,
}

/// The patches of an outline (`mask`, `w` x `h`, row-major; pixels touching at a corner are one
/// patch) of at least `smallest` pixels, in painting pixels from `origin` (the anchor's pixel).
pub fn blobs(mask: &[bool], w: usize, h: usize, origin: [f32; 2], smallest: usize) -> Vec<Blob> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut members = Vec::new();
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if members.len() < smallest {
            continue;
        }
        let at = |i: usize| [(i % w) as f32 + 0.5 - origin[0], (i / w) as f32 + 0.5 - origin[1]];
        let n = members.len() as f32;
        let centre = members.iter().map(|&i| at(i)).fold([0.0; 2], |a, b| [a[0] + b[0] / n, a[1] + b[1] / n]);
        let (lo, hi) = members.iter().map(|&i| at(i)).fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
        let bottom: Vec<[f32; 2]> = members.iter().map(|&i| at(i)).filter(|p| p[1] >= hi[1] - 0.5).collect();
        let foot = [bottom.iter().map(|p| p[0]).sum::<f32>() / bottom.len() as f32, hi[1] + 0.5];
        let (mut sxx, mut sxy, mut syy) = (0.0f32, 0.0f32, 0.0f32);
        for &i in &members {
            let p = at(i);
            let (dx, dy) = (p[0] - centre[0], p[1] - centre[1]);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let axis = [theta.cos(), theta.sin()];
        let reach = |d: [f32; 2]| {
            let (mut a, mut b) = (f32::MAX, f32::MIN);
            for &i in &members {
                let p = at(i);
                let t = (p[0] - centre[0]) * d[0] + (p[1] - centre[1]) * d[1];
                a = a.min(t);
                b = b.max(t);
            }
            b - a + 1.0
        };
        out.push(Blob { centre, foot, across: hi[0] - lo[0] + 1.0, down: hi[1] - lo[1] + 1.0, axis, length: reach(axis), breadth: reach([-axis[1], axis[0]]), pixels: members.len() });
    }
    out
}

/// The distance (pixels) from each solid pixel's centre to the nearest empty one's (0 for the
/// empty ones): an exact Euclidean transform, a row and then a column at a time (Felzenszwalb and
/// Huttenlocher).
fn distance_transform(solid: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f64 = 1e18;
    let mut d: Vec<f64> = solid.iter().map(|&s| if s { FAR } else { 0.0 }).collect();
    let mut f = Vec::new();
    let mut out = Vec::new();
    for x in 0..w {
        f.clear();
        f.extend((0..h).map(|y| d[y * w + x]));
        squared_1d(&f, &mut out);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        f.clear();
        f.extend_from_slice(&d[y * w..(y + 1) * w]);
        squared_1d(&f, &mut out);
        d[y * w..(y + 1) * w].copy_from_slice(&out);
    }
    d.iter().map(|v| v.sqrt() as f32).collect()
}

/// The squared distance transform of one line of `f` (0 where empty, far where solid): the lower
/// envelope of the parabolas rooted at each pixel.
fn squared_1d(f: &[f64], out: &mut Vec<f64>) {
    let n = f.len();
    out.clear();
    out.resize(n, 0.0);
    if n == 0 {
        return;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let meet = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
    for q in 1..n {
        let mut s = meet(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = meet(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *o = (q as f64 - p as f64).powi(2) + f[p];
    }
}

/// The eight neighbours' offsets, clockwise from north (Zhang and Suen's P2 to P9).
fn ring(w: usize) -> [isize; 8] {
    let w = w as isize;
    [-w, -w + 1, 1, w + 1, w, w - 1, -1, -w - 1]
}

/// Thins solid pixels (with a border of empty ones) to a skeleton a pixel wide (Zhang and Suen).
fn thin(px: &mut [bool], w: usize, h: usize) {
    let r = ring(w);
    let mut marked = Vec::new();
    loop {
        let mut changed = false;
        for step in 0..2 {
            marked.clear();
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    let i = y * w + x;
                    if !px[i] {
                        continue;
                    }
                    let n: [bool; 8] = std::array::from_fn(|k| px[(i as isize + r[k]) as usize]);
                    let b = n.iter().filter(|&&v| v).count();
                    let a = (0..8).filter(|&k| !n[k] && n[(k + 1) % 8]).count();
                    let [p2, _, p4, _, p6, _, p8, _] = n;
                    let keep = if step == 0 { (p2 && p4 && p6) || (p4 && p6 && p8) } else { (p2 && p4 && p8) || (p2 && p6 && p8) };
                    if (2..=6).contains(&b) && a == 1 && !keep {
                        marked.push(i);
                    }
                }
            }
            for &i in &marked {
                px[i] = false;
            }
            changed |= !marked.is_empty();
        }
        if !changed {
            break;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Node {
    /// A pixel where a line ends.
    End(usize),
    /// A junction (its index).
    Junction(usize),
    /// None: a closed loop.
    Loop,
}

struct Edge {
    pixels: Vec<usize>,
    ends: [Node; 2],
    closed: bool,
}

/// A skeleton read as a graph: its junctions (each the pixels of one place where three or more
/// lines meet) and the lines between them and its ends.
struct Graph {
    junctions: Vec<Vec<usize>>,
    junction_pixel: Vec<bool>,
    edges: Vec<Edge>,
}

impl Graph {
    fn of(skel: &[bool], w: usize, h: usize) -> Graph {
        let r = ring(w);
        // Each pixel's neighbours on the skeleton, a diagonal step left out where the pixel beside
        // both is on it too (a staircase's corner: that way round is the line).
        let neighbours = |i: usize| -> Vec<usize> {
            let mut out = Vec::with_capacity(8);
            for k in 0..8 {
                let j = (i as isize + r[k]) as usize;
                if !skel[j] {
                    continue;
                }
                if k % 2 == 1 {
                    let (a, b) = ((i as isize + r[k - 1]) as usize, (i as isize + r[(k + 1) % 8]) as usize);
                    if skel[a] || skel[b] {
                        continue;
                    }
                }
                out.push(j);
            }
            out
        };
        let _ = h;
        let pixels: Vec<usize> = (0..skel.len()).filter(|&i| skel[i]).collect();
        let degree: rustc_hash::FxHashMap<usize, Vec<usize>> = pixels.iter().map(|&i| (i, neighbours(i))).collect();
        // Junction pixels, gathered into places (those touching each other).
        let mut junction_of: rustc_hash::FxHashMap<usize, usize> = Default::default();
        let mut junctions: Vec<Vec<usize>> = Vec::new();
        for &p in &pixels {
            if degree[&p].len() < 3 || junction_of.contains_key(&p) {
                continue;
            }
            let id = junctions.len();
            let mut place = vec![p];
            junction_of.insert(p, id);
            let mut k = 0;
            while k < place.len() {
                let q = place[k];
                for o in r {
                    let n = (q as isize + o) as usize;
                    if skel[n] && degree[&n].len() >= 3 && !junction_of.contains_key(&n) {
                        junction_of.insert(n, id);
                        place.push(n);
                    }
                }
                k += 1;
            }
            junctions.push(place);
        }
        let mut junction_pixel = vec![false; skel.len()];
        for place in &junctions {
            for &p in place {
                junction_pixel[p] = true;
            }
        }
        let node = |p: usize| -> Option<Node> {
            if let Some(&j) = junction_of.get(&p) {
                Some(Node::Junction(j))
            } else if degree[&p].len() <= 1 {
                Some(Node::End(p))
            } else {
                None
            }
        };
        // The lines: from each node out along each of its ways, through the pixels with two
        // neighbours, to the next node.
        let mut walked: rustc_hash::FxHashSet<(usize, usize)> = Default::default();
        let mut on_edge = vec![false; skel.len()];
        let mut edges = Vec::new();
        for &start in &pixels {
            let Some(from) = node(start) else { continue };
            for &first in &degree[&start] {
                // (Not within a junction's own pixels.)
                if node(first).is_some_and(|n| n == from && matches!(n, Node::Junction(_))) {
                    continue;
                }
                if walked.contains(&(start, first)) {
                    continue;
                }
                let mut path = vec![start, first];
                let (mut prev, mut cur) = (start, first);
                let end = loop {
                    if let Some(n) = node(cur) {
                        break n;
                    }
                    let next = degree[&cur].iter().copied().find(|&n| n != prev);
                    let Some(next) = next else { break Node::End(cur) };
                    if next == start {
                        path.push(next);
                        break from;
                    }
                    path.push(next);
                    (prev, cur) = (cur, next);
                    if path.len() > skel.len() {
                        break Node::End(cur);
                    }
                };
                walked.insert((start, first));
                let n = path.len();
                walked.insert((path[n - 1], path[n - 2]));
                for &p in &path {
                    on_edge[p] = true;
                }
                edges.push(Edge { pixels: path, ends: [from, end], closed: false });
            }
        }
        // Closed loops with no node on them.
        for &start in &pixels {
            if on_edge[start] || degree[&start].len() != 2 {
                continue;
            }
            let mut path = vec![start];
            let (mut prev, mut cur) = (start, degree[&start][0]);
            while cur != start && path.len() <= skel.len() {
                path.push(cur);
                let next = degree[&cur].iter().copied().find(|&n| n != prev);
                let Some(next) = next else { break };
                (prev, cur) = (cur, next);
            }
            path.push(start);
            for &p in &path {
                on_edge[p] = true;
            }
            edges.push(Edge { pixels: path, ends: [Node::Loop, Node::Loop], closed: true });
        }
        Graph { junctions, junction_pixel, edges }
    }

    /// A junction's half-width: the widest of its pixels'.
    fn junction_radius(&self, j: usize, dt: &[f32]) -> f32 {
        self.junctions[j].iter().map(|&p| dt[p]).fold(0.0, f32::max)
    }

    /// The middle of a junction's pixels (pixel coordinates of an image `w` wide).
    fn junction_centre(&self, j: usize, w: usize) -> [f32; 2] {
        let px = &self.junctions[j];
        let n = px.len() as f32;
        let s = px.iter().fold([0.0f32; 2], |a, &p| [a[0] + (p % w) as f32 + 0.5, a[1] + (p / w) as f32 + 0.5]);
        [s[0] / n, s[1] / n]
    }
}

/// Evens out a line's pixel steps (its ends kept).
fn smooth(pts: &mut [[f32; 2]], closed: bool) {
    let n = pts.len();
    if n < 3 {
        return;
    }
    for _ in 0..2 {
        let prev = pts.to_vec();
        for i in 1..n - 1 {
            pts[i] = [0.25 * prev[i - 1][0] + 0.5 * prev[i][0] + 0.25 * prev[i + 1][0], 0.25 * prev[i - 1][1] + 0.5 * prev[i][1] + 0.25 * prev[i + 1][1]];
        }
        if closed {
            // (A loop's first point is its last: evened out with the points either side.)
            let p = [0.25 * prev[n - 2][0] + 0.5 * prev[0][0] + 0.25 * prev[1][0], 0.25 * prev[n - 2][1] + 0.5 * prev[0][1] + 0.25 * prev[1][1]];
            pts[0] = p;
            pts[n - 1] = p;
        }
    }
}

/// The fewest of a line's points that keep it within `tolerance` (Douglas and Peucker).
fn simplify(pts: &[[f32; 2]], tolerance: f32) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (pts[a], pts[b]);
        let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let (mut far, mut at) = (0.0f32, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len < 1e-6 { dist(*p, pa) } else { ((p[0] - pa[0]) * dy - (p[1] - pa[1]) * dx).abs() / len };
            if d > far {
                (far, at) = (d, i);
            }
        }
        if far > tolerance {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(w: usize, h: usize, inside: impl Fn(f32, f32) -> bool) -> Vec<bool> {
        (0..w * h).map(|i| inside((i % w) as f32 + 0.5, (i / w) as f32 + 0.5)).collect()
    }

    /// A bar is one stroke along its middle, from end to end, half as wide as it is.
    #[test]
    fn a_bar_is_one_stroke() {
        let m = mask(60, 20, |x, y| (5.0..55.0).contains(&x) && (6.0..14.0).contains(&y));
        let s = trace(&m, 60, 20, [0.0, 0.0]);
        assert_eq!(s.strokes.len(), 1, "{s:?}");
        let k = &s.strokes[0];
        assert!(k.free == [true, true]);
        assert!((k.radius - 4.0).abs() < 0.8, "radius {}", k.radius);
        assert!((k.length() - 50.0).abs() < 4.0, "length {}", k.length());
        assert!(k.points.iter().all(|p| (p[1] - 10.0).abs() < 1.0), "{:?}", k.points);
    }

    /// A T is three strokes meeting at one junction.
    #[test]
    fn a_tee_meets_at_a_junction() {
        let m = mask(60, 60, |x, y| ((5.0..55.0).contains(&x) && (10.0..18.0).contains(&y)) || ((26.0..34.0).contains(&x) && (10.0..55.0).contains(&y)));
        let s = trace(&m, 60, 60, [0.0, 0.0]);
        assert_eq!(s.junctions.len(), 1, "{s:?}");
        assert_eq!(s.strokes.len(), 3, "{s:?}");
        let j = s.junctions[0].at;
        assert!((j[0] - 30.0).abs() < 2.5 && (j[1] - 14.0).abs() < 4.0, "{j:?}");
        assert!(s.strokes.iter().all(|k| k.free.iter().filter(|&&f| f).count() == 1));
    }

    /// A line crossed by another is one width where it is crossed as elsewhere.
    #[test]
    fn a_crossed_line_keeps_its_width() {
        let m = mask(80, 60, |x, y| ((5.0..75.0).contains(&x) && (24.0..36.0).contains(&y)) || ((36.0..42.0).contains(&x) && (5.0..55.0).contains(&y)));
        let s = trace(&m, 80, 60, [0.0, 0.0]);
        let wide: Vec<f32> = s.strokes.iter().filter(|k| (k.points[0][1] - k.points[k.points.len() - 1][1]).abs() < 3.0).map(|k| k.radius).collect();
        assert!(wide.len() >= 2 && wide.iter().all(|&r| (r - wide[0]).abs() < 1e-6), "{s:?}");
    }

    /// A bend is one stroke turning the corner.
    #[test]
    fn an_elbow_is_one_stroke() {
        let m = mask(60, 60, |x, y| ((5.0..35.0).contains(&x) && (10.0..18.0).contains(&y)) || ((27.0..35.0).contains(&x) && (10.0..55.0).contains(&y)));
        let s = trace(&m, 60, 60, [0.0, 0.0]);
        assert_eq!(s.strokes.len(), 1, "{s:?}");
        assert!(s.strokes[0].points.len() >= 3);
    }

    /// Patches apart are blobs of their own; specks are left out.
    #[test]
    fn patches_are_blobs() {
        let m = mask(40, 30, |x, y| ((4.0..10.0).contains(&x) && (5.0..25.0).contains(&y)) || ((20.0..36.0).contains(&x) && (10.0..14.0).contains(&y)) || (x > 38.0 && y < 2.0));
        let b = blobs(&m, 40, 30, [0.0, 0.0], 5);
        assert_eq!(b.len(), 2, "{b:?}");
        let pillar = b.iter().find(|b| b.down > b.across).unwrap();
        assert!((pillar.foot[1] - 25.0).abs() < 0.01 && (pillar.foot[0] - 7.0).abs() < 0.01, "{pillar:?}");
        let bar = b.iter().find(|b| b.across > b.down).unwrap();
        assert!((bar.length - 16.0).abs() < 0.5 && (bar.breadth - 4.0).abs() < 0.5 && bar.axis[0].abs() > 0.99, "{bar:?}");
    }

    /// The exact distance transform.
    #[test]
    fn distances_are_euclidean() {
        let m = mask(9, 9, |x, y| x > 2.0 && y > 2.0 && x < 7.0 && y < 7.0);
        let d = distance_transform(&m, 9, 9);
        assert_eq!(d[4 * 9 + 4], 3.0);
        assert_eq!(d[10], 0.0);
        assert!((d[2 * 9 + 2] - 1.0).abs() < 1e-6);
    }
}
