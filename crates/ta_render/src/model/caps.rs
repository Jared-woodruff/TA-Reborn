//! Holes closed. TA's models were only ever seen from TA's fixed camera, high over the
//! battlefield, so many of their pieces lack faces that never faced it: undersides, the bottoms
//! of turrets, a track pod's inner side. Faces are drawn from the front alone (see
//! `ModelRenderer::new`), so in the 3D camera a missing face is a hole: through it the far faces'
//! insides are culled too, and one sees straight through the unit.
//!
//! When a model loads, each piece's faces are welded (TA's models repeat corners rather than
//! share them, and a face's edge may run past the corners of several of its neighbours'), the
//! edges only one face uses are gathered into loops, and each loop that is a hole in a shell is
//! closed with a cap, wound as the faces round it are (an edge two faces share runs one way in
//! one and the other way in the other). A loop round two or three missing sides of a box is bent:
//! it is cut into flat parts along the box's edges, each capped; an overhang's missing underside
//! is spanned by the least surface. A frame's missing underside (a loop round another) is closed
//! round the frame's middle. Faces wound against all their neighbours (culled from outside, seen
//! only from within) are turned round first, where TA's camera saw neither side of them (a
//! hull's underside wound into the hull): one it saw looked as its artist meant.
//!
//! The halves of barrels and booms TA built open underneath are made whole first (halves.rs),
//! so that what is left of them to close is a muzzle's end.
//!
//! A loop round a shallow shell open underneath (a thin hull, a turret's base, a walker's foot)
//! has its faces as near its plane as a sheet's outline would, but they stand up from it for most
//! of its length: it is closed as a rim, the shell's missing underside. (Not with caps falling on
//! faces of its piece, as a fin folded along its length cut at its fold would give: such a fin is
//! closed under it and across its ends, a thin wedge.)
//!
//! Left open: the outline of a lone face or a flat sheet (a fin, a ground plate, a flame), a hole
//! TA's camera looked into from above (the doorway a turret rises out of, a shipyard's basin), a
//! hole lying on another face (a turret's sides standing on its hull) or behind another piece's
//! (a doorway behind its doors), and a loop so bent or tangled that a cap would cut through the
//! model, hide its faces or face into it. Like the kit, it reads nothing but the faces, so mods'
//! models are mended too. A cap is drawn as the model's own faces are, in both art styles, in
//! dark metal (see `CAP_METAL` in model.rs); the kit leaves caps as they are.

use glam::{Vec2, Vec3};

/// Corners nearer than this are one (model units).
pub(super) const WELD: f32 = 0.05;
/// A corner this near a face's edge, between its ends, lies on it (a T-junction).
const ON_EDGE: f32 = 0.08;
/// A loop whose corners all lie within this of its plane (model units, or this share of its
/// size) is flat: one cap closes it.
const FLAT: f32 = 0.2;
const FLAT_SHARE: f32 = 0.06;
/// A face within this of a cap's plane, facing along it, lies on it.
const COVER: f32 = 0.4;
/// The most corners a loop may have and be capped, and be cut into flat parts.
const MOST_CORNERS: usize = 256;
const MOST_SPLIT: usize = 24;
/// A part of a hole facing up more than this was in sight of TA's camera from above: a doorway
/// a turret rises out of, a basin, an opening TA showed. It stays as it is.
pub(super) const SHOWN: f32 = 0.45;
/// A loop narrower than this on average (model units) is a crack between faces that nearly meet.
const CRACK: f32 = 0.3;

/// A face of a piece as `ModelLibrary::add` has it: its corners (piece space, z flipped) in TA's
/// order, clockwise seen from outside.
pub(super) type Polygon = Vec<Vec3>;

/// A piece of the model to mend: its faces and its place at rest.
pub(super) struct PieceFaces<'a> {
    pub faces: &'a [Polygon],
    pub offset: Vec3,
    /// Whether it may be mended at all (not an effect: a flash, a fire point).
    pub mend: bool,
}

/// What became of an open loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Verdict {
    /// Closed with caps (but for any parts left open for one of the next three reasons).
    Capped,
    /// TA's camera looked into it from above.
    Shown,
    /// It lies on another face of its piece (a turret's sides standing on a hull's top).
    Seated,
    /// A face of another piece lies over it, facing the same way (doors over a doorway).
    Behind,
    /// The outline of a lone face or a flat sheet (a fin, a plate, a flame).
    Sheet,
    /// The rim or the inside of a frame: two loops, one in the other, facing apart.
    Ring,
    /// Too bent for caps.
    Bent,
    /// Its outline crosses itself, seen square on.
    Tangled,
    /// Its caps would face into the model: the faces round it fold towards them.
    Inward,
    /// Its caps would hide faces of the model: looking in from one, the first face met faces out.
    Hidden,
    /// Its caps would cross the piece's faces.
    Crossing,
    /// Too slight to have a side, or too big to be a hole.
    Unsure,
}

/// A loop of open edges and what became of it.
#[derive(Clone, Debug)]
pub(super) struct Hole {
    pub verdict: Verdict,
    /// Its corners (piece space), in the order a cap's would be: clockwise seen from outside.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the scan of the installed models"))]
    pub corners: Vec<Vec3>,
    /// How many caps close it (more than one where it was cut into flat parts).
    #[cfg_attr(not(test), expect(dead_code, reason = "for the scan of the installed models"))]
    pub caps: usize,
    /// Another piece's face lies over it facing the other way (a turret's bottom on its hull).
    #[cfg_attr(not(test), expect(dead_code, reason = "for the scan of the installed models"))]
    pub on_other: bool,
    /// The faces round it.
    pub faces: Vec<u32>,
}

/// A cap: triangles of a hole's corners, clockwise seen from outside as the model's faces are.
#[derive(Clone, Debug)]
pub(super) struct Cap {
    pub corners: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Outwards.
    pub normal: Vec3,
    /// Flat and convex: one face of its corners, in order, as TA's are.
    pub convex: bool,
    /// The faces round it (the piece's, by their order).
    pub faces: Vec<u32>,
    /// Closing a shallow shell's rim (see `Piece::close`).
    pub rim: bool,
    /// Added since TA's look was kept as it was (a joint's opening under another piece, see
    /// `judge`; a cap's back, see `give_backs` in halves.rs): drawn after TA's own faces, which
    /// are shaded and fitted as if it weren't there, as the forms made whole are.
    pub late: bool,
}

/// A piece's mending: its faces to turn round, its holes and their caps.
#[derive(Clone, Debug, Default)]
pub(super) struct Mending {
    pub flipped: Vec<usize>,
    pub holes: Vec<Hole>,
    pub caps: Vec<Cap>,
    /// Edges two faces share running the same way, before any face was turned round (for the
    /// scan of the installed models).
    pub conflicts: usize,
    /// Open edges that make no loop (for the scan).
    pub broken: usize,
}

/// A piece's faces with their corners welded.
pub(super) struct Mesh {
    pub points: Vec<Vec3>,
    /// Each face's ring of points: its own corners and the corners of other faces lying on its
    /// edges, in its order. Empty for a face with less than three corners of its own.
    pub rings: Vec<Vec<u32>>,
}

/// The faces' corners as points, each face a ring of them (a face folded onto itself, an empty
/// one).
pub(super) fn weld(faces: &[Polygon]) -> Mesh {
    // Corners at one place are one point (TA's models repeat corners rather than share them)...
    let mut exact: rustc_hash::FxHashMap<[u32; 3], u32> = Default::default();
    let mut points: Vec<Vec3> = Vec::new();
    let id_of: Vec<u32> = faces
        .iter()
        .flatten()
        .map(|p| {
            *exact.entry(p.to_array().map(f32::to_bits)).or_insert_with(|| {
                points.push(*p);
                points.len() as u32 - 1
            })
        })
        .collect();
    // ...and so are points nearer than the weld (by x, those within it of each other).
    let mut parent: Vec<u32> = (0..points.len() as u32).collect();
    fn root(parent: &mut [u32], mut k: u32) -> u32 {
        while parent[k as usize] != k {
            parent[k as usize] = parent[parent[k as usize] as usize];
            k = parent[k as usize];
        }
        k
    }
    let mut by_x: Vec<u32> = (0..points.len() as u32).collect();
    by_x.sort_unstable_by(|&a, &b| points[a as usize].x.total_cmp(&points[b as usize].x));
    for (i, &a) in by_x.iter().enumerate() {
        let pa = points[a as usize];
        for &b in by_x[i + 1..].iter().take_while(|&&b| points[b as usize].x - pa.x <= WELD) {
            if points[b as usize].distance_squared(pa) <= WELD * WELD {
                let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
    }
    let mut id = vec![u32::MAX; points.len()];
    let mut welded = Vec::with_capacity(points.len());
    for k in 0..points.len() as u32 {
        let r = root(&mut parent, k) as usize;
        if id[r] == u32::MAX {
            id[r] = welded.len() as u32;
            welded.push(points[r]);
        }
        id[k as usize] = id[r];
    }
    let points = welded;
    let mut rings: Vec<Vec<u32>> = Vec::with_capacity(faces.len());
    let mut corner = 0;
    for face in faces {
        let mut ring: Vec<u32> = Vec::with_capacity(face.len());
        for _ in face {
            let id = id[id_of[corner] as usize];
            corner += 1;
            if ring.last() != Some(&id) {
                ring.push(id);
            }
        }
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 || (1..ring.len()).any(|k| ring[..k].contains(&ring[k])) {
            ring.clear();
        }
        rings.push(ring);
    }
    Mesh { points, rings }
}

/// Puts into its ring the corners of other faces lying on an edge no other face runs, between
/// its ends: an edge running past the corners of the faces along it. Such corners end edges no
/// other face runs too. Whether any ring changed.
pub(super) fn split_edges(mesh: &mut Mesh, edges: &[((u32, u32), u32, bool)]) -> bool {
    let open: Vec<((u32, u32), u32, bool)> = groups(edges).filter(|g| g.len() == 1).map(|g| g[0]).collect();
    if open.is_empty() {
        return false;
    }
    let points = &mesh.points;
    let mut ends: Vec<u32> = open.iter().flat_map(|&((a, b), ..)| [a, b]).collect();
    ends.sort_unstable();
    ends.dedup();
    // (By x: the ones near an edge lie in the slab its ends span.)
    ends.sort_by(|&a, &b| points[a as usize].x.total_cmp(&points[b as usize].x));
    let xs: Vec<f32> = ends.iter().map(|&v| points[v as usize].x).collect();
    // (Face, the edge as it runs it, the points on it in order.)
    let mut splits: Vec<(u32, u32, u32, Vec<u32>)> = Vec::new();
    for &((lo_id, hi_id), f, forward) in &open {
        let (a, b) = if forward { (lo_id, hi_id) } else { (hi_id, lo_id) };
        let (pa, pb) = (points[a as usize], points[b as usize]);
        let ab = pb - pa;
        let len2 = ab.length_squared();
        if len2 < 1e-8 {
            continue;
        }
        let (lo, hi) = (pa.min(pb) - Vec3::splat(ON_EDGE), pa.max(pb) + Vec3::splat(ON_EDGE));
        let margin = ON_EDGE / len2.sqrt();
        let (first, last) = (xs.partition_point(|&x| x < lo.x), xs.partition_point(|&x| x <= hi.x));
        let ring = &mesh.rings[f as usize];
        let mut between: Vec<(f32, u32)> = Vec::new();
        for &v in &ends[first..last] {
            let p = points[v as usize];
            if v == a || v == b || p.cmplt(lo).any() || p.cmpgt(hi).any() {
                continue;
            }
            let t = (p - pa).dot(ab) / len2;
            if t > margin && t < 1.0 - margin && (pa + ab * t).distance_squared(p) <= ON_EDGE * ON_EDGE && !ring.contains(&v) {
                between.push((t, v));
            }
        }
        if !between.is_empty() {
            between.sort_by(|x, y| x.0.total_cmp(&y.0));
            splits.push((f, a, b, between.into_iter().map(|(_, v)| v).collect()));
        }
    }
    for (f, a, b, on) in &splits {
        let ring = &mut mesh.rings[*f as usize];
        let n = ring.len();
        if let Some(k) = (0..n).find(|&k| ring[k] == *a && ring[(k + 1) % n] == *b) {
            for (j, &v) in on.iter().enumerate() {
                ring.insert(k + 1 + j, v);
            }
        }
    }
    !splits.is_empty()
}

/// Every edge of every face (between consecutive points of its ring): its points, lower first,
/// the face and whether the face runs it from the lower to the higher. Sorted by edge.
pub(super) fn edge_uses(mesh: &Mesh) -> Vec<((u32, u32), u32, bool)> {
    let mut edges = Vec::new();
    for (f, ring) in mesh.rings.iter().enumerate() {
        let n = ring.len();
        for k in 0..n {
            let (a, b) = (ring[k], ring[(k + 1) % n]);
            if a != b {
                edges.push(((a.min(b), a.max(b)), f as u32, a < b));
            }
        }
    }
    edges.sort_unstable();
    edges
}

/// The open edge (an index into `open`: start, end, face, as the face runs it) that follows open
/// edge `incoming` round its hole: round the point it ends at, face by face from its face across
/// the edges they share, to the first face whose edge going out from the point is open. None
/// where the faces round the point are not a simple fan (an edge three faces share, or two
/// running it the same way).
fn fan_next(mesh: &Mesh, edges: &[((u32, u32), u32, bool)], open: &[(u32, u32, u32)], incoming: usize) -> Option<usize> {
    let (_, v, mut face) = open[incoming];
    for _ in 0..64 {
        let ring = &mesh.rings[face as usize];
        let n = ring.len();
        let at = ring.iter().position(|&p| p == v)?;
        let w = ring[(at + 1) % n];
        let key = (v.min(w), v.max(w));
        let first = edges.partition_point(|e| e.0 < key);
        let uses = &edges[first..edges[first..].iter().position(|e| e.0 != key).map_or(edges.len(), |k| first + k)];
        match uses {
            [_] => return open.iter().position(|&(a, b, f)| a == v && b == w && f == face),
            [x, y] if x.2 != y.2 => face = if x.1 == face { y.1 } else { x.1 },
            _ => return None,
        }
    }
    None
}

/// Runs of `edges` sharing one edge.
pub(super) fn groups(edges: &[((u32, u32), u32, bool)]) -> impl Iterator<Item = &[((u32, u32), u32, bool)]> {
    edges.chunk_by(|a, b| a.0 == b.0)
}

/// Newell's term for the edge from `a` to `p`: summed round a ring of points, the ring's normal
/// as long as twice its area, pointing as the right-hand rule round it does.
fn newell_term(a: Vec3, p: Vec3) -> Vec3 {
    Vec3::new((a.y - p.y) * (a.z + p.z), (a.z - p.z) * (a.x + p.x), (a.x - p.x) * (a.y + p.y))
}

/// A ring of points wound as the model's faces (clockwise seen from outside): its outward
/// normal, its area, its middle and how far its points lie from its plane at most.
pub(super) fn plane(points: &[Vec3]) -> (Vec3, f32, Vec3, f32) {
    plane_of(points.iter().copied())
}

/// [`plane`] of the points an iterator runs through.
fn plane_of(points: impl Iterator<Item = Vec3> + Clone) -> (Vec3, f32, Vec3, f32) {
    let (mut twice, mut sum, mut count) = (Vec3::ZERO, Vec3::ZERO, 0);
    let (mut first, mut last) = (None, None);
    for p in points.clone() {
        if let Some(a) = last {
            twice -= newell_term(a, p);
        }
        first.get_or_insert(p);
        last = Some(p);
        sum += p;
        count += 1;
    }
    if let (Some(a), Some(p)) = (last, first) {
        twice -= newell_term(a, p);
    }
    let normal = twice.normalize_or_zero();
    let centre = sum / count.max(1) as f32;
    let bend = points.map(|p| (p - centre).dot(normal).abs()).fold(0.0, f32::max);
    (normal, twice.length() * 0.5, centre, bend)
}

/// How far from flat a loop of this area may be and still be capped flat.
fn tolerance(area: f32) -> f32 {
    FLAT.max(FLAT_SHARE * area.sqrt())
}

/// Faces wound against every neighbour they share an edge with (two or more of them), whose
/// neighbours agree with all of theirs: turned round, they face out as the faces round them do.
fn wrongly_wound(mesh: &Mesh, edges: &[((u32, u32), u32, bool)]) -> (Vec<usize>, usize) {
    let n = mesh.rings.len();
    let (mut agree, mut against) = (vec![0u32; n], vec![Vec::<u32>::new(); n]);
    let mut conflicts = 0;
    for g in groups(edges) {
        if g.len() != 2 || g[0].1 == g[1].1 {
            continue;
        }
        let (f, h) = (g[0].1 as usize, g[1].1 as usize);
        if g[0].2 != g[1].2 {
            agree[f] += 1;
            agree[h] += 1;
        } else {
            conflicts += 1;
            against[f].push(h as u32);
            against[h].push(f as u32);
        }
    }
    let flipped = (0..n)
        .filter(|&f| agree[f] == 0 && against[f].len() >= 2)
        .filter(|&f| against[f].iter().all(|&h| agree[h as usize] >= 1 && against[h as usize].iter().all(|&x| x as usize == f)))
        .collect();
    (flipped, conflicts)
}

/// A face of the model at rest, flattened: its plane, its outline in it and its extent, for the
/// faces lying on a cap.
pub(super) struct Flat {
    pub normal: Vec3,
    origin: Vec3,
    u: Vec3,
    v: Vec3,
    outline: Vec<Vec2>,
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Flat {
    /// The face of these corners moved by `offset`.
    pub(super) fn new(corners: &[Vec3], offset: Vec3) -> Option<Flat> {
        if corners.len() < 3 {
            return None;
        }
        let (normal, area, _, _) = plane(corners);
        if area < 1e-4 {
            return None;
        }
        let origin = corners[0] + offset;
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        let outline = corners.iter().map(|&p| Vec2::new((p + offset - origin).dot(u), (p + offset - origin).dot(v))).collect();
        let lo = corners.iter().fold(Vec3::MAX, |m, &p| m.min(p)) + offset;
        let hi = corners.iter().fold(Vec3::MIN, |m, &p| m.max(p)) + offset;
        Some(Flat { normal, origin, u, v, outline, lo, hi })
    }

    /// Whether `p` lies on it: within `gap` of its plane, inside its outline.
    pub(super) fn holds(&self, p: Vec3, gap: f32) -> bool {
        if p.cmplt(self.lo - Vec3::splat(gap)).any() || p.cmpgt(self.hi + Vec3::splat(gap)).any() {
            return false;
        }
        let d = p - self.origin;
        if d.dot(self.normal).abs() > gap {
            return false;
        }
        let q = Vec2::new(d.dot(self.u), d.dot(self.v));
        let n = self.outline.len();
        let mut inside = false;
        for k in 0..n {
            let (a, b) = (self.outline[k], self.outline[(k + 1) % n]);
            if (a.y > q.y) != (b.y > q.y) && a.x + (b.x - a.x) * (q.y - a.y) / (b.y - a.y) > q.x {
                inside = !inside;
            }
        }
        inside
    }
}

/// Ear clipping of a simple polygon (anticlockwise): triangles of its corners, or None if it
/// isn't simple.
fn ear_clip(points: &[Vec2]) -> Option<Vec<[u32; 3]>> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    // Simple: no two edges that don't meet cross.
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (c, d) = (points[j], points[(j + 1) % n]);
            let (d1, d2, d3, d4) = (cross(a, b, c), cross(a, b, d), cross(c, d, a), cross(c, d, b));
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                return None;
            }
        }
    }
    let mut out = Vec::with_capacity(n - 2);
    // Convex: a fan (leaving out its slivers, where corners lie in line).
    if (0..n).all(|k| cross(points[k], points[(k + 1) % n], points[(k + 2) % n]) >= -1e-9) {
        out.extend((1..n as u32 - 1).filter(|&k| cross(points[0], points[k as usize], points[k as usize + 1]) > 1e-9).map(|k| [0, k, k + 1]));
        return (!out.is_empty()).then_some(out);
    }
    let mut left: Vec<u32> = (0..n as u32).collect();
    while left.len() > 3 {
        let m = left.len();
        // The ear with the best-shaped triangle (the largest smallest angle); in a long outline,
        // the first ear.
        let mut best: Option<(f32, usize)> = None;
        for k in 0..m {
            if best.is_some() && m > 16 {
                break;
            }
            let (ia, ib, ic) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            let (a, b, c) = (points[ia as usize], points[ib as usize], points[ic as usize]);
            if cross(a, b, c) <= 1e-9 {
                continue;
            }
            let blocked = left.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = points[j as usize];
                if p.distance_squared(a) < 1e-10 || p.distance_squared(b) < 1e-10 || p.distance_squared(c) < 1e-10 {
                    return false;
                }
                cross(a, b, p) >= -1e-9 && cross(b, c, p) >= -1e-9 && cross(c, a, p) >= -1e-9
            });
            if blocked {
                continue;
            }
            let angle = |o: Vec2, x: Vec2, y: Vec2| (x - o).normalize_or_zero().dot((y - o).normalize_or_zero());
            let worst = angle(a, b, c).max(angle(b, c, a)).max(angle(c, a, b));
            if best.is_none_or(|(w, _)| worst < w) {
                best = Some((worst, k));
            }
        }
        let (_, k) = best?;
        out.push([left[(k + m - 1) % m], left[k], left[(k + 1) % m]]);
        left.remove(k);
    }
    let (a, b, c) = (points[left[0] as usize], points[left[1] as usize], points[left[2] as usize]);
    if cross(a, b, c) > 1e-9 {
        out.push([left[0], left[1], left[2]]);
    }
    (!out.is_empty()).then_some(out)
}

/// A flat (or nearly flat) ring of points, wound as the model's faces, as triangles wound so too.
fn triangulate(points: &[Vec3], normal: Vec3) -> Option<Vec<[u32; 3]>> {
    let n = points.len();
    // Along its longest edge.
    let k = (0..n).max_by(|&a, &b| points[a].distance_squared(points[(a + 1) % n]).total_cmp(&points[b].distance_squared(points[(b + 1) % n])))?;
    let e = points[(k + 1) % n] - points[k];
    let u = (e - normal * e.dot(normal)).normalize_or(normal.any_orthonormal_vector());
    let v = normal.cross(u);
    // Seen from outside the corners run clockwise: with v turned over they run anticlockwise,
    // as ear clipping takes them, and its triangles come out clockwise seen from outside.
    let flat: Vec<Vec2> = points.iter().map(|&p| Vec2::new(p.dot(u), -p.dot(v))).collect();
    ear_clip(&flat)
}

/// Whether segment `a`-`b` passes through triangle `t` (not merely touching its edges or corners).
pub(super) fn pierces(a: Vec3, b: Vec3, t: &[Vec3; 3]) -> bool {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let len = n.length();
    if len < 1e-6 {
        return false;
    }
    let n = n / len;
    let (da, db) = ((a - t[0]).dot(n), (b - t[0]).dot(n));
    const E: f32 = 0.01;
    if (da > -E && db > -E) || (da < E && db < E) {
        return false;
    }
    let p = a + (b - a) * (da / (da - db));
    // Inside, by a margin from every edge.
    (0..3).all(|k| {
        let (u, v) = (t[k], t[(k + 1) % 3]);
        let inward = n.cross(v - u).normalize_or_zero();
        (p - u).dot(inward) > 0.02
    })
}

/// The loop (as indices into `corners`, wound as a cap) cut into flat parts along diagonals, the
/// biggest flat part first: two or three missing sides of a box make one bent loop, the box's
/// edges between them the cuts. No part lies on one of the faces round the loop (`round`, each
/// a point on it and its normal): a sleeve open below and at its ends has every corner on its
/// loop, and its sides and top are flat parts of it, which capped would give its faces back.
fn flat_parts(corners: &[Vec3], ids: Vec<usize>, depth: u32, round: &[(Vec3, Vec3)], out: &mut Vec<Vec<usize>>) -> bool {
    let points: Vec<Vec3> = ids.iter().map(|&i| corners[i]).collect();
    let (normal, area, _, bend) = plane(&points);
    if bend <= tolerance(area) {
        out.push(ids);
        return true;
    }
    let n = ids.len();
    if depth == 0 || !(4..=MOST_SPLIT).contains(&n) {
        return false;
    }
    let mut best: Option<(f32, usize, usize)> = None;
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let a = points[i..=j].iter().copied();
            let b = points[j..].iter().chain(&points[..=i]).copied();
            let (a, b): (Vec<Vec3>, Vec<Vec3>) = (a.collect(), b.collect());
            for (len, part) in [(j - i + 1, &a), (n - j + i + 1, &b)] {
                let (pn, pa, _, pb) = plane(part);
                // Flat, a fair share of the hole, facing out of it as the hole does, and on none
                // of the faces round it.
                let on_face = || round.iter().any(|&(at, fnormal)| fnormal.dot(pn).abs() > 0.98 && part.iter().all(|p| (*p - at).dot(fnormal).abs() <= tolerance(pa)));
                if pb <= tolerance(pa) && (len >= 4 || pa >= 0.2 * area) && pn.dot(normal) > -0.2 && best.is_none_or(|(x, ..)| pa > x) && !on_face() {
                    best = Some((pa, i, j));
                }
            }
        }
    }
    let Some((_, i, j)) = best else { return false };
    let a: Vec<usize> = ids[i..=j].to_vec();
    let b: Vec<usize> = ids[j..].iter().chain(&ids[..=i]).copied().collect();
    flat_parts(corners, a, depth - 1, round, out) && flat_parts(corners, b, depth - 1, round, out)
}

/// A loop of open edges: its points wound as a cap would be, and for each of its edges (from a
/// point to the next) the face running it.
struct Loop {
    ids: Vec<u32>,
    faces: Vec<u32>,
}

/// What a piece's loops are judged with.
struct Piece<'a> {
    index: usize,
    pieces: &'a [PieceFaces<'a>],
    flats: &'a [Vec<Option<Flat>>],
    mesh: &'a Mesh,
    /// Each face's component (faces joined by their edges).
    component: Vec<u32>,
    centres: Vec<Vec3>,
    /// Each face's outward normal.
    normals: Vec<Vec3>,
    /// The piece's faces as triangles, each with its face (for rays, and crossings).
    hits: Vec<([Vec3; 3], usize)>,
    /// Whether shallow shells' rims are closed (see `close`), and openings facing up that TA's
    /// camera never saw into (see `judge`).
    rims: bool,
    /// The whole model at rest (model space), built when a hole facing up first asks.
    model: std::cell::OnceCell<Vec<([Vec3; 3], usize)>>,
}

impl Piece<'_> {
    /// Closes a loop (with `holes` in it: the insides of a frame whose underside it is): its
    /// verdict, its caps and whether another piece's face lies over it facing the other way.
    fn close(&self, lp: &Loop, holes: &[&Loop]) -> (Verdict, Vec<Cap>, bool) {
        let corners: Vec<Vec3> = lp.ids.iter().map(|&i| self.mesh.points[i as usize]).collect();
        let m = corners.len();
        let (normal, area, centre, bend) = plane(&corners);
        let perimeter: f32 = (0..m).map(|k| corners[k].distance(corners[(k + 1) % m])).sum();
        if !(3..=MOST_CORNERS).contains(&m) || normal == Vec3::ZERO || area < 1e-3 {
            return (Verdict::Unsure, Vec::new(), false);
        }
        let crack = holes.is_empty() && 4.0 * area / perimeter < CRACK;
        // (A shallow shell's rim, not a sheet's outline: see below.)
        let mut rim = false;
        if !crack {
            // The faces round it all in its plane: the outline of a sheet.
            let mut touching: Vec<u32> = lp.faces.iter().map(|&f| self.component[f as usize]).collect();
            touching.sort_unstable();
            touching.dedup();
            let deep = (0..self.mesh.rings.len())
                .filter(|&f| touching.binary_search(&self.component[f]).is_ok())
                .flat_map(|f| self.mesh.rings[f].iter())
                .any(|&i| (self.mesh.points[i as usize] - centre).dot(normal).abs() > tolerance(area) + bend);
            // Unless the faces along it stand up from its plane for most of its length, and it
            // faces down: the rim of a shallow shell open underneath (a hull, a turret's base, a
            // walker's foot), within its own bend of being flat, not a sheet's outline. (Its caps
            // are judged as any, and none may fall on faces of its piece: see `judge`.)
            let steep: f32 = (0..m).filter(|&k| self.normals[lp.faces[k] as usize].dot(normal).abs() < 0.5).map(|k| corners[k].distance(corners[(k + 1) % m])).sum();
            rim = self.rims && !deep && normal.y < -SHOWN && steep > 0.5 * perimeter;
            if !deep && !rim {
                return (Verdict::Sheet, Vec::new(), false);
            }
        }
        // The ways to close it, the likeliest first, each patches of triangles: flat, the loop
        // capped as it is; bent, the flat sides of a box it runs round, else the least surface
        // spanning it; a frame's underside with the frame's insides bridged in.
        let whole = |triangles: Vec<[u32; 3]>, flat: bool| Patch { points: corners.clone(), faces: lp.faces.iter().map(|&f| Some(f)).collect(), triangles, flat };
        let mut ways: Vec<Vec<Patch>> = Vec::new();
        if !holes.is_empty() {
            let flat = |l: &Loop| l.ids.iter().all(|&i| (self.mesh.points[i as usize] - centre).dot(normal).abs() <= tolerance(area));
            if !flat(lp) || !holes.iter().all(|h| flat(h)) {
                return (Verdict::Ring, Vec::new(), false);
            }
            let Some((points, faces)) = bridged(&self.mesh.points, lp, holes, normal) else {
                return (Verdict::Ring, Vec::new(), false);
            };
            let Some(triangles) = triangulate(&points, normal) else {
                return (Verdict::Ring, Vec::new(), false);
            };
            ways.push(vec![Patch { points, faces, triangles, flat: true }]);
        } else if crack || bend <= tolerance(area) {
            if let Some(triangles) = triangulate(&corners, normal) {
                ways.push(vec![whole(triangles, !crack)]);
            }
        } else {
            let mut cut = Vec::new();
            let mut round: Vec<(Vec3, Vec3)> = Vec::new();
            for &f in &lp.faces {
                let Some(&first) = self.mesh.rings[f as usize].first() else { continue };
                let at = self.mesh.points[first as usize];
                if !round.iter().any(|&(p, n)| n.dot(self.normals[f as usize]) > 0.999 && (at - p).dot(n).abs() < 1e-3) {
                    round.push((at, self.normals[f as usize]));
                }
            }
            if flat_parts(&corners, (0..m).collect(), 4, &round, &mut cut) {
                let patches: Option<Vec<Patch>> = cut
                    .into_iter()
                    .map(|part| {
                        let n = part.len();
                        let points: Vec<Vec3> = part.iter().map(|&i| corners[i]).collect();
                        let faces = (0..n).map(|k| (part[(k + 1) % n] == (part[k] + 1) % m).then(|| lp.faces[part[k]])).collect();
                        let triangles = triangulate(&points, plane(&points).0)?;
                        Some(Patch { points, faces, triangles, flat: true })
                    })
                    .collect();
                ways.extend(patches);
            }
        }
        let mut wrong = if bend > tolerance(area) { Verdict::Bent } else { Verdict::Tangled };
        for k in 0.. {
            // (The least surface last, only where nothing flat would do; for a shallow shell's
            // rim too long for it, the rim as it is seen square on, its corners where they are.)
            if k == ways.len() {
                if !holes.is_empty() || ways.iter().any(|w| w.len() == 1 && w[0].flat) || k > 1 {
                    break;
                }
                match least_area(&corners, normal).or_else(|| if rim && k == 0 { triangulate(&corners, normal) } else { None }) {
                    Some(triangles) => ways.push(vec![whole(triangles, false)]),
                    None => break,
                }
            }
            match self.judge(&ways[k], lp, holes, crack, rim) {
                Ok((caps, open, on_other)) if caps.is_empty() => return (open.unwrap_or(Verdict::Unsure), caps, on_other),
                Ok((caps, _, on_other)) => return (Verdict::Capped, caps, on_other),
                Err(verdict) => wrong = verdict,
            }
        }
        (wrong, Vec::new(), false)
    }

    /// The caps a way of closing a loop makes (none for a patch that needn't be closed: TA's
    /// camera looked into it from above, or a face lies on it), and why any is left open; or why
    /// the way is wrong (a cap facing into the model, hiding its faces or crossing them; for a
    /// shallow shell's `rim`, lying on faces of its piece, as a folded fin's outline cut at its
    /// fold gives).
    fn judge(&self, way: &[Patch], lp: &Loop, holes: &[&Loop], crack: bool, rim: bool) -> Result<(Vec<Cap>, Option<Verdict>, bool), Verdict> {
        let offset = self.pieces[self.index].offset;
        let around: Vec<u32> = lp.faces.iter().chain(holes.iter().flat_map(|h| h.faces.iter())).copied().collect();
        let (mut caps, mut open, mut on_other) = (Vec::new(), None, false);
        for patch in way {
            let points = &patch.points;
            let n = points.len();
            let (pn, pa, pc, _) = plane(points);
            if pa < 1e-3 || pn == Vec3::ZERO {
                continue;
            }
            // Facing up: TA's camera looked into it (a doorway a turret rises out of, a basin),
            // unless another piece hid it from every side (a walker's hip under its torso, a
            // knee under its thigh: a joint's opening, which moving shows).
            let joint = pn.y > SHOWN && !crack;
            if joint && (!self.rims || seen_from_above(&points.iter().map(|&p| p + offset).collect::<Vec<_>>(), pn, self.model.get_or_init(|| at_rest(self.pieces)))) {
                open.get_or_insert(Verdict::Shown);
                continue;
            }
            let triangles: Vec<([Vec3; 3], Vec3)> = patch
                .triangles
                .iter()
                .map(|t| {
                    let corners = t.map(|i| points[i as usize]);
                    (corners, plane(&corners).0)
                })
                .collect();
            // The faces along its edges fold back behind it (as a box's sides behind its missing
            // lid), not towards it, each edge by its length: a pit TA painted inside, its walls
            // facing into it, folds towards a cap over it all round. (An overhang's missing
            // underside meets the wall under it in a crease, along one side.)
            let (mut behind, mut front, mut all) = (0.0f32, 0.0f32, 0.0f32);
            for (k, f) in patch.faces.iter().enumerate() {
                let Some(f) = *f else { continue };
                let (a, b) = (points[k], points[(k + 1) % n]);
                let length = a.distance(b);
                // (The side of the triangle on this edge.)
                let j = (k + 1) % n;
                let local = patch.triangles.iter().position(|t| t.contains(&(k as u32)) && t.contains(&(j as u32))).map_or(pn, |t| triangles[t].1);
                let s = (self.centres[f as usize] - (a + b) * 0.5).dot(local);
                all += length;
                if s < -0.05 {
                    behind += length;
                } else if s > 0.05 {
                    front += length;
                }
            }
            if front > behind && front > all * 0.5 && !crack {
                return Err(Verdict::Inward);
            }
            // Lying on another face: of its piece (not one round the loop: an overhang's missing
            // underside lies under the overhang), or of another piece facing the same way.
            // (Samples: its triangles' middles, and a little in from its edges.)
            let mut samples: Vec<(Vec3, Vec3)> = triangles.iter().map(|(t, tn)| ((t[0] + t[1] + t[2]) / 3.0, *tn)).collect();
            for k in 0..n {
                let mid = (points[k] + points[(k + 1) % n]) * 0.5;
                let towards = pc - mid;
                samples.push((mid + towards.normalize_or_zero() * towards.length().min(0.3), pn));
            }
            let (lo, hi) = points.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
            // (The model's faces near it, and whether they're its piece's.)
            let near: Vec<(bool, &Flat)> = {
                let (lo, hi) = (lo + offset - Vec3::splat(COVER), hi + offset + Vec3::splat(COVER));
                let around = &around;
                self.flats
                    .iter()
                    .enumerate()
                    .flat_map(|(pj, flats)| {
                        flats.iter().enumerate().filter_map(move |(f, flat)| {
                            let own = pj == self.index;
                            flat.as_ref().filter(|flat| !(own && around.contains(&(f as u32))) && !(flat.lo.cmpgt(hi).any() || flat.hi.cmplt(lo).any())).map(|flat| (own, flat))
                        })
                    })
                    .collect()
            };
            // (How many of its samples have such a face on them.)
            let on = |own: bool, same: Option<bool>| {
                if !near.iter().any(|&(o, _)| o == own) {
                    return 0;
                }
                samples
                    .iter()
                    .filter(|&&(p, sn)| {
                        near.iter().any(|&(o, flat)| {
                            let facing = flat.normal.dot(sn);
                            o == own && facing.abs() > 0.95 && same.is_none_or(|s| (facing > 0.0) == s) && flat.holds(p + offset, COVER)
                        })
                    })
                    .count()
            };
            let covered = |own: bool, same: Option<bool>| on(own, same) * 2 > samples.len();
            if covered(true, None) {
                open.get_or_insert(Verdict::Seated);
                continue;
            }
            // A rim's cap lying on faces of its piece, for a quarter of it or more: the outline
            // of a fin folded along its length, cut at its fold, gives back the fin's own faces.
            // (The least surface spanning it closes it under the fin and across its ends.)
            if rim
                && samples
                    .iter()
                    .filter(|&&(p, sn)| self.flats[self.index].iter().flatten().any(|flat| flat.normal.dot(sn).abs() > 0.95 && flat.holds(p + offset, 0.05)))
                    .count()
                    * 4
                    > samples.len()
            {
                return Err(Verdict::Sheet);
            }
            // Hiding the piece's own faces: looking into the model from it, the first face met
            // within reach faces back out (an overhang's missing underside is not the side under
            // it).
            // (From the middles of its six biggest triangles at most.)
            let reach = 2.0 + 0.25 * pa.sqrt();
            let mut biggest: Vec<(f32, Vec3, Vec3)> = triangles.iter().map(|(t, tn)| ((t[1] - t[0]).cross(t[2] - t[0]).length(), (t[0] + t[1] + t[2]) / 3.0, *tn)).collect();
            biggest.sort_by(|a, b| b.0.total_cmp(&a.0));
            biggest.truncate(6);
            let (rlo, rhi) = biggest.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &(_, p, tn)| (lo.min(p).min(p - tn * reach), hi.max(p).max(p - tn * reach)));
            let near: Vec<([Vec3; 3], usize)> = self.hits.iter().filter(|(t, _)| !(t[0].min(t[1]).min(t[2]).cmpgt(rhi).any() || t[0].max(t[1]).max(t[2]).cmplt(rlo).any())).copied().collect();
            let hidden = biggest.iter().filter(|&&(_, p, tn)| first_hit(&near, p + tn * 0.01, -tn).is_some_and(|(d, f)| d <= reach && self.normals[f].dot(tn) > 0.3)).count();
            if hidden * 2 > biggest.len() {
                return Err(Verdict::Hidden);
            }
            // (A joint's opening with another piece lying in it anywhere, as a carrier's stalls
            // on its deck: its cap would lie on that piece, the two in one plane.)
            if covered(false, Some(true)) || joint && on(false, Some(true)) > 0 {
                open.get_or_insert(Verdict::Behind);
                continue;
            }
            on_other = on_other || covered(false, Some(false));
            // A bent patch must not cross the piece's faces, nor the faces cross it. (A flat one
            // lies where the missing face would have: what crosses it, a keel through a hull's
            // underside, would have crossed that face.)
            if !patch.flat {
                let (lo, hi) = (points.iter().fold(Vec3::MAX, |m, &p| m.min(p)) - Vec3::splat(0.1), points.iter().fold(Vec3::MIN, |m, &p| m.max(p)) + Vec3::splat(0.1));
                let crossing = self.hits.iter().filter(|(t, _)| !(t[0].min(t[1]).min(t[2]).cmpgt(hi).any() || t[0].max(t[1]).max(t[2]).cmplt(lo).any())).any(|(t, _)| {
                    triangles.iter().any(|(c, _)| (0..3).any(|k| pierces(t[k], t[(k + 1) % 3], c) || pierces(c[k], c[(k + 1) % 3], t)))
                });
                if crossing {
                    return Err(Verdict::Crossing);
                }
            }
            // (A flat convex cap is one polygon of its corners, as TA's faces are.)
            let n = points.len();
            let convex = patch.flat && (0..n).all(|k| (points[(k + 1) % n] - points[k]).cross(points[(k + 2) % n] - points[(k + 1) % n]).dot(pn) <= 1e-6);
            caps.push(Cap { corners: points.clone(), triangles: patch.triangles.clone(), normal: pn, convex, faces: patch.faces.iter().flatten().copied().collect(), rim, late: joint });
        }
        Ok((caps, open, on_other))
    }

    /// Whether looking into the model through a loop (from inside it, against `normal`) one sees
    /// out the other side: a frame's inside, rather than a post's open top under a dome.
    fn through(&self, lp: &Loop, normal: Vec3) -> bool {
        let points: Vec<Vec3> = lp.ids.iter().map(|&i| self.mesh.points[i as usize]).collect();
        let (n, ..) = plane(&points);
        // A point inside it: the middle of the biggest of its triangles.
        let Some(triangles) = triangulate(&points, n) else { return false };
        let area = |t: &[u32; 3]| (points[t[1] as usize] - points[t[0] as usize]).cross(points[t[2] as usize] - points[t[0] as usize]).length();
        let Some(t) = triangles.iter().max_by(|a, b| area(a).total_cmp(&area(b))) else {
            return false;
        };
        let from = (points[t[0] as usize] + points[t[1] as usize] + points[t[2] as usize]) / 3.0;
        first_hit(&self.hits, from, -normal).is_none_or(|(d, _)| d <= 0.05)
    }
}

/// The first of these triangles (each of a face) a ray meets, from either side: how far, and
/// which face.
pub(super) fn first_hit(triangles: &[([Vec3; 3], usize)], from: Vec3, dir: Vec3) -> Option<(f32, usize)> {
    let mut best: Option<(f32, usize)> = None;
    for &(t, face) in triangles {
        // (Moller and Trumbore's test.)
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            continue;
        }
        let s = from - t[0];
        let u = s.dot(p) / det;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) / det;
        let d = e2.dot(q) / det;
        if v >= 0.0 && u + v <= 1.0 && d > 1e-3 && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, face));
        }
    }
    best
}

/// A patch closing a hole, or part of one: a ring of points wound as the model's faces, the face
/// along each of its edges (none along a cut), its triangles, and whether it is flat.
struct Patch {
    points: Vec<Vec3>,
    faces: Vec<Option<u32>>,
    triangles: Vec<[u32; 3]>,
    flat: bool,
}

/// The triangles of a bent ring of points (wound as the model's faces) spanning the least
/// surface, as a soap film would (Barequet and Sharir's dynamic programme): a thin overhang's
/// missing underside rather than a flat cap across it. None where a triangle would face into the
/// model, against the ring's `normal`.
fn least_area(points: &[Vec3], normal: Vec3) -> Option<Vec<[u32; 3]>> {
    let n = points.len();
    if !(3..=MOST_SPLIT).contains(&n) {
        return None;
    }
    let area = |i: usize, k: usize, j: usize| (points[k] - points[i]).cross(points[j] - points[i]).length() * 0.5;
    // cost[i][j]: the least area of the ring from i to j closed by the chord j-i.
    let mut cost = vec![0f32; n * n];
    let mut split = vec![0usize; n * n];
    for len in 2..n {
        for i in 0..n - len {
            let j = i + len;
            let (mut best, mut at) = (f32::MAX, i + 1);
            for k in i + 1..j {
                let c = cost[i * n + k] + cost[k * n + j] + area(i, k, j);
                if c < best {
                    (best, at) = (c, k);
                }
            }
            cost[i * n + j] = best;
            split[i * n + j] = at;
        }
    }
    let mut out = Vec::with_capacity(n - 2);
    let mut stack = vec![(0, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j < i + 2 {
            continue;
        }
        let k = split[i * n + j];
        let t = [i as u32, k as u32, j as u32];
        let (tn, ta, ..) = plane(&t.map(|x| points[x as usize]));
        if ta < 1e-6 || tn.dot(normal) < -0.2 {
            return None;
        }
        out.push(t);
        stack.push((i, k));
        stack.push((k, j));
    }
    Some(out)
}

/// A frame's underside: its outer loop with its inner loops bridged in, each by a cut to the
/// nearest corner of the outline it sees, so that one ring of points runs round the underside
/// (the cut there and back). With the face along each edge (none along a cut).
fn bridged(points: &[Vec3], outer: &Loop, holes: &[&Loop], normal: Vec3) -> Option<(Vec<Vec3>, Vec<Option<u32>>)> {
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let flat = |p: Vec3| Vec2::new(p.dot(u), p.dot(v));
    let mut ring: Vec<Vec3> = outer.ids.iter().map(|&i| points[i as usize]).collect();
    let mut faces: Vec<Option<u32>> = outer.faces.iter().map(|&f| Some(f)).collect();
    let all: Vec<Vec<Vec3>> = holes.iter().map(|h| h.ids.iter().map(|&i| points[i as usize]).collect()).collect();
    for (hole, inner) in holes.iter().zip(&all) {
        // Whether the cut from `a` to `b` crosses an edge of the ring so far or of any hole.
        let crosses = |a: Vec2, b: Vec2, ring: &[Vec3]| {
            let cross = |o: Vec2, p: Vec2, q: Vec2| (p - o).perp_dot(q - o);
            std::iter::once(ring).chain(all.iter().map(|h| h.as_slice())).any(|r| {
                (0..r.len()).any(|k| {
                    let (c, d) = (flat(r[k]), flat(r[(k + 1) % r.len()]));
                    if c.distance(a) < 1e-4 || c.distance(b) < 1e-4 || d.distance(a) < 1e-4 || d.distance(b) < 1e-4 {
                        return false;
                    }
                    cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0
                })
            })
        };
        let mut best: Option<(f32, usize, usize)> = None;
        for (o, &p) in ring.iter().enumerate() {
            for (h, &q) in inner.iter().enumerate() {
                let d = p.distance_squared(q);
                if best.is_none_or(|(x, ..)| d < x) && !crosses(flat(p), flat(q), &ring) {
                    best = Some((d, o, h));
                }
            }
        }
        let (_, o, h) = best?;
        let n = inner.len();
        // The ring up to the cut's corner, the cut, round the hole and back, then on.
        let mut points_in: Vec<Vec3> = ring[..=o].to_vec();
        let mut faces_in: Vec<Option<u32>> = faces[..o].to_vec();
        faces_in.push(None);
        for k in 0..=n {
            points_in.push(inner[(h + k) % n]);
            faces_in.push(if k < n { Some(hole.faces[(h + k) % n]) } else { None });
        }
        points_in.extend_from_slice(&ring[o..]);
        faces_in.extend_from_slice(&faces[o..]);
        ring = points_in;
        faces = faces_in;
    }
    Some((ring, faces))
}

/// The model's faces at rest (model space) as triangles.
fn at_rest(pieces: &[PieceFaces]) -> Vec<([Vec3; 3], usize)> {
    pieces
        .iter()
        .flat_map(|p| p.faces.iter().flat_map(move |f| (1..f.len().saturating_sub(1)).map(move |k| ([f[0] + p.offset, f[k] + p.offset, f[k + 1] + p.offset], 0))))
        .collect()
}

/// Whether TA's camera (63.4 degrees up, the unit turned any way) could see this side of a face
/// of the model at rest (model space; `normal` the side's): a point of it, the middle or one a
/// little in from a corner, with no face of the model in the way.
fn seen_from_above(corners: &[Vec3], normal: Vec3, model: &[([Vec3; 3], usize)]) -> bool {
    let centre = corners.iter().copied().sum::<Vec3>() / corners.len().max(1) as f32;
    let points: Vec<Vec3> = std::iter::once(centre).chain(corners.iter().map(|&c| c + (centre - c) * 0.3)).collect();
    (0..8).any(|k| {
        let turn = std::f32::consts::TAU * k as f32 / 8.0;
        let eye = Vec3::new(turn.sin(), 2.0, turn.cos()).normalize();
        eye.dot(normal) > 0.05 && points.iter().any(|&p| first_hit(model, p + normal * 0.02, eye).is_none())
    })
}

/// The model's pieces mended, each as its own list of faces to turn round and caps to add.
/// `rims`: whether shallow shells' rims are closed (see `Piece::close`; not with `TA_FORMS=0`,
/// as the game did before it did).
pub(super) fn mend(pieces: &[PieceFaces], rims: bool) -> Vec<Mending> {
    // Every face of the model at rest (model space), flattened, for the faces lying on a cap.
    let flats: Vec<Vec<Option<Flat>>> = pieces
        .iter()
        .map(|p| {
            if !p.mend {
                return Vec::new();
            }
            p.faces
                .iter()
                .map(|f| Flat::new(f, p.offset))
                .collect()
        })
        .collect();
    (0..pieces.len()).map(|pi| if pieces[pi].mend { mend_piece(pi, pieces, &flats, rims) } else { Mending::default() }).collect()
}

fn mend_piece(pi: usize, pieces: &[PieceFaces], flats: &[Vec<Option<Flat>>], rims: bool) -> Mending {
    let faces = pieces[pi].faces;
    let mut out = Mending::default();
    if faces.is_empty() || faces.len() > 4096 {
        return out;
    }
    let mut mesh = weld(faces);
    let mut edges = edge_uses(&mesh);
    if split_edges(&mut mesh, &edges) {
        edges = edge_uses(&mesh);
    }
    let (mut flipped, conflicts) = wrongly_wound(&mesh, &edges);
    out.conflicts = conflicts;
    // Only where TA's camera saw neither side of it (a hull's underside wound into the hull,
    // under its deck): one it saw (a pit's floor) looked as its artist meant, whatever its
    // neighbours say.
    if !flipped.is_empty() {
        let model = at_rest(pieces);
        let offset = pieces[pi].offset;
        flipped.retain(|&f| {
            let corners: Vec<Vec3> = faces[f].iter().map(|&p| p + offset).collect();
            let normal = plane(&corners).0;
            !seen_from_above(&corners, normal, &model) && !seen_from_above(&corners, -normal, &model)
        });
    }
    for &f in &flipped {
        mesh.rings[f].reverse();
    }
    out.flipped = flipped;
    let edges = if out.flipped.is_empty() { edges } else { edge_uses(&mesh) };
    // Faces joined by their edges, and the open edges as their faces run them.
    let mut parent: Vec<u32> = (0..mesh.rings.len() as u32).collect();
    fn root(parent: &mut [u32], mut k: u32) -> u32 {
        while parent[k as usize] != k {
            parent[k as usize] = parent[parent[k as usize] as usize];
            k = parent[k as usize];
        }
        k
    }
    let mut open: Vec<(u32, u32, u32)> = Vec::new();
    for g in groups(&edges) {
        if g.len() == 1 {
            let ((lo, hi), f, forward) = g[0];
            open.push(if forward { (lo, hi, f) } else { (hi, lo, f) });
        } else {
            let first = root(&mut parent, g[0].1);
            for e in &g[1..] {
                let other = root(&mut parent, e.1);
                parent[other as usize] = first;
            }
        }
    }
    if open.is_empty() {
        return out;
    }
    // The open edges into loops, each edge followed by one starting where it ends; where a walk
    // comes back to a point it passed, the loop since is cut off.
    // (The open edges by the point each starts from; where in the walk each point was left from.)
    let mut from: Vec<(u32, u32)> = open.iter().enumerate().map(|(k, e)| (e.0, k as u32)).collect();
    from.sort_unstable();
    let mut left = vec![u32::MAX; mesh.points.len()];
    let mut used = vec![false; open.len()];
    let mut loops: Vec<Loop> = Vec::new();
    for start in 0..open.len() {
        if used[start] {
            continue;
        }
        let mut walk: Vec<usize> = vec![start];
        used[start] = true;
        left[open[start].0 as usize] = 0;
        loop {
            let end = open[*walk.last().unwrap()].1;
            if left[end as usize] != u32::MAX {
                let chain = walk.split_off(left[end as usize] as usize);
                for &e in &chain {
                    left[open[e].0 as usize] = u32::MAX;
                }
                // A cap runs each edge the other way: the chain backwards.
                let m = chain.len();
                loops.push(Loop { ids: (0..m).map(|j| open[chain[m - 1 - j]].1).collect(), faces: (0..m).map(|j| open[chain[m - 1 - j]].2).collect() });
                if walk.is_empty() {
                    break;
                }
                continue;
            }
            let first = from.partition_point(|&(p, _)| p < end);
            let mut next = from[first..].iter().take_while(|&&(p, _)| p == end).map(|&(_, e)| e as usize).filter(|&e| !used[e]);
            // Where loops touch at a point (two holes at a corner), the one the faces round the
            // point lead to: from the face of the edge coming in, round the point face by face
            // (across the edges they share) to the first whose edge going out is open.
            let chosen = match (next.next(), next.next()) {
                (Some(e), None) => Some(e),
                (Some(e), Some(_)) => fan_next(&mesh, &edges, &open, *walk.last().unwrap()).filter(|&k| !used[k]).or(Some(e)),
                (None, _) => None,
            };
            match chosen {
                Some(next) => {
                    used[next] = true;
                    left[end as usize] = walk.len() as u32;
                    walk.push(next);
                }
                None => {
                    out.broken += walk.len();
                    for &e in &walk {
                        left[open[e].0 as usize] = u32::MAX;
                    }
                    break;
                }
            }
        }
    }
    let component: Vec<u32> = (0..mesh.rings.len() as u32).map(|f| root(&mut parent, f)).collect();
    let centres = faces.iter().map(|f| f.iter().copied().sum::<Vec3>() / f.len().max(1) as f32).collect();
    let normals = faces.iter().map(|f| plane(f).0).collect();
    let hits = mesh
        .rings
        .iter()
        .zip(faces)
        .enumerate()
        .filter(|(_, (ring, _))| !ring.is_empty())
        .flat_map(|(f, (_, face))| (1..face.len() - 1).map(move |k| ([face[0], face[k], face[k + 1]], f)))
        .collect();
    let piece = Piece { index: pi, pieces, flats, mesh: &mesh, component, centres, normals, hits, rims, model: std::cell::OnceCell::new() };
    // Frames: a loop inside another, in its plane, facing the other way, through which one sees
    // out the other side (the inside and the rim of a frame's missing underside). Either would
    // close over the frame's middle: the outer one is closed round the inner ones. (A loop in
    // another that one can't see through, a post's open top under a dome's open bottom, is a
    // hole of its own.)
    let planes: Vec<(Vec3, f32, Vec3, f32)> = loops.iter().map(|lp| plane(&lp.ids.iter().map(|&i| mesh.points[i as usize]).collect::<Vec<_>>())).collect();
    let mut holes_of: Vec<Vec<usize>> = vec![Vec::new(); loops.len()];
    let mut inner = vec![false; loops.len()];
    for b in 0..loops.len() {
        let mut around: Option<(f32, usize)> = None;
        for a in 0..loops.len() {
            let ((na, aa, ca, ba), (nb, ab, cb, bb)) = (planes[a], planes[b]);
            if a == b || aa <= ab || na.dot(nb) > -0.9 || (cb - ca).dot(na).abs() > tolerance(aa) + ba + bb || around.is_some_and(|(x, _)| x <= aa) {
                continue;
            }
            let outer: Vec<Vec3> = loops[a].ids.iter().map(|&i| mesh.points[i as usize]).collect();
            let Some(flat) = Flat::new(&outer, Vec3::ZERO) else { continue };
            let inside = loops[b].ids.iter().filter(|&&i| flat.holds(mesh.points[i as usize] - na * (mesh.points[i as usize] - ca).dot(na), f32::INFINITY)).count();
            if inside * 2 > loops[b].ids.len() {
                around = Some((aa, a));
            }
        }
        if let Some((_, a)) = around
            && piece.through(&loops[b], planes[a].0)
        {
            holes_of[a].push(b);
            inner[b] = true;
        }
    }
    for (k, lp) in loops.iter().enumerate() {
        let holes: Vec<&Loop> = holes_of[k].iter().map(|&b| &loops[b]).collect();
        let (verdict, caps, on_other) = if inner[k] { (Verdict::Ring, Vec::new(), false) } else { piece.close(lp, &holes) };
        let corners = lp.ids.iter().map(|&i| mesh.points[i as usize]).collect();
        out.holes.push(Hole { verdict, corners, caps: caps.len(), on_other, faces: lp.faces.clone() });
        out.caps.extend(caps);
    }
    out
}


#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn outward(corners: &[Vec3]) -> Vec3 {
        plane(corners).0
    }

    /// A face of these corners wound as TA's are, facing `towards`.
    pub(in super::super) fn facing(mut corners: Vec<Vec3>, towards: Vec3) -> Polygon {
        if outward(&corners).dot(towards) < 0.0 {
            corners.reverse();
        }
        corners
    }

    /// A box's faces as TA's are (clockwise seen from outside, once z is flipped), from its corners,
    /// leaving out the ones named: 0 top, 1 bottom, 2 +x, 3 -x, 4 +z, 5 -z.
    pub(in super::super) fn open_box(min: Vec3, max: Vec3, without: &[usize]) -> Vec<Polygon> {
        let c = |x: bool, y: bool, z: bool| Vec3::new(if x { max.x } else { min.x }, if y { max.y } else { min.y }, if z { max.z } else { min.z });
        let quads = [
            [c(false, true, false), c(true, true, false), c(true, true, true), c(false, true, true)],
            [c(false, false, false), c(false, false, true), c(true, false, true), c(true, false, false)],
            [c(true, false, false), c(true, false, true), c(true, true, true), c(true, true, false)],
            [c(false, false, false), c(false, true, false), c(false, true, true), c(false, false, true)],
            [c(false, false, true), c(false, true, true), c(true, true, true), c(true, false, true)],
            [c(false, false, false), c(true, false, false), c(true, true, false), c(false, true, false)],
        ];
        let centre = (min + max) * 0.5;
        quads
            .iter()
            .enumerate()
            .filter(|(k, _)| !without.contains(k))
            .map(|(_, q)| {
                let mid = q.iter().copied().sum::<Vec3>() / 4.0;
                facing(q.to_vec(), mid - centre)
            })
            .collect()
    }

    /// One piece mended alone.
    fn mend_one(faces: &[Polygon]) -> Mending {
        mend(&[PieceFaces { faces, offset: Vec3::ZERO, mend: true }], true).remove(0)
    }

    #[test]
    fn a_box_without_its_bottom_gets_one_facing_down() {
        let m = mend_one(&open_box(Vec3::ZERO, Vec3::new(10.0, 4.0, 6.0), &[1]));
        assert_eq!(m.holes.len(), 1);
        assert_eq!(m.holes[0].verdict, Verdict::Capped, "{:?}", m.holes[0]);
        assert_eq!(m.caps.len(), 1);
        let cap = &m.caps[0];
        assert!(cap.normal.y < -0.99 && cap.convex, "{cap:?}");
        assert!(outward(&cap.corners).y < -0.99, "wound as the model's faces");
        for t in &cap.triangles {
            let corners = t.map(|i| cap.corners[i as usize]);
            assert!(outward(&corners).y < -0.99, "a triangle wound as the model's faces: {corners:?}");
        }
        assert!(m.flipped.is_empty() && m.conflicts == 0);
    }

    #[test]
    fn a_track_pod_without_its_bottom_and_inner_side_gets_both() {
        // A pod beside the hull (at -x): its bottom and its inner side (+x) missing make one bent
        // loop, cut along the edge between them.
        let m = mend_one(&open_box(Vec3::new(-12.0, 0.0, -10.0), Vec3::new(-8.0, 4.0, 10.0), &[1, 2]));
        assert_eq!(m.holes.len(), 1);
        assert_eq!((m.holes[0].verdict, m.holes[0].caps), (Verdict::Capped, 2), "{:?}", m.holes[0]);
        let mut normals: Vec<Vec3> = m.caps.iter().map(|c| c.normal).collect();
        normals.sort_by(|a, b| a.y.total_cmp(&b.y));
        assert!(normals[0].y < -0.99 && normals[1].x > 0.99, "{normals:?}");
        for cap in &m.caps {
            for t in &cap.triangles {
                assert!(outward(&t.map(|i| cap.corners[i as usize])).dot(cap.normal) > 0.99);
            }
        }
        // A box open at the top too: TA's camera looked in from above, so that side stays open.
        let m = mend_one(&open_box(Vec3::ZERO, Vec3::new(10.0, 4.0, 6.0), &[0, 2]));
        assert_eq!((m.holes[0].verdict, m.caps.len()), (Verdict::Capped, 1));
        assert!(m.caps[0].normal.x > 0.99);
        let m = mend_one(&open_box(Vec3::ZERO, Vec3::new(10.0, 4.0, 6.0), &[0]));
        assert_eq!((m.holes[0].verdict, m.caps.len()), (Verdict::Shown, 0));
    }

    #[test]
    fn an_overhangs_missing_underside_is_closed_under_it() {
        // A hull (x 0 to 10) under a deck reaching 2 past its side (to x 12), the deck sloping
        // down at its ends as a Mobile Artillery's: the overhang's underside, under the deck's
        // three faces, is missing. Flat sides would wall up the hull's side behind the overhang.
        let profile = [(-10.0, 2.0), (-6.0, 4.0), (6.0, 4.0), (10.0, 2.0)];
        let mut faces = Vec::new();
        for k in 0..3 {
            let ((z0, y0), (z1, y1)) = (profile[k], profile[k + 1]);
            faces.push(facing(vec![Vec3::new(0.0, y0, z0), Vec3::new(12.0, y0, z0), Vec3::new(12.0, y1, z1), Vec3::new(0.0, y1, z1)], Vec3::Y));
        }
        let wall = |x: f32| -> Vec<Vec3> { std::iter::once(Vec3::new(x, 0.0, -10.0)).chain(profile.iter().map(|&(z, y)| Vec3::new(x, y, z))).chain(std::iter::once(Vec3::new(x, 0.0, 10.0))).collect() };
        faces.push(facing(wall(10.0), Vec3::X));
        faces.push(facing(wall(0.0), -Vec3::X));
        faces.push(facing(vec![Vec3::new(0.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(0.0, 0.0, 10.0)], -Vec3::Y));
        for z in [10.0, -10.0] {
            faces.push(facing(vec![Vec3::new(0.0, 0.0, z), Vec3::new(10.0, 0.0, z), Vec3::new(10.0, 2.0, z), Vec3::new(0.0, 2.0, z)], Vec3::Z * z));
        }
        let m = mend_one(&faces);
        assert_eq!(m.holes.len(), 1, "{:?}", m.holes);
        assert_eq!(m.holes[0].verdict, Verdict::Capped, "{:?}", m.holes[0]);
        // Under the deck, between the hull's side and the deck's edge, facing down: no cap stands
        // over the hull's side.
        for cap in &m.caps {
            for t in &cap.triangles {
                let c = t.map(|i| cap.corners[i as usize]);
                assert!(c.iter().all(|p| p.x >= 10.0 - 1e-3), "{c:?}");
                assert!(outward(&c).y < -0.5, "{c:?}");
            }
        }
    }

    #[test]
    fn a_frames_missing_underside_is_closed_round_its_middle() {
        // A square frame round a hole, its underside missing: the rim and the inside are two
        // loops, one in the other; a cap on either would close over the hole in the middle.
        let square = |r: f32, y: f32| [Vec3::new(-r, y, -r), Vec3::new(r, y, -r), Vec3::new(r, y, r), Vec3::new(-r, y, r)];
        let (outer, inner, (outer_top, inner_top)) = (square(10.0, 0.0), square(6.0, 0.0), (square(10.0, 2.0), square(6.0, 2.0)));
        let mut faces = Vec::new();
        for k in 0..4 {
            let j = (k + 1) % 4;
            let middle = (outer[k] + outer[j]) * 0.5;
            faces.push(facing(vec![outer_top[k], outer_top[j], inner_top[j], inner_top[k]], Vec3::Y));
            faces.push(facing(vec![outer[k], outer[j], outer_top[j], outer_top[k]], middle));
            faces.push(facing(vec![inner[k], inner[j], inner_top[j], inner_top[k]], -middle));
        }
        let m = mend_one(&faces);
        let mut verdicts: Vec<Verdict> = m.holes.iter().map(|h| h.verdict).collect();
        verdicts.sort();
        assert_eq!(verdicts, [Verdict::Capped, Verdict::Ring], "{:?}", m.holes);
        // One cap, facing down, round the middle: as much as the frame's underside, nothing over
        // the middle.
        assert_eq!(m.caps.len(), 1);
        let cap = &m.caps[0];
        let mut area = 0.0;
        for t in &cap.triangles {
            let [a, b, c] = t.map(|i| cap.corners[i as usize]);
            let mid = (a + b + c) / 3.0;
            assert!(mid.x.abs() > 6.0 - 1e-3 || mid.z.abs() > 6.0 - 1e-3, "a triangle over the middle: {a} {b} {c}");
            assert!(outward(&[a, b, c]).y < -0.99);
            area += (b - a).cross(c - a).length() * 0.5;
        }
        assert!((area - (400.0 - 144.0)).abs() < 1e-2, "{area}");
        assert!(!cap.convex, "drawn as its triangles");
        // With a lid over the frame's middle (a post's open top under a dome's open bottom), the
        // inner loop is a hole of its own, and the outer one is closed whole.
        faces.push(facing(inner_top.to_vec(), Vec3::Y));
        let verdicts: Vec<Verdict> = mend_one(&faces).holes.iter().map(|h| h.verdict).collect();
        assert!(!verdicts.contains(&Verdict::Ring), "{verdicts:?}");
    }

    #[test]
    fn a_shallow_shells_rim_is_closed_and_a_folded_fin_not_on_its_own_faces() {
        // A plate with a skirt round it, open underneath (a shallow hull, a walker's foot): its
        // faces lie within the loop's tolerance of its plane, as a sheet's would, but the skirt
        // stands up from it all round: a rim, closed facing down.
        let m = mend_one(&open_box(Vec3::ZERO, Vec3::new(20.0, 1.2, 30.0), &[1]));
        assert_eq!((m.holes.len(), m.holes[0].verdict), (1, Verdict::Capped), "{:?}", m.holes);
        assert!(m.caps[0].normal.y < -0.99);
        // Open at the top, it is a tray TA's camera looked into: no rim.
        let m = mend_one(&open_box(Vec3::ZERO, Vec3::new(20.0, 1.2, 30.0), &[0]));
        assert_eq!(m.holes[0].verdict, Verdict::Sheet, "{:?}", m.holes);
        // A fin folded steeply along its length (two faces meeting at a ridge, open below and at
        // its ends): its outline cut at the fold would give back the fin's own faces; closed
        // instead under it and across its ends, a thin wedge, nothing lying on its faces.
        let (ridge, foot) = ([Vec3::new(0.0, 6.0, 0.0), Vec3::new(0.0, 6.0, 10.0)], 0.8);
        let fin = vec![
            facing(vec![ridge[0], ridge[1], Vec3::new(-foot, 0.0, 10.0), Vec3::new(-foot, 0.0, 0.0)], -Vec3::X),
            facing(vec![ridge[0], ridge[1], Vec3::new(foot, 0.0, 10.0), Vec3::new(foot, 0.0, 0.0)], Vec3::X),
        ];
        let m = mend_one(&fin);
        assert_eq!(m.holes[0].verdict, Verdict::Capped, "{:?}", m.holes);
        for cap in &m.caps {
            for t in &cap.triangles {
                let n = outward(&t.map(|i| cap.corners[i as usize]));
                assert!(n.y < -0.9 || n.z.abs() > 0.9, "under it or across an end, not on a face: {n}");
            }
        }
    }

    #[test]
    fn a_short_sleeve_open_below_and_at_its_ends_is_closed_under_it_and_across_them() {
        // The Sentinel's sleeves, between a gun's housing and its barrel: two sides and a top,
        // narrowing towards the muzzle, open below and at both ends, too short to be a barrel
        // made whole (halves.rs). Every corner is on the one bent loop round its openings, and
        // its sides and top are flat parts of that loop: capped there, they would give back the
        // sleeve's own faces. Closed instead under it and across its ends.
        let (back, front) = ((1.75f32, 3.0f32), (1.25f32, 2.0f32));
        let at = |(w, h): (f32, f32), x: f32, top: bool, z: f32| Vec3::new(x * w, if top { h } else { 0.0 }, z);
        let sleeve = vec![
            facing(vec![at(back, -1.0, false, 0.0), at(back, -1.0, true, 0.0), at(front, -1.0, true, 4.5), at(front, -1.0, false, 4.5)], -Vec3::X),
            facing(vec![at(back, 1.0, false, 0.0), at(back, 1.0, true, 0.0), at(front, 1.0, true, 4.5), at(front, 1.0, false, 4.5)], Vec3::X),
            facing(vec![at(back, -1.0, true, 0.0), at(back, 1.0, true, 0.0), at(front, 1.0, true, 4.5), at(front, -1.0, true, 4.5)], Vec3::Y),
        ];
        let m = mend_one(&sleeve);
        assert_eq!((m.holes.len(), m.holes[0].verdict), (1, Verdict::Capped), "{:?}", m.holes);
        assert_eq!(m.caps.len(), 3, "{:?}", m.caps);
        for cap in &m.caps {
            let n = outward(&cap.corners);
            assert!(n.y < -0.99 || n.z.abs() > 0.95, "under it or across an end, not on a face: {n}");
        }
    }

    #[test]
    fn an_opening_facing_up_under_another_piece_is_closed_but_not_one_ta_showed() {
        // A walker's hip, open on top, its torso over it: TA's camera never saw into it, and as
        // the torso turns it shows. Closed, facing up.
        let hip = open_box(Vec3::new(-6.0, 0.0, -3.0), Vec3::new(6.0, 4.0, 3.0), &[0]);
        let torso = open_box(Vec3::new(-10.0, 5.0, -5.0), Vec3::new(10.0, 14.0, 5.0), &[]);
        let both = |hip: &[Polygon]| mend(&[PieceFaces { faces: hip, offset: Vec3::ZERO, mend: true }, PieceFaces { faces: &torso, offset: Vec3::ZERO, mend: true }], true).remove(0);
        let m = both(&hip);
        assert_eq!(m.holes.iter().map(|h| h.verdict).collect::<Vec<_>>(), [Verdict::Capped], "{:?}", m.holes);
        assert!(m.caps[0].normal.y > 0.99);
        // Alone, TA's camera looked into it: as TA built it.
        let m = mend_one(&hip);
        assert_eq!(m.holes[0].verdict, Verdict::Shown);
        // So it did beside a piece that hides it from one side only.
        let mast = open_box(Vec3::new(-6.0, 5.0, -8.0), Vec3::new(6.0, 30.0, -4.0), &[]);
        let m = mend(&[PieceFaces { faces: &hip, offset: Vec3::ZERO, mend: true }, PieceFaces { faces: &mast, offset: Vec3::ZERO, mend: true }], true).remove(0);
        assert_eq!(m.holes[0].verdict, Verdict::Shown);
        // Doors lying over it: a doorway behind its doors, as before.
        let doors = vec![facing(vec![Vec3::new(-6.0, 4.02, -3.0), Vec3::new(6.0, 4.02, -3.0), Vec3::new(6.0, 4.02, 3.0), Vec3::new(-6.0, 4.02, 3.0)], Vec3::Y)];
        let m = mend(&[PieceFaces { faces: &hip, offset: Vec3::ZERO, mend: true }, PieceFaces { faces: &doors, offset: Vec3::ZERO, mend: true }], true).remove(0);
        assert_ne!(m.holes[0].verdict, Verdict::Capped, "{:?}", m.holes);
    }

    #[test]
    fn a_lone_face_a_flat_sheet_and_a_painted_pit_stay_open() {
        let quad = facing(vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 0.0)], Vec3::Y);
        let m = mend_one(&[quad]);
        assert_eq!((m.holes.len(), m.holes[0].verdict, m.caps.len()), (1, Verdict::Sheet, 0));
        // A pit TA painted inside (its walls and floor facing into it), open at the top: closed,
        // it would face down into the pit.
        let pit: Vec<Polygon> = open_box(Vec3::ZERO, Vec3::new(10.0, 4.0, 6.0), &[0])
            .into_iter()
            .map(|mut f| {
                f.reverse();
                f
            })
            .collect();
        let m = mend_one(&pit);
        assert_eq!((m.holes.len(), m.holes[0].verdict, m.caps.len()), (1, Verdict::Inward, 0), "{:?}", m.holes);
    }

    #[test]
    fn an_underside_wound_into_the_hull_is_turned_round_but_not_a_face_ta_showed() {
        // The bottom (1) wound up into the box: TA's camera, high above, saw neither side of it.
        let mut faces = open_box(Vec3::ZERO, Vec3::splat(8.0), &[]);
        faces[1].reverse();
        let m = mend_one(&faces);
        assert_eq!(m.flipped, vec![1]);
        assert!(m.holes.is_empty());
        // A side (2) wound into the box: TA's camera saw it from outside (as a hole, as its
        // artist left it), so it stays as it is.
        let mut faces = open_box(Vec3::ZERO, Vec3::splat(8.0), &[]);
        faces[2].reverse();
        let m = mend_one(&faces);
        assert!(m.flipped.is_empty() && m.conflicts == 4, "{m:?}");
    }

    #[test]
    fn a_turrets_open_bottom_on_its_hull_and_a_doorway_behind_doors() {
        // A hull, a turret on it (its own piece) without a bottom: closed all the same (a turret
        // may rise), noted as lying on another piece.
        let hull = open_box(Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0), &[]);
        let turret = open_box(Vec3::new(-4.0, 0.0, -4.0), Vec3::new(4.0, 3.0, 4.0), &[1]);
        let mended = mend(&[PieceFaces { faces: &hull, offset: Vec3::ZERO, mend: true }, PieceFaces { faces: &turret, offset: Vec3::new(0.0, 5.0, 0.0), mend: true }], true);
        let hole = &mended[1].holes[0];
        assert!(hole.verdict == Verdict::Capped && hole.on_other, "{hole:?}");
        // A doorway: a box open at the front (+z), a door of its own piece in it.
        let bay = open_box(Vec3::ZERO, Vec3::splat(10.0), &[4]);
        let door = vec![facing(vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 10.0, 0.0), Vec3::new(10.0, 10.0, 0.0), Vec3::new(10.0, 0.0, 0.0)], Vec3::Z)];
        let mended = mend(&[PieceFaces { faces: &bay, offset: Vec3::ZERO, mend: true }, PieceFaces { faces: &door, offset: Vec3::new(0.0, 0.0, 10.0), mend: true }], true);
        assert_eq!(mended[0].holes[0].verdict, Verdict::Behind);
        assert!(mended[0].caps.is_empty());
    }

    #[test]
    fn a_t_junction_closes_an_edge() {
        // A box whose top is two faces, and whose sides run the whole length past their shared
        // corners.
        let mut faces = open_box(Vec3::ZERO, Vec3::new(10.0, 4.0, 6.0), &[0]);
        for x in [0.0, 5.0] {
            faces.push(facing(vec![Vec3::new(x, 4.0, 0.0), Vec3::new(x + 5.0, 4.0, 0.0), Vec3::new(x + 5.0, 4.0, 6.0), Vec3::new(x, 4.0, 6.0)], Vec3::Y));
        }
        let m = mend_one(&faces);
        assert!(m.holes.is_empty(), "{:?}", m.holes);
    }

    /// The pieces of a model as `ModelLibrary::add` sees them: faces (piece space, z flipped,
    /// the selection plate left out), places at rest, and whether each may be mended.
    pub(in super::super) fn model_faces(model: &ta_formats::model3do::Model3do) -> (Vec<Vec<Polygon>>, Vec<Vec3>, Vec<bool>) {
        use ta_formats::model3do::FIXED_ONE;
        let fixed = |v: [i32; 3]| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE;
        let mut rest: Vec<Vec3> = Vec::new();
        for p in &model.pieces {
            let off = fixed(p.offset);
            let at = p.parent.map_or(off, |parent| rest[parent] + off);
            rest.push(at);
        }
        let faces = model
            .pieces
            .iter()
            .map(|piece| {
                piece
                    .primitives
                    .iter()
                    .enumerate()
                    .filter(|&(k, prim)| Some(k) != piece.selection_primitive && prim.indices.len() >= 3)
                    .map(|(_, prim)| prim.indices.iter().map(|&i| fixed(piece.vertices[i as usize])).collect())
                    .collect()
            })
            .collect();
        let mend = model.pieces.iter().map(|p| !super::super::is_effect_piece(&p.name)).collect();
        (faces, rest, mend)
    }

    /// Whether forms are made whole, as the game has it (`TA_FORMS=0`: not).
    fn forms() -> bool {
        std::env::var("TA_FORMS").map_or(true, |v| v != "0")
    }

    /// The pieces' faces as the game mends them: the halves of barrels and booms made whole
    /// first (see halves.rs; not with `TA_FORMS=0`).
    fn whole(faces: &mut [Vec<Polygon>], rest: &[Vec3], mendable: &[bool]) {
        if !forms() {
            return;
        }
        let pieces: Vec<PieceFaces> = faces.iter().zip(rest).zip(mendable).map(|((f, &offset), &mend)| PieceFaces { faces: f, offset, mend }).collect();
        let halves = super::super::halves::complete(&pieces);
        for (f, h) in faces.iter_mut().zip(halves) {
            f.extend(h.mirrored.into_iter().map(|m| m.corners));
        }
    }

    /// The install's files (base game, expansions, downloadable units), if there is one.
    pub(in super::super) fn install() -> Option<ta_vfs::Vfs> {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        Some(vfs)
    }

    /// Every installed model's open loops (once its halves are made whole, as the game has
    /// them): how many, what became of them and why, and the 30 models showing a 3D camera the
    /// most hole (square units: loops not lying on other faces, not a sheet's outline, not at the
    /// foot of a model standing on the ground). `cargo test --release -p ta_render --lib
    /// scan_holes -- --ignored --nocapture` (`TA_SCAN_MODELS=a,b` lists those models' loops one
    /// by one instead, `all` every model's).
    #[test]
    #[ignore]
    fn scan_holes_in_every_installed_model() {
        use std::collections::BTreeMap;
        let Some(vfs) = install() else { return };
        // Units by their models: names, and whether they fly.
        let mut units: BTreeMap<String, (String, bool)> = BTreeMap::new();
        for path in vfs.list("units", Some("fbi"), true) {
            let Some(bytes) = vfs.read_opt(&path) else { continue };
            let doc = ta_formats::tdf::parse_bytes(&bytes);
            let Some(info) = doc.section("UNITINFO") else { continue };
            let name = format!("{} ({})", info.get_str("UnitName", ""), info.get_str("Name", ""));
            units.entry(info.get_str("Objectname", "").to_ascii_lowercase()).or_insert((name, info.get_bool_or("canfly", false)));
        }
        let detail: Vec<String> = std::env::var("TA_SCAN_MODELS").map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()).collect()).unwrap_or_default();
        let mut elapsed = std::time::Duration::ZERO;
        let (mut models, mut with_holes, mut flipped, mut conflicts, mut broken, mut caps, mut triangles) = (0, 0, Vec::new(), 0, 0, 0, 0);
        let (mut down, mut seen, mut on_other) = (0, 0, 0);
        let mut verdicts: BTreeMap<Verdict, (usize, f32)> = BTreeMap::new();
        let mut ranking: Vec<(f32, f32, String)> = Vec::new();
        for path in vfs.list("objects3d", Some("3do"), true) {
            let Ok(model) = ta_formats::model3do::Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            models += 1;
            let (mut faces, rest, mendable) = model_faces(&model);
            whole(&mut faces, &rest, &mendable);
            let pieces: Vec<PieceFaces> = faces.iter().zip(&rest).zip(&mendable).map(|((f, &offset), &mend)| PieceFaces { faces: f, offset, mend }).collect();
            let started = std::time::Instant::now();
            let mended = mend(&pieces, forms());
            elapsed += started.elapsed();
            let stem = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".3do").to_ascii_lowercase();
            let unit = units.get(&stem);
            // The model's lowest point at rest: holes there facing down are on the ground.
            let low = faces.iter().zip(&rest).flat_map(|(f, &o)| f.iter().flatten().map(move |p| p.y + o.y)).fold(f32::MAX, f32::min);
            let (mut visible, mut left, mut lines) = (0.0f32, 0.0f32, Vec::new());
            for (pi, m) in mended.iter().enumerate() {
                if !m.flipped.is_empty() {
                    flipped.push(format!("{path} {} faces {:?}", model.pieces[pi].name, m.flipped));
                    lines.push(format!("    piece {pi} {}: faces turned round {:?}", model.pieces[pi].name, m.flipped));
                }
                conflicts += m.conflicts;
                broken += m.broken;
                caps += m.caps.len();
                triangles += m.caps.iter().map(|c| c.triangles.len()).sum::<usize>();
                for h in &m.holes {
                    let (normal, area, _, bend) = plane(&h.corners);
                    let e = verdicts.entry(h.verdict).or_default();
                    e.0 += 1;
                    e.1 += area;
                    let top = h.corners.iter().map(|p| p.y + rest[pi].y).fold(f32::MIN, f32::max);
                    let grounded = normal.y < -0.7 && top <= low + 1.0 && !unit.is_some_and(|u| u.1);
                    if h.verdict == Verdict::Capped {
                        // (TA's camera, 63.4 degrees up, saw no face turned more than 26.6 down.)
                        if normal.y < -0.447 { down += 1 } else { seen += 1 }
                        on_other += usize::from(h.on_other);
                    }
                    if !matches!(h.verdict, Verdict::Sheet | Verdict::Seated | Verdict::Behind) && !grounded && !h.on_other {
                        visible += area;
                        if h.verdict != Verdict::Capped {
                            left += area;
                        }
                    }
                    lines.push(format!(
                        "    piece {pi} {:12} {:9?} {} caps, {:3} corners, area {:7.1}, bend {:5.2}, facing ({:5.2},{:5.2},{:5.2}){}{}",
                        model.pieces[pi].name,
                        h.verdict,
                        h.caps,
                        h.corners.len(),
                        area,
                        bend,
                        normal.x,
                        normal.y,
                        normal.z,
                        if grounded { ", on the ground" } else { "" },
                        if h.on_other { ", on another piece" } else { "" }
                    ));
                }
            }
            with_holes += usize::from(mended.iter().any(|m| !m.holes.is_empty()));
            let label = format!("{path} {}", unit.map_or("", |u| u.0.as_str()));
            if detail.contains(&stem) || detail.iter().any(|d| d == "all") {
                eprintln!("{label}: {visible:.0} square units open to a 3D camera, {left:.0} left open\n{}", lines.join("\n"));
            }
            ranking.push((visible, left, label));
        }
        if detail.is_empty() {
            eprintln!("The 30 models showing a 3D camera the most hole (square units; left open after mending):");
            ranking.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (visible, left, label) in ranking.iter().take(30) {
                eprintln!("{visible:7.0} {left:7.0}  {label}");
            }
            eprintln!("The 15 left the most open:");
            ranking.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (visible, left, label) in ranking.iter().take(15) {
                eprintln!("{visible:7.0} {left:7.0}  {label}");
            }
        }
        eprintln!("{models} models, {with_holes} with open loops; mended in {:.0} ms", elapsed.as_secs_f64() * 1000.0);
        for (v, (count, area)) in &verdicts {
            eprintln!("  {v:?}: {count} loops, {area:.0} square units");
        }
        eprintln!("  closed: {caps} caps, {triangles} triangles; {down} facing down (TA never showed them), {seen} that TA's camera could see; {on_other} on another piece");
        eprintln!("  {broken} open edges make no loop (no hole to close: faces folded against each other)");
        eprintln!("  faces wound against their neighbours, turned round ({} pieces; {conflicts} edges two faces run the same way):", flipped.len());
        for line in &flipped {
            eprintln!("    {line}");
        }
    }

    /// Loops left open as a sheet's outline whose faces fold (a channel, a ridge, a fin bent
    /// along its length), the biggest first: what the sheet test still takes for sheets, to look
    /// at for shells one sees through. `cargo test --release -p ta_render --lib
    /// scan_folded_sheets -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scan_folded_sheets() {
        let Some(vfs) = install() else { return };
        let mut rows = Vec::new();
        for path in vfs.list("objects3d", Some("3do"), true) {
            if path.contains("_dead") {
                continue;
            }
            let Ok(model) = ta_formats::model3do::Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            let (mut faces, rest, mendable) = model_faces(&model);
            whole(&mut faces, &rest, &mendable);
            let pieces: Vec<PieceFaces> = faces.iter().zip(&rest).zip(&mendable).map(|((f, &offset), &mend)| PieceFaces { faces: f, offset, mend }).collect();
            for (pi, m) in mend(&pieces, forms()).iter().enumerate() {
                for h in m.holes.iter().filter(|h| h.verdict == Verdict::Sheet) {
                    let mut normals: Vec<Vec3> = h.faces.iter().map(|&f| plane(&faces[pi][f as usize]).0).collect();
                    normals.dedup_by(|a, b| a.dot(*b) > 0.999);
                    let spread = normals.iter().flat_map(|a| normals.iter().map(move |b| a.dot(*b))).fold(1.0f32, f32::min);
                    if spread > 0.85 {
                        continue;
                    }
                    let (n, area, _, bend) = plane(&h.corners);
                    let (lo, hi) = h.corners.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
                    let size = hi - lo;
                    let mut faces_round = h.faces.clone();
                    faces_round.sort_unstable();
                    faces_round.dedup();
                    rows.push((area, format!("{:<28} {:12} area {area:6.1} bend {bend:4.1} faces {:2} spread {:5.2} facing ({:5.2},{:5.2},{:5.2}) size ({:5.1},{:5.1},{:5.1})", path.trim_start_matches("objects3d/"), model.pieces[pi].name, faces_round.len(), spread, n.x, n.y, n.z, size.x, size.y, size.z)));
                }
            }
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, line) in &rows {
            eprintln!("{line}");
        }
        eprintln!("{} folded sheets", rows.len());
    }

    /// `TA_SCAN_MODELS=armstump,corraid TA_HOLES_OUT=<dir>`: writes those models at rest as OBJ
    /// (front faces anticlockwise), their faces in one object, their caps in another, the faces
    /// turned round in a third and the loops left open, as caps would close them, in one object
    /// for each reason. On this computer only.
    #[test]
    #[ignore]
    fn dump_holes() {
        let (Ok(names), Ok(out)) = (std::env::var("TA_SCAN_MODELS"), std::env::var("TA_HOLES_OUT")) else { return };
        let Some(vfs) = install() else { return };
        std::fs::create_dir_all(&out).unwrap();
        for name in names.split(',').map(|s| s.trim()) {
            let Some(bytes) = vfs.read_opt(&format!("objects3d/{name}.3do")) else {
                eprintln!("no model {name}");
                continue;
            };
            let model = ta_formats::model3do::Model3do::parse(&bytes).unwrap();
            let (mut faces, rest, mendable) = model_faces(&model);
            whole(&mut faces, &rest, &mendable);
            let pieces: Vec<PieceFaces> = faces.iter().zip(&rest).zip(&mendable).map(|((f, &offset), &mend)| PieceFaces { faces: f, offset, mend }).collect();
            let mut objects: std::collections::BTreeMap<String, Vec<Vec<Vec3>>> = Default::default();
            for (pi, m) in mend(&pieces, forms()).iter().enumerate() {
                for (f, face) in faces[pi].iter().enumerate() {
                    let mut face: Vec<Vec3> = face.iter().map(|&p| p + rest[pi]).collect();
                    let key = if m.flipped.contains(&f) {
                        face.reverse();
                        "flipped"
                    } else {
                        "faces"
                    };
                    objects.entry(key.into()).or_default().push(face);
                }
                for cap in &m.caps {
                    for t in &cap.triangles {
                        objects.entry("caps".into()).or_default().push(t.iter().map(|&i| cap.corners[i as usize] + rest[pi]).collect());
                    }
                }
                for h in m.holes.iter().filter(|h| h.verdict != Verdict::Capped) {
                    objects.entry(format!("skip_{:?}", h.verdict)).or_default().push(h.corners.iter().map(|&p| p + rest[pi]).collect());
                }
            }
            let mut obj = String::new();
            let mut count = 0usize;
            for (key, polygons) in &objects {
                obj += &format!("o {key}\n");
                for poly in polygons {
                    for p in poly {
                        obj += &format!("v {} {} {}\n", p.x, p.y, p.z);
                    }
                    // (OBJ's front faces run anticlockwise; the model's run clockwise.)
                    let ids: Vec<String> = (0..poly.len()).rev().map(|k| (count + k + 1).to_string()).collect();
                    obj += &format!("f {}\n", ids.join(" "));
                    count += poly.len();
                }
            }
            std::fs::write(std::path::Path::new(&out).join(format!("{name}.obj")), obj).unwrap();
        }
    }
}
