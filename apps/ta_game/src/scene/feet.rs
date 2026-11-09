//! The feet of walkers, measured on their models, for the prints they leave in the ground (the
//! ambient effects lay them, see `effects_ambient.rs`).
//!
//! A walker stands on its feet: the faces of a model's pieces whose lowest vertices lie down at
//! the ground when it rests. Each cluster of such vertices (those of one face, and the faces
//! that share them) is a pad, the rectangle it covers in the model's own axes (x across, z along,
//! ahead positive); a foot is the pads that hang from one leg (a Krogoth's heel and its toes, two
//! pads of one foot). So a Peewee's print is the Peewee's boot, a Commander's its own, a
//! Krogoth's its two pads, and a Sumo's four feet each leave one.
//!
//! What the walker's script hides once it is built is no foot (`still::standing_pose`): TA:
//! Escalation's Commanders carry in their models the body they are upgraded to, legs and guns
//! down to the ground. Nor is a face lying flat on the ground on its own: the same Commanders
//! carry a quad 108 units square there (an explode point for their arrival), Escalation's gate
//! generators one 300 across. Seen from above they are nothing; measured as feet, the quad left a
//! black band behind the Commander. (Where the script has the pieces stand is not taken: its
//! sandbox answers no questions about the world, so a walker that finds the ground with its feet,
//! TA: Escalation's Gorgon, stands nowhere in it, and one turning rings round itself as it arrives,
//! Total Mayhem's Commander, has them down at the ground.)

use glam::Vec3;
use ta_formats::model3do::{FIXED_ONE, Model3do};

/// One pad of a foot as the model rests: its middle across and along the model, and its width
/// across and length along it (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    pub at: [f32; 2],
    pub size: [f32; 2],
}

/// A foot: the pads that hang from one leg.
#[derive(Clone, Debug, PartialEq)]
pub struct Foot {
    pub pads: Vec<Pad>,
}

impl Foot {
    /// Where the foot is, across the model (the mean of its pads' middles).
    pub fn across(&self) -> f32 {
        self.pads.iter().map(|p| p.at[0]).sum::<f32>() / self.pads.len().max(1) as f32
    }

    /// Where the foot is, along the model.
    pub fn along(&self) -> f32 {
        self.pads.iter().map(|p| p.at[1]).sum::<f32>() / self.pads.len().max(1) as f32
    }
}

/// The heights (world units over the model's origin, which TA stands on the ground) between which
/// a vertex counts as part of a sole. Every walker's feet rest within a tenth of a unit of the
/// origin; what hangs lower is a hand, a lathe or a gun, and a sole is up to a couple of units
/// thick.
pub(super) const SOLE: std::ops::RangeInclusive<f32> = -2.0..=2.0;
/// Pads whose rectangles come within this distance of each other are one pad (a heel and a toe
/// made as two pieces, touching).
const JOIN: f32 = 0.75;
/// The least a pad measures either way (a spider's leg ends in a point).
pub(super) const LEAST: f32 = 2.0;
/// How many pad lengths apart the pads of one foot may lie, along and across.
const FOOT_REACH: f32 = 2.2;
/// A pad smaller than this share (by area) of the model's biggest is no foot: a gun or a hand
/// hanging down to the ground, the nub of a heel.
pub(super) const SMALLEST_PAD: f32 = 0.2;
/// A face whose corners all lie at the ground within this much of one height lies flat on it: a
/// sole's own underside or pad in a piece with a face rising from the ground (a leg's, the sole's
/// sides), else a plate (an unflagged selection plate, a shadow, a mod's explode point).
const FLAT: f32 = 0.05;

/// Whether a ground unit walks (leaves footprints, not tread marks): kbots and Commanders.
/// Movement classes can't tell them apart (many kbots use TANKSH2); Upright and the TED class can
/// (a few walkers, the Fido and the Sumo, aren't marked Upright), and the rule for big walkers
/// (`ta_sim::defs::big_walker`: the mods' giant spiders are tanks by their TED class).
pub fn walks(t: &ta_sim::UnitType) -> bool {
    t.def.upright || t.big_walker || matches!(t.def.tedclass.as_str(), "KBOT" | "COMMANDER")
}

/// The pieces of a unit type's model its script hides once the unit is built (as
/// `still::standing_pose` stands it: what it hides is drawn as nothing).
pub fn hidden_when_built(t: &ta_sim::UnitType) -> Vec<bool> {
    super::standing_pose(t).iter().map(|m| *m == glam::Mat4::ZERO).collect()
}

/// Pieces that draw a muzzle's flash, a fire point or a flare, hidden by the script until used;
/// they often hang below the ground (a flash 23 units down), and are no part of a foot.
pub(super) fn is_effect(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["flare", "flash", "fire", "muzzle", "emit", "beam", "spray", "smoke", "puff"].iter().any(|k| n.contains(k))
}

/// A union-find over vertices.
struct Sets(Vec<usize>);

impl Sets {
    fn find(&mut self, mut a: usize) -> usize {
        while self.0[a] != a {
            self.0[a] = self.0[self.0[a]];
            a = self.0[a];
        }
        a
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[b] = a;
        }
    }
}

/// A rectangle on the ground, and the piece most of its vertices belong to.
#[derive(Clone, Copy)]
struct Rect {
    lo: [f32; 2],
    hi: [f32; 2],
    owner: usize,
    weight: usize,
}

/// The feet of a model at rest, in the order a walker lays them (left and right in turn, a
/// quadruped's diagonal pairs), leaving out the pieces `hidden` says its script hides (by the
/// model's pieces; none past its end): none when it draws nothing near the ground.
pub fn measure(m: &Model3do, hidden: &[bool]) -> Vec<Foot> {
    // Where each piece lies at rest (model space: z negated, as the renderer loads it).
    let mut at: Vec<Vec3> = Vec::with_capacity(m.pieces.len());
    for p in &m.pieces {
        let o = Vec3::new(p.offset[0] as f32, p.offset[1] as f32, -(p.offset[2] as f32)) / FIXED_ONE;
        at.push(p.parent.and_then(|q| at.get(q).copied()).unwrap_or(Vec3::ZERO) + o);
    }
    // Every piece's vertices in model space.
    let mut first = Vec::with_capacity(m.pieces.len());
    let mut points: Vec<Vec3> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (k, (p, &origin)) in m.pieces.iter().zip(&at).enumerate() {
        first.push(points.len());
        for v in &p.vertices {
            points.push(origin + Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE);
            owner.push(k);
        }
    }
    // (A face's corners, as indices into `points`.)
    let first = &first;
    let corners = |k: usize, j: usize| {
        let (p, base) = (&m.pieces[k], first[k]);
        let n = p.vertices.len();
        p.primitives[j].indices.iter().filter(move |&&i| (i as usize) < n).map(move |&i| base + i as usize)
    };
    // The faces the model shows: not the selection plate, a degenerate face, an effect piece or a
    // piece the script hides.
    let shown = |k: usize, j: usize| {
        let p = &m.pieces[k];
        !(Some(j) == p.selection_primitive || p.primitives[j].indices.len() < 3 || is_effect(&p.name) || hidden.get(k).copied().unwrap_or(false))
    };
    // A face lying flat on the ground: every corner down at it, at one height.
    let flat = |k: usize, j: usize| {
        let (lo, hi) = corners(k, j).map(|i| points[i].y).fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        SOLE.contains(&lo) && SOLE.contains(&hi) && hi - lo <= FLAT
    };
    // The pieces with a face rising from the ground (a leg, a sole's sides).
    let rises: Vec<bool> = (0..m.pieces.len())
        .map(|k| (0..m.pieces[k].primitives.len()).any(|j| shown(k, j) && !flat(k, j) && corners(k, j).any(|i| SOLE.contains(&points[i].y))))
        .collect();
    // The faces that bear on the ground: those shown that rise from it, and those lying flat on it
    // in a piece that has one (a sole's underside, the pad an Invader's leg ends in); in a piece
    // without, a face lying flat on the ground is a plate.
    let bearing = |k: usize, j: usize| shown(k, j) && (!flat(k, j) || rises[k]);
    let mut drawn = vec![false; points.len()];
    for (k, p) in m.pieces.iter().enumerate() {
        for j in (0..p.primitives.len()).filter(|&j| bearing(k, j)) {
            for i in corners(k, j) {
                drawn[i] = true;
            }
        }
    }
    let low = |i: usize| drawn[i] && SOLE.contains(&points[i].y);
    // The vertices of one face that are down at the ground belong to one pad.
    let mut sets = Sets((0..points.len()).collect());
    for (k, p) in m.pieces.iter().enumerate() {
        for j in (0..p.primitives.len()).filter(|&j| bearing(k, j)) {
            let mut down = corners(k, j).filter(|&i| low(i));
            if let Some(a) = down.next() {
                for b in down {
                    sets.join(a, b);
                }
            }
        }
    }
    // Each set's rectangle.
    let mut rects: Vec<Rect> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    let mut votes: Vec<Vec<(usize, usize)>> = Vec::new();
    for i in (0..points.len()).filter(|&i| low(i)) {
        let root = sets.find(i);
        let k = roots.iter().position(|&r| r == root).unwrap_or_else(|| {
            roots.push(root);
            rects.push(Rect { lo: [f32::MAX; 2], hi: [f32::MIN; 2], owner: owner[i], weight: 0 });
            votes.push(Vec::new());
            roots.len() - 1
        });
        let r = &mut rects[k];
        r.lo = [r.lo[0].min(points[i].x), r.lo[1].min(points[i].z)];
        r.hi = [r.hi[0].max(points[i].x), r.hi[1].max(points[i].z)];
        match votes[k].iter_mut().find(|v| v.0 == owner[i]) {
            Some(v) => v.1 += 1,
            None => votes[k].push((owner[i], 1)),
        }
    }
    for (r, v) in rects.iter_mut().zip(&votes) {
        // (The piece most of the set's vertices belong to; the first of a tie.)
        let (piece, weight) = v.iter().copied().fold((v[0].0, 0), |best, c| if c.1 > best.1 { c } else { best });
        r.owner = piece;
        r.weight = weight;
    }
    // Rectangles that touch are one pad (unless they lie either side of the middle: two feet side
    // by side, as a Fark's, are two).
    let side = |r: &Rect| {
        let across = r.lo[0] + r.hi[0];
        if across.abs() < 1.0 { 0 } else if across > 0.0 { 1 } else { -1 }
    };
    'merge: loop {
        for a in 0..rects.len() {
            for b in a + 1..rects.len() {
                let (x, y) = (rects[a], rects[b]);
                if side(&x) * side(&y) >= 0 && (0..2).all(|k| x.lo[k] <= y.hi[k] + JOIN && y.lo[k] <= x.hi[k] + JOIN) {
                    let keep = if x.weight >= y.weight { x } else { y };
                    rects[a] = Rect { lo: [x.lo[0].min(y.lo[0]), x.lo[1].min(y.lo[1])], hi: [x.hi[0].max(y.hi[0]), x.hi[1].max(y.hi[1])], owner: keep.owner, weight: x.weight + y.weight };
                    rects.swap_remove(b);
                    continue 'merge;
                }
            }
        }
        break;
    }
    // The pads that hang from one leg, stand on one side and lie near one another (within a few
    // of their lengths) are a foot.
    let pads: Vec<(usize, Pad)> = rects
        .iter()
        .map(|r| (r.owner, Pad { at: [(r.lo[0] + r.hi[0]) * 0.5, (r.lo[1] + r.hi[1]) * 0.5], size: [(r.hi[0] - r.lo[0]).max(LEAST), (r.hi[1] - r.lo[1]).max(LEAST)] }))
        .collect();
    let biggest = pads.iter().map(|(_, p)| p.size[0] * p.size[1]).fold(0.0, f32::max);
    let mut feet: Vec<(usize, bool, Vec<Pad>)> = Vec::new();
    for &(owner, pad) in pads.iter().filter(|(_, p)| p.size[0] * p.size[1] >= SMALLEST_PAD * biggest) {
        let (leg, left) = (m.pieces[owner].parent.unwrap_or(owner), pad.at[0] >= 0.0);
        let near = |q: &Pad| (0..2).all(|k| (pad.at[k] - q.at[k]).abs() <= FOOT_REACH * pad.size[1].max(q.size[1]));
        match feet.iter_mut().find(|f| f.0 == leg && f.1 == left && f.2.iter().any(near)) {
            Some(f) => f.2.push(pad),
            None => feet.push((leg, left, vec![pad])),
        }
    }
    let mut feet: Vec<Foot> = feet.into_iter().map(|f| Foot { pads: f.2 }).collect();
    for f in &mut feet {
        f.pads.sort_by(|a, b| a.at[1].total_cmp(&b.at[1]).then(a.at[0].total_cmp(&b.at[0])));
    }
    in_step(feet)
}

/// Feet in the order they come down: left and right in turn; with more (a crab's, a spider's),
/// each side's from the back forward, the sides alternating and the second side's the other way
/// (a diagonal pair at a time).
fn in_step(feet: Vec<Foot>) -> Vec<Foot> {
    let (mut left, mut right): (Vec<Foot>, Vec<Foot>) = feet.into_iter().partition(|f| f.across() >= 0.0);
    left.sort_by(|a, b| a.along().total_cmp(&b.along()));
    right.sort_by(|a, b| b.along().total_cmp(&a.along()));
    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut l, mut r) = (left.into_iter(), right.into_iter());
    loop {
        match (l.next(), r.next()) {
            (None, None) => break,
            (a, b) => out.extend(a.into_iter().chain(b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::model3do::{Piece, Primitive};

    const ONE: i32 = FIXED_ONE as i32;

    /// A box piece `w` wide, `h` tall and `l` long (file axes: z away from the unit's front)
    /// standing on its origin: five faces (no underside), as TA draws them.
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

    /// A walker: a body on two legs, each ending in a foot `w` x `l` (the file's z is the unit's
    /// -z), and a muzzle flash hanging 20 units under the ground.
    fn walker(w: f32, l: f32) -> Model3do {
        let mut pieces = vec![
            boxy("body", None, [0.0, 12.0, 0.0], 10.0, 8.0, 6.0),
            boxy("lleg", Some(0), [4.0, -12.0, 0.0], 2.0, 12.0, 2.0),
            boxy("lfoot", Some(1), [0.0, 0.0, 0.0], w, 1.5, l),
            boxy("rleg", Some(0), [-4.0, -12.0, 0.0], 2.0, 12.0, 2.0),
            boxy("rfoot", Some(3), [0.0, 0.0, 0.0], w, 1.5, l),
            boxy("flare", Some(0), [0.0, -32.0, 0.0], 3.0, 20.0, 3.0),
        ];
        for k in 0..pieces.len() {
            if let Some(p) = pieces[k].parent {
                pieces[p].children.push(k);
            }
        }
        Model3do { pieces }
    }

    #[test]
    fn a_walkers_feet_are_the_soles_of_its_legs_and_no_flash_under_them() {
        let feet = measure(&walker(5.0, 9.0), &[]);
        assert_eq!(feet.len(), 2, "{feet:?}");
        for f in &feet {
            assert_eq!(f.pads.len(), 1);
            let p = f.pads[0];
            assert!((p.size[0] - 5.0).abs() < 1e-3 && (p.size[1] - 9.0).abs() < 1e-3, "{p:?}");
            assert!((p.at[0].abs() - 4.0).abs() < 1e-3 && p.at[1].abs() < 1e-3, "{p:?}");
        }
        assert!(feet[0].across() * feet[1].across() < 0.0, "one each side");
    }

    #[test]
    fn pads_of_one_leg_are_one_foot_and_a_pair_of_legs_two() {
        // The left foot a heel and a toe, a gap between, both hung from the left leg.
        let mut m = walker(6.0, 6.0);
        let heel = m.pieces.len();
        m.pieces.push(boxy("ltoes", Some(1), [0.0, 0.0, -11.0], 5.0, 1.5, 4.0));
        m.pieces[1].children.push(heel);
        let feet = measure(&m, &[]);
        assert_eq!(feet.len(), 2);
        let left = feet.iter().find(|f| f.across() > 0.0).unwrap();
        assert_eq!(left.pads.len(), 2, "heel and toes");
        // From the back forward (the file's z runs the other way).
        assert!(left.pads[0].at[1] < left.pads[1].at[1]);
        assert!((left.pads[0].size[1] - 6.0).abs() < 1e-3 && (left.pads[1].size[1] - 4.0).abs() < 1e-3, "the heel, then the toes: {left:?}");
        let right = feet.iter().find(|f| f.across() < 0.0).unwrap();
        assert_eq!(right.pads.len(), 1);
    }

    #[test]
    fn four_feet_come_down_in_diagonal_pairs() {
        let mut m = walker(3.0, 3.0);
        for (name, x, z) in [("lback", 4.0, 6.0), ("rback", -4.0, 6.0)] {
            let leg = m.pieces.len();
            m.pieces.push(boxy(name, Some(0), [x, -12.0, z], 3.0, 1.5, 3.0));
            m.pieces[0].children.push(leg);
        }
        let feet = measure(&m, &[]);
        assert_eq!(feet.len(), 4);
        // Left back, right front, left front, right back: never two of one side in a row.
        assert!(feet.windows(2).all(|w| w[0].across() * w[1].across() < 0.0), "{feet:?}");
    }

    #[test]
    fn two_feet_side_by_side_are_two_and_a_hand_hanging_low_is_no_foot() {
        // Feet 7 wide at +-3.5: their soles touch in the middle, and are not one foot. A gun
        // hanging from the body down to 3 units under the ground is no sole.
        let mut m = walker(7.0, 9.0);
        m.pieces[1].offset[0] = 3 * ONE + ONE / 2;
        m.pieces[3].offset[0] = -3 * ONE - ONE / 2;
        let gun = m.pieces.len();
        m.pieces.push(boxy("gun", Some(0), [9.0, -20.0, 0.0], 2.0, 5.0, 2.0));
        m.pieces[0].children.push(gun);
        let feet = measure(&m, &[]);
        assert_eq!(feet.len(), 2, "{feet:?}");
        assert!(feet.iter().all(|f| f.pads.len() == 1 && (f.pads[0].size[0] - 7.0).abs() < 1e-3), "{feet:?}");
    }

    #[test]
    fn a_hovering_model_has_no_feet() {
        let mut m = walker(5.0, 9.0);
        m.pieces[0].offset[1] += 20 * ONE;
        m.pieces[1].offset[1] = 0;
        m.pieces[3].offset[1] = 0;
        assert!(measure(&m, &[]).is_empty());
    }

    /// A quad `w` x `l` lying flat on its origin, its one face facing down (file axes), as TA:
    /// Escalation's `flip` under its Commanders.
    fn plate(name: &str, parent: Option<usize>, offset: [f32; 3], w: f32, l: f32) -> Piece {
        let (x, z) = (w / 2.0, l / 2.0);
        let v = |x: f32, z: f32| [(x * ONE as f32) as i32, 0, (z * ONE as f32) as i32];
        Piece {
            name: name.into(),
            parent,
            offset: offset.map(|o| (o * ONE as f32) as i32),
            vertices: vec![v(-x, -z), v(-x, z), v(x, z), v(x, -z)],
            primitives: vec![Primitive { color_index: 1, indices: vec![3, 2, 1, 0], texture: None }],
            selection_primitive: None,
            children: Vec::new(),
        }
    }

    /// Adds `piece` to `m`, hung from its parent; its index.
    fn hang(m: &mut Model3do, piece: Piece) -> usize {
        let k = m.pieces.len();
        if let Some(p) = piece.parent {
            m.pieces[p].children.push(k);
        }
        m.pieces.push(piece);
        k
    }

    fn sizes(feet: &[Foot]) -> Vec<[f32; 2]> {
        feet.iter().flat_map(|f| f.pads.iter().map(|p| [(p.size[0] * 10.0).round() / 10.0, (p.size[1] * 10.0).round() / 10.0])).collect()
    }

    #[test]
    fn a_plate_flat_on_the_ground_is_no_foot() {
        // TA: Escalation's Commanders: a quad 108 units square lying on the ground under the
        // walker (an explode point for its arrival), hung from its body. Measured as a sole it
        // was the only foot (the real ones under a fifth of its size), a print 121 units square
        // every stride: a black band behind the Commander.
        let mut m = walker(5.0, 9.0);
        hang(&mut m, plate("flip", Some(0), [0.0, -12.0, 0.0], 108.0, 108.0));
        let feet = measure(&m, &[]);
        assert_eq!(sizes(&feet), vec![[5.0, 9.0], [5.0, 9.0]], "{feet:?}");
        // So is one standing up off the ground on a short stalk, and one facing up: a plate either way.
        let mut up = plate("shadow", Some(0), [0.0, -11.0, 0.0], 40.0, 40.0);
        up.primitives[0].indices.reverse();
        hang(&mut m, up);
        assert_eq!(measure(&m, &[]), feet);
        // A sole's own flat underside changes nothing: its corners are its sides' too.
        for foot in [2, 4] {
            m.pieces[foot].primitives.push(Primitive { color_index: 1, indices: vec![3, 2, 1, 0], texture: None });
        }
        assert_eq!(measure(&m, &[]), feet);
    }

    #[test]
    fn a_flat_pad_on_a_leg_is_the_foot_it_is() {
        // A leg (a thin upright plane coming down to the ground) ending in a flat pad 2.5 x 4 lying
        // on it, the pad's corners its own (TA's Invader): the pad, whole, is its foot.
        let v = |x: f32, y: f32, z: f32| [(x * ONE as f32) as i32, (y * ONE as f32) as i32, (z * ONE as f32) as i32];
        let face = |i: &[u16]| Primitive { color_index: 1, indices: i.to_vec(), texture: None };
        let leg = Piece {
            name: "leg1".into(),
            parent: None,
            offset: [0; 3],
            vertices: vec![v(-1.25, 0.1, -2.0), v(-1.25, 0.1, 2.0), v(1.25, 0.1, 2.0), v(1.25, 0.1, -2.0), v(0.0, 0.15, -0.5), v(0.0, 0.15, 0.5), v(0.0, 6.0, 0.5), v(0.0, 6.0, -0.5)],
            primitives: vec![face(&[3, 2, 1, 0]), face(&[4, 5, 6, 7])],
            selection_primitive: None,
            children: Vec::new(),
        };
        let mut m = Model3do { pieces: vec![leg] };
        assert_eq!(sizes(&measure(&m, &[])), vec![[2.5, 4.0]]);
        // Without the leg the pad is a plate on its own.
        m.pieces[0].primitives.pop();
        assert!(measure(&m, &[]).is_empty());
    }

    #[test]
    fn what_the_script_hides_is_no_foot() {
        // A second, bigger body's legs in the same model (TA: Escalation's Commander and the one
        // it is upgraded to), hidden by the script until then: at rest they merge with the feet.
        let mut m = walker(5.0, 9.0);
        let mut hidden = Vec::new();
        for x in [6.0, -6.0] {
            let leg = hang(&mut m, boxy("leg2", Some(0), [x, -12.0, 0.0], 3.0, 12.0, 3.0));
            hidden.extend([leg, hang(&mut m, boxy("foot2", Some(leg), [0.0, 0.0, 0.0], 9.0, 1.5, 16.0))]);
        }
        assert_eq!(sizes(&measure(&m, &[])), vec![[9.0, 16.0], [9.0, 16.0]], "at rest the hidden feet are measured with the shown");
        let mut shut = vec![false; m.pieces.len()];
        for &k in &hidden {
            shut[k] = true;
        }
        assert_eq!(sizes(&measure(&m, &shut)), vec![[5.0, 9.0], [5.0, 9.0]]);
        // A hidden leg's foot is still a foot (TA hides a piece, not what hangs from it).
        shut[hidden[1]] = false;
        assert_eq!(sizes(&measure(&m, &shut)), vec![[9.0, 16.0], [5.0, 9.0]]);
        // (Pieces past the list's end are shown.)
        assert_eq!(measure(&m, &shut[..2]), measure(&m, &[]));
    }

    /// Every walker of the install (and of the mod `TA_TEST_MOD=<mod folder>` mounted over it;
    /// `TA_UNITS=a,b` for some): its footprint, how many feet and the pads, as the game measures
    /// them (without what its script hides once built), and with those pieces where that differs
    /// (`cargo test --release -p ta_game --bin ta-reborn feet::tests::the_feet_of_every_walker --
    /// --ignored --nocapture`).
    #[test]
    #[ignore = "reads the install"]
    fn the_feet_of_every_walker() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        // (`TA_TEST_MOD`: a mod's folder, mounted over the game as the game mounts it.)
        let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).unwrap().expect("a mod's folder"));
        let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
        mounted.install.unwrap();
        mounted.mods.unwrap();
        let vfs = mounted.vfs;
        let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let (mut walkers, mut without, mut posing) = (0, Vec::new(), std::time::Duration::ZERO);
        let show = |feet: &[Foot]| feet.iter().map(|f| f.pads.iter().map(|p| format!("{:.1}x{:.1}@({:.1},{:.1})", p.size[0], p.size[1], p.at[0], p.at[1])).collect::<Vec<_>>().join("+")).collect::<Vec<_>>().join("  ");
        for t in &types.types {
            if t.locomotion != ta_sim::Locomotion::Ground || !walks(t) || only.as_ref().is_some_and(|o| !o.contains(&t.name)) {
                continue;
            }
            let Some(model) = vfs.read_opt(&format!("objects3d/{}.3do", t.def.object.to_ascii_lowercase())).and_then(|b| Model3do::parse(&b).ok()) else { continue };
            walkers += 1;
            let started = std::time::Instant::now();
            let hidden = hidden_when_built(t);
            posing += started.elapsed();
            let (feet, all) = (measure(&model, &hidden), measure(&model, &[]));
            if feet.is_empty() {
                without.push(t.name.clone());
            }
            println!("{:<10} {}x{} {} feet: {}", t.name, t.footprint.0, t.footprint.1, feet.len(), show(&feet));
            if all != feet {
                println!("{:<10} with what the script hides, {} feet: {}", "", all.len(), show(&all));
            }
        }
        println!("{walkers} walkers ({:.1} ms running their scripts to see what they hide); without feet: {without:?}", posing.as_secs_f64() * 1000.0);
    }
}
