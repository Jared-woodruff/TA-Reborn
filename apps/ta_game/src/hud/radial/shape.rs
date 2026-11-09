//! The radial menu's shapes as meshes. A sector of the ring is tessellated as a grid of rows
//! (inner edge to outer) and columns (across its angle), so that every vertex can be given a light
//! of its own: that is what shades the plates (a dome lit from the top left), bevels their rims,
//! and lets a texture lie inside the shape rather than over its bounding box.
//!
//! A sector is cut as a machined plate is: its sides are parallel to the line between it and its
//! neighbour (so the gap is as wide at the rim as at the hub, not wider), and its corners are
//! round. Everything here is plain geometry, with no egui context to draw in (the drawing is
//! `paint.rs`), so it is tested for what it should be: whole, inside the ring, and where the
//! pointer's picking (`radial::pick`) says it is.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Pos2, TextureId, Vec2, pos2, vec2};

/// Where the light comes from on the screen: the top left, as the icon atlas is lit
/// (`tools/hud/icons.py`).
pub const LIGHT: Vec2 = vec2(-0.6459, -0.7634);

/// The angle between the points of a curve drawn as a polyline (radians): fine enough that no
/// facet shows at the largest the ring is drawn.
pub const STEP: f32 = 0.1;

/// Where a point at `angle` (clockwise from straight up) and `radius` from `centre` lies.
pub fn polar(centre: Pos2, angle: f32, radius: f32) -> Pos2 {
    centre + vec2(angle.sin(), -angle.cos()) * radius
}

/// How squarely a surface whose outward normal is `normal` faces the light: 1 towards it, -1 away.
pub fn facing(normal: Vec2) -> f32 {
    normal.x * LIGHT.x + normal.y * LIGHT.y
}

/// The segments a curve of `span` radians is cut into.
pub fn segments(span: f32) -> usize {
    ((span.abs() / STEP).ceil() as usize).clamp(2, 96)
}

/// How many rows each round corner is cut into.
const CORNER_ROWS: usize = 3;

/// A sector of the ring: where it lies and how it is cut.
#[derive(Clone, Copy, Debug)]
pub struct Sector {
    pub centre: Pos2,
    /// The radii of its inner and outer edge.
    pub r0: f32,
    pub r1: f32,
    /// The lines the gaps to its neighbours are centred on (clockwise from up, `a0 < a1`).
    pub a0: f32,
    pub a1: f32,
    /// How wide the gap between neighbours is (points): the sides run parallel to its middle line.
    pub gap: f32,
    /// The radius of its four corners (points).
    pub corner: f32,
}

/// One row of a sector's grid.
#[derive(Clone, Copy, Debug)]
struct Row {
    r: f32,
    lo: f32,
    hi: f32,
}

impl Sector {
    /// The sector's rows, inner edge first: where the straight sides are, a row at the inner
    /// and outer edge, and between them the quarter circle of each corner.
    fn rows(&self) -> Vec<Row> {
        let depth = (self.r1 - self.r0).max(0.0);
        let corner = self.corner.clamp(0.0, depth / 2.0);
        let mut at: Vec<(f32, f32)> = Vec::new();
        if corner >= 0.75 {
            // (How far along the radius, and how far in from the side, each step of the arc is.)
            let arc = |k: usize| {
                let theta = std::f32::consts::FRAC_PI_2 * k as f32 / CORNER_ROWS as f32;
                (corner * (1.0 - theta.cos()), corner * (1.0 - theta.sin()))
            };
            at.extend((0..=CORNER_ROWS).map(|k| arc(k)).map(|(d, inset)| (self.r0 + d, inset)));
            at.extend((0..=CORNER_ROWS).rev().map(|k| arc(k)).map(|(d, inset)| (self.r1 - d, inset)));
        } else {
            at.extend([(self.r0, 0.0), (self.r1, 0.0)]);
        }
        at.dedup_by(|b, a| (b.0 - a.0).abs() < 1e-3 && (b.1 - a.1).abs() < 1e-3);
        at.into_iter()
            .map(|(r, inset)| {
                // (The side is a straight line parallel to the gap's middle one: at radius r the
                // angle it makes with it is the arcsine of its distance over r.)
                let delta = ((self.gap / 2.0 + inset) / r.max(1.0)).clamp(0.0, 1.0).asin();
                let (lo, hi) = (self.a0 + delta, self.a1 - delta);
                if lo < hi { Row { r, lo, hi } } else { Row { r, lo: (self.a0 + self.a1) / 2.0, hi: (self.a0 + self.a1) / 2.0 } }
            })
            .collect()
    }

    /// The sector as a grid of triangles.
    pub fn tessellate(&self) -> Tess {
        let rows = self.rows();
        let cols = segments(self.a1 - self.a0);
        let mut points = Vec::with_capacity(rows.len() * (cols + 1));
        let mut param = Vec::with_capacity(points.capacity());
        let depth = (self.r1 - self.r0).max(1e-3);
        for row in &rows {
            for k in 0..=cols {
                let u = k as f32 / cols as f32;
                points.push(polar(self.centre, row.lo + (row.hi - row.lo) * u, row.r));
                param.push([u, (row.r - self.r0) / depth]);
            }
        }
        let stride = (cols + 1) as u32;
        let mut indices = Vec::with_capacity((rows.len() - 1) * cols * 6);
        for j in 0..rows.len() as u32 - 1 {
            for k in 0..cols as u32 {
                let a = j * stride + k;
                indices.extend([a, a + 1, a + stride, a + 1, a + stride + 1, a + stride]);
            }
        }
        // The outline, clockwise on the screen: along the outer edge, down the right side, back
        // along the inner edge, up the left.
        let last = rows.len() as u32 - 1;
        let at = |j: u32, k: u32| j * stride + k;
        let mut outline: Vec<u32> = (0..=cols as u32).map(|k| at(last, k)).collect();
        outline.extend((0..last).rev().map(|j| at(j, cols as u32)));
        outline.extend((0..cols as u32).rev().map(|k| at(0, k)));
        outline.extend((1..last).map(|j| at(j, 0)));
        Tess { points, param, indices, outline }
    }
}

/// A shape as a grid: its points, where each lies across it (`u` from the first side to the last,
/// `v` from the inner edge to the outer, both 0 to 1), its triangles and the points of its
/// outline in order, clockwise on the screen.
#[derive(Clone, Debug)]
pub struct Tess {
    pub points: Vec<Pos2>,
    pub param: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub outline: Vec<u32>,
}

impl Tess {
    /// The points of the outline, in order.
    pub fn outline_points(&self) -> Vec<Pos2> {
        self.outline.iter().map(|&i| self.points[i as usize]).collect()
    }

    /// The outline's outward normals, one to each of its points (unit length).
    pub fn outline_normals(&self) -> Vec<Vec2> {
        let p = self.outline_points();
        let n = p.len();
        (0..n)
            .map(|i| {
                let t = p[(i + 1) % n] - p[(i + n - 1) % n];
                let normal = vec2(t.y, -t.x);
                if normal.length() > 1e-6 { normal.normalized() } else { Vec2::ZERO }
            })
            .collect()
    }

    /// The shape filled, each point's colour from `colour(position, u, v)`.
    pub fn fill(&self, colour: impl Fn(Pos2, f32, f32) -> Color32) -> Mesh {
        let vertices = self.points.iter().zip(&self.param).map(|(&p, &[u, v])| Vertex::untextured(p, colour(p, u, v))).collect();
        Mesh { indices: self.indices.clone(), vertices, texture_id: TextureId::default() }
    }

    /// The shape filled with `texture`, laid over the screen at `tile` points a repeat (so that
    /// neighbouring shapes match), tinted.
    pub fn textured(&self, texture: TextureId, tile: f32, tint: Color32) -> Mesh {
        let vertices = self.points.iter().map(|&p| Vertex { pos: p, uv: pos2(p.x / tile, p.y / tile), color: tint }).collect();
        Mesh { indices: self.indices.clone(), vertices, texture_id: texture }
    }

    /// A band along the outline, from `offsets.0` to `offsets.1` points out along the normal
    /// (negative: inside), the colours at its two edges from `colour(index along the outline,
    /// normal)`: a bevel on the inside, a glow or a shadow outside.
    pub fn strip(&self, offsets: (f32, f32), colour: impl Fn(usize, Vec2) -> (Color32, Color32)) -> Mesh {
        let p = self.outline_points();
        let normals = self.outline_normals();
        let n = p.len();
        let mut mesh = Mesh::default();
        for i in 0..n {
            let (a, b) = colour(i, normals[i]);
            mesh.vertices.push(Vertex::untextured(p[i] + normals[i] * offsets.0, a));
            mesh.vertices.push(Vertex::untextured(p[i] + normals[i] * offsets.1, b));
        }
        for i in 0..n as u32 {
            let (a, b) = (2 * i, 2 * ((i + 1) % n as u32));
            mesh.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
        mesh
    }

    /// Whether `p` is inside the outline.
    #[cfg(test)]
    pub fn contains(&self, p: Pos2) -> bool {
        let o = self.outline_points();
        let mut inside = false;
        for i in 0..o.len() {
            let (a, b) = (o[i], o[(i + 1) % o.len()]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    }

    /// The area the triangles cover.
    #[cfg(test)]
    pub fn area(&self) -> f32 {
        self.indices
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (self.points[t[0] as usize], self.points[t[1] as usize], self.points[t[2] as usize]);
                ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0
            })
            .sum()
    }
}

/// A ring (or an arc of one) between radii `r0` and `r1` from angle `a0` to `a1`, in `across`
/// bands, each point's colour from `colour(angle, w)` with `w` from -1 at the inner edge to 1 at the
/// outer: a steel tube bent into a circle, a gauge's track.
pub fn ring(centre: Pos2, (r0, r1): (f32, f32), (a0, a1): (f32, f32), across: usize, colour: impl Fn(f32, f32) -> Color32) -> Mesh {
    let cols = segments(a1 - a0);
    let across = across.max(1);
    let mut mesh = Mesh::default();
    for j in 0..=across {
        let w = -1.0 + 2.0 * j as f32 / across as f32;
        let r = r0 + (r1 - r0) * j as f32 / across as f32;
        for k in 0..=cols {
            let a = a0 + (a1 - a0) * k as f32 / cols as f32;
            mesh.vertices.push(Vertex::untextured(polar(centre, a, r), colour(a, w)));
        }
    }
    let stride = (cols + 1) as u32;
    for j in 0..across as u32 {
        for k in 0..cols as u32 {
            let a = j * stride + k;
            mesh.indices.extend([a, a + 1, a + stride, a + 1, a + stride + 1, a + stride]);
        }
    }
    mesh
}

/// A disc of concentric rings, each at its radius with its colour (the first at the middle): a pool
/// of shade, a lamp's glow.
pub fn disc(centre: Pos2, stops: &[(f32, Color32)]) -> Mesh {
    let cols = 72;
    let mut mesh = Mesh::default();
    for &(r, colour) in stops {
        for k in 0..cols {
            let a = std::f32::consts::TAU * k as f32 / cols as f32;
            mesh.vertices.push(Vertex::untextured(polar(centre, a, r), colour));
        }
    }
    let n = cols as u32;
    // (A disc of fewer than two rings has nothing between them.)
    for j in 0..(stops.len() as u32).saturating_sub(1) {
        for k in 0..n {
            let (a, b) = (j * n + k, j * n + (k + 1) % n);
            mesh.indices.extend([a, b, a + n, b, b + n, a + n]);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sector(count: usize, i: usize, gap: f32, corner: f32) -> Sector {
        let step = std::f32::consts::TAU / count as f32;
        let a = i as f32 * step;
        Sector { centre: pos2(300.0, 300.0), r0: 52.0, r1: 132.0, a0: a - step / 2.0, a1: a + step / 2.0, gap, corner }
    }

    /// Every vertex lies inside the ring between the radii, and the triangles are whole and
    /// reach no vertex that is not there.
    #[test]
    fn a_sector_is_a_whole_grid_inside_the_ring() {
        for count in [1usize, 2, 3, 5, 8, 9, 12] {
            for i in 0..count {
                let s = sector(count, i, 3.0, 5.0);
                let t = s.tessellate();
                assert_eq!(t.points.len(), t.param.len());
                assert_eq!(t.indices.len() % 3, 0);
                assert!(t.indices.iter().all(|&k| (k as usize) < t.points.len()), "{count}/{i}: an index out of range");
                for p in &t.points {
                    assert!(p.x.is_finite() && p.y.is_finite());
                    let r = (*p - s.centre).length();
                    assert!(r >= s.r0 - 1e-2 && r <= s.r1 + 1e-2, "{count}/{i}: {r} outside the ring");
                }
                assert!(t.area() > 0.0);
            }
        }
    }

    /// The outline visits each point of the grid's rim once, in a closed loop, and nothing inside.
    #[test]
    fn the_outline_is_the_grids_rim() {
        let s = sector(8, 3, 3.0, 5.0);
        let t = s.tessellate();
        let mut seen = t.outline.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), t.outline.len(), "no point twice");
        // (Along the outer and inner edge, and up each side.)
        let (rows, cols) = (s.rows().len(), segments(s.a1 - s.a0));
        assert_eq!(t.outline.len(), 2 * cols + 2 * (rows - 1));
        let normals = t.outline_normals();
        assert_eq!(normals.len(), t.outline.len());
        assert!(normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-3), "unit normals");
    }

    /// The normals point out of the shape: a step along each lands outside it, a step against it inside.
    #[test]
    fn the_normals_point_outwards() {
        for count in [2usize, 5, 9] {
            let t = sector(count, 1, 3.0, 5.0).tessellate();
            let (points, normals) = (t.outline_points(), t.outline_normals());
            for (p, n) in points.iter().zip(&normals) {
                assert!(!t.contains(*p + *n * 1.5), "{count}: out of the shape at {p:?}");
                assert!(t.contains(*p - *n * 1.5), "{count}: in the shape at {p:?}");
            }
        }
    }

    /// The point where the pointer's picking puts a choice (the middle of its sector, half way
    /// between the edges) is on the plate that is drawn there, whatever the count.
    #[test]
    fn a_plate_covers_the_place_the_pointer_picks_it_at() {
        for count in 1..=9usize {
            for i in 0..count {
                let a = crate::radial::angle(i, count);
                let s = if count == 1 { Sector { a0: a - 3.0, a1: a + 3.0, ..sector(1, 0, 3.0, 5.0) } } else { sector(count, i, 3.0, 5.0) };
                let t = s.tessellate();
                let mid = polar(s.centre, a, (s.r0 + s.r1) / 2.0);
                assert!(t.contains(mid), "{i} of {count}");
                // ...and a point between neighbours is on neither.
                if count > 1 {
                    let between = polar(s.centre, a + std::f32::consts::PI / count as f32, (s.r0 + s.r1) / 2.0);
                    assert!(!t.contains(between), "{i} of {count}: the gap");
                }
            }
        }
    }

    /// The gap between two neighbours is as wide at the rim as at the hub: each side runs parallel
    /// to the line between them, half the gap from it.
    #[test]
    fn the_gap_between_neighbours_does_not_widen_outwards() {
        let s = sector(8, 0, 4.0, 0.0);
        let divider = polar(s.centre, s.a1, 1.0) - s.centre;
        let rows = s.rows();
        assert_eq!(rows.len(), 2, "square corners: a row at each edge");
        for row in rows {
            let edge = polar(s.centre, row.hi, row.r) - s.centre;
            let distance = (edge.x * divider.y - edge.y * divider.x).abs();
            assert!((distance - 2.0).abs() < 0.01, "at {}: {distance} from the line between them", row.r);
        }
    }

    /// Round corners cut the plate's corners: less area than the square-cornered one, by about what
    /// four corners of that radius take.
    #[test]
    fn round_corners_take_the_corners_off() {
        let square = sector(8, 0, 3.0, 0.0).tessellate();
        let round = sector(8, 0, 3.0, 5.0).tessellate();
        let taken = square.area() - round.area();
        let expected = 4.0 * (1.0 - std::f32::consts::FRAC_PI_4) * 25.0;
        assert!(taken > 0.0);
        assert!((taken - expected).abs() < expected * 0.35, "took {taken}, about {expected} expected");
        // (And the corner is round: the outline has points off the square's corner.)
        assert!(round.outline.len() > square.outline.len());
    }

    /// A sector too thin for its corners, or too narrow, still comes out whole.
    #[test]
    fn a_thin_sector_does_not_fold() {
        let thin = Sector { centre: pos2(0.0, 0.0), r0: 50.0, r1: 53.0, a0: 0.0, a1: 0.5, gap: 3.0, corner: 6.0 };
        let narrow = Sector { centre: pos2(0.0, 0.0), r0: 50.0, r1: 130.0, a0: 0.0, a1: 0.03, gap: 3.0, corner: 5.0 };
        for s in [thin, narrow] {
            let t = s.tessellate();
            assert!(t.points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
            assert!(t.indices.iter().all(|&k| (k as usize) < t.points.len()));
        }
    }

    /// A ring of bands and a disc of stops are whole meshes.
    #[test]
    fn rings_and_discs_are_whole() {
        let m = ring(pos2(0.0, 0.0), (40.0, 46.0), (0.0, std::f32::consts::TAU), 4, |a, w| Color32::from_gray(((a + w + 2.0) * 20.0) as u8));
        assert!(m.is_valid() && !m.indices.is_empty());
        let d = disc(pos2(0.0, 0.0), &[(0.0, Color32::BLACK), (50.0, Color32::from_black_alpha(100)), (90.0, Color32::TRANSPARENT)]);
        assert!(d.is_valid() && !d.indices.is_empty());
        // Nothing between fewer than two rings: an empty mesh, not a wrapped count.
        assert!(disc(pos2(0.0, 0.0), &[]).indices.is_empty());
        assert!(disc(pos2(0.0, 0.0), &[(10.0, Color32::WHITE)]).indices.is_empty());
    }

    /// Light from the top left: a surface facing up and left is lit, down and right shaded.
    #[test]
    fn the_light_comes_from_the_top_left() {
        assert!(facing(vec2(-0.7, -0.7)) > 0.95);
        assert!(facing(vec2(0.7, 0.7)) < -0.95);
        assert!(facing(vec2(0.7, -0.7)).abs() < 0.2);
        assert!((LIGHT.length() - 1.0).abs() < 1e-3);
    }

    /// A strip along the outline has two vertices to each of its points and closes on itself.
    #[test]
    fn a_strip_closes_round_the_outline() {
        let t = sector(6, 2, 3.0, 5.0).tessellate();
        let m = t.strip((0.0, -2.0), |_, _| (Color32::WHITE, Color32::TRANSPARENT));
        assert_eq!(m.vertices.len(), 2 * t.outline.len());
        assert_eq!(m.indices.len(), 6 * t.outline.len());
        assert!(m.is_valid());
    }
}
