//! What a 3D camera sees of a model's faces from behind: looked at from all round (from a little
//! under the level to high over it), at rest, the faces whose backs are the nearest thing in
//! sight somewhere: the insides of shells through their holes, sheets from their backs. Faces
//! are drawn from the front alone, so where a face's back is nearest one sees past it, through
//! the unit or into it (halves.rs gives such faces their backs; xray.rs measures what is left).

use glam::{Vec2, Vec3};

/// A triangle of the model at rest (model space), its outward normal, and whose it is.
pub(super) struct Tri<T> {
    pub p: [Vec3; 3],
    pub normal: Vec3,
    pub of: T,
}

/// The directions looked from (from the eye into the scene): elevations from a little under the
/// level to high over it, all round, `per_ring` to each.
pub(super) fn directions(elevations: &[f32], per_ring: usize) -> Vec<Vec3> {
    let mut out = Vec::new();
    for (r, &el) in elevations.iter().enumerate() {
        for k in 0..per_ring {
            // (Every other ring turned half a step.)
            let az = std::f32::consts::TAU * (k as f32 + 0.5 * (r % 2) as f32) / per_ring as f32;
            let e = el.to_radians();
            out.push(-Vec3::new(e.cos() * az.sin(), e.sin(), e.cos() * az.cos()));
        }
    }
    out
}

/// Looking along `d` (orthographic, `pixel` model units square, at most `most` across): for each
/// triangle, the area where it is the nearest thing in sight seen from behind (`behind`); the
/// area the model covers. A triangle and one facing the other way in its plane (a sheet and its
/// back, a door of two faces): the one facing the eye is in sight, as the game's culling has it.
pub(super) fn look<T>(tris: &[Tri<T>], d: Vec3, pixel: f32, most: usize, behind: &mut [f32]) -> f32 {
    const TIE: f32 = 0.02;
    if tris.is_empty() {
        return 0.0;
    }
    let (lo, hi) = tris.iter().flat_map(|t| t.p).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let (centre, radius) = ((lo + hi) * 0.5, ((hi - lo) * 0.5).length().max(0.5));
    let n = ((2.0 * radius / pixel).ceil() as usize).clamp(16, most);
    let px = 2.0 * radius / n as f32;
    let u = d.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let v = u.cross(d);
    let mut depth = vec![f32::MAX; n * n];
    let mut what = vec![u32::MAX; n * n];
    for (i, t) in tris.iter().enumerate() {
        let front = t.normal.dot(d) <= 0.05;
        let s: [Vec2; 3] = t.p.map(|p| {
            let q = p - centre;
            Vec2::new((q.dot(u) + radius) / px, (q.dot(v) + radius) / px)
        });
        let z: [f32; 3] = t.p.map(|p| (p - centre).dot(d));
        let area = (s[1] - s[0]).perp_dot(s[2] - s[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let (a, b) = (s[0].min(s[1]).min(s[2]), s[0].max(s[1]).max(s[2]));
        let (x0, x1) = ((a.x - 0.5).ceil().max(0.0) as usize, ((b.x - 0.5).floor() as isize).min(n as isize - 1));
        let (y0, y1) = ((a.y - 0.5).ceil().max(0.0) as usize, ((b.y - 0.5).floor() as isize).min(n as isize - 1));
        if x1 < 0 || y1 < 0 {
            continue;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = (s[2] - s[1]).perp_dot(c - s[1]) / area;
                let w1 = (s[0] - s[2]).perp_dot(c - s[2]) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let zz = w0 * z[0] + w1 * z[1] + w2 * z[2];
                let k = y * n + x;
                if if front { zz < depth[k] + TIE } else { zz < depth[k] - TIE } {
                    depth[k] = zz;
                    what[k] = i as u32;
                }
            }
        }
    }
    let mut covered = 0.0;
    for &w in &what {
        if w == u32::MAX {
            continue;
        }
        covered += px * px;
        if tris[w as usize].normal.dot(d) > 0.05 {
            behind[w as usize] += px * px;
        }
    }
    covered
}
