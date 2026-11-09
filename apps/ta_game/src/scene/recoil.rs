//! Guns recoiling clear of what is round them (for the eye alone).
//!
//! TA's fire scripts kick a gun back as it fires (`move barrel to z-axis [-2.4]`), and as TA's
//! engine moves pieces (Spring's too), in the frame of the piece it hangs from: level in it,
//! whatever the gun's own pitch. Most guns hang from a sleeve their aim pitches, or a turret it
//! turns, and kick back along themselves. A gun its aim turns itself kicks back out of line with
//! itself, and TA's models were built for its camera high over the battlefield, which never
//! showed what that does. The Core Raider's barrel turns up inside a sleeve no script turns
//! (base, turret, sleeve, barrel), and kicked back six units level while pitched up to fire, it
//! came up through the top of the sleeve; the Triton's came up through the top of its turret. So
//! the scene draws such a gun's recoil only as far back as the gun goes clear of the pieces
//! round it:
//!
//! * A gun here is a piece the unit's fire functions move (`FirePrimary`... and what they call
//!   or start) that its aim functions turn themselves (about x or y). Guns carried by what their
//!   aim turns, and doors and racks moved as missiles leave, keep their scripts' moves: they go
//!   back along themselves, into what TA's artists built round them. So does a gun turned less
//!   than 8 degrees out of its kick's line (the Copperhead's guns at a target on the ground).
//! * What it can run into is what it is mounted in: the pieces held fast to the piece it hangs
//!   from (no script moves or turns any between them), up to the first a script turns (the
//!   Raider's sleeve and turret), posed only by the gun's own turns.
//! * Clear: the gun and what it carries, slid back, cross no face of those pieces (as drawn: TA's
//!   own, made whole and capped) that they did not cross where the kick started, but faces they
//!   back into head on (a barrel sliding into its turret through the turret's front); nor do
//!   those pieces' edges cross the gun's faces so. What the gun was clearly through already (a
//!   barrel's root in the mouth of its sleeve) is its way in; a face it only grazed there may
//!   stay grazed where it was.
//! * Its way back is the script's while that runs clear for the whole kick. Where it runs into
//!   something sooner, the gun goes back along itself as it points (in its own frame) if that
//!   runs clear further; either way no further than it runs clear. The Raider's barrel, pitched
//!   up, kicks back along itself until its end meets the floor of the sleeve; level, the whole
//!   six units, as TA has it.
//!
//! Reckoned once for every unit of a type, for each whole degree of the gun's own turns, and
//! only for units big enough on screen to see it (as walkers' steps, see `walk`).
//! `TA_RECOIL_ROOM=0` draws the scripts' recoils as they are. Nothing here feeds back into the
//! game: the simulation, its scripts and its pieces' positions are untouched (a shell leaves
//! the muzzle where the game has it, before the kick).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use ta_sim::UnitType;

use super::{PieceState, local_rotation, piece_matrices_between, walk};

/// Faces a gun meets within this angle of head on (their normal against its way back), it backs
/// into: cos 60 degrees.
const HEAD_ON: f32 = 0.5;
/// An edge crosses a face where its ends lie at least this far either side of the face's plane
/// (model units): edges sliding along a face, or touching it, do not.
const THROUGH: f32 = 0.01;
/// A gun goes clearly through a face where an edge's end lies at least this far behind it
/// (model units); less, it grazes it. And a graze that moves this far is no longer where it was.
const DEEP: f32 = 0.25;
/// Steps along a kick to find where it stops running clear, then halvings of the step it does.
const STEPS: usize = 8;
const HALVINGS: usize = 5;
/// A kick shorter than this (model units) is no kick.
const LEAST: f32 = 1e-3;

/// `TA_RECOIL_ROOM=0`: the scripts' recoils drawn as they are, for comparisons.
fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TA_RECOIL_ROOM").map_or(true, |v| v != "0"))
}

/// Markers rather than parts of a body: muzzle flashes, fire points and the like (as the
/// animation scan and the renderer's effect pieces have them; the Commander's `mlasflsh`).
fn marker(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["flare", "fire", "spray", "emit", "flash", "flsh", "muzzle"].iter().any(|m| name.contains(m)) || ["point", "beam"].iter().any(|m| name.starts_with(m))
}

/// A unit type's recoiling guns, and its pieces' faces as drawn.
pub(crate) struct Guns {
    guns: Vec<Gun>,
    /// Each piece's triangles (piece space, counter-clockwise seen from outside, as the renderer
    /// draws them); none for markers.
    faces: Vec<Vec<Face>>,
    /// Each piece's matrix at rest (the unit's frame), its parent and its place in it.
    rest: Vec<Mat4>,
    parents: Vec<Option<usize>>,
    offsets: Vec<Vec3>,
    /// The ways back reckoned so far, shared by every unit of the type: by gun, its own turns
    /// (whole degrees) and its kick's way (its parent's frame, hundredths).
    ways: std::sync::Mutex<HashMap<(usize, [i16; 3], [i16; 3]), Way>>,
}

struct Gun {
    piece: usize,
    /// The axes its fire functions move it along (bit 0 x, 1 y, 2 z).
    axes: u8,
    /// The pieces it carries, itself among them.
    carried: Vec<bool>,
    /// What it is mounted in: the pieces held fast to the piece it hangs from (no script moves
    /// or turns any between them), up to the first a script turns (the turret that carries it all),
    /// but for the gun and what it carries. What it can run into, posed only by its own turns.
    mount: Vec<bool>,
}

/// A gun kicked back within this of its own line (the cosine of 8 degrees) kicks back along
/// itself near enough: as its script has it.
const IN_LINE: f32 = 0.990;

/// A triangle of a piece's faces and the plane it lies in (the same for every triangle of a
/// face, and for faces of the piece in one plane).
#[derive(Clone, Copy)]
struct Face {
    corners: [Vec3; 3],
    plane: u64,
}

/// A plane's identity: its normal and its distance from the origin, rounded.
fn plane_id(normal: Vec3, d: f32) -> u64 {
    let q = |v: f32, s: f32| (v * s).round() as i64 as u64 & 0xFFFF;
    q(normal.x, 64.0) | q(normal.y, 64.0) << 16 | q(normal.z, 64.0) << 32 | q(d, 16.0) << 48
}

impl Guns {
    /// A type's guns that recoil, from its script, with its model's triangles as drawn (each
    /// corner with its piece, piece space, three to a triangle: `ModelLibrary::corners`). None
    /// for a type with none.
    pub(crate) fn of(t: &UnitType, corners: impl Iterator<Item = (u32, Vec3)>) -> Option<Guns> {
        let program = t.script.as_ref()?;
        let script = &program.script;
        let n = t.model.pieces.len();
        // What the functions of a kind (and those they call or start) move and turn, by piece.
        let channels = |kind: &str| -> Vec<[bool; 6]> {
            let mut out = vec![[false; 6]; script.pieces.len()];
            let mut seen = vec![false; script.functions.len()];
            for (i, f) in script.functions.iter().enumerate() {
                if f.name.to_ascii_lowercase().starts_with(kind) {
                    walk::channels_moved(script, i, &mut out, &mut seen);
                }
            }
            (0..n).map(|mi| t.model_to_cob.get(mi).copied().flatten().and_then(|c| out.get(c).copied()).unwrap_or([false; 6])).collect()
        };
        let (fire, aim, any) = (channels("fire"), channels("aim"), channels(""));
        let parents: Vec<Option<usize>> = t.model.pieces.iter().map(|p| p.parent).collect();
        // (Turned by its aim itself, about x or y: a gun on a sleeve or a turret that turns kicks
        // back along itself as its script has it.)
        let points_itself = |i: usize| aim[i][3] || aim[i][4];
        let animated = |i: usize| any[i].iter().any(|&c| c);
        let carried = |gun: usize| -> Vec<bool> {
            let mut c = vec![false; n];
            c[gun] = true;
            // (Parents come before their children.)
            for i in gun + 1..n {
                if parents[i].is_some_and(|p| c[p]) {
                    c[i] = true;
                }
            }
            c
        };
        let mount = |gun: usize, carried: &[bool]| -> Vec<bool> {
            let mut m = vec![false; n];
            let mut root = parents[gun];
            while let Some(p) = root.filter(|&p| !animated(p)) {
                match parents[p] {
                    Some(q) => root = Some(q),
                    None => break,
                }
            }
            if let Some(r) = root {
                m[r] = true;
                for i in r + 1..n {
                    if !carried[i] && !animated(i) && parents[i].is_some_and(|p| m[p]) {
                        m[i] = true;
                    }
                }
            }
            m
        };
        let guns: Vec<Gun> = (0..n)
            .filter_map(|i| {
                let axes = (0..3).filter(|&k| fire[i][k]).fold(0u8, |m, k| m | 1 << k);
                (axes != 0 && points_itself(i) && !marker(&t.model.pieces[i].name)).then(|| {
                    let carried = carried(i);
                    Gun { piece: i, axes, mount: mount(i, &carried), carried }
                })
            })
            .collect();
        if guns.is_empty() {
            return None;
        }
        log::debug!("{}: recoiling guns {:?}", t.name, guns.iter().map(|g| (&t.model.pieces[g.piece].name, g.axes)).collect::<Vec<_>>());
        let mut faces = vec![Vec::new(); n];
        let corners: Vec<(u32, Vec3)> = corners.collect();
        for tri in corners.chunks_exact(3) {
            let p = tri[0].0 as usize;
            // (A connector's sides span two pieces: they follow both.)
            if tri.iter().any(|c| c.0 as usize != p) || p >= n || marker(&t.model.pieces[p].name) {
                continue;
            }
            let c = [tri[0].1, tri[1].1, tri[2].1];
            let normal = (c[1] - c[0]).cross(c[2] - c[0]);
            if normal.length_squared() < 1e-10 {
                continue;
            }
            let normal = normal.normalize();
            faces[p].push(Face { corners: c, plane: plane_id(normal, normal.dot(c[0])) });
        }
        let still = vec![([0; 3], [0; 3], true); n];
        let mut rest = Vec::new();
        piece_matrices_between(t, &still, &still, 1.0, Mat4::IDENTITY, &mut rest);
        let offsets = t.model.pieces.iter().map(|p| Vec3::new(p.offset.x.to_f32(), p.offset.y.to_f32(), p.offset.z.to_f32())).collect();
        Some(Guns { guns, faces, rest, parents, offsets, ways: Default::default() })
    }

    /// How many guns recoil.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.guns.len()
    }

    /// The matrices to reckon gun `g`'s way back with, turned `turns` itself (TA's angle units):
    /// what it is mounted in at rest, the gun and what it carries turned so, nothing else (zero).
    fn posed(&self, g: &Gun, turns: [f32; 3]) -> Vec<Mat4> {
        let mut m: Vec<Mat4> = self.rest.iter().zip(&g.mount).map(|(r, &mounted)| if mounted { *r } else { Mat4::ZERO }).collect();
        let parent = self.parents[g.piece].map_or(Mat4::IDENTITY, |q| self.rest[q]);
        m[g.piece] = parent * Mat4::from_rotation_translation(local_rotation(turns), self.offsets[g.piece]);
        for p in g.piece + 1..m.len() {
            if g.carried[p]
                && let Some(q) = self.parents[p]
            {
                m[p] = m[q] * (self.rest[q].inverse() * self.rest[p]);
            }
        }
        m
    }

    /// How gun `gi` turned `turns` itself is drawn kicked back `reach` along `dir` (its parent's
    /// frame, unit length): reckoned once for every unit of the type, for each whole degree of its
    /// turns.
    fn way(&self, gi: usize, turns: [f32; 3], dir: Vec3, reach: f32) -> Way {
        let degrees = turns.map(|a| (a * 360.0 / 65536.0).round() as i16);
        let key = (gi, degrees, dir.to_array().map(|d| (d * 100.0).round() as i16));
        // (Reckoned for a shorter kick that ran clear all the way, it is reckoned again.)
        let known = self.ways.lock().ok().and_then(|ways| ways.get(&key).copied());
        if let Some(w) = known.filter(|w| w.reach >= reach - LEAST || w.room < w.reach - LEAST) {
            return w;
        }
        let g = &self.guns[gi];
        let turns = degrees.map(|d| d as f32 * 65536.0 / 360.0);
        let w = reckon(self, g, &self.posed(g, turns), turns, dir, reach);
        if let Ok(mut ways) = self.ways.lock() {
            ways.insert(key, w);
        }
        w
    }
}

/// A gun's kick in a state: its move along its recoil axes, in its parent's frame (model units;
/// script x moves negated, as `piece_matrices_between` draws them).
fn kick(s: &[PieceState], g: &Gun) -> Vec3 {
    let Some(&(offset, _, _)) = s.get(g.piece) else { return Vec3::ZERO };
    let along = |k: usize| if g.axes & 1 << k != 0 { offset[k] as f32 / 65536.0 } else { 0.0 };
    Vec3::new(-along(0), along(1), along(2))
}

/// How a gun's kick is drawn: reckoned for a kick up to `reach` long, back along the gun as it
/// points rather than as the script moves it (`along`), and as far as it goes clear.
#[derive(Clone, Copy, Debug)]
struct Way {
    reach: f32,
    along: bool,
    room: f32,
}

/// A unit's guns as drawn: the states posed with their kicks clear.
#[derive(Default)]
pub(super) struct Recoil {
    prev: Vec<PieceState>,
    cur: Vec<PieceState>,
}

impl Recoil {
    /// The script states to pose a unit with this frame, given its states at the tick before and
    /// this one: its guns' kicks as far as they run clear (`close`: the unit is big enough on
    /// screen to see them; else the script's).
    pub(super) fn states<'a>(&'a mut self, guns: Option<&Guns>, close: bool, prev: &'a [PieceState], cur: &'a [PieceState]) -> (&'a [PieceState], &'a [PieceState]) {
        let Some(guns) = guns.filter(|_| close && enabled()) else { return (prev, cur) };
        let kicked = |s: &[PieceState]| guns.guns.iter().any(|g| kick(s, g).length() >= LEAST);
        if !kicked(prev) && !kicked(cur) {
            return (prev, cur);
        }
        let (mut before, mut now) = (std::mem::take(&mut self.prev), std::mem::take(&mut self.cur));
        clear(guns, prev, &mut before);
        clear(guns, cur, &mut now);
        (self.prev, self.cur) = (before, now);
        (&self.prev, &self.cur)
    }
}

/// State `s` with its guns' kicks as far as they run clear, into `out`.
fn clear(guns: &Guns, s: &[PieceState], out: &mut Vec<PieceState>) {
    out.clear();
    out.extend_from_slice(s);
    for (gi, g) in guns.guns.iter().enumerate() {
        let v = kick(s, g);
        let reach = v.length();
        if reach < LEAST {
            continue;
        }
        let dir = v / reach;
        let turned = turns(&s[g.piece]);
        let along = local_rotation(turned) * dir;
        // (Kicked back near enough along its own line: as the script has it.)
        if along.dot(dir) >= IN_LINE {
            continue;
        }
        let way = guns.way(gi, turned, dir, reach);
        if !way.along && way.room >= reach - LEAST {
            continue;
        }
        let back = if way.along { along } else { dir } * reach.min(way.room);
        // (Back into the script's values: x moves negated.)
        let p = &mut out[g.piece].0;
        for (k, v) in [-back.x, back.y, back.z].into_iter().enumerate() {
            let base = if g.axes & 1 << k != 0 { 0 } else { p[k] };
            p[k] = base.saturating_add((v * 65536.0).round() as i32);
        }
    }
}

/// A state's turns, as `local_rotation` takes them.
fn turns(s: &PieceState) -> [f32; 3] {
    s.1.map(|a| a as f32)
}

/// Reckons how gun `g`, turned `turns` itself and posed with what it is mounted in as `m` (the
/// unit's frame; see `Guns::posed`), is drawn kicked back `reach` along `dir` (its parent's
/// frame).
fn reckon(guns: &Guns, g: &Gun, m: &[Mat4], turns: [f32; 3], dir: Vec3, reach: f32) -> Way {
    let parent = guns.parents[g.piece].and_then(|p| m.get(p)).copied().unwrap_or(Mat4::IDENTITY);
    let scripts = parent.transform_vector3(dir).normalize_or_zero();
    let room = room(guns, g, m, scripts, reach);
    if room >= reach - LEAST {
        return Way { reach, along: false, room };
    }
    let along = parent.transform_vector3(local_rotation(turns) * dir).normalize_or_zero();
    let further = self::room(guns, g, m, along, reach);
    log::debug!("gun piece {}: a kick of {reach:.2} runs clear {room:.2} as scripted, {further:.2} along the gun", g.piece);
    if further > room + LEAST { Way { reach, along: true, room: further } } else { Way { reach, along: false, room } }
}

/// A face placed in the unit's frame.
#[derive(Clone, Copy)]
struct Placed {
    corners: [Vec3; 3],
    normal: Vec3,
    /// Its piece and plane.
    plane: (usize, u64),
}

/// Where an edge `a`-`b` passes through a triangle (its ends clearly either side of the
/// triangle's plane and the point where it passes the plane inside the triangle), and how far
/// its end behind the face (against the face's outward `normal`) lies behind it.
fn through(a: Vec3, b: Vec3, c: &[Vec3; 3], normal: Vec3) -> Option<(Vec3, f32)> {
    let (da, db) = ((a - c[0]).dot(normal), (b - c[0]).dot(normal));
    if da * db >= 0.0 || da.abs() < THROUGH || db.abs() < THROUGH {
        return None;
    }
    let p = a + (b - a) * (da / (da - db));
    let (e1, e2, v) = (c[1] - c[0], c[2] - c[0], p - c[0]);
    let (d00, d01, d11, d20, d21) = (e1.dot(e1), e1.dot(e2), e2.dot(e2), v.dot(e1), v.dot(e2));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return None;
    }
    let (s, u) = ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den);
    (s > 1e-4 && u > 1e-4 && s + u < 1.0 - 1e-4).then_some((p, -da.min(db)))
}

/// An edge through a face: of the gun through another piece's face (`gun_edge`) or of another
/// piece through the gun's; which edge (of the gun's or the others' edges), the face's piece and
/// plane, where it passes, how far its end behind the face lies behind it, and whether the gun
/// backs into the face head on.
#[derive(Clone, Copy)]
struct Crossing {
    gun_edge: bool,
    edge: usize,
    plane: (usize, u64),
    at: Vec3,
    depth: f32,
    head_on: bool,
}

/// An edge of a piece's faces: its ends, and whether a face it bounds meets a gun going back
/// along `dir` head on (an edge of the face a barrel slides into).
#[derive(Clone, Copy)]
struct Edge {
    a: Vec3,
    b: Vec3,
    head_on: bool,
}

/// The edges of triangles, each once, but those within a face (two triangles of a piece in one
/// plane either side: the diagonal a quad is drawn with), which cross nothing the face does not.
fn edges(faces: &[Placed], dir: Vec3) -> Vec<Edge> {
    let key = |p: Vec3| p.to_array().map(f32::to_bits);
    let mut by_ends: std::collections::HashMap<([u32; 3], [u32; 3]), (Edge, Vec<(usize, u64)>)> = std::collections::HashMap::new();
    let mut order = Vec::new();
    for f in faces {
        let head_on = f.normal.dot(dir) <= -HEAD_ON;
        for k in 0..3 {
            let (a, b) = (f.corners[k], f.corners[(k + 1) % 3]);
            let (ka, kb) = (key(a), key(b));
            let ends = if ka < kb { (ka, kb) } else { (kb, ka) };
            let entry = by_ends.entry(ends).or_insert_with(|| {
                order.push(ends);
                (Edge { a, b, head_on: false }, Vec::new())
            });
            entry.0.head_on |= head_on;
            entry.1.push(f.plane);
        }
    }
    order
        .into_iter()
        .filter_map(|ends| {
            let (edge, planes) = by_ends.remove(&ends)?;
            (!(planes.len() == 2 && planes[0] == planes[1])).then_some(edge)
        })
        .collect()
}

/// How far a gun (and what it carries) goes back along `dir` (the unit's frame, unit length),
/// `most` at the most, running clear of the other pieces as posed (`m`): see the module's notes.
fn room(guns: &Guns, g: &Gun, m: &[Mat4], dir: Vec3, most: f32) -> f32 {
    if dir == Vec3::ZERO {
        return most;
    }
    let (mut moving, mut others) = (Vec::new(), Vec::new());
    for (p, faces) in guns.faces.iter().enumerate() {
        // (Hidden pieces are drawn with no matrix.)
        let Some(&mat) = m.get(p).filter(|mat| **mat != Mat4::ZERO) else { continue };
        let to = if g.carried[p] { &mut moving } else { &mut others };
        for f in faces {
            let corners = f.corners.map(|c| mat.transform_point3(c));
            let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]).normalize_or_zero();
            to.push(Placed { corners, normal, plane: (p, f.plane) });
        }
    }
    if moving.is_empty() {
        return most;
    }
    // Only the faces near its way back.
    let (lo, hi) = moving.iter().flat_map(|f| f.corners).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), c| (lo.min(c), hi.max(c)));
    let (lo, hi) = (lo.min(lo + dir * most) - 0.1, hi.max(hi + dir * most) + 0.1);
    others.retain(|f| {
        let (a, b) = f.corners.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), &c| (a.min(c), b.max(c)));
        a.cmple(hi).all() && b.cmpge(lo).all()
    });
    if others.is_empty() {
        return most;
    }
    let (moving_edges, other_edges) = (edges(&moving, dir), edges(&others, dir));
    // Which of the gun's edges may meet which of the others' faces anywhere along its way back,
    // and the other way round (their boxes, the gun's swept back, overlap): only those are tried.
    let boxed = |points: &[Vec3]| points.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let swept = |points: &[Vec3]| {
        let (lo, hi) = boxed(points);
        (lo.min(lo + dir * most) - 0.05, hi.max(hi + dir * most) + 0.05)
    };
    let meet = |a: (Vec3, Vec3), b: (Vec3, Vec3)| a.0.cmple(b.1).all() && b.0.cmple(a.1).all();
    let other_boxes: Vec<(Vec3, Vec3)> = others.iter().map(|f| boxed(&f.corners)).collect();
    let moving_boxes: Vec<(Vec3, Vec3)> = moving.iter().map(|f| swept(&f.corners)).collect();
    let gun_pairs: Vec<(usize, usize)> = moving_edges
        .iter()
        .enumerate()
        .flat_map(|(e, edge)| {
            let reach = swept(&[edge.a, edge.b]);
            other_boxes.iter().enumerate().filter(move |(_, b)| meet(reach, **b)).map(move |(f, _)| (e, f))
        })
        .collect();
    let other_pairs: Vec<(usize, usize)> = other_edges
        .iter()
        .enumerate()
        .flat_map(|(e, edge)| {
            let at = boxed(&[edge.a, edge.b]);
            moving_boxes.iter().enumerate().filter(move |(_, b)| meet(at, **b)).map(move |(f, _)| (e, f))
        })
        .collect();
    // What crosses what with the gun `t` back. (Another piece's edge through the gun's face is
    // the gun going into that piece head on where the edge bounds a face it meets so: the rim of
    // the opening a barrel slides into; or where the gun's face it crosses leads it back.)
    let crossings = |t: f32| -> Vec<Crossing> {
        let shift = dir * t;
        let mut out = Vec::new();
        for &(edge, fi) in &gun_pairs {
            let (e, f) = (&moving_edges[edge], &others[fi]);
            if let Some((at, depth)) = through(e.a + shift, e.b + shift, &f.corners, f.normal) {
                out.push(Crossing { gun_edge: true, edge, plane: f.plane, at, depth, head_on: f.normal.dot(dir) <= -HEAD_ON });
            }
        }
        for &(edge, fi) in &other_pairs {
            let (e, f) = (&other_edges[edge], &moving[fi]);
            if let Some((at, depth)) = through(e.a, e.b, &f.corners.map(|c| c + shift), f.normal) {
                out.push(Crossing { gun_edge: false, edge, plane: f.plane, at, depth, head_on: e.head_on || f.normal.dot(dir) >= HEAD_ON });
            }
        }
        out
    };
    // Its way in: the faces it goes clearly through where the kick starts (a barrel's root in
    // its sleeve's mouth, a gun block sunk in its turret). A face it only grazes there (a
    // pitched barrel's top skimming the underside of its sleeve's top at the mouth) it may go on
    // grazing where it does, not cut through further along.
    let start = crossings(0.0);
    let way_in: HashSet<(bool, (usize, u64))> = start.iter().filter(|c| c.depth >= DEEP).map(|c| (c.gun_edge, c.plane)).collect();
    let grazes: Vec<Crossing> = start.into_iter().filter(|c| c.depth < DEEP).collect();
    let clear = |t: f32| {
        crossings(t).iter().all(|c| {
            c.head_on
                || way_in.contains(&(c.gun_edge, c.plane))
                || grazes.iter().any(|g| g.gun_edge == c.gun_edge && g.edge == c.edge && g.plane == c.plane && g.at.distance(c.at) < DEEP)
        })
    };
    let mut last = 0.0;
    for k in 1..=STEPS {
        let t = most * k as f32 / STEPS as f32;
        if clear(t) {
            last = t;
            continue;
        }
        let (mut lo, mut hi) = (last, t);
        for _ in 0..HALVINGS {
            let mid = (lo + hi) * 0.5;
            if clear(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        return lo;
    }
    most
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box's triangles (counter-clockwise seen from outside), `lo` to `hi`, leaving out the
    /// sides `open` names (0 -x, 1 +x, 2 -y, 3 +y, 4 -z, 5 +z).
    fn cuboid(lo: Vec3, hi: Vec3, open: &[usize]) -> Vec<[Vec3; 3]> {
        let c = |x: bool, y: bool, z: bool| Vec3::new(if x { hi.x } else { lo.x }, if y { hi.y } else { lo.y }, if z { hi.z } else { lo.z });
        // Each side's corners counter-clockwise seen from outside.
        let sides = [
            [c(false, false, false), c(false, false, true), c(false, true, true), c(false, true, false)],
            [c(true, false, false), c(true, true, false), c(true, true, true), c(true, false, true)],
            [c(false, false, false), c(true, false, false), c(true, false, true), c(false, false, true)],
            [c(false, true, false), c(false, true, true), c(true, true, true), c(true, true, false)],
            [c(false, false, false), c(false, true, false), c(true, true, false), c(true, false, false)],
            [c(false, false, true), c(true, false, true), c(true, true, true), c(false, true, true)],
        ];
        let mut out = Vec::new();
        for (k, s) in sides.iter().enumerate() {
            if !open.contains(&k) {
                out.push([s[0], s[1], s[2]]);
                out.push([s[0], s[2], s[3]]);
            }
        }
        out
    }

    /// The Raider's rig in miniature: a turret, a sleeve (a closed box two units square, eight
    /// long, sticking out of the turret's front) no script turns, and a barrel in its mouth that
    /// the aim pitches and the fire kicks back six units along z.
    fn raider() -> (UnitType, Vec<(u32, Vec3)>) {
        // Pieces: turret (0), sleeve (1, its pivot at its back), barrel (2, at the sleeve's
        // mouth), flare (3).
        let names = ["turret", "sleeve", "barrel", "flare"];
        let parents = [None, Some(0), Some(1), Some(2)];
        let offsets = [Vec3::ZERO, Vec3::new(0.0, 1.0, 2.0), Vec3::new(0.0, 0.0, 7.5), Vec3::new(0.0, 0.0, 10.0)];
        let model = ta_sim::defs::ModelDef {
            pieces: names
                .iter()
                .zip(parents)
                .zip(offsets)
                .map(|((name, parent), o)| ta_sim::defs::PieceDef { name: (*name).into(), parent, offset: ta_sim::FxVec3::new(ta_sim::Fx::from_f32(o.x), ta_sim::Fx::from_f32(o.y), ta_sim::Fx::from_f32(o.z)), top: ta_sim::Fx::ZERO })
                .collect(),
            height: ta_sim::Fx::from_int(4),
            reach: ta_sim::Fx::from_int(20),
        };
        let mut t = test_type(model);
        t.model_to_cob = vec![Some(0), Some(1), Some(2), Some(3)];
        t.script = Some(std::sync::Arc::new(raider_program(&names)));
        let mut corners = Vec::new();
        let mut put = |piece: u32, tris: Vec<[Vec3; 3]>| corners.extend(tris.into_iter().flat_map(|t| t.map(|c| (piece, c))));
        put(0, cuboid(Vec3::new(-4.0, -1.0, -6.0), Vec3::new(4.0, 2.5, 2.5), &[]));
        put(1, cuboid(Vec3::new(-1.0, -1.0, -0.5), Vec3::new(1.0, 1.0, 7.6), &[]));
        put(2, cuboid(Vec3::new(-0.6, -0.6, -0.4), Vec3::new(0.6, 0.6, 10.0), &[]));
        (t, corners)
    }

    /// A script whose `AimPrimary` turns the barrel (COB piece 2) about x and whose
    /// `FirePrimary` moves it along z: `turn barrel to x-axis <0> now; return` and `move barrel to
    /// z-axis [0] now; return`.
    fn raider_program(names: &[&str]) -> ta_sim::cob::CobProgram {
        use ta_formats::cob::{CobFunction, CobScript, op};
        let code = vec![op::PUSH_CONSTANT, 0, op::TURN_NOW, 2, 0, op::RETURN, op::PUSH_CONSTANT, 0, op::MOVE_NOW, 2, 2, op::RETURN];
        let script = CobScript {
            functions: vec![CobFunction { name: "AimPrimary".into(), offset: 0 }, CobFunction { name: "FirePrimary".into(), offset: 6 }],
            pieces: names.iter().map(|n| n.to_string()).collect(),
            code,
            static_var_count: 0,
        };
        ta_sim::cob::CobProgram::new(script, &names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
    }

    /// A unit type of no game data's, with `model`.
    fn test_type(model: ta_sim::defs::ModelDef) -> UnitType {
        let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
        data.units.push(ta_data::UnitDef { name: "TESTGUN".into(), object: "testgun".into(), ..Default::default() });
        let mut t = ta_sim::UnitTypes::compile(&data, &|_| None).types.remove(0);
        t.model = std::sync::Arc::new(model);
        t
    }

    /// The barrel's state: pitched up `pitch` degrees, kicked back `kick` units.
    fn barrel(pitch: f32, kick: f32) -> Vec<PieceState> {
        let mut s = vec![([0; 3], [0; 3], true); 4];
        s[2].1[0] = (-pitch / 360.0 * 65536.0) as i32;
        s[2].0[2] = (-kick * 65536.0) as i32;
        s
    }

    /// The barrel's corners (its box's) where the states put them, in the sleeve's frame.
    fn barrel_corners(t: &UnitType, s: &[PieceState]) -> Vec<Vec3> {
        let mut m = Vec::new();
        piece_matrices_between(t, s, s, 1.0, Mat4::IDENTITY, &mut m);
        let to_sleeve = m[1].inverse() * m[2];
        let (lo, hi) = (Vec3::new(-0.6, -0.6, -0.4), Vec3::new(0.6, 0.6, 10.0));
        (0..8).map(|k| to_sleeve.transform_point3(Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z }))).collect()
    }

    /// The barrel's middle at its back end, in the sleeve's frame.
    fn barrel_end(t: &UnitType, s: &[PieceState]) -> Vec3 {
        let c = barrel_corners(t, s);
        (c[0] + c[1] + c[2] + c[3]) / 4.0
    }

    #[test]
    fn a_level_kick_runs_clear_as_the_script_has_it() {
        let (t, corners) = raider();
        let guns = Guns::of(&t, corners.into_iter()).unwrap();
        assert_eq!(guns.len(), 1, "the barrel is the gun; the flare is a marker");
        let s = barrel(0.0, 6.0);
        let mut r = Recoil::default();
        let (_, drawn) = r.states(Some(&guns), true, &s, &s);
        assert_eq!(drawn, &s[..], "six units back into an eight-unit sleeve, level: as TA has it");
    }

    /// Whether the barrel's box, posed so, comes out of the sleeve's box (2 square, -0.5 to 7.6
    /// along z) anywhere along the sleeve: its edges through the sleeve's top, floor or sides.
    fn out_of_the_sleeve(t: &UnitType, s: &[PieceState]) -> bool {
        let c = barrel_corners(t, s);
        // Along each of the barrel's four long edges (corner k to k + 4), the points within the
        // sleeve's length must lie within its cross-section.
        (0..4).any(|k| {
            (0..=64).any(|i| {
                let p = c[k].lerp(c[k + 4], i as f32 / 64.0);
                (-0.5..7.6).contains(&p.z) && (p.x.abs() > 1.0 + 2.0 * THROUGH || p.y.abs() > 1.0 + 2.0 * THROUGH)
            })
        })
    }

    #[test]
    fn a_pitched_kick_goes_back_only_as_far_as_it_runs_clear_of_the_sleeve() {
        let (t, corners) = raider();
        let guns = Guns::of(&t, corners.into_iter()).unwrap();
        let s = barrel(20.0, 6.0);
        // As the script has it, the barrel slides back level and, pitched up, comes up through
        // the sleeve's top.
        assert!(out_of_the_sleeve(&t, &s));
        assert!(!out_of_the_sleeve(&t, &barrel(20.0, 0.0)), "at rest it pitches in the sleeve's mouth");
        let mut r = Recoil::default();
        let (_, drawn) = r.states(Some(&guns), true, &s, &s);
        assert!(!out_of_the_sleeve(&t, drawn), "drawn, it stays in the sleeve");
        let moved = barrel_end(&t, &barrel(20.0, 0.0)) - barrel_end(&t, drawn);
        assert!(moved.length() > 0.3 && moved.length() < 2.5, "a short kick back: {moved:?}");
        assert!(moved.z > 0.0, "back: {moved:?}");
        // The kick as it comes back down the same way: the same room, reckoned once.
        let half = barrel(20.0, 3.0);
        let (_, drawn_half) = r.states(Some(&guns), true, &half, &half);
        assert!(!out_of_the_sleeve(&t, drawn_half));
        assert_eq!(guns.ways.lock().unwrap().len(), 1, "reckoned once");
        // Not close on screen: as the script has it.
        let mut far = Recoil::default();
        let (_, as_is) = far.states(Some(&guns), false, &s, &s);
        assert_eq!(as_is, &s[..]);
    }

    /// Kicked back along the barrel where that runs clear further: a barrel pivoting in the
    /// mouth of a sleeve with room under it, but none over it.
    #[test]
    fn a_pitched_kick_goes_back_along_the_gun_where_that_has_more_room() {
        let (mut t, mut corners) = raider();
        // A sleeve whose top hugs the barrel (just over the pitched barrel where it leaves the
        // mouth) and whose floor lies far below: level back, the pitched barrel at once meets the
        // top; along itself (down as it points up) it has room.
        corners.retain(|c| c.0 != 1);
        corners.extend(cuboid(Vec3::new(-1.0, -4.0, -0.5), Vec3::new(1.0, 0.75, 7.6), &[]).into_iter().flat_map(|tri| tri.map(|c| (1u32, c))));
        let guns = Guns::of(&t, corners.into_iter()).unwrap();
        let s = barrel(20.0, 6.0);
        let mut r = Recoil::default();
        let (_, drawn) = r.states(Some(&guns), true, &s, &s);
        let moved = barrel_end(&t, &barrel(20.0, 0.0)) - barrel_end(&t, drawn);
        assert!(moved.length() > 1.0, "{moved:?}");
        // Along the barrel, pointing 20 degrees up: back and down at that slope.
        let slope = moved.y / moved.z;
        assert!((slope - 20f32.to_radians().tan()).abs() < 0.05, "back along the barrel: {moved:?}");
        t.script = None;
        assert!(Guns::of(&t, std::iter::empty()).is_none(), "no script, no guns");
    }

    /// The installed game's data and unit types (None without an install).
    fn install() -> Option<(ta_vfs::Vfs, std::sync::Arc<ta_data::GameData>, std::sync::Arc<ta_sim::UnitTypes>)> {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        Some((vfs, data, types))
    }

    /// A unit type's model as the game draws it (with no paint), and its guns.
    fn drawn(vfs: &ta_vfs::Vfs, data: &ta_data::GameData, t: &UnitType, library: &mut ta_render::model::ModelLibrary) -> Option<(ta_formats::model3do::Model3do, Guns)> {
        let object = t.def.object.to_ascii_lowercase();
        let model = ta_formats::model3do::Model3do::parse(&vfs.read_opt(&format!("objects3d/{object}.3do"))?).ok()?;
        let id = library.add_shape(&object, &model, &data.palette);
        let guns = Guns::of(t, library.corners(id))?;
        Some((model, guns))
    }

    /// The installed Core Raider (skipped without TA): its barrel, which its aim pitches inside
    /// a sleeve no script turns, kicked back six units by its fire. Level, the whole kick, as
    /// TA has it; pitched up as it lobs a shell, as the script has it the barrel's top comes up
    /// through the sleeve's (the user's report), and as drawn it goes back along itself and stays
    /// in the sleeve.
    #[test]
    fn the_raiders_barrel_kicks_back_inside_its_sleeve() {
        let Some((vfs, data, types)) = install() else { return };
        let t = types.get(types.id("CORRAID").unwrap());
        let mut library = ta_render::model::ModelLibrary::default();
        let (model, guns) = drawn(&vfs, &data, t, &mut library).unwrap();
        assert_eq!(guns.guns.iter().map(|g| t.model.pieces[g.piece].name.as_str()).collect::<Vec<_>>(), ["barrel"]);
        let piece = |name: &str| t.model.pieces.iter().position(|p| p.name.eq_ignore_ascii_case(name)).unwrap();
        let (sleeve, barrel) = (piece("sleeve"), piece("barrel"));
        let pose = |pitch: f32, kick: f32| {
            let mut s: Vec<PieceState> = vec![([0; 3], [0; 3], true); t.model.pieces.len()];
            s[barrel].1[0] = (-pitch / 360.0 * 65536.0) as i32;
            s[barrel].0[2] = (-kick * 65536.0) as i32;
            s
        };
        // The barrel's faces' edges (TA's), in the sleeve's frame: the points of them within the
        // sleeve's length must lie within its cross-section (its faces' extent, TA's).
        let fixed = |v: [i32; 3]| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / ta_formats::model3do::FIXED_ONE;
        let extent = |p: usize| model.pieces[p].vertices.iter().map(|&v| fixed(v)).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        let (lo, hi) = extent(sleeve);
        let out_of_sleeve = |s: &[PieceState]| {
            let mut m = Vec::new();
            piece_matrices_between(t, s, s, 1.0, Mat4::IDENTITY, &mut m);
            let to_sleeve = m[sleeve].inverse() * m[barrel];
            model.pieces[barrel].primitives.iter().filter(|p| p.indices.len() >= 3).any(|p| {
                (0..p.indices.len()).any(|k| {
                    let (a, b) = (fixed(model.pieces[barrel].vertices[p.indices[k] as usize]), fixed(model.pieces[barrel].vertices[p.indices[(k + 1) % p.indices.len()] as usize]));
                    (0..=32).any(|i| {
                        let q = to_sleeve.transform_point3(a.lerp(b, i as f32 / 32.0));
                        q.z > lo.z + 0.1 && q.z < hi.z - 0.1 && (q.y > hi.y + 0.05 || q.y < lo.y - 0.05 || q.x.abs() > hi.x.max(-lo.x) + 0.05)
                    })
                })
            })
        };
        let mut r = Recoil::default();
        // Level: the whole kick.
        let level = pose(0.0, 6.0);
        assert!(!out_of_sleeve(&level));
        let (_, drawn_level) = r.states(Some(&guns), true, &level, &level);
        assert_eq!(drawn_level, &level[..], "level, as TA has it");
        for pitch in [10.0, 18.0, 25.0, 34.0] {
            let s = pose(pitch, 6.0);
            assert!(out_of_sleeve(&s), "{pitch}: as the script has it, through the sleeve");
            let (_, drawn) = r.states(Some(&guns), true, &s, &s);
            assert!(!out_of_sleeve(drawn), "{pitch}: as drawn, in the sleeve");
            // Back along the barrel, a short way.
            let mut m = Vec::new();
            piece_matrices_between(t, drawn, drawn, 1.0, Mat4::IDENTITY, &mut m);
            let mut rest = Vec::new();
            piece_matrices_between(t, &pose(pitch, 0.0), &pose(pitch, 0.0), 1.0, Mat4::IDENTITY, &mut rest);
            let moved = m[barrel].w_axis.truncate() - rest[barrel].w_axis.truncate();
            let axis = rest[barrel].transform_vector3(Vec3::Z);
            assert!(moved.length() > 0.2 && moved.length() < 6.0, "{pitch}: {moved:?}");
            assert!(moved.normalize().dot(-axis) > 0.99, "{pitch}: back along the barrel {moved:?} {axis:?}");
        }
    }

    /// Every unit's guns that recoil (skipped without TA): firing at targets ahead near, at mid
    /// range and far (pitched low to high), every tick, as drawn against their scripts' kicks:
    /// the least share of a kick drawn, which way, and what reckoning them costs. Run with
    /// `cargo test --release -p ta_game recoil_room_report -- --ignored --nocapture`
    /// (`TA_UNITS=CORRAID,ARMSTUMP` for some).
    #[test]
    #[ignore = "reads the locally installed original game; a report"]
    fn recoil_room_report() {
        use ta_sim::{Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, Target};
        let Some((vfs, data, types)) = install() else { return };
        let map = ta_formats::tnt::TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
        let info = ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
        let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
        let players: Vec<PlayerSetup> = (0..2)
            .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(50_000), energy: Fx::from_int(500_000), ai: false })
            .collect();
        let target_type = types.id("CORGOL").unwrap();
        let mut library = ta_render::model::ModelLibrary::default();
        let (mut reckonings, mut spent) = (0usize, std::time::Duration::ZERO);
        let mut rows: Vec<(f32, String)> = Vec::new();
        for t in &types.types {
            if t.locomotion == Locomotion::Air || t.weapons.iter().all(Option::is_none) || only.as_ref().is_some_and(|o| !o.contains(&t.name.to_ascii_uppercase())) {
                continue;
            }
            let Some((_, guns)) = drawn(&vfs, &data, t, &mut library) else { continue };
            let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
            // Somewhere it can stand: level land, or open water for ships.
            let wet = t.def.min_water_depth > 0.0 || t.locomotion == Locomotion::Naval;
            let (w, h) = sim.world.size_px();
            let sea = sim.world.sea_level;
            let spot = (8..(h.to_int() / 64 - 8)).flat_map(|z| (8..(w.to_int() / 64 - 8)).map(move |x| (x * 64, z * 64))).find(|&(x, z)| {
                let at = |dx: i64, dz: i64| sim.world.height_at(Fx::from_int(x + dx), Fx::from_int(z + dz));
                let around = [at(0, 0), at(-96, 0), at(96, 0), at(0, -96), at(0, 96), at(0, 300)];
                if wet { around.iter().all(|y| *y < sea - Fx::from_int(30)) } else { around.iter().all(|y| *y > sea && (*y - around[0]).abs() < Fx::from_int(6)) }
            });
            let Some((x, z)) = spot else { continue };
            let at = FxVec3::new(Fx::from_int(x), if wet { sea } else { sim.world.height_at(Fx::from_int(x), Fx::from_int(z)) }, Fx::from_int(z));
            let Some(id) = sim.spawn_unit(t.id, 0, at, 0, true) else { continue };
            for _ in 0..60 {
                sim.step(&[]);
            }
            let range = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).range.to_f64()).fold(0.0, f64::max);
            let reload = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).reload_ticks).max().unwrap_or(30);
            let mut recoil = Recoil::default();
            // Per gun: the least share of its kick drawn (and the kick, the pitch, along the gun),
            // and its biggest kick.
            let mut least: Vec<(f32, f32, f32, bool)> = vec![(1.0, 0.0, 0.0, false); guns.guns.len()];
            let mut biggest = vec![0.0f32; guns.guns.len()];
            for share in [0.25, 0.55, 0.9] {
                let Some(u) = sim.unit(id) else { break };
                let d = (range * share).clamp(48.0, 1200.0);
                let spot = u.pos + FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::from_f64(d));
                let Some(target) = sim.spawn_unit(target_type, 1, spot, 0, true) else { continue };
                sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Attack(Target::Unit(target)), queue: false }]);
                for _ in 0..(reload * 2 + 90).clamp(150, 600) {
                    sim.step(&[]);
                    let Some(u) = sim.unit(id) else { break };
                    let s: Vec<PieceState> = (0..t.model.pieces.len()).map(|i| u.piece_anim(t, i)).collect();
                    let before = guns.ways.lock().unwrap().len();
                    let started = std::time::Instant::now();
                    let (_, drawn) = recoil.states(Some(&guns), true, &s, &s);
                    let drawn = drawn.to_vec();
                    let reckoned = guns.ways.lock().unwrap().len() - before;
                    if reckoned > 0 {
                        spent += started.elapsed();
                        reckonings += reckoned;
                    }
                    for (gi, g) in guns.guns.iter().enumerate() {
                        let script = kick(&s, g).length();
                        if script < 0.5 {
                            continue;
                        }
                        biggest[gi] = biggest[gi].max(script);
                        // (The drawn kick: the gun's offset from its still pose, all its guns'
                        // kicks taken out.)
                        let mut still = s.clone();
                        for g in &guns.guns {
                            for k in 0..3 {
                                if g.axes & 1 << k != 0 {
                                    still[g.piece].0[k] = 0;
                                }
                            }
                        }
                        let moved = Vec3::from_array([0, 1, 2].map(|k| (drawn[g.piece].0[k] - still[g.piece].0[k]) as f32 / 65536.0));
                        let ratio = moved.length() / script;
                        if ratio < least[gi].0 - 1e-3 {
                            let pitch = -(s[g.piece].1[0] as f32) / 65536.0 * 360.0;
                            // (Drawn along the gun: not along the script's axes alone.)
                            let along = (0..3).any(|k| g.axes & 1 << k == 0 && moved[k].abs() > 0.01);
                            least[gi] = (ratio, script, pitch, along);
                        }
                    }
                }
                sim.remove_unit(target);
            }
            for ((g, &(ratio, script, pitch, along)), &most) in guns.guns.iter().zip(&least).zip(&biggest) {
                let kicked = if ratio < 1.0 { format!("kick {script:4.1}: drawn {:3.0}% (its own pitch {pitch:.0}){}", ratio * 100.0, if along { ", along the gun" } else { "" }) } else if most > 0.0 { format!("kick {most:4.1}: drawn whole") } else { "never kicked here".into() };
                rows.push((if most > 0.0 { ratio } else { 2.0 }, format!("{:10} {:12} {kicked}", t.name, t.model.pieces[g.piece].name)));
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, row) in &rows {
            println!("{row}");
        }
        println!(
            "{} guns, {} kicked here, {} drawn shorter; {reckonings} reckonings, {:.3} ms each",
            rows.len(),
            rows.iter().filter(|r| r.0 <= 1.0).count(),
            rows.iter().filter(|r| r.0 < 0.999).count(),
            spent.as_secs_f64() * 1000.0 / reckonings.max(1) as f64
        );
    }
}
