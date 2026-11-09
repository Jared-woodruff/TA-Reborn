//! Reborn's detail rounds TA's curves. Where TA drew a curve with a few facets (a dome, a barrel,
//! a rounded hull) its corners take the curve's normals (smooth.rs), so it is shaded round but
//! its outline stays a polygon. Here such facets are cut into smaller ones laid on the curve
//! their corners' normals describe (Phong tessellation: each point drawn towards the tangent
//! planes at the corners), so the outline is as round as the shading. The cuts match along every
//! smooth edge (each run of facets meeting smoothly is cut alike, and a point on an edge is
//! reckoned from the edge alone, the same way round from either side); hard folds and edges no
//! other facet shares stay straight, the curve easing off towards them. Only real curves are
//! rounded: a run of at least [`RUN`] facets whose normals turn through [`TURNING`] all told (a
//! barrel, a dome, a hull's rounded flank), not a shallow fold between a few plates, which
//! smooth.rs shades smooth but which is drawn angular. Texture coordinates and the edges' wear
//! run on across the cut facets as across the whole.

use std::collections::HashMap;

use glam::Vec3;

use super::kit::Kit;
use super::{ModelVertex, f16_bits};

/// How far the corners' tangent planes draw a facet's points (Phong tessellation's α: the
/// usual three quarters, nearly a circle's arc over a polygon's sides).
const ALPHA: f32 = 0.55;
/// A facet is curved when a corner's normal strays from its own by more than this (a cosine,
/// about 8 degrees).
const CURVED: f32 = 0.990;
/// Two facets meet smoothly along an edge when their normals at its ends are alike to this (a
/// cosine, about 2 degrees).
const ALIKE: f32 = 0.9994;
/// Each this much (model units) a run of facets would bow out is another cut along its edges,
/// up to [`MOST`]; a run bowing out less than it stays as it is (a road wheel's facets).
const BOW: f32 = 0.15;
const MOST: usize = 4;
/// A run of facets is a curve when it has at least this many and its normals turn through at
/// least this much all told (the cosine of the widest angle between two of them).
const RUN: usize = 6;
const TURNING: f32 = 0.5;

type Key = [i32; 3];
/// A facet's side along an edge: the facet, which of its edges, its normals at the edge's ends.
type Side = (usize, usize, Vec3, Vec3);

fn key(p: Vec3) -> Key {
    [p.x, p.y, p.z].map(|x| (x * 4096.0).round() as i32)
}

fn normal_of(v: &ModelVertex) -> Vec3 {
    Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32).normalize_or_zero()
}

/// IEEE half-precision bits as a number (the edges' distances, see `ModelVertex::edge_dist`).
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((bits >> 10) & 0x1F) as i32;
    let frac = (bits & 0x3FF) as f32;
    if exp == 0 {
        return sign * frac * 2f32.powi(-24);
    }
    sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15)
}

/// The point a fraction `t` of the way from `a` to `b`, drawn towards their tangent planes (normals
/// `na`, `nb`) as much as `alpha`.
fn phong_on_edge(a: Vec3, b: Vec3, na: Vec3, nb: Vec3, t: f32, alpha: f32) -> Vec3 {
    let p = a + (b - a) * t;
    let pa = p - (p - a).dot(na) * na;
    let pb = p - (p - b).dot(nb) * nb;
    p * (1.0 - alpha) + (pa * (1.0 - t) + pb * t) * alpha
}

/// How far the middle of an edge bows out (its Phong midpoint from its straight one).
fn bow(a: Vec3, b: Vec3, na: Vec3, nb: Vec3) -> f32 {
    (phong_on_edge(a, b, na, nb, 0.5, ALPHA) - (a + b) * 0.5).length()
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// One of a mesh's triangles, as the rounding sees it.
struct Facet {
    corners: [u32; 3],
    positions: [Vec3; 3],
    normals: [Vec3; 3],
    curved: bool,
    /// Its edges (from corner `k` to the next) shared smoothly with another curved facet.
    smooth: [bool; 3],
}

/// Rounds a piece's mesh (Reborn's kit for it) where its facets draw a curve: returns how many
/// triangles it added, and how far it moved a point the most (model units).
pub(super) fn round(mesh: &mut Kit) -> (usize, f32) {
    let facets: Vec<Facet> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&corners| {
            let v = corners.map(|i| &mesh.vertices[i as usize]);
            let positions = v.map(|v| Vec3::from_array(v.pos));
            let normals = v.map(normal_of);
            let face = (positions[1] - positions[0]).cross(positions[2] - positions[0]);
            // (TA's winding: the face's normal is the corners' cross product the other way round.)
            let face = if face.length_squared() > 1e-10 { -face.normalize() } else { Vec3::ZERO };
            // (Either way round: a back laid over a front, as the kit's track backs are, counts.)
            let curved = face != Vec3::ZERO && normals.iter().all(|n| *n != Vec3::ZERO) && normals.iter().any(|n| n.dot(face).abs() < CURVED);
            Facet { corners, positions, normals, curved, smooth: [false; 3] }
        })
        .collect();
    if !facets.iter().any(|f| f.curved) {
        return (0, 0.0);
    }
    let mut facets = facets;
    // Every curved facet's edges by their ends (the lesser key first), with the normals at them.
    let mut edges: HashMap<(Key, Key), Vec<Side>> = HashMap::new();
    for (i, f) in facets.iter().enumerate().filter(|(_, f)| f.curved) {
        for k in 0..3 {
            let (a, b) = (k, (k + 1) % 3);
            let (ka, kb) = (key(f.positions[a]), key(f.positions[b]));
            let entry = if ka <= kb { ((ka, kb), f.normals[a], f.normals[b]) } else { ((kb, ka), f.normals[b], f.normals[a]) };
            edges.entry(entry.0).or_default().push((i, k, entry.1, entry.2));
        }
    }
    // Smooth edges join their facets into runs, each cut alike.
    let mut parent: Vec<usize> = (0..facets.len()).collect();
    for shared in edges.values() {
        for (x, &(i, ki, na, nb)) in shared.iter().enumerate() {
            for &(j, kj, ma, mb) in &shared[x + 1..] {
                if i != j && na.dot(ma) >= ALIKE && nb.dot(mb) >= ALIKE {
                    facets[i].smooth[ki] = true;
                    facets[j].smooth[kj] = true;
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut most_bow: HashMap<usize, f32> = HashMap::new();
    let mut runs: HashMap<usize, (usize, Vec<Vec3>)> = HashMap::new();
    for (i, f) in facets.iter().enumerate().filter(|(_, f)| f.curved) {
        let b = (0..3).map(|k| bow(f.positions[k], f.positions[(k + 1) % 3], f.normals[k], f.normals[(k + 1) % 3])).fold(0.0f32, f32::max);
        let root = find(&mut parent, i);
        let entry = most_bow.entry(root).or_insert(0.0);
        *entry = entry.max(b);
        let (count, normals) = runs.entry(root).or_default();
        *count += 1;
        for n in f.normals {
            if normals.len() < 64 && !normals.iter().any(|m| m.dot(n) > 0.999) {
                normals.push(n);
            }
        }
    }
    // (A run too few, or turning too little, is a fold, not a curve: it stays as drawn.)
    for (root, (count, normals)) in &runs {
        let widest = normals.iter().flat_map(|a| normals.iter().map(move |b| a.dot(*b))).fold(1.0f32, f32::min);
        if *count < RUN || widest > TURNING {
            most_bow.insert(*root, 0.0);
        }
    }
    // (A smooth edge's ends' normals as one of its facets has them: both reckon its points alike.)
    let canonical: HashMap<(Key, Key), (Vec3, Vec3)> = edges.iter().map(|(k, shared)| (*k, (shared[0].2, shared[0].3))).collect();
    let mut indices = Vec::with_capacity(mesh.indices.len());
    let before = mesh.indices.len() / 3;
    let mut moved = 0.0f32;
    for (i, f) in facets.iter().enumerate() {
        let cuts = if f.curved {
            let root = find(&mut parent, i);
            (1 + (most_bow[&root] / BOW) as usize).min(MOST)
        } else {
            1
        };
        if cuts < 2 {
            indices.extend_from_slice(&f.corners);
            continue;
        }
        moved = moved.max(cut(mesh, f, cuts, &canonical, &mut indices));
    }
    mesh.indices = indices;
    (mesh.indices.len() / 3 - before, moved)
}

/// A facet cut `n` ways along each edge into `n`² triangles on its curve; returns how far it moved
/// a point the most.
fn cut(mesh: &mut Kit, f: &Facet, n: usize, canonical: &HashMap<(Key, Key), (Vec3, Vec3)>, indices: &mut Vec<u32>) -> f32 {
    let corners = f.corners.map(|i| mesh.vertices[i as usize]);
    let uv = |v: &ModelVertex| [v.uv[0] as f32, v.uv[1] as f32];
    let dist = |v: &ModelVertex| v.edge_dist.map(half);
    // The vertex at grid point (i, j, k) of `n` (barycentric i/n, j/n, k/n of the corners).
    let point = |i: usize, j: usize, k: usize| -> ModelVertex {
        let l = [i as f32 / n as f32, j as f32 / n as f32, k as f32 / n as f32];
        let p = f.positions[0] * l[0] + f.positions[1] * l[1] + f.positions[2] * l[2];
        // On an edge: reckoned from the edge alone (from its lesser end), so that the facet across
        // it has the same point; straight on a hard edge.
        let on_edge = [(i, j, 0, 1), (j, k, 1, 2), (k, i, 2, 0)].into_iter().find(|&(_, _, a, b)| [i, j, k][3 - a - b] == 0);
        let position = match on_edge {
            Some((ga, gb, a, b)) => {
                let edge = a; // (corner a to corner b is edge a when b follows it)
                let edge = if (a + 1) % 3 == b { edge } else { b };
                if !f.smooth[edge] {
                    p
                } else {
                    let (pa, pb) = (f.positions[a], f.positions[b]);
                    let (ka, kb) = (key(pa), key(pb));
                    let ((lo, hi), steps) = if ka <= kb { ((pa, pb), gb) } else { ((pb, pa), ga) };
                    let (nlo, nhi) = canonical[&if ka <= kb { (ka, kb) } else { (kb, ka) }];
                    phong_on_edge(lo, hi, nlo, nhi, steps as f32 / n as f32, ALPHA)
                }
            }
            None => {
                // Inside: towards the corners' tangent planes, easing off towards a hard edge.
                let mut alpha = ALPHA;
                for (edge, opposite) in [(0usize, 2usize), (1, 0), (2, 1)] {
                    if !f.smooth[edge] {
                        let x = (l[opposite] * 3.0).min(1.0);
                        alpha *= x * x * (3.0 - 2.0 * x);
                    }
                }
                let q: Vec3 = (0..3).map(|c| (p - (p - f.positions[c]).dot(f.normals[c]) * f.normals[c]) * l[c]).sum();
                p * (1.0 - alpha) + q * alpha
            }
        };
        let normal = (f.normals[0] * l[0] + f.normals[1] * l[1] + f.normals[2] * l[2]).normalize_or_zero();
        let [u0, u1, u2] = [uv(&corners[0]), uv(&corners[1]), uv(&corners[2])];
        let [d0, d1, d2] = [dist(&corners[0]), dist(&corners[1]), dist(&corners[2])];
        let mut v = corners[0];
        v.pos = position.to_array();
        v.normal = [(normal.x * 127.0).round() as i8, (normal.y * 127.0).round() as i8, (normal.z * 127.0).round() as i8, corners[0].normal[3]];
        v.uv = [0, 1].map(|c| (u0[c] * l[0] + u1[c] * l[1] + u2[c] * l[2]).round().clamp(0.0, 65535.0) as u16);
        v.edge_dist = [0, 1, 2, 3].map(|c| f16_bits((d0[c] * l[0] + d1[c] * l[1] + d2[c] * l[2]).max(0.0)));
        v
    };
    // The grid's vertices, row by row (i from n down to 0; along a row j up, k = n - i - j).
    let mut at = HashMap::new();
    let mut moved = 0.0f32;
    for i in (0..=n).rev() {
        for j in 0..=(n - i) {
            let k = n - i - j;
            at.insert((i, j), mesh.vertices.len() as u32);
            let v = if (i, j, k) == (n, 0, 0) { corners[0] } else if (i, j, k) == (0, n, 0) { corners[1] } else if (i, j, k) == (0, 0, n) { corners[2] } else { point(i, j, k) };
            let flat = (f.positions[0] * i as f32 + f.positions[1] * j as f32 + f.positions[2] * k as f32) / n as f32;
            moved = moved.max((Vec3::from_array(v.pos) - flat).length());
            mesh.vertices.push(v);
        }
    }
    // Triangles in the facet's own winding (corner 0, 1, 2 as it has them).
    for i in (1..=n).rev() {
        for j in 0..=(n - i) {
            // (i, j), (i-1, j+1), (i-1, j): a triangle the facet's way round.
            indices.extend_from_slice(&[at[&(i, j)], at[&(i - 1, j + 1)], at[&(i - 1, j)]]);
            if j < n - i {
                indices.extend_from_slice(&[at[&(i, j)], at[&(i, j + 1)], at[&(i - 1, j + 1)]]);
            }
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(p: Vec3, n: Vec3) -> ModelVertex {
        ModelVertex {
            pos: p.to_array(),
            normal: [(n.x * 127.0) as i8, (n.y * 127.0) as i8, (n.z * 127.0) as i8, 64],
            uv: [0, 0],
            layer: 0xFFFF,
            color: [200, 200, 200, 0],
            piece: 0,
            edge_dist: [f16_bits(1.0); 4],
            edge_weight: [0; 4],
            tread: [0.0; 2],
        }
    }

    /// An octagonal barrel, its sides' corners sharing the barrel's normals, and a flat cap on
    /// its end: TA's way of drawing a round barrel.
    fn barrel() -> Kit {
        let mut mesh = Kit::default();
        let (r, length, sides) = (4.0f32, 20.0f32, 8);
        let at = |s: usize| {
            let a = s as f32 / sides as f32 * std::f32::consts::TAU;
            (Vec3::new(a.cos() * r, a.sin() * r, 0.0), Vec3::new(a.cos(), a.sin(), 0.0))
        };
        for s in 0..sides {
            let ((p0, n0), (p1, n1)) = (at(s), at(s + 1));
            let base = mesh.vertices.len() as u32;
            let z = Vec3::Z * length;
            mesh.vertices.extend([vertex(p0, n0), vertex(p1, n1), vertex(p1 + z, n1), vertex(p0 + z, n0)]);
            // (TA's winding, as model.rs lays its faces: the normal is the cross product reversed.)
            mesh.indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }
        let base = mesh.vertices.len() as u32;
        for s in 0..sides {
            mesh.vertices.push(vertex(at(s).0, Vec3::NEG_Z));
        }
        for k in 1..sides as u32 - 1 {
            mesh.indices.extend_from_slice(&[base, base + k, base + k + 1]);
        }
        mesh
    }

    #[test]
    fn a_faceted_barrel_comes_out_round_and_its_cap_stays_flat() {
        let mut mesh = barrel();
        let before = mesh.indices.len() / 3;
        let cap: Vec<Vec3> = mesh.vertices[32..].iter().map(|v| Vec3::from_array(v.pos)).collect();
        let (added, moved) = round(&mut mesh);
        assert!(added > 0 && mesh.indices.len() / 3 == before + added, "{added}");
        assert!(moved > 0.1 && moved < 0.4, "its sides' middles come out towards the circle: {moved}");
        // Every point on the sides is nearer the circle the octagon was drawn for.
        let (r, apothem) = (4.0f32, 4.0 * (std::f32::consts::PI / 8.0).cos());
        let mut nearest = f32::MAX;
        for t in mesh.indices.chunks_exact(3) {
            for &i in t {
                let p = Vec3::from_array(mesh.vertices[i as usize].pos);
                if p.z > 0.5 && p.z < 19.5 {
                    let radius = p.truncate().length();
                    assert!(radius <= r + 0.05, "bowed past the circle: {radius}");
                    nearest = nearest.min(radius);
                }
            }
        }
        assert!(nearest > apothem + 0.05, "the sides' middles come out towards the circle: {nearest} vs {apothem}");
        // The cap, flat, is as it was.
        for t in mesh.indices.chunks_exact(3) {
            let p = [t[0], t[1], t[2]].map(|i| Vec3::from_array(mesh.vertices[i as usize].pos));
            if p.iter().all(|p| p.z.abs() < 1e-4) {
                assert!(p.iter().all(|p| cap.iter().any(|c| (*c - *p).length() < 1e-5)), "the cap is not cut");
            }
        }
    }

    /// The cut barrel is whole: every edge of its sides is shared by two triangles but the rims
    /// at its ends (no crack where facets meet, across a side's diagonal or between sides), and
    /// every triangle keeps its facet's winding.
    #[test]
    fn neighbours_meet_on_their_shared_edges_and_keep_their_winding() {
        let mut mesh = barrel();
        round(&mut mesh);
        let mut edges: HashMap<(Key, Key), usize> = HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            let p = [t[0], t[1], t[2]].map(|i| Vec3::from_array(mesh.vertices[i as usize].pos));
            if p.iter().all(|p| p.z.abs() < 1e-4) {
                continue; // (The cap.)
            }
            for k in 0..3 {
                let (a, b) = (key(p[k]), key(p[(k + 1) % 3]));
                *edges.entry(if a <= b { (a, b) } else { (b, a) }).or_default() += 1;
            }
        }
        let rim = |k: &Key| k[2] == 0 || k[2] == 20 * 4096;
        for ((a, b), count) in &edges {
            let expected = if rim(a) && rim(b) && a[2] == b[2] { 1 } else { 2 };
            assert_eq!(*count, expected, "edge {a:?} {b:?}");
        }
        for t in mesh.indices.chunks_exact(3) {
            let v = [t[0], t[1], t[2]].map(|i| &mesh.vertices[i as usize]);
            let p = v.map(|v| Vec3::from_array(v.pos));
            let face = -(p[1] - p[0]).cross(p[2] - p[0]);
            if face.length_squared() < 1e-8 { continue; }
            let shading = v.iter().map(|v| normal_of(v)).sum::<Vec3>();
            assert!(face.dot(shading) > 0.0, "a cut triangle faces the way its facet did");
        }
    }

    #[test]
    fn halves_read_back() {
        for x in [0.0f32, 0.25, 1.0, 3.5, 100.0, 1000.0] {
            assert!((half(f16_bits(x)) - x).abs() <= x * 1e-3 + 1e-6, "{x}");
        }
    }
}
