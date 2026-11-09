//! Big walkers' feet on the ground (for the eye alone).
//!
//! Big walkers stand level (`ta_sim::UnitType::level`, `ta_sim::defs::big_walker`): tilted with
//! the slope under their footprint, a cell or a few under legs that reach far beyond it, they
//! rocked with every bump. Drawn level on rough ground, though, their feet would hang over the
//! dips and sink into the rises; so each foot is drawn as far up or down as the ground under it
//! lies from the ground under the walker's middle, which its body stands on, and its leg bends to
//! reach there: the knee bends (or straightens) and the hip swings so that the ankle comes to its
//! place, and the foot keeps the attitude the walk gives it (two bones of inverse kinematics on
//! the leg's own pieces, the knee kept to the side it bends to). A lifted foot moves as much as a
//! planted one: its step goes over the ground under it.
//!
//! Each type's legs are found once, on its model at rest: its feet (the pieces whose faces come
//! down to the ground, read as `feet.rs` reads soles), and for each the pieces from the one hung
//! from its body (what carries another foot too) down to the foot; the hip is the first, the knee
//! the joint furthest off the line from the hip to the ankle. A leg of one piece swings at its hip
//! so that its sole comes to the ground's height (a strut reaching out), or is lifted (a foot hung
//! straight under the body). A foot reaches up or down at most [`REACH`] of its leg's length, and
//! a leg stretches no straighter than it is long.
//!
//! Only the walkers the rule made level are posed so (big walkers without `Upright=1`): TA's
//! upright units keep TA's look, and the mods' walkers that keep their own feet on the ground
//! by script (TA: Enhanced's Cavor, TA: Escalation's walking giants) are upright. The
//! simulation's pieces are untouched.

use glam::{Mat4, Quat, Vec3};
use ta_formats::model3do::{FIXED_ONE, Model3do};

use super::feet::{LEAST as LEAST_PAD, SMALLEST_PAD, SOLE, is_effect};

/// The furthest a foot reaches up or down to the ground, as a share of its leg's length (over a
/// cliff's edge it stays as it is drawn).
pub(crate) const REACH: f32 = 0.35;
/// Smallest correction worth posing (world units).
const LEAST: f32 = 0.05;
/// How much of its length a leg straightens or bends to at most (its bones never quite in line,
/// nor folded flat).
const STRAIGHTEST: f32 = 0.995;
const FOLDED: f32 = 0.02;

/// One leg of a walker.
#[derive(Clone, Debug)]
pub(crate) struct Leg {
    /// What it hangs from: the deepest piece that carries another foot too (the body, a
    /// shoulder).
    body: usize,
    /// Its pieces, from the one hung from the body (the hip) down to the foot.
    chain: Vec<usize>,
    /// Where the knee is in `chain` (0: the leg swings at its hip alone).
    knee: usize,
    /// The pieces the hip, the knee and the foot carry (each itself and all hung from it).
    hip_carries: Vec<usize>,
    knee_carries: Vec<usize>,
    foot_carries: Vec<usize>,
    /// Where the foot touches the ground, in the foot piece's own space.
    sole: Vec3,
}

impl Leg {
    /// The foot's piece.
    pub(crate) fn foot(&self) -> usize {
        self.chain.last().copied().unwrap_or(0)
    }
}

/// Every piece hung from `piece` (and itself), by the model's parents (listed before their
/// children, as TA's models are).
fn carried(m: &Model3do, piece: usize) -> Vec<usize> {
    let mut out = vec![piece];
    for (i, p) in m.pieces.iter().enumerate().skip(piece + 1) {
        if p.parent.is_some_and(|q| out.contains(&q)) {
            out.push(i);
        }
    }
    out
}

/// The legs of a walker's model at rest (none for a model with fewer than two): its feet, the
/// pieces whose faces come down to the ground (as `feet.rs` reads soles, but each piece its own: a
/// Juggernaut's four feet, side by side, are four; toes hung from a foot go with the foot), and
/// each one's leg, the pieces from the one hung from what carries another foot too (the body; for
/// an arm a walker leans on, its shoulders) down to it.
pub(crate) fn legs(m: &Model3do) -> Vec<Leg> {
    // Where each piece lies at rest (model space: z negated, as the renderer loads it).
    let mut at: Vec<Vec3> = Vec::with_capacity(m.pieces.len());
    for p in &m.pieces {
        let o = Vec3::new(p.offset[0] as f32, p.offset[1] as f32, -(p.offset[2] as f32)) / FIXED_ONE;
        at.push(p.parent.and_then(|q| at.get(q).copied()).unwrap_or(Vec3::ZERO) + o);
    }
    // Each piece's sole: the rectangle (across, along) its drawn faces' vertices at the ground
    // cover.
    let soles: Vec<Option<([f32; 2], [f32; 2])>> = m
        .pieces
        .iter()
        .zip(&at)
        .map(|(p, &origin)| {
            if is_effect(&p.name) {
                return None;
            }
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for (j, prim) in p.primitives.iter().enumerate() {
                if Some(j) == p.selection_primitive || prim.indices.len() < 3 {
                    continue;
                }
                for v in prim.indices.iter().filter_map(|&i| p.vertices.get(i as usize)) {
                    let v = origin + Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE;
                    if SOLE.contains(&v.y) {
                        (lo, hi) = ([lo[0].min(v.x), lo[1].min(v.z)], [hi[0].max(v.x), hi[1].max(v.z)]);
                    }
                }
            }
            (lo[0] <= hi[0]).then_some((lo, hi))
        })
        .collect();
    let area = |s: &([f32; 2], [f32; 2])| (s.1[0] - s.0[0]).max(LEAST_PAD) * (s.1[1] - s.0[1]).max(LEAST_PAD);
    let biggest = soles.iter().flatten().map(area).fold(0.0, f32::max);
    // Each piece's line from the model's root down to it.
    let line = |mut piece: usize| {
        let mut out = vec![piece];
        while let Some(p) = m.pieces.get(piece).and_then(|p| p.parent) {
            if out.len() > m.pieces.len() {
                break;
            }
            out.push(p);
            piece = p;
        }
        out.reverse();
        out
    };
    // The feet: pieces standing on the ground, not hung from another such (a gun or a hand barely
    // touching it is none, as in `feet.rs`).
    let standing: Vec<usize> = (0..m.pieces.len()).filter(|&i| soles[i].as_ref().is_some_and(|s| area(s) >= SMALLEST_PAD * biggest)).collect();
    let feet: Vec<usize> = standing.iter().copied().filter(|&i| !line(i)[..line(i).len() - 1].iter().any(|a| standing.contains(a))).collect();
    let lines: Vec<Vec<usize>> = feet.iter().map(|&f| line(f)).collect();
    let mut legs = Vec::new();
    for (k, &foot_piece) in feet.iter().enumerate() {
        // What carries another foot too is the body (or a shoulder): the leg is what carries this
        // one alone.
        let carries_another = |piece: usize| lines.iter().enumerate().any(|(j, l)| j != k && l.contains(&piece));
        let l = &lines[k];
        let Some(body) = l.iter().rposition(|&p| carries_another(p)) else { continue };
        let chain: Vec<usize> = l[body + 1..].to_vec();
        if chain.is_empty() {
            continue;
        }
        let Some((lo, hi)) = soles[foot_piece] else { continue };
        let sole = Vec3::new((lo[0] + hi[0]) * 0.5, 0.0, (lo[1] + hi[1]) * 0.5) - at[foot_piece];
        // The ankle: the foot's own joint; with a foot of its own, the knee is the joint furthest
        // off the line from the hip to it.
        let knee = if chain.len() >= 3 {
            let (hip, ankle) = (at[chain[0]], at[foot_piece]);
            let line = (ankle - hip).normalize_or_zero();
            let off = |p: Vec3| {
                let d = p - hip;
                (d - line * d.dot(line)).length()
            };
            (1..chain.len() - 1).max_by(|&a, &b| off(at[chain[a]]).total_cmp(&off(at[chain[b]]))).unwrap_or(0)
        } else {
            0
        };
        legs.push(Leg {
            body: l[body],
            hip_carries: carried(m, chain[0]),
            knee_carries: if knee > 0 { carried(m, chain[knee]) } else { Vec::new() },
            foot_carries: carried(m, foot_piece),
            knee,
            chain,
            sole,
        });
    }
    if legs.len() < 2 { Vec::new() } else { legs }
}

/// Turns the pieces `carries` (world matrices in `out`) by `q` about `pivot`.
fn turn(out: &mut [Mat4], carries: &[usize], pivot: Vec3, q: Quat) {
    let x = Mat4::from_translation(pivot) * Mat4::from_quat(q) * Mat4::from_translation(-pivot);
    for &i in carries {
        if let Some(m) = out.get_mut(i) {
            *m = x * *m;
        }
    }
}

fn joint(out: &[Mat4], piece: usize) -> Option<Vec3> {
    let m = out.get(piece)?;
    (m.determinant().abs() > 1e-9).then(|| m.w_axis.truncate())
}

/// Brings a walker's feet to the ground: `out` its pieces' world matrices as posed (level, its
/// middle standing on the ground at height `base`), `forward` the way it faces, `ground` the
/// ground's height at a place. Each foot is moved up or down by how far the ground under it lies
/// from `base`, its leg bent to reach there.
pub(crate) fn plant(legs: &[Leg], out: &mut [Mat4], base: f32, forward: Vec3, ground: &dyn Fn(f32, f32) -> f32) {
    for leg in legs {
        let (Some(&foot), Some(&hip_piece)) = (leg.chain.last(), leg.chain.first()) else { continue };
        let Some(m) = out.get(foot).copied().filter(|m| m.determinant().abs() > 1e-9) else { continue };
        let (Some(hip), Some(ankle)) = (joint(out, hip_piece), joint(out, foot)) else { continue };
        let sole = m.transform_point3(leg.sole);
        // (How long the leg is: from the body it hangs from to its sole.)
        let span = (sole - joint(out, leg.body).unwrap_or(hip)).length();
        if !span.is_finite() || span < 1.0 {
            continue;
        }
        let dh = (ground(sole.x, sole.z) - base).clamp(-REACH * span, REACH * span);
        if !dh.is_finite() || dh.abs() < LEAST {
            continue;
        }
        let knee = joint(out, leg.chain[leg.knee]).filter(|&k| leg.knee > 0 && (k - hip).length() >= 0.5 && (ankle - k).length() >= 0.5);
        let Some(knee) = knee else {
            if leg.chain.len() == 1 {
                let v = sole - hip;
                if glam::Vec2::new(v.x, v.z).length() >= v.y.abs() {
                    // A strut reaching out from the body: it swings its sole to the ground.
                    swing(out, &leg.hip_carries, hip, sole, sole.y + dh);
                } else {
                    // A foot hung under the body (a Juggernaut's, in its housing): up or down, as
                    // its walk moves it.
                    let x = Mat4::from_translation(Vec3::Y * dh);
                    for &i in &leg.hip_carries {
                        if let Some(m) = out.get_mut(i) {
                            *m = x * *m;
                        }
                    }
                }
            } else if let Some(q) = swing(out, &leg.hip_carries, hip, ankle, ankle.y + dh) {
                // (A thigh and a foot: the thigh swings the ankle to its height; the foot keeps
                // its attitude.)
                let now = joint(out, foot).unwrap_or(ankle);
                turn(out, &leg.foot_carries, now, q.inverse());
            }
            continue;
        };
        let (a, b) = ((knee - hip).length(), (ankle - knee).length());
        // Where the ankle goes, as near it as the leg reaches.
        let wanted = ankle + Vec3::Y * dh;
        let to = wanted - hip;
        let d = to.length().clamp((a - b).abs() + FOLDED * (a + b), STRAIGHTEST * (a + b));
        let Some(x) = to.try_normalize() else { continue };
        // The knee to the side it bends to now (a leg in line: forward).
        let line = (ankle - hip).normalize_or_zero();
        let bent = knee - hip;
        let side = bent - line * bent.dot(line) + forward * (0.01 * a);
        let side = side - x * side.dot(x);
        let Some(y) = side.try_normalize() else { continue };
        let along = (a * a - b * b + d * d) / (2.0 * d);
        let knee_to = hip + x * along + y * (a * a - along * along).max(0.0).sqrt();
        let ankle_to = hip + x * d;
        // The hip swings the knee there, the knee the ankle; the foot keeps its attitude.
        let (Some(from), Some(towards)) = ((knee - hip).try_normalize(), (knee_to - hip).try_normalize()) else { continue };
        let at_hip = Quat::from_rotation_arc(from, towards);
        turn(out, &leg.hip_carries, hip, at_hip);
        let swung = hip + at_hip * (ankle - hip);
        let (Some(from), Some(towards)) = ((swung - knee_to).try_normalize(), (ankle_to - knee_to).try_normalize()) else { continue };
        let at_knee = Quat::from_rotation_arc(from, towards);
        turn(out, &leg.knee_carries, knee_to, at_knee);
        let now = joint(out, foot).unwrap_or(ankle_to);
        turn(out, &leg.foot_carries, now, (at_knee * at_hip).inverse());
    }
}

/// Swings what a hip at `hip` carries so that its point `point` comes to height `y` (as near as
/// it reaches), keeping its distance from the hip and its way round: the turn it took.
fn swing(out: &mut [Mat4], carries: &[usize], hip: Vec3, point: Vec3, y: f32) -> Option<Quat> {
    let v = point - hip;
    let length = v.length();
    let flat = glam::Vec2::new(v.x, v.z);
    let across = flat.length();
    if length < 1.0 || across < 1e-3 {
        return None;
    }
    let dy = (y - hip.y).clamp(-STRAIGHTEST * length, STRAIGHTEST * length);
    let reach = (length * length - dy * dy).max(0.0).sqrt();
    let flat = flat / across * reach;
    let to = Vec3::new(flat.x, dy, flat.y);
    let q = Quat::from_rotation_arc(v / length, to.try_normalize()?);
    turn(out, carries, hip, q);
    Some(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::feet::measure;
    use ta_formats::model3do::{Piece, Primitive};

    const ONE: i32 = FIXED_ONE as i32;

    /// A box piece `w` wide, `h` tall and `l` long standing on its origin (file axes).
    fn boxy(name: &str, parent: Option<usize>, offset: [f32; 3], w: f32, h: f32, l: f32) -> Piece {
        let (x, z) = (w / 2.0, l / 2.0);
        let v = |x: f32, y: f32, z: f32| [(x * ONE as f32) as i32, (y * ONE as f32) as i32, (z * ONE as f32) as i32];
        let face = |i: &[u16]| Primitive { color_index: 1, indices: i.to_vec(), texture: None };
        Piece {
            name: name.into(),
            parent,
            offset: offset.map(|o| (o * ONE as f32) as i32),
            vertices: vec![v(-x, 0.0, -z), v(x, 0.0, -z), v(x, 0.0, z), v(-x, 0.0, z), v(-x, h, -z), v(x, h, -z), v(x, h, z), v(-x, h, z)],
            primitives: vec![face(&[4, 5, 6, 7]), face(&[0, 1, 5, 4]), face(&[1, 2, 6, 5]), face(&[2, 3, 7, 6]), face(&[3, 0, 4, 7])],
            selection_primitive: None,
            children: Vec::new(),
        }
    }

    fn link(mut pieces: Vec<Piece>) -> Model3do {
        for k in 0..pieces.len() {
            if let Some(p) = pieces[k].parent {
                pieces[p].children.push(k);
            }
        }
        Model3do { pieces }
    }

    /// A piece that draws nothing (a model's root).
    fn empty(name: &str) -> Piece {
        Piece { name: name.into(), parent: None, offset: [0; 3], vertices: Vec::new(), primitives: Vec::new(), selection_primitive: None, children: Vec::new() }
    }

    /// A biped 60 tall: a body, and each leg a thigh from the hip (40 up, 10 to the side), a shin
    /// from the knee (20 up, 8 ahead: the knee bent forward) and a foot from the ankle (2 up).
    fn biped() -> Model3do {
        let mut pieces = vec![empty("base"), boxy("body", Some(0), [0.0, 40.0, 0.0], 24.0, 20.0, 12.0)];
        for (side, x) in [("l", 10.0), ("r", -10.0)] {
            let thigh = pieces.len();
            pieces.push(boxy(&format!("{side}thigh"), Some(1), [x, 0.0, 0.0], 4.0, 1.0, 4.0));
            // (The file's z runs back: -8 is ahead.)
            pieces.push(boxy(&format!("{side}shin"), Some(thigh), [0.0, -20.0, -8.0], 4.0, 1.0, 4.0));
            pieces.push(boxy(&format!("{side}foot"), Some(thigh + 1), [0.0, -20.0, 8.0], 6.0, 2.0, 10.0));
        }
        link(pieces)
    }

    /// A pod on four struts, each one piece from its hip on the body (30 up) to its sole 40 out.
    fn pod() -> Model3do {
        let mut pieces = vec![empty("base"), boxy("body", Some(0), [0.0, 30.0, 0.0], 20.0, 20.0, 20.0)];
        for (x, z) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let mut strut = boxy("leg", Some(1), [x * 8.0, 0.0, z * 8.0], 4.0, 1.0, 4.0);
            // From the hip out and down to the sole, 32 out each way and 30 down.
            for v in &mut strut.vertices {
                v[0] += (x * 32.0 * ONE as f32) as i32;
                v[2] += (z * 32.0 * ONE as f32) as i32;
                v[1] -= 30 * ONE;
            }
            pieces.push(strut);
        }
        link(pieces)
    }

    /// The world matrices of a model at rest, its root at `root`.
    fn rest(m: &Model3do, root: Mat4) -> Vec<Mat4> {
        let mut out: Vec<Mat4> = Vec::new();
        for p in &m.pieces {
            let o = Vec3::new(p.offset[0] as f32, p.offset[1] as f32, -(p.offset[2] as f32)) / FIXED_ONE;
            let local = Mat4::from_translation(o);
            out.push(match p.parent {
                Some(q) => out[q] * local,
                None => root * local,
            });
        }
        out
    }

    fn soles(legs: &[Leg], out: &[Mat4]) -> Vec<Vec3> {
        legs.iter().map(|l| out[*l.chain.last().unwrap()].transform_point3(l.sole)).collect()
    }

    #[test]
    fn a_bipeds_legs_run_from_its_hips_to_its_feet_and_bend_at_the_knee() {
        let m = biped();
        let legs = legs(&m);
        assert_eq!(legs.len(), 2);
        for l in &legs {
            assert_eq!(l.chain.len(), 3, "thigh, shin, foot: {l:?}");
            assert_eq!(l.knee, 1, "the shin's joint is the knee");
            assert!(l.sole.y.abs() < 0.01 && (l.sole.x.abs() < 0.01), "under the ankle: {:?}", l.sole);
        }
    }

    /// On ground sloping up to the left (the model's +x), the left foot is set on the higher ground
    /// and the right on the lower, each by its knee and hip: the hips stay where they are, the
    /// knees stay bent forward, and the feet keep their attitude.
    #[test]
    fn feet_come_down_on_the_ground_under_them() {
        let m = biped();
        let legs = legs(&m);
        let root = Mat4::from_translation(Vec3::new(100.0, 50.0, 200.0));
        let posed = rest(&m, root);
        let mut out = posed.clone();
        let ground = |x: f32, _z: f32| 50.0 + (x - 100.0) * 0.2;
        plant(&legs, &mut out, 50.0, Vec3::Z, &ground);
        for (l, s) in legs.iter().zip(soles(&legs, &out)) {
            assert!((s.y - ground(s.x, s.z)).abs() < 0.05, "sole at {s}, ground {}", ground(s.x, s.z));
            let hip = l.chain[0];
            assert!((out[hip].w_axis - posed[hip].w_axis).length() < 1e-3, "the hip stays");
            let (k0, k1) = (posed[l.chain[1]].w_axis.truncate(), out[l.chain[1]].w_axis.truncate());
            assert!(k1.z > posed[hip].w_axis.z && k0.z > posed[hip].w_axis.z, "the knee forward: {k1}");
            let foot = l.chain[2];
            let (r0, r1) = (posed[foot].to_scale_rotation_translation().1, out[foot].to_scale_rotation_translation().1);
            assert!(r0.angle_between(r1) < 1e-3, "the foot keeps its attitude");
        }
        // Level ground: nothing moves.
        let mut flat = posed.clone();
        plant(&legs, &mut flat, 50.0, Vec3::Z, &|_, _| 50.0);
        assert!(flat.iter().zip(&posed).all(|(a, b)| a.abs_diff_eq(*b, 1e-5)));
    }

    /// Toes hung from a foot go with the foot: the leg runs from the hip to the foot.
    #[test]
    fn a_heel_and_its_toes_are_one_foot() {
        let mut m = biped();
        for foot in [4, 7] {
            let toes = m.pieces.len();
            m.pieces.push(boxy("toes", Some(foot), [0.0, 0.0, -12.0], 6.0, 2.0, 5.0));
            m.pieces[foot].children.push(toes);
        }
        let legs = legs(&m);
        assert_eq!(legs.len(), 2);
        assert!(legs.iter().all(|l| [4, 7].contains(&l.foot()) && l.chain.len() == 3), "{legs:?}");
    }

    /// Four feet hung straight from the body, the two of a side close enough to be taken for one
    /// foot (a Juggernaut's): each is brought to the ground under it, up or down.
    #[test]
    fn feet_taken_for_one_come_down_each() {
        let mut pieces = vec![empty("base"), boxy("body", Some(0), [0.0, 30.0, 0.0], 40.0, 30.0, 60.0), empty("feet")];
        pieces[2].parent = Some(0);
        for (x, z) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            pieces.push(boxy("foot", Some(2), [x * 22.0, 6.0, z * 20.0], 12.0, 4.0, 20.0));
            for v in &mut pieces.last_mut().unwrap().vertices {
                v[1] -= 6 * ONE;
            }
        }
        let m = link(pieces);
        let feet = measure(&m, &[]);
        assert_eq!(feet.len(), 2, "two of a side taken for one: {feet:?}");
        let legs = legs(&m);
        assert_eq!(legs.len(), 4, "{legs:?}");
        let mut out = rest(&m, Mat4::IDENTITY);
        let ground = |x: f32, z: f32| x * 0.1 + z * 0.15;
        plant(&legs, &mut out, 0.0, Vec3::Z, &ground);
        for s in soles(&legs, &out) {
            assert!((s.y - ground(s.x, s.z)).abs() < 0.01, "sole at {s}");
        }
    }

    #[test]
    fn a_strut_swings_its_sole_to_the_ground() {
        let m = pod();
        let legs = legs(&m);
        assert_eq!(legs.len(), 4);
        assert!(legs.iter().all(|l| l.chain.len() == 1 && l.knee == 0));
        let mut out = rest(&m, Mat4::IDENTITY);
        let ground = |x: f32, z: f32| x * 0.1 - z * 0.05;
        plant(&legs, &mut out, 0.0, Vec3::Z, &ground);
        // (A strut keeps its length: its sole comes in or goes out a little, onto ground a little
        // higher or lower than where it stood.)
        for s in soles(&legs, &out) {
            assert!((s.y - ground(s.x, s.z)).abs() < 0.5, "sole at {s}");
        }
    }

    /// Over a hole deeper than it reaches, a foot comes down as far as its leg goes (straight, not
    /// further), and over a cliff's foot it stays where it was drawn; nothing comes apart.
    #[test]
    fn a_leg_reaches_no_further_than_it_is_long() {
        let m = biped();
        let legs = legs(&m);
        let posed = rest(&m, Mat4::IDENTITY);
        for depth in [-15.0, -400.0, 400.0] {
            let mut out = posed.clone();
            plant(&legs, &mut out, 0.0, Vec3::Z, &|_, _| depth);
            for (l, s) in legs.iter().zip(soles(&legs, &out)) {
                assert!(s.is_finite());
                let (hip, knee, ankle) = (out[l.chain[0]].w_axis.truncate(), out[l.chain[1]].w_axis.truncate(), out[l.chain[2]].w_axis.truncate());
                let (a, b) = ((knee - hip).length(), (ankle - knee).length());
                let (a0, b0) = ((posed[l.chain[1]].w_axis - posed[l.chain[0]].w_axis).length(), (posed[l.chain[2]].w_axis - posed[l.chain[1]].w_axis).length());
                assert!((a - a0).abs() < 1e-3 && (b - b0).abs() < 1e-3, "the bones keep their lengths");
                assert!((ankle - hip).length() <= a + b + 1e-3);
                // (From the body to the sole, 41.2.)
                assert!(s.y >= -REACH * 41.3 && s.y <= REACH * 41.3, "{depth}: within its reach, {s}");
            }
        }
    }

    /// The big walkers the rule levels (`TA_TEST_MOD=<mod folder>` for a mod's, `TA_UNITS=a,b` for
    /// others), each walking 1,200 px east across Cinder Basin's rolling southern plateau with its
    /// script's walk: how far the feet its walk has down (within a unit of the lowest it puts each)
    /// are off the ground (as the scene draws it without the map's terrain loaded: the
    /// simulation's heights) against where its walk has them over level ground, tilted as before
    /// (by the slope under its footprint), level, and level with its feet brought down; and how
    /// deep its walk puts a foot. `cargo test --release -p ta_game --bin ta-reborn footing::tests::
    /// feet_on_rough_ground -- --ignored --nocapture` (`TA_TRACE=1` its legs, `ticks` its feet a
    /// second at a time).
    #[test]
    #[ignore = "reads the install"]
    fn feet_on_rough_ground() {
        use ta_sim::{Command, Fx, FxVec3, Order};
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        // (`TA_TEST_MOD`: a mod's folder, mounted over the game as the game mounts it.)
        let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).unwrap().expect("a mod's folder"));
        let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
        mounted.install.unwrap();
        mounted.mods.unwrap();
        let vfs = mounted.vfs;
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        let map = ta_formats::tnt::TntMap::parse(&vfs.read("maps/Cinder Basin.tnt").unwrap()).unwrap();
        let info = ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&vfs.read("maps/Cinder Basin.ota").unwrap()));
        let players: Vec<ta_sim::PlayerSetup> = (0..2)
            .map(|i| ta_sim::PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
            .collect();
        let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
        let p95 = |mut v: Vec<f32>| {
            v.sort_by(|a, b| a.total_cmp(b));
            if v.is_empty() { 0.0 } else { v[((v.len() - 1) as f32 * 0.95) as usize] }
        };
        for t in types.types.iter().filter(|t| only.as_ref().map_or(t.big_walker && !t.def.upright, |o| o.contains(&t.name))) {
            let Some(m) = vfs.read_opt(&format!("objects3d/{}.3do", t.def.object.to_ascii_lowercase())).and_then(|b| Model3do::parse(&b).ok()) else { continue };
            let legs = legs(&m);
            if legs.is_empty() {
                println!("{:<12} no legs to bring down", t.name);
                continue;
            }
            if std::env::var("TA_TRACE").is_ok() {
                for l in &legs {
                    let names: Vec<&str> = l.chain.iter().map(|&i| m.pieces[i].name.as_str()).collect();
                    println!("    leg {names:?}, knee {}, sole {:?}", l.knee, l.sole);
                }
            }
            let mut sim = ta_sim::Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
            let Some(id) = sim.spawn_unit(t.id, 0, FxVec3::from_ints(8700, 0, 13600), 16384, true) else { continue };
            sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(9900, 0, 13600)), queue: false }]);
            let (half_l, half_w) = (t.footprint.1.clamp(1, 8) as f32 * 8.0, t.footprint.0.clamp(1, 8) as f32 * 8.0);
            // Feet down (their walk has them within a unit of the lowest it puts them): how far
            // each is off the ground under it, against where its walk has it (on level ground, the
            // ground), tilted, level, and brought down; and the lowest its walk puts a foot (under
            // its middle).
            let mut samples: Vec<(usize, f32, [f32; 3])> = Vec::new();
            let mut out = Vec::new();
            for _ in 0..1200 {
                sim.step(&[]);
                let Some(u) = sim.unit(id) else { break };
                let world = &sim.world;
                let p = Vec3::from_array(u.pos.to_f32());
                let ground = |x: f32, z: f32| super::super::simulated_ground(world, x, z);
                let h = u.heading as f32 * std::f32::consts::TAU / 65536.0;
                let (s, c) = h.sin_cos();
                let (fwd, left) = (glam::Vec2::new(s, c) * half_l, glam::Vec2::new(c, -s) * half_w);
                let at = |d: glam::Vec2| ground(p.x + d.x, p.z + d.y);
                let (pitch, roll) = (-(at(fwd) - at(-fwd)).atan2(half_l * 2.0), (at(left) - at(-left)).atan2(half_w * 2.0));
                let base = ground(p.x, p.z);
                let root = Mat4::from_translation(Vec3::new(p.x, base, p.z)) * Mat4::from_rotation_y(h);
                super::super::piece_matrices(u, t, root * Mat4::from_rotation_x(pitch.clamp(-0.5, 0.5)) * Mat4::from_rotation_z(roll.clamp(-0.5, 0.5)), &mut out);
                let was = soles(&legs, &out);
                super::super::piece_matrices(u, t, root, &mut out);
                let flat = soles(&legs, &out);
                plant(&legs, &mut out, base, Vec3::new(s, 0.0, c), &ground);
                let now = soles(&legs, &out);
                if std::env::var("TA_TRACE").is_ok_and(|v| v == "ticks") && sim.tick.is_multiple_of(30) {
                    let line: Vec<String> = flat.iter().zip(&now).map(|(f, n)| format!("({:.0},{:.0}) walk {:.1} ground {:.1} now {:.1}", f.x - p.x, f.z - p.z, f.y - base, ground(f.x, f.z) - base, n.y - base)).collect();
                    println!("    {}: {}", sim.tick, line.join("; "));
                }
                for (k, ((w, f), n)) in was.iter().zip(&flat).zip(&now).enumerate() {
                    let walk = f.y - base;
                    samples.push((k, walk, [w.y - ground(w.x, w.z) - walk, f.y - ground(f.x, f.z) - walk, n.y - ground(n.x, n.z) - walk]));
                }
            }
            let lowest: Vec<f32> = (0..legs.len()).map(|k| samples.iter().filter(|s| s.0 == k).map(|s| s.1).fold(f32::MAX, f32::min)).collect();
            let down: Vec<&[f32; 3]> = samples.iter().filter(|s| s.1 <= lowest[s.0] + 1.0).map(|s| &s.2).collect();
            let [tilted, level, planted] = [0, 1, 2].map(|i| down.iter().map(|e| e[i].abs()).collect::<Vec<f32>>());
            let sunk = lowest.iter().copied().fold(0.0f32, f32::min);
            let moved = sim.unit(id).map_or(0.0, |u| u.pos.x.to_f32() - 8700.0);
            println!(
                "{:<12} {} legs, walked {moved:.0} px: feet down off the ground (p95, px) tilted {:.1}, level {:.1}, brought down {:.1}; its walk puts them {:.1} deep",
                t.name,
                legs.len(),
                p95(tilted),
                p95(level),
                p95(planted),
                (-sunk).max(0.0)
            );
        }
    }

    #[test]
    fn hidden_pieces_stay_hidden() {
        let m = biped();
        let legs = legs(&m);
        let mut out = rest(&m, Mat4::IDENTITY);
        out[4] = Mat4::ZERO;
        plant(&legs, &mut out, 0.0, Vec3::Z, &|x, _| x * 0.3);
        assert_eq!(out[4], Mat4::ZERO);
    }
}
