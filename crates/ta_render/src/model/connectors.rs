//! Connectors: TA Reborn's own columns between pieces TA left apart. TA's high camera hid the air
//! between some pieces: the Arm Construction Kbot's feet hang below its body (and drop further as
//! it strides), the Krogoth's head rests on the tip of a pyramid. A 3D camera looking from the
//! side sees them float. A connector is a round column from one piece to the other: a rod whose
//! ends belong to the two pieces (each end's vertices drawn with its own piece's matrix, so the
//! rod stretches, shortens and swings as the animation moves them), and a sleeve on the first
//! piece where they are apart at rest (a leg's hydraulics, a neck). Hidden with either piece:
//! each vertex knows the piece at the other end (see `piece` below, and unit.wgsl's
//! `piece_matrix`).
//!
//! Which pieces are joined is a list by model, from what the animation scan finds coming apart
//! (`cargo test --release -p ta_sim --test animation_scan joint_scan -- --ignored
//! --nocapture`) checked by eye; where the column goes comes from the pieces' shapes.

use glam::Vec3;

use super::{FAR_FROM_EDGES, ModelVertex, f16_bits, face_normal};

/// The pieces joined, by model (its 3DO's name): the piece a column stands on (its sleeve's)
/// and the piece it reaches, along a vertical line through the second's pivot.
const JOINED: &[(&str, &[Join])] = &[
    // The Arm Construction Kbot: two feet under its body.
    ("armck", &[Join::new("pelvis", "rfoot").radius(1.7), Join::new("pelvis", "lfoot").radius(1.7)]),
    // The Arm Advanced Construction Kbot: hips under its body.
    ("armack", &[Join::new("torso", "rthigh"), Join::new("torso", "lthigh")]),
    // Bodies riding up off a flat pelvis as they walk: a waist.
    ("corpyro", &[Join::new("pelvis", "torso")]),
    ("corthud", &[Join::new("pelvis", "torso")]),
    // Legs, arms and guns their walk takes away from what they hang from.
    ("corack", &[Join::new("pelvis", "lforeleg"), Join::new("pelvis", "rforeleg")]),
    ("armpw", &[Join::new("luparm", "lloarm"), Join::new("ruparm", "rloarm")]),
    ("corcom", &[Join::new("ruparm", "biggun")]),
    ("cordecom", &[Join::new("ruparm", "biggun")]),
    ("corfast", &[Join::new("luleg", "llleg"), Join::new("ruleg", "rlleg")]),
    ("armsnipe", &[Join::new("torso", "leftshoulder")]),
    // The Krogoth: a neck under its head.
    ("corkrog", &[Join::new("torso", "head").radius(2.4)]),
];

/// Two pieces joined: the column stands on `a` and reaches `b`.
#[derive(Clone, Copy, Debug)]
struct Join {
    a: &'static str,
    b: &'static str,
    /// The sleeve's radius, where the pieces' sizes do not give a good one.
    radius: Option<f32>,
}

impl Join {
    const fn new(a: &'static str, b: &'static str) -> Self {
        Join { a, b, radius: None }
    }

    const fn radius(self, radius: f32) -> Self {
        Join { radius: Some(radius), ..self }
    }
}

/// The sides of a column.
const SIDES: usize = 10;
/// How far a column's ends reach into its pieces (model units, at most).
const INSET: f32 = 0.6;
/// Apart by this much at rest (model units), a column has a sleeve.
const SLEEVED: f32 = 1.0;
/// The rod is this much of the sleeve's radius.
const ROD: f32 = 0.6;
/// The sleeve: dark steel, as the caps closing TA's holes; the rod: polished.
const SLEEVE_METAL: ([u8; 3], f32, f32) = ([58, 58, 55], 0.55, 0.62);
const ROD_METAL: ([u8; 3], f32, f32) = ([132, 134, 136], 0.9, 0.28);

/// A model's connectors: vertices in their pieces' frames and triangles over them.
#[derive(Default)]
pub(super) struct Connectors {
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
}

/// A vertex's `piece` for a connector: its own piece in the low 16 bits, the piece at the
/// connector's other end (plus one) in the high 16 (the shader hides the connector with it).
fn piece(own: usize, other: usize) -> u32 {
    own as u32 | ((other as u32 + 1) << 16)
}

/// The piece a vertex is drawn with (a connector's carries another in its high bits).
pub fn own_piece(piece: u32) -> u32 {
    piece & 0xFFFF
}

/// The connectors of the model called `name` (its 3DO's name), if it has any: `names`, its
/// pieces' names; `faces`, each piece's faces in its own frame; `rest`, each piece's pivot at
/// rest in the model.
pub(super) fn build(name: &str, names: &[&str], faces: &[Vec<Vec<Vec3>>], rest: &[Vec3]) -> Connectors {
    let mut out = Connectors::default();
    let Some((_, joins)) = JOINED.iter().find(|(model, _)| model.eq_ignore_ascii_case(model_name(name))) else { return out };
    let find = |piece: &str| names.iter().position(|n| n.eq_ignore_ascii_case(piece));
    for join in *joins {
        let (Some(a), Some(b)) = (find(join.a), find(join.b)) else {
            log::warn!("connector {}/{} of {name}: no such piece", join.a, join.b);
            continue;
        };
        if let Some(column) = Column::between(&faces[a], rest[a], &faces[b], rest[b], join.radius) {
            column.emit(a, b, rest, &mut out);
        }
    }
    out
}

/// "objects3d/ARMCK.3do" → "ARMCK".
fn model_name(name: &str) -> &str {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.strip_suffix(".3do").or_else(|| file.strip_suffix(".3DO")).unwrap_or(file)
}

/// Where a column goes, in the model at rest: on a vertical line, from `from` (on piece a) to
/// `to` (on piece b), its sleeve (if any) from `from` to `sleeve_end`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Column {
    from: Vec3,
    to: Vec3,
    radius: f32,
    sleeve_end: Option<Vec3>,
}

impl Column {
    /// The column between pieces a and b (their faces in their own frames, their pivots at rest),
    /// along the vertical line through b's pivot: from a's surface facing b to b's facing a,
    /// each end a little into its piece. None when the line meets neither.
    fn between(a_faces: &[Vec<Vec3>], a_at: Vec3, b_faces: &[Vec<Vec3>], b_at: Vec3, radius: Option<f32>) -> Option<Column> {
        let model = |faces: &[Vec<Vec3>], at: Vec3| -> Vec<Vec<Vec3>> { faces.iter().map(|f| f.iter().map(|&p| p + at).collect()).collect() };
        let (a, b) = (model(a_faces, a_at), model(b_faces, b_at));
        let (a_lo, a_hi) = bounds(&a)?;
        let (b_lo, b_hi) = bounds(&b)?;
        let (x, z) = (b_at.x, b_at.z);
        // Which way b lies from a, and the surfaces facing each other on the line (else the
        // pieces' bounds there).
        let below = (b_lo.y + b_hi.y) < (a_lo.y + a_hi.y);
        let (ha, hb) = (heights(&a, x, z), heights(&b, x, z));
        if ha.is_empty() && hb.is_empty() {
            return None;
        }
        let pick = |h: &[f32], low: bool, fallback: f32| -> f32 {
            let it = h.iter().copied();
            if h.is_empty() { fallback } else if low { it.fold(f32::MAX, f32::min) } else { it.fold(f32::MIN, f32::max) }
        };
        let (ya, yb) = if below { (pick(&ha, true, a_lo.y), pick(&hb, false, b_hi.y)) } else { (pick(&ha, false, a_hi.y), pick(&hb, true, b_lo.y)) };
        let down = if below { -1.0 } else { 1.0 };
        // (Into each piece: a against the way to b, b along it.)
        let inset = |lo: Vec3, hi: Vec3| INSET.min((hi.y - lo.y) * 0.3);
        let from = Vec3::new(x, ya - down * inset(a_lo, a_hi), z);
        let to = Vec3::new(x, yb + down * inset(b_lo, b_hi), z);
        let gap = (yb - ya) * down;
        let narrowest = |lo: Vec3, hi: Vec3| (hi.x - lo.x).min(hi.z - lo.z);
        let radius = radius.unwrap_or_else(|| (0.28 * narrowest(a_lo, a_hi).min(narrowest(b_lo, b_hi))).clamp(0.6, 3.0));
        let sleeve_end = (gap >= SLEEVED).then(|| Vec3::new(x, ya + down * gap * 0.45, z));
        Some(Column { from, to, radius, sleeve_end })
    }

    /// The column's faces: the rod from a's end (drawn with a) to b's (drawn with b), and the
    /// sleeve on a with its end closed.
    fn emit(&self, a: usize, b: usize, rest: &[Vec3], out: &mut Connectors) {
        let rod = self.radius * ROD;
        tube(out, (self.from, a, b), (self.to, b, a), rod, ROD_METAL, rest);
        if let Some(end) = self.sleeve_end {
            tube(out, (self.from, a, b), (end, a, b), self.radius, SLEEVE_METAL, rest);
            let down = (end - self.from).normalize_or(Vec3::NEG_Y);
            disc(out, end, down, self.radius, rod, (a, b), SLEEVE_METAL, rest);
        }
    }
}

/// The bounds of faces, if any.
fn bounds(faces: &[Vec<Vec3>]) -> Option<(Vec3, Vec3)> {
    let mut points = faces.iter().flatten().copied().peekable();
    points.peek()?;
    Some(points.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// Where faces meet the vertical line through (x, z): their heights there.
fn heights(faces: &[Vec<Vec3>], x: f32, z: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for face in faces {
        for k in 1..face.len().saturating_sub(1) {
            let (p, q, r) = (face[0], face[k], face[k + 1]);
            // (Barycentric co-ordinates in the ground's plane; upright faces have none.)
            let den = (q.z - r.z) * (p.x - r.x) + (r.x - q.x) * (p.z - r.z);
            if den.abs() < 1e-6 {
                continue;
            }
            let u = ((q.z - r.z) * (x - r.x) + (r.x - q.x) * (z - r.z)) / den;
            let v = ((r.z - p.z) * (x - r.x) + (p.x - r.x) * (z - r.z)) / den;
            if u >= -1e-4 && v >= -1e-4 && u + v <= 1.0 + 1e-4 {
                out.push(u * p.y + v * q.y + (1.0 - u - v) * r.y);
            }
        }
    }
    out
}

/// A vertex of a connector: at `at` in the model (drawn with piece `own`, the other end's piece
/// `other`), facing `normal`, in `metal`.
fn vertex(at: Vec3, normal: Vec3, (own, other): (usize, usize), metal: ([u8; 3], f32, f32), rest: &[Vec3]) -> ModelVertex {
    let ([r, g, b], metallic, roughness) = metal;
    ModelVertex {
        pos: (at - rest[own]).to_array(),
        normal: [(normal.x * 127.0) as i8, (normal.y * 127.0) as i8, (normal.z * 127.0) as i8, (roughness * 127.0).round() as i8],
        uv: [0, 0],
        // Untextured (one frame), a physical finish.
        layer: 0xFFFF | (1 << 16) | (1 << 28),
        color: [r, g, b, (metallic * 255.0).round() as u8],
        piece: piece(own, other),
        edge_dist: [f16_bits(FAR_FROM_EDGES); 4],
        edge_weight: [0; 4],
        tread: [0.0; 2],
    }
}

/// A face of a connector, `corners` (each with its vertex) wound as TA's faces are (facing
/// `outward`), fanned as the model's faces are.
fn face(out: &mut Connectors, mut corners: Vec<(Vec3, ModelVertex)>, outward: Vec3) {
    let points: Vec<Vec3> = corners.iter().map(|(p, _)| *p).collect();
    if face_normal(&points).dot(outward) < 0.0 {
        corners.reverse();
    }
    let start = out.vertices.len() as u32;
    out.vertices.extend(corners.into_iter().map(|(_, v)| v));
    for k in 1..(out.vertices.len() as u32 - start) - 1 {
        out.indices.extend_from_slice(&[start, start + k + 1, start + k]);
    }
}

/// A round tube from one end to the other (each end's point, its piece and the other end's),
/// its sides only (its ends lie in its pieces, or are closed by `disc`).
fn tube(out: &mut Connectors, (p, p_own, p_other): (Vec3, usize, usize), (q, q_own, q_other): (Vec3, usize, usize), radius: f32, metal: ([u8; 3], f32, f32), rest: &[Vec3]) {
    let axis = (q - p).normalize_or(Vec3::Y);
    let (u, w) = axis.any_orthonormal_pair();
    let round = |k: usize| {
        let t = k as f32 / SIDES as f32 * std::f32::consts::TAU;
        u * t.cos() + w * t.sin()
    };
    for k in 0..SIDES {
        let (r0, r1) = (round(k), round(k + 1));
        let corner = |at: Vec3, n: Vec3, ends: (usize, usize)| (at, vertex(at, n, ends, metal, rest));
        face(
            out,
            vec![corner(p + r0 * radius, r0, (p_own, p_other)), corner(p + r1 * radius, r1, (p_own, p_other)), corner(q + r1 * radius, r1, (q_own, q_other)), corner(q + r0 * radius, r0, (q_own, q_other))],
            (r0 + r1).normalize_or(r0),
        );
    }
}

/// The ring closing a sleeve's end round its rod: from `inner` to `outer` radius at `at`, facing
/// `facing`, drawn with the sleeve's piece.
#[allow(clippy::too_many_arguments)]
fn disc(out: &mut Connectors, at: Vec3, facing: Vec3, outer: f32, inner: f32, ends: (usize, usize), metal: ([u8; 3], f32, f32), rest: &[Vec3]) {
    let (u, w) = facing.any_orthonormal_pair();
    let round = |k: usize| {
        let t = k as f32 / SIDES as f32 * std::f32::consts::TAU;
        u * t.cos() + w * t.sin()
    };
    for k in 0..SIDES {
        let (r0, r1) = (round(k), round(k + 1));
        let corner = |p: Vec3| (p, vertex(p, facing, ends, metal, rest));
        face(out, vec![corner(at + r0 * inner), corner(at + r1 * inner), corner(at + r1 * outer), corner(at + r0 * outer)], facing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box's faces in its own frame: `lo` to `hi`.
    fn cube(lo: Vec3, hi: Vec3) -> Vec<Vec<Vec3>> {
        let c = |x: bool, y: bool, z: bool| Vec3::new(if x { hi.x } else { lo.x }, if y { hi.y } else { lo.y }, if z { hi.z } else { lo.z });
        vec![
            vec![c(false, true, false), c(true, true, false), c(true, true, true), c(false, true, true)],
            vec![c(false, false, false), c(false, false, true), c(true, false, true), c(true, false, false)],
            vec![c(false, false, false), c(true, false, false), c(true, true, false), c(false, true, false)],
            vec![c(false, false, true), c(false, true, true), c(true, true, true), c(true, false, true)],
            vec![c(false, false, false), c(false, true, false), c(false, true, true), c(false, false, true)],
            vec![c(true, false, false), c(true, false, true), c(true, true, true), c(true, true, false)],
        ]
    }

    /// A foot hanging below a body: the column runs from the body's underside down into the
    /// foot's top, on the line through the foot's pivot, with a sleeve on the body.
    #[test]
    fn a_column_bridges_a_foot_hanging_below_a_body() {
        let body = cube(Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 8.0, 10.0));
        let foot = cube(Vec3::new(-2.0, -6.0, -4.0), Vec3::new(2.0, 0.0, 4.0));
        let (body_at, foot_at) = (Vec3::new(0.0, 10.0, 0.0), Vec3::new(-5.0, 6.0, 1.0));
        let column = Column::between(&body, body_at, &foot, foot_at, None).unwrap();
        // The body's underside at y 10, the foot's top at 6: 4 apart; each end 0.6 into its piece.
        assert!((column.from - Vec3::new(-5.0, 10.6, 1.0)).length() < 1e-4, "{column:?}");
        assert!((column.to - Vec3::new(-5.0, 5.4, 1.0)).length() < 1e-4, "{column:?}");
        assert!((column.radius - 0.28 * 4.0).abs() < 1e-4, "the narrower piece's width sets it: {column:?}");
        assert!((column.sleeve_end.unwrap() - Vec3::new(-5.0, 10.0 - 4.0 * 0.45, 1.0)).length() < 1e-4);
        // Its vertices in their own pieces' frames, each knowing the other end's piece.
        let rest = [body_at, foot_at];
        let mut out = Connectors::default();
        column.emit(0, 1, &rest, &mut out);
        assert_eq!(out.indices.len() % 3, 0);
        let (on_body, on_foot): (Vec<&ModelVertex>, Vec<&ModelVertex>) = out.vertices.iter().partition(|v| own_piece(v.piece) == 0);
        assert!(on_body.iter().all(|v| v.piece >> 16 == 2) && on_foot.iter().all(|v| v.piece >> 16 == 1));
        assert!(on_foot.iter().all(|v| (v.pos[1] - (5.4 - 6.0)).abs() < 1e-4), "the rod's foot end, in the foot's frame");
        assert!(on_body.iter().all(|v| v.pos[1] <= 10.6 - 10.0 + 1e-4 && v.pos[1] >= 10.0 - 1.8 - 10.0 - 1e-4));
    }

    /// Every side faces out as TA's faces do (the triangles the renderer makes from them too).
    #[test]
    fn a_columns_faces_face_outward() {
        let mut out = Connectors::default();
        let rest = [Vec3::ZERO, Vec3::ZERO];
        tube(&mut out, (Vec3::new(0.0, 10.0, 0.0), 0, 1), (Vec3::new(0.0, 0.0, 0.0), 1, 0), 2.0, ROD_METAL, &rest);
        for t in out.indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from_array(out.vertices[i as usize].pos));
            let centre = (a + b + c) / 3.0;
            let outward = Vec3::new(centre.x, 0.0, centre.z);
            // (The renderer's triangle (0, k + 1, k) of a face wound as TA's.)
            assert!(face_normal(&[a, c, b]).dot(outward) > 0.0);
        }
    }

    /// A head resting on a point above a torso: the neck rises from the torso's top into the
    /// head along the head's pivot.
    #[test]
    fn a_neck_rises_into_a_head_above() {
        let torso = cube(Vec3::new(-8.0, 0.0, -6.0), Vec3::new(8.0, 20.0, 6.0));
        let head = cube(Vec3::new(-4.0, 6.0, -4.0), Vec3::new(4.0, 14.0, 4.0));
        let column = Column::between(&torso, Vec3::ZERO, &head, Vec3::new(0.0, 20.0, 0.0), Some(2.0)).unwrap();
        assert!((column.from.y - 19.4).abs() < 1e-4 && (column.to.y - 26.6).abs() < 1e-4, "{column:?}");
        assert_eq!(column.radius, 2.0);
        assert!(column.sleeve_end.is_some_and(|e| e.y > 20.0 && e.y < 26.0));
    }

    /// Every connector listed joins two pieces of its model as installed, and makes a column
    /// (skipped without a TA install).
    #[test]
    fn every_listed_connector_joins_its_models_pieces() {
        let Some(vfs) = crate::model::caps::tests::install() else { return };
        for (model, joins) in JOINED {
            let Some(bytes) = vfs.read_opt(&format!("objects3d/{model}.3do")) else { continue };
            let m = ta_formats::model3do::Model3do::parse(&bytes).unwrap();
            let fixed = |v: [i32; 3]| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / ta_formats::model3do::FIXED_ONE;
            let mut rest: Vec<Vec3> = Vec::new();
            for p in &m.pieces {
                let at = p.parent.map_or(Vec3::ZERO, |q| rest[q]) + fixed(p.offset);
                rest.push(at);
            }
            let faces: Vec<Vec<Vec<Vec3>>> = m.pieces.iter().map(|p| p.primitives.iter().filter(|q| q.indices.len() >= 3).map(|q| q.indices.iter().map(|&i| fixed(p.vertices[i as usize])).collect()).collect()).collect();
            let names: Vec<&str> = m.pieces.iter().map(|p| p.name.as_str()).collect();
            let built = build(model, &names, &faces, &rest);
            assert!(built.indices.len() >= joins.len() * SIDES * 6, "{model}: {} triangles for {} joins", built.indices.len() / 3, joins.len());
            for join in *joins {
                let find = |piece: &str| names.iter().position(|n| n.eq_ignore_ascii_case(piece));
                assert!(find(join.a).is_some() && find(join.b).is_some(), "{model}: {}/{}", join.a, join.b);
            }
        }
    }

    #[test]
    fn models_are_found_by_their_files_name() {
        assert_eq!(model_name("objects3d/ARMCK.3do"), "ARMCK");
        assert_eq!(model_name("corkrog"), "corkrog");
        assert_eq!(own_piece(piece(3, 7)), 3);
        assert_eq!(piece(3, 7) >> 16, 8);
    }
}
