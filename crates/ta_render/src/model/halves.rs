//! Halves made whole. TA's modellers built each part for a camera high over the battlefield,
//! and many a barrel, cannon, missile rail or crane boom is only the half of it that camera saw:
//! most of TA's guns are square tubes with their undersides left out, the top and the two sides
//! painted to look round from above (the Bulldog's and the Stumpy's barrels), open underneath
//! along their length and at both ends. Faces are drawn from the front alone, so in the 3D camera
//! such a barrel seen from the side or from below is a half-pipe with a lip, seen from in front a
//! lone plate, and seen end on it all but vanishes.
//!
//! When a model loads, before its holes are closed (see caps.rs), each open shell of a piece
//! (faces joined by their edges) that is such a half is made whole by mirroring its faces, their
//! paint with them:
//! - a box missing a side, its faces the sides rising from the opening's edges, a top in one
//!   plane facing away from it and maybe ends: the top mirrored across the plane halfway between
//!   it and the opening (tilted with it where a barrel narrows towards its muzzle) falls on the
//!   opening and closes it, its corners on the opening's. A square tube, its underside painted as
//!   its top.
//! - a round form, a half-pipe of three facets or more (a half-hexagon, a half-octagon), every
//!   corner about as far from the middle of the opening as the opening's edges are: its faces
//!   mirrored across the opening, the lower half under the upper.
//!
//! Only shells open underneath (TA's camera never looked into them, nor sees the faces closing
//! them), open along one plane and at their ends alone, narrow as barrels and booms are and long
//! (a channel, open at its ends, half as long again as it is across, a shell closed at its ends
//! twice as long), with nothing of the model lying on the opening or where the other half goes.
//! A triangle (a gun ridged like a roof, whose fire point lies inside the triangle), a knee or a
//! turret too short to be a barrel, or a hull's missing underside is no such half: caps.rs closes
//! it as before. What is left open once a half is made whole, a channel's ends, caps.rs closes
//! too: a barrel's muzzle a dark bore. Like the caps, it reads nothing but the faces, so mods'
//! models are made whole too, in both art styles.
//!
//! Once the holes are closed, faces whose backs a 3D camera sees (a seaplane's wings, a fin,
//! the insides of what no cap could close) are given their backs (see `give_backs`).
//!
//! The faces these add are late (see `FaceSource::late` in model.rs): drawn after TA's own,
//! which are shaded and fitted as if they weren't there, so that TA's own faces look exactly as
//! they did (`making_forms_whole_leaves_tas_own_faces_as_they_were` checks every installed
//! model). `TA_FORMS=0` leaves TA's forms as it built them.

use glam::Vec3;

use super::caps::{self, Flat, PieceFaces, Polygon};
use super::sight;
use rayon::prelude::*;

/// A corner this near a plane lies in it (model units), more by this share of the shell's
/// width.
const IN_PLANE: f32 = 0.06;
const IN_PLANE_SHARE: f32 = 0.02;
/// The most a shell may be across or deep and be a barrel, a boom or a pipe (model units): a
/// hull, a building's wing or a factory's door is bigger.
const MOST_ACROSS: f32 = 10.0;
/// The least a shell may be across (model units): finer, it is a wire.
const LEAST_ACROSS: f32 = 0.25;
/// How many times as long as it is across or deep a shell must be: a channel (open at its ends,
/// a muzzle) half as long again, one closed at its ends (a box missing its underside) twice as
/// long. (Shorter, it is a box, a knee or a turret, which caps.rs closes.)
const CHANNEL_LONG: f32 = 1.5;
const CLOSED_LONG: f32 = 2.0;
/// A round form's corners lie within these shares of the opening's half-width from the middle
/// of the opening, and it is at least this share of it deep.
const ROUND: (f32, f32) = (0.8, 1.25);
const ROUND_DEEP: f32 = 0.6;

/// A face of a piece mirrored to make a half whole: the face it mirrors (its paint), its corners
/// (piece space), wound as the model's faces, and whether it is that face's back (a sheet's, see
/// `give_backs`).
#[derive(Clone, Debug)]
pub(super) struct Mirrored {
    pub face: usize,
    pub corners: Polygon,
    pub back: bool,
}

/// What became of an open shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Verdict {
    /// A box missing a side: its top mirrored onto it.
    Boxed,
    /// A round form: mirrored across its opening.
    Rounded,
    /// Faces seen from behind: their backs added (see `give_backs`).
    Backed,
    /// Open other than along one plane and at its ends (or barely open).
    Shape,
    /// Its opening faces up or sideways: TA's camera looked into it.
    Up,
    /// Too big, too fine or too short for a barrel.
    Size,
    /// Neither a box missing a side nor round.
    Neither,
    /// Something of the model lies on its opening or where its other half would be.
    InTheWay,
}

/// An open shell of a piece and what became of it.
#[derive(Clone, Debug)]
pub(super) struct Shell {
    /// Its faces (indices into the piece's).
    #[cfg_attr(not(test), expect(dead_code, reason = "for the scan of the installed models"))]
    pub faces: Vec<usize>,
    pub verdict: Verdict,
    /// Across its opening, how deep and how long (model units), where it was measured.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the scan of the installed models"))]
    pub size: [f32; 3],
}

/// A piece's halves made whole: the faces mirrored, and its open shells.
#[derive(Clone, Debug, Default)]
pub(super) struct Halves {
    pub mirrored: Vec<Mirrored>,
    pub shells: Vec<Shell>,
}

/// The model at rest (model space), for what lies on an opening or in the way of a half: its
/// faces' triangles, each with its face's number, and the faces flattened.
struct Model {
    /// Each piece's first face's number.
    first: Vec<usize>,
    triangles: Vec<([Vec3; 3], usize)>,
    flats: Vec<Option<Flat>>,
}

/// The model's pieces' halves made whole.
pub(super) fn complete(pieces: &[PieceFaces]) -> Vec<Halves> {
    // (Built when a shell first asks, as most models have none to make whole.)
    let model = std::cell::OnceCell::new();
    (0..pieces.len()).map(|pi| if pieces[pi].mend { complete_piece(pi, pieces, &model) } else { Halves::default() }).collect()
}

impl Model {
    fn of(pieces: &[PieceFaces]) -> Model {
        let mut model = Model { first: Vec::with_capacity(pieces.len()), triangles: Vec::new(), flats: Vec::new() };
        for p in pieces {
            model.first.push(model.flats.len());
            for face in p.faces {
                // (Effect pieces, a muzzle's flash, are in nobody's way.)
                let id = model.flats.len();
                if p.mend && face.len() >= 3 {
                    model.triangles.extend((1..face.len() - 1).map(|k| ([face[0] + p.offset, face[k] + p.offset, face[k + 1] + p.offset], id)));
                    model.flats.push(Flat::new(face, p.offset));
                } else {
                    model.flats.push(None);
                }
            }
        }
        model
    }
}

/// A piece's shells: its faces welded, each face's shell (faces joined by the edges they share)
/// and the open edges, each with its face, as the face runs it.
fn shells(faces: &[Polygon]) -> (caps::Mesh, Vec<usize>, Vec<(usize, Vec3, Vec3)>) {
    let mut mesh = caps::weld(faces);
    let mut edges = caps::edge_uses(&mesh);
    if caps::split_edges(&mut mesh, &edges) {
        edges = caps::edge_uses(&mesh);
    }
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], mut k: usize) -> usize {
        while parent[k] != k {
            parent[k] = parent[parent[k]];
            k = parent[k];
        }
        k
    }
    let mut open: Vec<(usize, Vec3, Vec3)> = Vec::new();
    for g in caps::groups(&edges) {
        if g.len() == 1 {
            let ((lo, hi), f, forward) = g[0];
            let (a, b) = (mesh.points[lo as usize], mesh.points[hi as usize]);
            open.push((f as usize, if forward { a } else { b }, if forward { b } else { a }));
        } else {
            let first = root(&mut parent, g[0].1 as usize);
            for e in &g[1..] {
                let other = root(&mut parent, e.1 as usize);
                parent[other] = first;
            }
        }
    }
    let shell_of: Vec<usize> = (0..faces.len()).map(|f| root(&mut parent, f)).collect();
    (mesh, shell_of, open)
}

fn complete_piece(pi: usize, pieces: &[PieceFaces], model: &std::cell::OnceCell<Model>) -> Halves {
    let faces = pieces[pi].faces;
    let mut out = Halves::default();
    if faces.len() < 2 || faces.len() > 4096 {
        return out;
    }
    let (mesh, shell_of, open) = shells(faces);
    let mut roots: Vec<usize> = open.iter().map(|&(f, ..)| shell_of[f]).collect();
    roots.sort_unstable();
    roots.dedup();
    for r in roots {
        let members: Vec<usize> = (0..faces.len()).filter(|&f| shell_of[f] == r && !mesh.rings[f].is_empty()).collect();
        if members.len() < 2 {
            continue;
        }
        let edges: Vec<(Vec3, Vec3)> = open.iter().filter(|&&(f, ..)| shell_of[f] == r).map(|&(_, a, b)| (a, b)).collect();
        let (verdict, size, mirrored) = make_whole(pi, pieces, model, &members, &edges);
        out.mirrored.extend(mirrored);
        out.shells.push(Shell { faces: members, verdict, size });
    }
    out
}

/// A shell's opening: its plane (through `origin`, `out` pointing out of it, away from the
/// shell), the shell's axis along it and the way across it; the shell's extent along the axis,
/// across and under the opening.
struct Opening {
    origin: Vec3,
    out: Vec3,
    axis: Vec3,
    across: Vec3,
    along: (f32, f32),
    side: (f32, f32),
    deep: f32,
    /// Corners this near a plane lie in it.
    tol: f32,
}

impl Opening {
    /// How far into the shell `p` lies (0 in the opening's plane).
    fn depth(&self, p: Vec3) -> f32 {
        -(p - self.origin).dot(self.out)
    }

    fn t(&self, p: Vec3) -> f32 {
        (p - self.origin).dot(self.axis)
    }

    fn u(&self, p: Vec3) -> f32 {
        (p - self.origin).dot(self.across)
    }

    fn in_plane(&self, p: Vec3) -> bool {
        self.depth(p).abs() <= self.tol
    }

    /// Whether an edge from `a` to `b` is a lip: in the opening's plane, running along it.
    fn lip(&self, a: Vec3, b: Vec3, along: f32) -> bool {
        self.in_plane(a) && self.in_plane(b) && (b - a).normalize_or_zero().dot(self.axis).abs() >= along
    }
}

/// A plane faces are mirrored across: its normal, and how far along it from the origin it lies.
#[derive(Clone, Copy)]
struct Mirror {
    normal: Vec3,
    at: f32,
}

impl Mirror {
    fn of(&self, p: Vec3) -> Vec3 {
        p - self.normal * 2.0 * (p.dot(self.normal) - self.at)
    }

    /// A face mirrored, wound as the model's faces (the order turned round, as a mirror turns
    /// the winding).
    fn face(&self, corners: &[Vec3]) -> Polygon {
        corners.iter().rev().map(|&p| self.of(p)).collect()
    }
}

/// What a shell is: its opening, if it is open along one plane (and at its ends) alone; else why
/// not. (Its corners, and its open edges as its faces run them.)
fn opening_of(corners: &[Vec3], edges: &[(Vec3, Vec3)]) -> Result<(Opening, bool), Verdict> {
    // The longest open edge runs along the opening (a barrel's lip); the opening's plane holds
    // it and the farthest edge running beside it (the other lip).
    let &(a0, b0) = edges.iter().max_by(|x, y| x.0.distance_squared(x.1).total_cmp(&y.0.distance_squared(y.1))).ok_or(Verdict::Shape)?;
    let axis = (b0 - a0).normalize_or_zero();
    if axis == Vec3::ZERO {
        return Err(Verdict::Shape);
    }
    let mid0 = (a0 + b0) * 0.5;
    let beside = edges
        .iter()
        .filter(|(a, b)| (*b - *a).normalize_or_zero().dot(axis).abs() >= 0.9)
        .map(|(a, b)| {
            let d = (*a + *b) * 0.5 - mid0;
            d - axis * d.dot(axis)
        })
        .max_by(|x, y| x.length_squared().total_cmp(&y.length_squared()))
        .ok_or(Verdict::Shape)?;
    let normal = axis.cross(beside).normalize_or_zero();
    if beside.length() < 2.0 * IN_PLANE || normal == Vec3::ZERO {
        return Err(Verdict::Shape);
    }
    // Out of the opening, away from the shell.
    let mean = corners.iter().map(|p| (*p - mid0).dot(normal)).sum::<f32>() / corners.len().max(1) as f32;
    let out = if mean > 0.0 { -normal } else { normal };
    // The axis halfway between the lips (a sleeve narrowing towards its muzzle has them
    // converging), so that its ends lie square across it.
    let tol = IN_PLANE + IN_PLANE_SHARE * beside.length();
    let lipwise: Vec3 = edges
        .iter()
        .filter(|(a, b)| (*a - mid0).dot(out).abs() <= tol && (*b - mid0).dot(out).abs() <= tol && (*b - *a).normalize_or_zero().dot(axis).abs() >= 0.9)
        .map(|(a, b)| (*b - *a) * (*b - *a).dot(axis).signum())
        .sum();
    let axis = (lipwise - out * lipwise.dot(out)).normalize_or(axis);
    let across = out.cross(axis);
    let range = |f: &dyn Fn(Vec3) -> f32| corners.iter().map(|&p| f(p)).fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
    let along = range(&|p| (p - mid0).dot(axis));
    let side = range(&|p| (p - mid0).dot(across));
    let (low, high) = range(&|p| -(p - mid0).dot(out));
    let tol = IN_PLANE + IN_PLANE_SHARE * (side.1 - side.0);
    let opening = Opening { origin: mid0, out, axis, across, along, side, deep: high, tol };
    // All of the shell on its side of the opening.
    if low < -opening.tol {
        return Err(Verdict::Shape);
    }
    // Every open edge along the opening (a lip) or at an end; the lips run nearly its length on
    // both sides.
    let at_end = |p: Vec3| {
        let t = opening.t(p);
        if (t - along.0).abs() <= opening.tol {
            Some(false)
        } else if (t - along.1).abs() <= opening.tol {
            Some(true)
        } else {
            None
        }
    };
    let (mut lips, mut channel) = (0.0, false);
    for &(a, b) in edges {
        let end = at_end(a).is_some() && at_end(a) == at_end(b);
        if opening.lip(a, b, 0.9) {
            lips += a.distance(b);
        } else if end {
            channel |= !(opening.in_plane(a) && opening.in_plane(b));
        } else {
            return Err(Verdict::Shape);
        }
    }
    if lips < 1.5 * (along.1 - along.0) {
        return Err(Verdict::Shape);
    }
    Ok((opening, channel))
}

/// Whether faces, once welded, are closed along an opening: no edge in its plane running along
/// it that only one face uses (edges across it, at a channel's ends, may be), and every edge
/// shared as a shell's are (by two faces, running it opposite ways).
fn closed_along(faces: &[Polygon], opening: &Opening) -> bool {
    let mut mesh = caps::weld(faces);
    let mut edges = caps::edge_uses(&mesh);
    if caps::split_edges(&mut mesh, &edges) {
        edges = caps::edge_uses(&mesh);
    }
    caps::groups(&edges).all(|g| match g {
        [((lo, hi), ..)] => {
            let (a, b) = (mesh.points[*lo as usize], mesh.points[*hi as usize]);
            !opening.lip(a, b, 0.5)
        }
        [x, y] => x.2 != y.2,
        _ => false,
    })
}

/// A box missing a side, made whole: its faces are its sides (rising from the lips), its top
/// (the rest, in one plane, facing away from the opening) and its ends, if it has them (facing
/// along it). The top mirrored across the plane halfway between it and the opening (tilted with
/// it, where a barrel narrows towards its muzzle) falls on the opening; its corners, each moved
/// onto the corner of the opening it fell by, close it. None if it is no such box.
fn boxed(faces: &[Polygon], members: &[usize], opening: &Opening) -> Option<Vec<(usize, Polygon)>> {
    let across = opening.side.1 - opening.side.0;
    let (mut top, mut low) = (Vec::new(), Vec::new());
    for &f in members {
        let n = caps::plane(&faces[f]).0;
        let touching = faces[f].iter().filter(|&&p| opening.in_plane(p)).count();
        if n.dot(opening.axis).abs() > 0.7 {
            // An end.
        } else if touching == 0 {
            top.push(f);
        } else if touching < 2 {
            return None;
        }
        low.extend(faces[f].iter().copied().filter(|&p| opening.in_plane(p)));
    }
    // The top in one plane, facing away from the opening.
    let &biggest = top.iter().max_by(|&&a, &&b| caps::plane(&faces[a]).1.total_cmp(&caps::plane(&faces[b]).1))?;
    let (normal, _, centre, _) = caps::plane(&faces[biggest]);
    if normal.dot(opening.out) > -0.7 || top.iter().any(|&f| faces[f].iter().any(|&p| (p - centre).dot(normal).abs() > opening.tol)) {
        return None;
    }
    // Halfway between the top's plane and the opening's (the points as far from each, inside).
    let k = normal - opening.out;
    let mirror = Mirror { normal: k.normalize(), at: (centre.dot(normal) - opening.origin.dot(opening.out)) / k.length() };
    let snap = (0.12 * across).max(opening.tol);
    top.iter()
        .map(|&f| {
            let mut corners = mirror.face(&faces[f]);
            for p in corners.iter_mut() {
                let near = low.iter().copied().min_by(|a, b| a.distance_squared(*p).total_cmp(&b.distance_squared(*p)))?;
                if near.distance(*p) > snap {
                    return None;
                }
                *p = near;
            }
            let (n, area, ..) = caps::plane(&corners);
            (area > 1e-3 && n.dot(opening.out) > 0.9).then_some((f, corners))
        })
        .collect()
}

/// Makes a shell of piece `pi` (its faces `members`, its open edges as they run them) whole, if
/// it is a half: what it is, its size, and the faces mirrored.
fn make_whole(pi: usize, pieces: &[PieceFaces], model: &std::cell::OnceCell<Model>, members: &[usize], edges: &[(Vec3, Vec3)]) -> (Verdict, [f32; 3], Vec<Mirrored>) {
    let faces = pieces[pi].faces;
    let offset = pieces[pi].offset;
    let corners: Vec<Vec3> = members.iter().flat_map(|&f| faces[f].iter().copied()).collect();
    let (opening, channel) = match opening_of(&corners, edges) {
        Ok(o) => o,
        Err(verdict) => return (verdict, [0.0; 3], Vec::new()),
    };
    let (across, deep, long) = (opening.side.1 - opening.side.0, opening.deep, opening.along.1 - opening.along.0);
    let size = [across, deep, long];
    let no = |verdict| (verdict, size, Vec::new());
    // Open underneath: TA's camera never looked in.
    if opening.out.y > -caps::SHOWN {
        return no(Verdict::Up);
    }
    let long_enough = long >= if channel { CHANNEL_LONG } else { CLOSED_LONG } * across.max(deep);
    if across > MOST_ACROSS || deep > MOST_ACROSS || across < LEAST_ACROSS || deep < LEAST_ACROSS || !long_enough {
        return no(Verdict::Size);
    }
    // Nothing of the model lying on the opening (a pipe laid on a deck, a plate under a channel).
    // (Points along the middle of the opening.)
    let model = model.get_or_init(|| Model::of(pieces));
    let own = |id: usize| id >= model.first[pi] && members.contains(&(id - model.first[pi]));
    let middle = |share: f32| {
        let t = opening.along.0 + (opening.along.1 - opening.along.0) * share;
        opening.origin + opening.axis * t + opening.across * (opening.side.0 + opening.side.1) * 0.5
    };
    let shares = [0.1, 0.3, 0.5, 0.7, 0.9];
    let covered = shares
        .iter()
        .filter(|&&s| {
            let p = middle(s) + offset;
            model.flats.iter().enumerate().any(|(id, flat)| !own(id) && flat.as_ref().is_some_and(|flat| flat.normal.dot(opening.out).abs() > 0.9 && flat.holds(p, 0.15)))
        })
        .count();
    if covered >= 2 {
        return no(Verdict::InTheWay);
    }
    let shell: Vec<Polygon> = members.iter().map(|&f| faces[f].clone()).collect();
    let whole = |new: &[(usize, Polygon)]| {
        let all: Vec<Polygon> = shell.iter().cloned().chain(new.iter().map(|(_, c)| c.clone())).collect();
        closed_along(&all, &opening)
    };
    let made = |verdict, new: Vec<(usize, Polygon)>| (verdict, size, new.into_iter().map(|(face, corners)| Mirrored { face, corners, back: false }).collect());
    if let Some(boxed) = boxed(faces, members, &opening).filter(|b| whole(b)) {
        return made(Verdict::Boxed, boxed);
    }
    // Round: a prism (its faces along the axis, but for its ends) of three facets or more,
    // every corner about as far from the middle of the opening as the opening's edges are.
    let lips: Vec<f32> = edges.iter().filter(|(a, b)| opening.lip(*a, *b, 0.9)).flat_map(|(a, b)| [opening.u(*a), opening.u(*b)]).collect();
    let (lo, hi) = lips.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &u| (lo.min(u), hi.max(u)));
    let (radius, centre) = ((hi - lo) * 0.5, (hi + lo) * 0.5);
    let mut facets: Vec<Vec3> = Vec::new();
    let mut prism = true;
    for &f in members {
        let n = caps::plane(&faces[f]).0;
        let end = n.dot(opening.axis).abs() > 0.9;
        prism &= end || n.dot(opening.axis).abs() < 0.1;
        if !end && !facets.iter().any(|m| m.dot(n) > 0.99) {
            facets.push(n);
        }
    }
    let round = prism
        && facets.len() >= 3
        && deep >= ROUND_DEEP * radius
        && corners.iter().all(|&p| {
            let r = (opening.u(p) - centre).hypot(opening.depth(p));
            (ROUND.0 * radius..=ROUND.1 * radius).contains(&r)
        });
    if !round {
        return no(Verdict::Neither);
    }
    // Nothing where its lower half goes (it would cut into a hull, a deck): looking out of the
    // opening from its middle, nothing within its depth; nor any face crossing the half's.
    let across_opening = Mirror { normal: opening.out, at: opening.origin.dot(opening.out) };
    let rounded: Vec<(usize, Polygon)> = members.iter().map(|&f| (f, across_opening.face(&faces[f]))).collect();
    let blocked = shares.iter().any(|&s| {
        let from = middle(s) + offset - opening.out * 0.01;
        caps::first_hit(&model.triangles, from, opening.out).is_some_and(|(d, id)| !own(id) && d <= deep)
    });
    let (lo3, hi3) = rounded.iter().flat_map(|(_, c)| c.iter()).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p + offset), hi.max(p + offset)));
    let crossing = model.triangles.iter().filter(|(t, id)| !own(*id) && !(t[0].min(t[1]).min(t[2]).cmpgt(hi3).any() || t[0].max(t[1]).max(t[2]).cmplt(lo3).any())).any(|(t, _)| {
        rounded.iter().any(|(_, c)| {
            (1..c.len() - 1).any(|k| {
                let tri = [c[0] + offset, c[k] + offset, c[k + 1] + offset];
                (0..3).any(|e| caps::pierces(t[e], t[(e + 1) % 3], &tri) || caps::pierces(tri[e], tri[(e + 1) % 3], t))
            })
        })
    });
    if blocked || crossing {
        return no(Verdict::InTheWay);
    }
    if !whole(&rounded) {
        return no(Verdict::Neither);
    }
    made(Verdict::Rounded, rounded)
}

/// A face's back: its corners the other way round from the same first corner, so that drawn as
/// a fan from it (as the model's faces are) it is cut along the same diagonals, and a face TA
/// built bent (a quad whose corners are not in one plane) and its back are one surface.
fn back_of(face: &[Vec3]) -> Polygon {
    face.iter().take(1).chain(face.iter().skip(1).rev()).copied().collect()
}

/// Faces whose backs a 3D camera sees, given their backs: painted as they are, facing the other
/// way. TA built many a part for its camera alone, high over the battlefield: a wing or a lid
/// as one face facing up, a fin as one face, a pit or a dome painted inside, a box with the
/// sides it never saw left out. Faces are drawn from the front alone, so from below a wing is
/// not there at all, a fin vanishes edge on and stays gone, and through a hole the caps could
/// not close (see caps.rs) one sees past the far side's insides, through the unit.
///
/// Each piece is looked at alone, with its caps, from all round as the 3D camera may (from a
/// little under the level to high over it, see sight.rs): alone, because pieces move (a door
/// opens, a nanolathe's case swings out, a turret turns, a leg steps) and what lies over one at
/// rest says nothing of what shows of it when they do. Each face or cap whose back is the
/// nearest thing in sight somewhere is given its back (a cap's is a late cap, see caps.rs):
/// not a face lying flat at the model's foot (the ground is under it: a pad, a hover's skirt
/// facing down, which TA's camera never saw either). And a sheet that is a piece of its own
/// (a seaplane's wings lie folded over its hull until it flies, a hatch's lid opens) is given
/// its back unless it lies at the model's foot.
pub(super) fn give_backs(pieces: &[PieceFaces], mendings: &mut [caps::Mending], halves: &mut [Halves]) {
    // The 3D camera's directions; how fine the look (model units a pixel, pixels at most across).
    const ELEVATIONS: [f32; 6] = [-12.0, 2.0, 16.0, 32.0, 52.0, 75.0];
    const PER_RING: usize = 10;
    const PIXEL: f32 = 0.6;
    const MOST_PIXELS: usize = 160;
    // A face seen from behind over this much (square units, summed over the directions) is given
    // its back: less is a glimpse through a crack where faces nearly meet.
    const SEEN: f32 = 0.75;
    // Too dense a model (a mod's) keeps its faces as they are.
    const MOST_TRIANGLES: usize = 60_000;
    // What a triangle of the model is: a face or a cap of a piece.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Of {
        Face(usize, usize),
        Cap(usize, usize),
    }
    // The model at rest with its caps.
    let mut tris: Vec<sight::Tri<Of>> = Vec::new();
    for (pi, (p, m)) in pieces.iter().zip(mendings.iter()).enumerate().filter(|(_, (p, _))| p.mend) {
        for (f, face) in p.faces.iter().enumerate().filter(|(_, f)| f.len() >= 3) {
            let mut normal = caps::plane(face).0;
            if m.flipped.contains(&f) {
                normal = -normal;
            }
            tris.extend((1..face.len() - 1).map(|k| sight::Tri { p: [face[0] + p.offset, face[k] + p.offset, face[k + 1] + p.offset], normal, of: Of::Face(pi, f) }));
        }
        for (c, cap) in m.caps.iter().enumerate() {
            for t in &cap.triangles {
                let q = t.map(|i| cap.corners[i as usize] + p.offset);
                tris.push(sight::Tri { p: q, normal: caps::plane(&q).0, of: Of::Cap(pi, c) });
            }
        }
    }
    if tris.is_empty() || tris.len() > MOST_TRIANGLES {
        return;
    }
    let foot = tris.iter().flat_map(|t| t.p).map(|p| p.y).fold(f32::MAX, f32::min);
    let at_foot = |corners: &[Vec3], offset: Vec3| corners.iter().all(|q| q.y + offset.y <= foot + 0.5);
    // Each piece looked at alone: the model's pieces move (a door opens, a nanolathe's case
    // swings out, a turret turns, a leg steps), and what lies over a piece at rest says nothing of
    // what shows of it when they do. (Each piece and direction on a core of its own.)
    let mut by_piece: Vec<Vec<sight::Tri<Of>>> = (0..pieces.len()).map(|_| Vec::new()).collect();
    for t in tris {
        let (Of::Face(pi, _) | Of::Cap(pi, _)) = t.of;
        by_piece[pi].push(t);
    }
    let dirs = sight::directions(&ELEVATIONS, PER_RING);
    let seen: std::collections::BTreeMap<Of, f32> = by_piece
        .par_iter()
        .flat_map(|tris| dirs.par_iter().map(move |&d| (tris, d)))
        .map(|(tris, d)| {
            let mut behind = vec![0.0f32; tris.len()];
            sight::look(tris, d, PIXEL, MOST_PIXELS, &mut behind);
            tris.iter().zip(behind).filter(|(_, a)| *a > 0.0).map(|(t, a)| (t.of, a)).collect::<Vec<_>>()
        })
        .flatten()
        .fold(std::collections::BTreeMap::new, |mut m, (of, a)| {
            *m.entry(of).or_default() += a;
            m
        })
        .reduce(std::collections::BTreeMap::new, |mut a, b| {
            for (k, v) in b {
                *a.entry(k).or_default() += v;
            }
            a
        });
    // (The piece's own faces come first, the halves made whole after them.)
    let own: Vec<usize> = pieces.iter().zip(halves.iter()).map(|(p, h)| p.faces.len() - h.mirrored.len()).collect();
    let mut given: Vec<Vec<usize>> = vec![Vec::new(); pieces.len()];
    let mut capped: Vec<Vec<usize>> = vec![Vec::new(); pieces.len()];
    for (of, area) in seen {
        if area < SEEN {
            continue;
        }
        match of {
            Of::Face(pi, f) => {
                // (A plate lying at the model's foot: the ground is under it.)
                let (p, face) = (&pieces[pi], &pieces[pi].faces[f]);
                if caps::plane(face).0.y.abs() > 0.7 && at_foot(face, p.offset) {
                    continue;
                }
                given[pi].push(f);
            }
            // A cap seen from behind, through another hole: what it closes is seen into, its
            // floor dark as the cap is.
            Of::Cap(pi, c) => capped[pi].push(c),
        }
    }
    // Sheets that are pieces of their own (see above).
    for (pi, (p, m)) in pieces.iter().zip(mendings.iter()).enumerate() {
        if !p.mend || p.faces.len() > 4096 || !m.holes.iter().any(|h| h.verdict == caps::Verdict::Sheet) {
            continue;
        }
        let (_, shell_of, _) = shells(p.faces);
        for h in m.holes.iter().filter(|h| h.verdict == caps::Verdict::Sheet) {
            let Some(&first) = h.faces.first() else { continue };
            let members: Vec<usize> = (0..p.faces.len()).filter(|&f| shell_of[f] == shell_of[first as usize] && p.faces[f].len() >= 3).collect();
            if members.len() == own[pi] && !members.iter().all(|&f| at_foot(&p.faces[f], p.offset)) {
                given[pi].extend(members);
            }
        }
    }
    for (pi, mut faces) in given.into_iter().enumerate() {
        faces.sort_unstable();
        faces.dedup();
        if faces.is_empty() {
            continue;
        }
        let p = &pieces[pi];
        for &f in &faces {
            // (Painted as the face of TA's it is, or that it was mirrored from.)
            let source = if f < own[pi] { f } else { halves[pi].mirrored[f - own[pi]].face };
            let mut drawn = p.faces[f].clone();
            if mendings[pi].flipped.contains(&f) {
                drawn.reverse();
            }
            halves[pi].mirrored.push(Mirrored { face: source, corners: back_of(&drawn), back: true });
        }
        halves[pi].shells.push(Shell { faces, verdict: Verdict::Backed, size: [0.0; 3] });
    }
    for (m, caps) in mendings.iter_mut().zip(capped) {
        for c in caps {
            let cap = &m.caps[c];
            let back = caps::Cap {
                corners: if cap.convex { back_of(&cap.corners) } else { cap.corners.clone() },
                triangles: if cap.convex { (1..cap.corners.len() as u32 - 1).map(|k| [0, k, k + 1]).collect() } else { cap.triangles.iter().map(|&[a, b, c]| [a, c, b]).collect() },
                normal: -cap.normal,
                convex: cap.convex,
                faces: cap.faces.clone(),
                rim: cap.rim,
                late: true,
            };
            m.caps.push(back);
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::caps::tests::{facing, open_box};
    use super::*;

    fn complete_one(faces: &[Polygon]) -> Halves {
        complete(&[PieceFaces { faces, offset: Vec3::ZERO, mend: true }]).remove(0)
    }

    /// As the game makes a model whole (see `ModelLibrary::add`): the halves mirrored, the holes
    /// capped, the sheets given backs. The pieces' faces with the mirrored ones after their own,
    /// what became of each, and the caps.
    pub(in super::super) fn make_whole_as_the_game(mut faces: Vec<Vec<Polygon>>, rest: &[Vec3], mendable: &[bool]) -> (Vec<Vec<Polygon>>, Vec<Halves>, Vec<caps::Mending>) {
        fn as_pieces<'a>(faces: &'a [Vec<Polygon>], rest: &[Vec3], mendable: &[bool]) -> Vec<PieceFaces<'a>> {
            faces.iter().zip(rest).zip(mendable).map(|((f, &offset), &mend)| PieceFaces { faces: f.as_slice(), offset, mend }).collect()
        }
        let mut halves = complete(&as_pieces(&faces, rest, mendable));
        for (f, h) in faces.iter_mut().zip(&halves) {
            f.extend(h.mirrored.iter().map(|m| m.corners.clone()));
        }
        let mut mended = caps::mend(&as_pieces(&faces, rest, mendable), true);
        let before: Vec<usize> = halves.iter().map(|h| h.mirrored.len()).collect();
        give_backs(&as_pieces(&faces, rest, mendable), &mut mended, &mut halves);
        for ((f, h), &k) in faces.iter_mut().zip(&halves).zip(&before) {
            f.extend(h.mirrored[k..].iter().map(|m| m.corners.clone()));
        }
        (faces, halves, mended)
    }

    #[test]
    fn a_wing_of_one_side_gets_its_back_but_not_a_plate_on_a_hull() {
        // A fuselage, and a wing reaching out from its side, of the same piece: one face facing
        // up, nothing under it.
        let hull = open_box(Vec3::new(-2.0, 0.0, -8.0), Vec3::new(2.0, 3.0, 8.0), &[]);
        let wing = facing(vec![Vec3::new(2.0, 1.5, -4.0), Vec3::new(12.0, 1.5, 0.0), Vec3::new(12.0, 1.5, 3.0), Vec3::new(2.0, 1.5, 4.0)], Vec3::Y);
        let with = |extra: &Polygon| hull.iter().cloned().chain(std::iter::once(extra.clone())).collect::<Vec<_>>();
        let (faces, halves, _) = make_whole_as_the_game(vec![with(&wing)], &[Vec3::ZERO], &[true]);
        assert_eq!(halves[0].shells.iter().filter(|s| s.verdict == Verdict::Backed).count(), 1, "{:?}", halves[0].shells);
        let back = faces[0].last().unwrap();
        assert_eq!(halves[0].mirrored.last().unwrap().face, 6, "painted as the wing");
        assert!(caps::plane(back).0.y < -0.99 && back.iter().all(|p| (p.y - 1.5).abs() < 1e-5), "{back:?}");
        // A plate lying on the hull's top: its underside is the hull's. One side.
        let plate = facing(vec![Vec3::new(-1.5, 3.0, -2.0), Vec3::new(1.5, 3.0, -2.0), Vec3::new(1.5, 3.0, 2.0), Vec3::new(-1.5, 3.0, 2.0)], Vec3::Y);
        let (faces, halves, _) = make_whole_as_the_game(vec![with(&plate)], &[Vec3::ZERO], &[true]);
        assert_eq!(faces[0].len(), 7, "{:?}", halves[0].shells);
        // The plate a piece of its own (a hatch's lid, which opens): given its back.
        let (faces, _, _) = make_whole_as_the_game(vec![hull.clone(), vec![plate]], &[Vec3::ZERO; 2], &[true; 2]);
        assert_eq!(faces[1].len(), 2);
        // A fin standing up, one face: from its other side it is not there at all. Given its back.
        let fin = vec![facing(vec![Vec3::new(0.0, 3.0, 4.0), Vec3::new(0.0, 3.0, 8.0), Vec3::new(0.0, 7.0, 8.0)], Vec3::X)];
        let (faces, _, _) = make_whole_as_the_game(vec![hull.clone(), fin], &[Vec3::ZERO; 2], &[true; 2]);
        assert_eq!(faces[1].len(), 2);
        assert!(caps::plane(&faces[1][1]).0.x < -0.99, "facing the other way");
        // A plate lying on the ground under the hull (a hover's skirt): the ground's. One side.
        let skirt = vec![facing(vec![Vec3::new(-6.0, 0.0, -10.0), Vec3::new(6.0, 0.0, -10.0), Vec3::new(6.0, 0.0, 10.0), Vec3::new(-6.0, 0.0, 10.0)], -Vec3::Y)];
        let (faces, _, _) = make_whole_as_the_game(vec![hull, skirt], &[Vec3::ZERO; 2], &[true; 2]);
        assert_eq!(faces[1].len(), 1);
    }

    #[test]
    fn a_dome_painted_inside_gets_its_backs_and_a_closed_box_none() {
        // A dome of four faces painted inside, open below (an orb TA showed from above, the
        // inside of its far side): from outside, every face of it is seen from behind.
        let apex = Vec3::new(0.0, 10.0, 0.0);
        let rim = [Vec3::new(-4.0, 6.0, -4.0), Vec3::new(4.0, 6.0, -4.0), Vec3::new(4.0, 6.0, 4.0), Vec3::new(-4.0, 6.0, 4.0)];
        let dome: Vec<Polygon> = (0..4).map(|k| {
            let (a, b) = (rim[k], rim[(k + 1) % 4]);
            let mid = (a + b + apex) / 3.0;
            facing(vec![a, b, apex], Vec3::new(0.0, 8.0, 0.0) - mid)
        }).collect();
        let (faces, halves, _) = make_whole_as_the_game(vec![dome], &[Vec3::ZERO], &[true]);
        assert_eq!(faces[0].len(), 8, "{:?}", halves[0].shells);
        // A box closed all round: nothing of it is seen from behind.
        let shut = caps::tests::open_box(Vec3::ZERO, Vec3::new(6.0, 4.0, 8.0), &[]);
        let (faces, _, _) = make_whole_as_the_game(vec![shut], &[Vec3::ZERO], &[true]);
        assert_eq!(faces[0].len(), 6);
    }

    /// A prism along z from `z0` to `z1` whose cross-section is the polyline `section` (x, y),
    /// its faces facing away from `inside` (a point of the cross-section), and its ends closed
    /// (half-polygons: the section closed straight across).
    fn prism(section: &[(f32, f32)], inside: (f32, f32), z0: f32, z1: f32, ends: bool) -> Vec<Polygon> {
        let mid = |x: f32, y: f32| Vec3::new(x, y, (z0 + z1) * 0.5);
        let mut faces: Vec<Polygon> = section
            .windows(2)
            .map(|w| {
                let ((xa, ya), (xb, yb)) = (w[0], w[1]);
                let quad = vec![Vec3::new(xa, ya, z0), Vec3::new(xb, yb, z0), Vec3::new(xb, yb, z1), Vec3::new(xa, ya, z1)];
                let out = mid((xa + xb) * 0.5, (ya + yb) * 0.5) - mid(inside.0, inside.1);
                facing(quad, out)
            })
            .collect();
        if ends {
            for (z, dir) in [(z0, -Vec3::Z), (z1, Vec3::Z)] {
                faces.push(facing(section.iter().map(|&(x, y)| Vec3::new(x, y, z)).collect(), dir));
            }
        }
        faces
    }

    #[test]
    fn a_square_barrel_without_its_underside_gets_one_painted_as_its_top() {
        // The Bulldog's barrel: two sides and a top, 2 by 2, 17.5 long, open below and at its
        // ends.
        let faces = prism(&[(-1.0, 16.25), (-1.0, 18.25), (1.0, 18.25), (1.0, 16.25)], (0.0, 17.25), -3.75, 13.75, false);
        let top = faces.iter().position(|f| f.iter().all(|p| (p.y - 18.25).abs() < 1e-4)).unwrap();
        let halves = complete_one(&faces);
        assert_eq!(halves.shells.len(), 1);
        assert_eq!(halves.shells[0].verdict, Verdict::Boxed, "{:?}", halves.shells);
        assert_eq!(halves.mirrored.len(), 1, "the sides fall on themselves");
        let bottom = &halves.mirrored[0];
        assert_eq!(bottom.face, top);
        assert!(bottom.corners.iter().all(|p| (p.y - 16.25).abs() < 1e-4), "{:?}", bottom.corners);
        assert!(caps::plane(&bottom.corners).0.y < -0.99, "facing down");
        // Once whole, its holes are its ends alone: the caps close them, facing along it.
        let mut all = faces.clone();
        all.push(bottom.corners.clone());
        let m = caps::mend(&[PieceFaces { faces: &all, offset: Vec3::ZERO, mend: true }], true).remove(0);
        assert_eq!(m.caps.len(), 2, "{:?}", m.holes);
        assert!(m.caps.iter().all(|c| c.normal.z.abs() > 0.99));
    }

    #[test]
    fn a_long_box_missing_its_underside_and_a_tapered_channel_are_boxes() {
        // Closed at its ends, three times as long as it is across.
        let faces = open_box(Vec3::new(-1.0, 0.0, -6.0), Vec3::new(1.0, 2.0, 6.0), &[1]);
        let halves = complete_one(&faces);
        assert_eq!(halves.shells[0].verdict, Verdict::Boxed);
        assert_eq!(halves.mirrored.len(), 1);
        // A sleeve narrowing towards its muzzle, its sides square to the opening.
        let (w0, w1) = (1.75, 1.0);
        let corner = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let faces = vec![
            facing(vec![corner(-w0, 0.0, 0.0), corner(-w0, 2.5, 0.0), corner(-w1, 2.5, 6.0), corner(-w1, 0.0, 6.0)], -Vec3::X),
            facing(vec![corner(w0, 0.0, 0.0), corner(w0, 2.5, 0.0), corner(w1, 2.5, 6.0), corner(w1, 0.0, 6.0)], Vec3::X),
            facing(vec![corner(-w0, 2.5, 0.0), corner(w0, 2.5, 0.0), corner(w1, 2.5, 6.0), corner(-w1, 2.5, 6.0)], Vec3::Y),
        ];
        let halves = complete_one(&faces);
        assert_eq!(halves.shells[0].verdict, Verdict::Boxed, "{:?}", halves.shells);
    }

    #[test]
    fn a_half_hexagon_pipe_gets_its_lower_half() {
        let s = 1.4f32;
        let h = s * 3f32.sqrt() * 0.5;
        let faces = prism(&[(-s, 0.0), (-s * 0.5, h), (s * 0.5, h), (s, 0.0)], (0.0, 0.1), 0.0, 20.0, false);
        let halves = complete_one(&faces);
        assert_eq!(halves.shells[0].verdict, Verdict::Rounded, "{:?}", halves.shells);
        assert_eq!(halves.mirrored.len(), 3);
        for m in &halves.mirrored {
            assert!(m.corners.iter().all(|p| p.y <= 1e-4), "under the opening");
            assert!(caps::plane(&m.corners).0.y < -0.4, "facing down and out");
        }
        // With its ends closed (half-hexagons), they are mirrored too: a closed hexagonal prism.
        let faces = prism(&[(-s, 0.0), (-s * 0.5, h), (s * 0.5, h), (s, 0.0)], (0.0, 0.1), 0.0, 20.0, true);
        let halves = complete_one(&faces);
        assert_eq!((halves.shells[0].verdict, halves.mirrored.len()), (Verdict::Rounded, 5));
    }

    #[test]
    fn a_triangle_a_hull_an_open_trough_and_a_pipe_on_a_deck_stay_as_they_are() {
        // A gun ridged like a roof (two faces): no round half.
        let faces = prism(&[(-0.87, 0.0), (0.0, 1.5), (0.87, 0.0)], (0.0, 0.5), 0.0, 14.0, false);
        let halves = complete_one(&faces);
        assert!(halves.mirrored.is_empty());
        assert_eq!(halves.shells[0].verdict, Verdict::Neither);
        // A hull's missing underside: too big for a barrel.
        let faces = open_box(Vec3::new(-12.0, 0.0, -30.0), Vec3::new(12.0, 6.0, 30.0), &[1]);
        assert_eq!(complete_one(&faces).shells[0].verdict, Verdict::Size);
        // A small box missing its underside, as long as it is across (a turret): the caps'.
        let faces = open_box(Vec3::new(-3.0, 0.0, -3.0), Vec3::new(3.0, 3.0, 3.0), &[1]);
        assert_eq!(complete_one(&faces).shells[0].verdict, Verdict::Size);
        // A trough, open at the top: TA's camera looked into it.
        let faces = prism(&[(-1.0, 2.0), (-1.0, 0.0), (1.0, 0.0), (1.0, 2.0)], (0.0, 1.0), 0.0, 10.0, false);
        assert_eq!(complete_one(&faces).shells[0].verdict, Verdict::Up);
        // A half-pipe laid on a deck: the deck lies on its opening.
        let s = 1.4f32;
        let h = s * 3f32.sqrt() * 0.5;
        let mut faces = prism(&[(-s, 0.0), (-s * 0.5, h), (s * 0.5, h), (s, 0.0)], (0.0, 0.1), 0.0, 20.0, false);
        faces.push(facing(vec![Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, 25.0), Vec3::new(-5.0, 0.0, 25.0)], Vec3::Y));
        let halves = complete_one(&faces);
        assert!(halves.mirrored.is_empty());
        assert!(halves.shells.iter().any(|s| s.verdict == Verdict::InTheWay), "{:?}", halves.shells);
    }

    /// `TA_SCAN_MODELS=armbull,armstump TA_FORMS_OUT=<dir>`: writes those models at rest as OBJ
    /// (front faces anticlockwise), their own faces, the faces mirrored to make halves whole and
    /// the caps closing what is left open each in an object of its own, as the game builds them.
    /// On this computer only.
    #[test]
    #[ignore]
    fn dump_halves() {
        use super::super::caps::tests::{install, model_faces};
        let (Ok(names), Ok(out)) = (std::env::var("TA_SCAN_MODELS"), std::env::var("TA_FORMS_OUT")) else { return };
        let Some(vfs) = install() else { return };
        std::fs::create_dir_all(&out).unwrap();
        for name in names.split(',').map(|s| s.trim().to_ascii_lowercase()) {
            let Some(bytes) = vfs.read_opt(&format!("objects3d/{name}.3do")) else {
                eprintln!("no model {name}");
                continue;
            };
            let model = ta_formats::model3do::Model3do::parse(&bytes).unwrap();
            let (faces, rest, mendable) = model_faces(&model);
            let own: Vec<usize> = faces.iter().map(Vec::len).collect();
            let (faces, _, mended) = make_whole_as_the_game(faces, &rest, &mendable);
            // (Each piece's in objects of its own, `<piece>.faces`, `.mirrored` and `.caps`.)
            let mut objects: std::collections::BTreeMap<String, Vec<Vec<Vec3>>> = Default::default();
            for (pi, m) in mended.iter().enumerate().filter(|&(pi, _)| mendable[pi]) {
                let piece = format!("{pi:02}_{}", model.pieces[pi].name);
                for (f, face) in faces[pi].iter().enumerate() {
                    let mut face: Vec<Vec3> = face.iter().map(|&p| p + rest[pi]).collect();
                    if m.flipped.contains(&f) {
                        face.reverse();
                    }
                    objects.entry(format!("{piece}.{}", if f < own[pi] { "faces" } else { "mirrored" })).or_default().push(face);
                }
                for cap in &m.caps {
                    for t in &cap.triangles {
                        objects.entry(format!("{piece}.caps")).or_default().push(t.iter().map(|&i| cap.corners[i as usize] + rest[pi]).collect());
                    }
                }
            }
            let mut obj = String::new();
            let mut count = 0usize;
            for (key, polygons) in &objects {
                obj += &format!("o {key}
");
                for poly in polygons {
                    for p in poly {
                        obj += &format!("v {} {} {}
", p.x, p.y, p.z);
                    }
                    // (OBJ's front faces run anticlockwise; the model's run clockwise.)
                    let ids: Vec<String> = (0..poly.len()).rev().map(|k| (count + k + 1).to_string()).collect();
                    obj += &format!("f {}
", ids.join(" "));
                    count += poly.len();
                }
            }
            std::fs::write(std::path::Path::new(&out).join(format!("{name}.obj")), obj).unwrap();
        }
    }

    /// Every installed model's open shells: how many, what became of them, and the models made
    /// whole with their pieces. `cargo test --release -p ta_render --lib scan_halves -- --ignored
    /// --nocapture` (`TA_SCAN_MODELS=a,b` lists those models' shells one by one).
    #[test]
    #[ignore]
    fn scan_halves_in_every_installed_model() {
        use super::super::caps::tests::{install, model_faces};
        use std::collections::BTreeMap;
        let Some(vfs) = install() else { return };
        let verdict_shown = std::env::var("TA_SCAN_VERDICT").ok();
        let aim = std::env::var("TA_SCAN_AIM").is_ok();
        let detail: Vec<String> = std::env::var("TA_SCAN_MODELS").map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()).collect()).unwrap_or_default();
        let (mut models, mut whole, mut faces_added) = (0, Vec::new(), 0);
        let mut verdicts: BTreeMap<Verdict, usize> = BTreeMap::new();
        let mut elapsed = std::time::Duration::ZERO;
        for path in vfs.list("objects3d", Some("3do"), true) {
            let Ok(model) = ta_formats::model3do::Model3do::parse(&vfs.read(&path).unwrap()) else { continue };
            models += 1;
            let (faces, rest, mendable) = model_faces(&model);
            let started = std::time::Instant::now();
            let (_, halves, _) = make_whole_as_the_game(faces.clone(), &rest, &mendable);
            elapsed += started.elapsed();
            let stem = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".3do").to_ascii_lowercase();
            let mut made = Vec::new();
            for (pi, h) in halves.iter().enumerate() {
                faces_added += h.mirrored.len();
                for s in &h.shells {
                    *verdicts.entry(s.verdict).or_default() += 1;
                    if matches!(s.verdict, Verdict::Boxed | Verdict::Rounded | Verdict::Backed) {
                        made.push(format!("{} {:?}", model.pieces[pi].name, s.verdict));
                        // (`TA_SCAN_AIM=1`: where each one is in the model at rest, for aiming a
                        // camera at it: with the unit facing south, x east and z south of it.)
                        if aim {
                            let corners: Vec<Vec3> = s.faces.iter().flat_map(|&f| faces[pi][f].iter().map(|&p| p + rest[pi])).collect();
                            let (lo, hi) = corners.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
                            let c = (lo + hi) * 0.5;
                            eprintln!("aim {stem} {} {:.2} {:.2} {:.2} {:.2}", model.pieces[pi].name, c.x, c.y, c.z, (hi - lo).max_element());
                        }
                    }
                    if detail.contains(&stem) || verdict_shown.as_deref() == Some(&format!("{:?}", s.verdict)) && !stem.ends_with("_dead") {
                        eprintln!("{stem} piece {pi} {:12} {:9?} {} faces, {:5.1} across, {:5.1} deep, {:5.1} long", model.pieces[pi].name, s.verdict, s.faces.len(), s.size[0], s.size[1], s.size[2]);
                    }
                }
            }
            if !made.is_empty() {
                whole.push(format!("{stem}: {}", made.join(", ")));
            }
        }
        if detail.is_empty() {
            for line in &whole {
                eprintln!("{line}");
            }
        }
        eprintln!("{models} models, {} with halves made whole or sheets given backs ({faces_added} faces mirrored), as the game does it (with the caps) in {:.0} ms", whole.len(), elapsed.as_secs_f64() * 1000.0);
        for (v, n) in &verdicts {
            eprintln!("  {v:?}: {n} shells");
        }
    }
}
