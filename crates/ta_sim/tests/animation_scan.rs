//! Every unit's animations against its model's geometry (skipped without a TA install; run
//! with `cargo test --release -p ta_sim --test animation_scan -- --ignored --nocapture`).
//!
//! `animation_scan` plays each unit's scripts through their main animations in the
//! simulation (Create, switched off and on, moving and stopping, aiming at four headings,
//! building or producing and stopping, which runs their open and close animations) and
//! looks at the pieces where the simulation puts them: pieces carried across the model's
//! middle by moves, far outside the unit and its footprint, inside their mirror-image
//! partner, through pieces they clear at rest, into the ground under a walker, and turrets
//! not pointing where their aim scripts were told, and walkers and vehicles standing on
//! nothing (their lowest point up off the ground: legs held up in the air). It prints the
//! units ranked by how bad they look and fails on any finding that is not TA's own
//! (`ACCEPTED`). `TA_UNITS=ARMAVP,CORVP` plays a few, `TA_SCAN_ALL=1` lists every unit with a
//! finding, `TA_SCAN_DEBUG=1` prints every sample's pieces, and `TA_SCAN_FLIP=move-x,turn-y`
//! judges the poses with those axes' signs the other way round (to see what such a mistake
//! looks like). `TA_TEST_MOD=<absolute mod folder>` (its mod.toml, as the launcher installs
//! them) mounts a mod over the game and plays the units whose script or model it brings,
//! reporting what it finds without failing (a mod's units have quirks of their own).
//!
//! `script_conventions` re-derives the sign each script axis takes in our frame
//! (docs/FORMATS.md) from the shipped scripts alone: every pose a script holds, read with
//! one axis's sign flipped, against the model's geometry; recoils; walkers' feet; and which
//! way turns of more than half a turn go.
//!
//! `recoil_scan` (a report) fires every unit's guns at targets near, at mid range and far,
//! every tick, and lists how each recoiling gun is rigged (turned by its aim itself, or carried
//! by a piece its aim turns) and what its recoil passes through that it was clear of the moment
//! before, as the scripts move it (`TA_RECOIL_ALONG=1`: along the guns instead). The scene draws
//! recoils only as far as they run clear (`apps/ta_game/src/scene/recoil.rs`).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use rayon::prelude::*;
use ta_data::{GameData, MapInfo};
use ta_formats::cob::{CobScript, op, operand_count};
use ta_formats::model3do::Model3do;
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, Target, Unit, UnitId, UnitType, UnitTypes};

// ---------------------------------------------------------------------------------------
// Geometry (f64: this is a test, nothing here feeds the simulation).

type V3 = [f64; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

type M3 = [[f64; 3]; 3];

fn mm(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}

fn mv(a: &M3, v: V3) -> V3 {
    [dot(a[0], v), dot(a[1], v), dot(a[2], v)]
}

/// `aᵀ·v`: back into a rotated frame.
fn mtv(a: &M3, v: V3) -> V3 {
    [a[0][0] * v[0] + a[1][0] * v[1] + a[2][0] * v[2], a[0][1] * v[0] + a[1][1] * v[1] + a[2][1] * v[2], a[0][2] * v[0] + a[1][2] * v[1] + a[2][2] * v[2]]
}

/// Right-handed rotation about an axis (0 x, 1 y, 2 z) by TA angle units (65536 a turn), as
/// `ta_sim::fixed::rotate_x/y/z`.
fn rot(axis: usize, units: f64) -> M3 {
    let (s, c) = (units * std::f64::consts::TAU / 65536.0).sin_cos();
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

/// A rigid transform: `p ↦ r·p + t`.
#[derive(Clone, Copy, Debug)]
struct Xf {
    r: M3,
    t: V3,
}

impl Xf {
    fn apply(&self, p: V3) -> V3 {
        add(mv(&self.r, p), self.t)
    }
    fn unapply(&self, p: V3) -> V3 {
        mtv(&self.r, sub(p, self.t))
    }
}

/// The signs script values take in our frame (the 3DO's with z negated): moves along x, y, z
/// and turns about x, y, z. Ours, as `ta_sim::unit::piece_world_position` and the renderer's
/// piece matrices apply them (docs/FORMATS.md).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Conv {
    m: [f64; 3],
    t: [f64; 3],
}

const OURS: Conv = Conv { m: [-1.0, 1.0, 1.0], t: [1.0, 1.0, -1.0] };

/// The convention the scan judges poses by: ours, or with `TA_SCAN_FLIP=move-x,turn-y...`
/// those axes the other way round (to see what a sign the other way would look like).
fn scan_conv() -> Conv {
    static CONV: std::sync::OnceLock<Conv> = std::sync::OnceLock::new();
    *CONV.get_or_init(|| {
        let mut c = OURS;
        for f in std::env::var("TA_SCAN_FLIP").unwrap_or_default().split(',').map(str::trim).filter(|f| !f.is_empty()) {
            let (kind, axis) = f.split_once('-').expect("TA_SCAN_FLIP: move-x, turn-y...");
            c = c.flip(kind == "turn", ["x", "y", "z"].iter().position(|a| *a == axis).expect("an axis x, y or z"));
        }
        c
    })
}

impl Conv {
    fn flip(mut self, turn: bool, axis: usize) -> Conv {
        if turn {
            self.t[axis] = -self.t[axis];
        } else {
            self.m[axis] = -self.m[axis];
        }
        self
    }
}

/// A model piece: its place in the hierarchy and its geometry in its own frame.
struct Part {
    name: String,
    parent: Option<usize>,
    offset: V3,
    verts: Vec<V3>,
    edges: Vec<(usize, usize)>,
    tris: Vec<[usize; 3]>,
    /// Corners, edge middles and face middles: what is tested for where the piece is.
    samples: Vec<V3>,
    /// Bounds of its corners in its own frame.
    lo: V3,
    hi: V3,
}

impl Part {
    fn solid(&self) -> bool {
        !self.tris.is_empty()
    }
}

struct Geo {
    parts: Vec<Part>,
    /// Bounds of the whole model at rest.
    lo: V3,
    hi: V3,
    /// Each piece's mirror image across x = 0 at rest, if it has one.
    partner: Vec<Option<usize>>,
    /// Pairs of pieces whose surfaces cross at rest.
    rest_pairs: HashSet<(usize, usize)>,
}

/// Pose: per model piece, (move, turn) as the script has it (pixels, angle units).
type Pose = Vec<(V3, V3)>;

fn rest_pose(geo: &Geo) -> Pose {
    vec![([0.0; 3], [0.0; 3]); geo.parts.len()]
}

fn load_geo(m: &Model3do) -> Geo {
    let f = |v: [i32; 3]| [v[0] as f64 / 65536.0, v[1] as f64 / 65536.0, -(v[2] as f64) / 65536.0];
    let mut parts = Vec::new();
    for p in &m.pieces {
        let verts: Vec<V3> = p.vertices.iter().map(|v| f(*v)).collect();
        let (mut edges, mut tris, mut samples) = (Vec::new(), Vec::new(), Vec::new());
        let mut used = vec![false; verts.len()];
        for (k, prim) in p.primitives.iter().enumerate() {
            if Some(k) == p.selection_primitive {
                continue;
            }
            let idx: Vec<usize> = prim.indices.iter().map(|&i| i as usize).filter(|&i| i < verts.len()).collect();
            if idx.len() < 3 {
                continue;
            }
            for w in 0..idx.len() {
                let (a, b) = (idx[w], idx[(w + 1) % idx.len()]);
                used[a] = true;
                if a != b {
                    edges.push((a.min(b), a.max(b)));
                    samples.push(scale(add(verts[a], verts[b]), 0.5));
                }
            }
            for w in 1..idx.len() - 1 {
                tris.push([idx[0], idx[w], idx[w + 1]]);
            }
            samples.push(scale(idx.iter().fold([0.0; 3], |s, &i| add(s, verts[i])), 1.0 / idx.len() as f64));
        }
        samples.extend(verts.iter().zip(&used).filter(|(_, u)| **u).map(|(v, _)| *v));
        edges.sort_unstable();
        edges.dedup();
        let (lo, hi) = bounds(samples.iter().copied());
        parts.push(Part { name: p.name.to_ascii_lowercase(), parent: p.parent, offset: f(p.offset), verts, edges, tris, samples, lo, hi });
    }
    let mut geo = Geo { parts, lo: [0.0; 3], hi: [0.0; 3], partner: Vec::new(), rest_pairs: HashSet::new() };
    let xf = transforms(&geo, &rest_pose(&geo), &OURS);
    let (lo, hi) = bounds(geo.parts.iter().enumerate().flat_map(|(i, p)| p.samples.iter().map(move |s| (i, *s))).map(|(i, s)| xf[i].apply(s)));
    geo.lo = lo;
    geo.hi = hi;
    geo.partner = mirror_partners(&geo, &xf);
    let w = world_verts(&geo, &xf);
    for i in 0..geo.parts.len() {
        for j in i + 1..geo.parts.len() {
            if pair_crossings(&geo, &w, i, j) > 0 {
                geo.rest_pairs.insert((i, j));
            }
        }
    }
    geo
}

fn bounds(points: impl Iterator<Item = V3>) -> (V3, V3) {
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] { ([0.0; 3], [0.0; 3]) } else { (lo, hi) }
}

/// Every piece's transform into the unit's frame: `T(offset + move) · Ry · Rx · Rz`.
fn transforms(geo: &Geo, pose: &Pose, c: &Conv) -> Vec<Xf> {
    let mut out: Vec<Xf> = Vec::with_capacity(geo.parts.len());
    for (i, p) in geo.parts.iter().enumerate() {
        let (m, t) = pose[i];
        let local = Xf {
            r: mm(&rot(1, c.t[1] * t[1]), &mm(&rot(0, c.t[0] * t[0]), &rot(2, c.t[2] * t[2]))),
            t: [p.offset[0] + c.m[0] * m[0], p.offset[1] + c.m[1] * m[1], p.offset[2] + c.m[2] * m[2]],
        };
        out.push(match p.parent.filter(|&q| q < out.len()) {
            Some(q) => Xf { r: mm(&out[q].r, &local.r), t: out[q].apply(local.t) },
            None => local,
        });
    }
    out
}

fn world_verts(geo: &Geo, xf: &[Xf]) -> Vec<Vec<V3>> {
    geo.parts.iter().zip(xf).map(|(p, x)| p.verts.iter().map(|v| x.apply(*v)).collect()).collect()
}

fn centroid(geo: &Geo, xf: &[Xf], i: usize) -> Option<V3> {
    let s = &geo.parts[i].samples;
    (!s.is_empty()).then(|| scale(s.iter().fold([0.0; 3], |acc, p| add(acc, xf[i].apply(*p))), 1.0 / s.len() as f64))
}

/// Pieces that are each other's mirror image across x = 0 at rest (a left and a right door).
fn mirror_partners(geo: &Geo, xf: &[Xf]) -> Vec<Option<usize>> {
    let n = geo.parts.len();
    let mut partner = vec![None; n];
    let cs: Vec<Option<V3>> = (0..n).map(|i| centroid(geo, xf, i)).collect();
    for i in 0..n {
        let Some(a) = cs[i].filter(|c| c[0].abs() > 1.5 && geo.parts[i].solid()) else { continue };
        let ext = |k: usize| {
            let (lo, hi) = bounds(geo.parts[k].samples.iter().map(|s| xf[k].apply(*s)));
            sub(hi, lo)
        };
        let ea = ext(i);
        let size = ea.iter().fold(0.0f64, |m, v| m.max(*v)).max(1.0);
        for (j, cj) in cs.iter().enumerate() {
            let Some(b) = cj.filter(|_| j != i && geo.parts[j].solid() && geo.parts[j].verts.len() == geo.parts[i].verts.len()) else { continue };
            let eb = ext(j);
            let close = (a[0] + b[0]).abs() < 0.5 + 0.05 * size && (a[1] - b[1]).abs() < 0.5 + 0.05 * size && (a[2] - b[2]).abs() < 0.5 + 0.05 * size;
            let alike = (0..3).all(|k| (ea[k] - eb[k]).abs() < 0.5 + 0.1 * size);
            if close && alike {
                partner[i] = Some(j);
            }
        }
    }
    partner
}

/// How far either end of an edge must be from a face for the edge to count as through it.
const XING_DEPTH: f64 = 0.5;

/// A segment through a triangle, its ends clearly either side of the triangle's plane.
fn seg_tri(a: V3, b: V3, t: [V3; 3]) -> bool {
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let n = cross(e1, e2);
    let nl = dot(n, n).sqrt();
    if nl < 1e-9 {
        return false;
    }
    let n = scale(n, 1.0 / nl);
    let (da, db) = (dot(sub(a, t[0]), n), dot(sub(b, t[0]), n));
    if da * db >= 0.0 || da.abs() < XING_DEPTH || db.abs() < XING_DEPTH {
        return false;
    }
    let p = add(a, scale(sub(b, a), da / (da - db)));
    let v2 = sub(p, t[0]);
    let (d00, d01, d11, d20, d21) = (dot(e1, e1), dot(e1, e2), dot(e2, e2), dot(v2, e1), dot(v2, e2));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return false;
    }
    let (v, w) = ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den);
    v > 1e-4 && w > 1e-4 && v + w < 1.0 - 1e-4
}

/// Edges of either piece through the other's faces.
fn pair_crossings(geo: &Geo, w: &[Vec<V3>], i: usize, j: usize) -> usize {
    let through = |a: usize, b: usize| {
        let mut n = 0;
        for &(p, q) in &geo.parts[a].edges {
            for t in &geo.parts[b].tris {
                if seg_tri(w[a][p], w[a][q], [w[b][t[0]], w[b][t[1]], w[b][t[2]]]) {
                    n += 1;
                }
            }
        }
        n
    };
    through(i, j) + through(j, i)
}

/// Surface crossings between pieces that clear each other at rest, per piece pair.
fn new_crossings(geo: &Geo, xf: &[Xf], visible: Option<&[bool]>) -> Vec<(usize, usize, usize)> {
    let w = world_verts(geo, xf);
    let b: Vec<(V3, V3)> = w.iter().map(|v| bounds(v.iter().copied())).collect();
    let overlap = |a: (V3, V3), c: (V3, V3)| (0..3).all(|k| a.0[k] <= c.1[k] + 0.1 && c.0[k] <= a.1[k] + 0.1);
    let shown = |i: usize| visible.is_none_or(|v| v.get(i).copied().unwrap_or(true));
    let mut out = Vec::new();
    for i in 0..geo.parts.len() {
        if !geo.parts[i].solid() || !shown(i) {
            continue;
        }
        for j in i + 1..geo.parts.len() {
            if !geo.parts[j].solid() || !shown(j) || geo.rest_pairs.contains(&(i, j)) || !overlap(b[i], b[j]) {
                continue;
            }
            let n = pair_crossings(geo, &w, i, j);
            if n > 0 {
                out.push((i, j, n));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Playing a unit's animations in the simulation.

/// The unit's pieces at one moment of a stage.
struct Sample {
    stage: &'static str,
    pose: Pose,
    visible: Vec<bool>,
}

fn snapshot(u: &Unit, t: &UnitType) -> (Pose, Vec<bool>) {
    let n = t.model.pieces.len();
    let mut pose = vec![([0.0; 3], [0.0; 3]); n];
    let mut visible = vec![true; n];
    if let Some(c) = &u.cob {
        for i in 0..n {
            if let Some(&Some(ci)) = t.model_to_cob.get(i) {
                let p = &c.pieces[ci];
                pose[i] = (p.offset.map(|v| v as f64 / 65536.0), p.rotation.map(|v| v as f64));
                visible[i] = p.visible;
            }
        }
    }
    (pose, visible)
}

/// The unit's own transform, as `piece_world_position` composes it.
fn unit_xf(u: &Unit) -> Xf {
    let r = mm(&rot(1, u.heading as f64), &mm(&rot(0, u.pitch as f64), &rot(2, u.roll as f64)));
    Xf { r, t: [u.pos.x.to_f64(), u.pos.y.to_f64(), u.pos.z.to_f64()] }
}

/// A spot on dry, level land and one in open water deep enough for ships.
fn spots(sim: &Sim) -> (FxVec3, Option<FxVec3>) {
    let (w, h) = sim.world.size_px();
    let (w, h) = (w.to_int(), h.to_int());
    let sea = sim.world.sea_level;
    let (mut land, mut water) = (None, None);
    let at = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    let mut z = 256;
    while z < h - 256 && (land.is_none() || water.is_none()) {
        let mut x = 256;
        while x < w - 256 {
            let here = at(x, z);
            let around = [at(x - 96, z), at(x + 96, z), at(x, z - 96), at(x, z + 96), at(x + 160, z + 160), at(x - 160, z - 160), at(x + 300, z), at(x, z + 300)];
            if land.is_none() && here > sea && around.iter().all(|a| (*a - here).abs() < Fx::from_int(6) && *a > sea) {
                land = Some(FxVec3::new(Fx::from_int(x), here, Fx::from_int(z)));
            }
            if water.is_none() && here < sea - Fx::from_int(30) && around.iter().all(|a| *a < sea - Fx::from_int(20)) {
                water = Some(FxVec3::new(Fx::from_int(x), sea, Fx::from_int(z)));
            }
            x += 64;
        }
        z += 64;
    }
    (land.expect("no level land on the test map"), water)
}

/// How a turret pointed after aiming at a target: degrees between its forward (+z, as turrets
/// rest) and the heading its aim script was given, in the horizontal plane. (The engine's
/// headings and the script's turns about y must agree: with either sign the other way round,
/// turrets aim at the mirror image of their target.)
struct AimCheck {
    stage: &'static str,
    turret: usize,
    error_deg: f64,
}

/// The pieces an aim function turns about y straight to its heading argument
/// (`turn turret to y-axis heading`): its turrets, which rest facing forward.
fn heading_turrets(t: &UnitType, func: &str) -> Vec<usize> {
    let Some(p) = &t.script else { return Vec::new() };
    let s = &p.script;
    let Some(fi) = s.function(func) else { return Vec::new() };
    let start = s.functions[fi].offset as usize;
    let end = s.functions.iter().map(|f| f.offset as usize).filter(|&o| o > start).min().unwrap_or(s.code.len());
    let mut out = Vec::new();
    let (mut pc, mut prev) = (start, None);
    while pc < end.min(s.code.len()) {
        let opc = s.code[pc];
        let Some(n) = operand_count(opc) else { break };
        let arg = |k: usize| s.code.get(pc + 1 + k).copied();
        if matches!(opc, op::TURN | op::TURN_NOW) && arg(1) == Some(1) && prev == Some((op::PUSH_LOCAL, Some(0)))
            && let Some(&Some(mp)) = arg(0).and_then(|c| p.piece_map.get(c as usize))
            && !out.contains(&mp)
        {
            out.push(mp);
        }
        prev = Some((opc, if n > 0 { arg(0) } else { None }));
        pc += 1 + n;
    }
    out
}

/// Ticks between samples of a stage.
const EVERY: u32 = 5;

/// Runs `ticks` ticks after giving `commands`, sampling every `every` ticks and at the end.
#[allow(clippy::too_many_arguments)]
fn play(sim: &mut Sim, id: UnitId, t: &UnitType, commands: &[Command], ticks: u32, every: u32, stage: &'static str, out: &mut Vec<Sample>) {
    for k in 1..=ticks {
        sim.step(if k == 1 { commands } else { &[] });
        if k % every == 0 || k == ticks {
            let Some(u) = sim.unit(id) else { return };
            let (pose, visible) = snapshot(u, t);
            out.push(Sample { stage, pose, visible });
        }
    }
}

/// Plays a unit through its main animations; returns what it looked like and how its guns
/// pointed.
fn drive(sim: &mut Sim, t: &UnitType, at: FxVec3, target_type: usize, check: &mut dyn FnMut(&Sim, UnitId)) -> Option<(Vec<Sample>, Vec<AimCheck>)> {
    let id = sim.spawn_unit(t.id, 0, at, 0, true)?;
    let mut out = Vec::new();
    let mut aims = Vec::new();
    play(sim, id, t, &[], 90, EVERY, "built", &mut out);
    check(sim, id);
    if t.def.on_offable {
        play(sim, id, t, &[Command::SetActive { player: 0, units: vec![id], on: false }], 90, EVERY, "switched off", &mut out);
        play(sim, id, t, &[Command::SetActive { player: 0, units: vec![id], on: true }], 90, EVERY, "switched on", &mut out);
        check(sim, id);
    }
    if t.max_speed > Fx::ZERO {
        let here = sim.unit(id)?.pos;
        let to = here + FxVec3::new(Fx::from_int(120), Fx::ZERO, Fx::from_int(260));
        play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Move(to), queue: false }], 150, EVERY, "moving", &mut out);
        check(sim, id);
        play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Stop, queue: false }], 90, EVERY, "stopped", &mut out);
    }
    let range = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).range).max();
    if let Some(range) = range.filter(|_| t.locomotion != Locomotion::Air) {
        let d = (range.to_f64() * 0.6).clamp(80.0, 300.0);
        for (stage, bearing) in [("aim front", 0.0f64), ("aim left", 90.0), ("aim back", 180.0), ("aim right", 270.0)] {
            let Some(u) = sim.unit(id) else { break };
            // Bearings relative to the unit's heading (+z ahead, +x to its left).
            let h = u.heading as f64 / 65536.0 * 360.0 + bearing;
            let (s, c) = h.to_radians().sin_cos();
            let spot = u.pos + FxVec3::new(Fx::from_f64(s * d), Fx::ZERO, Fx::from_f64(c * d));
            let Some(target) = sim.spawn_unit(target_type, 1, spot, 0, true) else { continue };
            // Each turret, once its aim script has answered that it is on target and the turret
            // has come to rest (some answer while it still turns), against the heading the
            // engine asked for (relative to the body).
            sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Attack(Target::Unit(target)), queue: false }]);
            let mut was = [true; 3];
            let mut pending: Vec<(usize, usize, i32)> = Vec::new();
            for k in 1..=90 {
                if k > 1 {
                    sim.step(&[]);
                }
                let Some(u) = sim.unit(id) else { break };
                let (pose, visible) = snapshot(u, t);
                for (slot, was) in was.iter_mut().enumerate() {
                    let ws = &u.weapons[slot];
                    if t.weapons[slot].is_some() && ws.aimed && !*was {
                        pending.extend(heading_turrets(t, ["AimPrimary", "AimSecondary", "AimTertiary"][slot]).into_iter().map(|turret| (slot, turret, ws.aim_heading)));
                    }
                    *was = ws.aimed;
                }
                let turning = |piece: usize| t.model_to_cob[piece].zip(u.cob.as_ref()).is_some_and(|(c, cob)| cob.pieces[c].is_turning(1));
                pending.retain(|&(slot, turret, heading)| {
                    if u.weapons[slot].aim_heading != heading {
                        return false;
                    }
                    if turning(turret) {
                        return true;
                    }
                    let dir = mv(&chain_rotation(t, &pose, turret), [0.0, 0.0, 1.0]);
                    if dir[0].hypot(dir[2]) >= 0.3 {
                        let a = dir[0].atan2(dir[2]) - heading as f64 / 65536.0 * std::f64::consts::TAU;
                        aims.push(AimCheck { stage, turret, error_deg: a.sin().atan2(a.cos()).abs().to_degrees() });
                    }
                    false
                });
                if k % EVERY == 0 {
                    out.push(Sample { stage, pose, visible });
                }
            }
            sim.remove_unit(target);
            play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Stop, queue: false }], 30, 15, stage, &mut out);
        }
        check(sim, id);
    }
    if t.is_builder && !t.build_options.is_empty() {
        let here = sim.unit(id)?.pos;
        if t.is_factory {
            if let Some(o) = t.build_options.iter().copied().find(|&o| sim.types.get(o).locomotion != Locomotion::Static) {
                play(sim, id, t, &[Command::Produce { player: 0, factory: id, type_id: o, count: 1 }], 300, EVERY, "producing", &mut out);
                check(sim, id);
                play(sim, id, t, &[Command::Produce { player: 0, factory: id, type_id: o, count: -1 }], 150, EVERY, "stopped producing", &mut out);
            }
        } else {
            let site = here + FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::from_int(110));
            let o = t.build_options[0];
            play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Build { type_id: o, pos: site }, queue: false }], 240, EVERY, "building", &mut out);
            check(sim, id);
            play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Stop, queue: false }], 90, EVERY, "stopped building", &mut out);
        }
    }
    check(sim, id);
    Some((out, aims))
}

/// A piece's rotation in the unit's frame (ours), from the script's pose.
fn chain_rotation(t: &UnitType, pose: &Pose, piece: usize) -> M3 {
    let mut chain = Vec::new();
    let mut cur = Some(piece);
    while let Some(i) = cur {
        chain.push(i);
        cur = t.model.pieces[i].parent;
        if chain.len() > 256 {
            break;
        }
    }
    let mut r = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for &i in chain.iter().rev() {
        let tn = pose[i].1;
        let c = scan_conv();
        r = mm(&r, &mm(&rot(1, c.t[1] * tn[1]), &mm(&rot(0, c.t[0] * tn[0]), &rot(2, c.t[2] * tn[2]))));
    }
    r
}

// ---------------------------------------------------------------------------------------
// What looks wrong.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    /// A piece clearly off to one side at rest carried to the other side.
    Crossed,
    /// A piece ending up inside its mirror-image partner.
    IntoPartner,
    /// Points far outside the unit's bounds and footprint, sideways.
    Strayed,
    /// Pieces that clear each other at rest passing through each other.
    Through,
    /// Pieces sinking into the ground under a walker or a vehicle on level land.
    Sunk,
    /// A turret not pointing where its aim script was told.
    AimOff,
    /// A walker or vehicle on land standing on nothing: its lowest point (of what shows) well
    /// up off the ground while it stands or walks, its legs held up in the air.
    Lifted,
}

#[derive(Clone, Debug)]
struct Finding {
    kind: Kind,
    what: String,
    stage: &'static str,
    /// How bad: pixels (crossed, strayed, inside, sunk), crossings, degrees.
    amount: f64,
    /// A limb: the piece hangs from one with faces (not the hull itself).
    limb: bool,
}

impl Finding {
    fn score(&self) -> f64 {
        match self.kind {
            Kind::Crossed => 3.0 * self.amount,
            Kind::IntoPartner => 30.0 + 3.0 * self.amount,
            Kind::Strayed => self.amount,
            Kind::Through => self.amount / 2.0,
            Kind::Sunk => self.amount / 4.0,
            Kind::AimOff => self.amount / 3.0,
            Kind::Lifted => self.amount,
        }
    }
}

/// How deep a piece's middle is inside another piece's (rest) box, inset by 15% a side.
fn depth_inside(geo: &Geo, xf: &[Xf], i: usize, j: usize) -> Option<f64> {
    let c = centroid(geo, xf, i)?;
    let local = xf[j].unapply(c);
    let (lo, hi) = (geo.parts[j].lo, geo.parts[j].hi);
    let inset = |k: usize| (hi[k] - lo[k]) * 0.15;
    let depth = (0..3).map(|k| (local[k] - lo[k] - inset(k)).min(hi[k] - inset(k) - local[k])).fold(f64::MAX, f64::min);
    (depth > 0.0).then_some(depth)
}

/// Whether a piece and every piece it hangs from are unturned: where it is, moves alone put
/// it.
fn moved_only(geo: &Geo, pose: &Pose, mut i: usize) -> bool {
    for _ in 0..256 {
        if pose[i].1 != [0.0; 3] {
            return false;
        }
        match geo.parts[i].parent {
            Some(p) => i = p,
            None => return true,
        }
    }
    true
}

fn findings(geo: &Geo, t: &UnitType, samples: &[Sample], aims: &[AimCheck]) -> Vec<Finding> {
    let rest = transforms(geo, &rest_pose(geo), &OURS);
    let rest_c: Vec<Option<V3>> = (0..geo.parts.len()).map(|i| centroid(geo, &rest, i)).collect();
    let size = sub(geo.hi, geo.lo);
    let side_margin = (size[0] * 0.12).max(3.0);
    // The unit's room: its rest bounds and footprint, and a wide margin round them.
    let (fx, fz) = (t.footprint.0 as f64 * 8.0, t.footprint.1 as f64 * 8.0);
    let room_lo = [geo.lo[0].min(-fx), geo.lo[2].min(-fz)];
    let room_hi = [geo.hi[0].max(fx), geo.hi[2].max(fz)];
    let margin = (size.iter().fold(0.0f64, |m, v| m.max(*v)) * 0.75).max(24.0);
    // Feet through the ground show on a walker or a vehicle; buildings hide their pop-up
    // guns below it.
    let mobile_on_land = t.locomotion == Locomotion::Ground && t.def.min_water_depth <= 0.0;
    // What it stands on: the ground (the model's origin), or its lowest point at rest if that
    // is higher; up off it by more than this, it stands on nothing.
    let ground = geo.lo[1].max(0.0);
    let lift_most = (0.2 * (geo.hi[1] - ground)).max(10.0);
    let rest_inside: Vec<bool> = (0..geo.parts.len()).map(|i| geo.partner[i].is_some_and(|j| depth_inside(geo, &rest, i, j).is_some())).collect();
    let limb = |mut i: usize| {
        while let Some(p) = geo.parts[i].parent {
            if geo.parts[p].solid() {
                return true;
            }
            i = p;
        }
        false
    };
    let mut worst: BTreeMap<(Kind, String), Finding> = BTreeMap::new();
    let mut note = |f: Finding| {
        let key = (f.kind, f.what.clone());
        if worst.get(&key).is_none_or(|w| w.amount < f.amount) {
            worst.insert(key, f);
        }
    };
    for s in samples {
        let xf = transforms(geo, &s.pose, &scan_conv());
        let shown = |i: usize| s.visible.get(i).copied().unwrap_or(true);
        for (i, p) in geo.parts.iter().enumerate() {
            if !shown(i) || !p.solid() {
                continue;
            }
            // Across the middle, by moves alone (turns carry pieces round to the other side as
            // they should: turrets, spinning radars, a gatling's barrels).
            if let (Some(c0), Some(c)) = (rest_c[i], centroid(geo, &xf, i))
                && moved_only(geo, &s.pose, i)
                && c0[0].abs() > side_margin
                && c[0].abs() > side_margin
                && (c[0] > 0.0) != (c0[0] > 0.0)
            {
                note(Finding { kind: Kind::Crossed, what: p.name.clone(), stage: s.stage, amount: c[0].abs().min(c0[0].abs()), limb: limb(i) });
            }
            // Far out, sideways.
            let (mut out, mut sunk) = (0.0f64, 0.0f64);
            for q in &p.samples {
                let w = xf[i].apply(*q);
                for (k, axis) in [(0, 0), (1, 2)] {
                    out = out.max(room_lo[k] - margin - w[axis]).max(w[axis] - room_hi[k] - margin);
                }
                sunk = sunk.max(geo.lo[1] - w[1]);
            }
            if out > 0.0 {
                note(Finding { kind: Kind::Strayed, what: p.name.clone(), stage: s.stage, amount: out, limb: limb(i) });
            }
            if mobile_on_land && sunk > 4.0 {
                note(Finding { kind: Kind::Sunk, what: p.name.clone(), stage: s.stage, amount: sunk, limb: limb(i) });
            }
            // Inside its mirror image (when it is not at rest).
            if let Some(j) = geo.partner[i].filter(|&j| shown(j) && !rest_inside[i])
                && let Some(depth) = depth_inside(geo, &xf, i, j)
            {
                note(Finding { kind: Kind::IntoPartner, what: format!("{} in {}", p.name, geo.parts[j].name), stage: s.stage, amount: depth, limb: limb(i) });
            }
        }
        // Standing on nothing: the lowest point of what shows well up off the ground while it
        // stands, walks or has stopped (a walker's legs held up in the air).
        if mobile_on_land && matches!(s.stage, "built" | "moving" | "stopped") {
            let low = geo
                .parts
                .iter()
                .enumerate()
                .filter(|&(i, p)| shown(i) && p.solid())
                .flat_map(|(i, p)| p.samples.iter().map(move |q| (i, *q)))
                .map(|(i, q)| xf[i].apply(q)[1])
                .fold(f64::MAX, f64::min);
            if low < f64::MAX && low - ground > lift_most {
                note(Finding { kind: Kind::Lifted, what: "its lowest point".into(), stage: s.stage, amount: low - ground, limb: false });
            }
        }
        // Through each other: pieces that clear each other at rest (but a piece and the one
        // it hangs from, and a gun's recoil into its turret while aiming).
        if !s.stage.starts_with("aim") {
            for (i, j, n) in new_crossings(geo, &xf, Some(&s.visible)) {
                if geo.parts[j].parent != Some(i) && geo.parts[i].parent != Some(j) && n >= 6 {
                    note(Finding { kind: Kind::Through, what: format!("{} / {}", geo.parts[i].name, geo.parts[j].name), stage: s.stage, amount: n as f64, limb: limb(i) && limb(j) });
                }
            }
        }
    }
    for a in aims {
        if a.error_deg > 30.0 {
            note(Finding { kind: Kind::AimOff, what: geo.parts[a.turret].name.clone(), stage: a.stage, amount: a.error_deg, limb: limb(a.turret) });
        }
    }
    worst.into_values().collect()
}

/// Units whose own scripts pass pieces through each other as TA plays them (TA's models are
/// not watertight): the Core Kbot Lab's halves fold out over its pad and nanolathes; the Core
/// Construction Hovercraft's and Advanced Construction Sub's hatch doors swing the short way
/// round, down through the hull past the stowed nanolathe (turns go the short way, see
/// docs/FORMATS.md); the Implosion Devices' collars spin round their towers; the Moho Metal
/// Makers' lids nest into each other and the Core one's glowing core turns through its base
/// as it rises; the Advanced Sonar Station's dishes fold down past its wheel and the Advanced
/// Aircraft Plant's nanolathes reach past each other's boxes; the rest are walking or building
/// limbs grazing each other.
const THROUGH_AS_TA: &[&str] = &[
    "ARMAAP", "ARMASON", "ARMCK", "ARMDEV1", "ARMFARK", "ARMFIDO", "ARMMMKR", "CORACK", "CORACSUB", "CORCH", "CORCK", "CORDEV1", "CORKROG", "CORLAB", "CORMMKR",
];

/// Fewer crossings than this between two pieces anywhere are left to the list: a limb grazing
/// another for a moment.
const THROUGH_GRAZE: f64 = 10.0;

/// A walker's toes dig this far into the ground for a moment as it stops mid-stride (a hull
/// never should).
const SUNK_AS_TA: f64 = 12.0;

fn accepted(unit: &str, f: &Finding) -> bool {
    match f.kind {
        Kind::Through => f.amount < THROUGH_GRAZE || THROUGH_AS_TA.contains(&unit),
        Kind::Sunk => f.limb && f.amount <= SUNK_AS_TA,
        _ => false,
    }
}

/// The game, and the mod `TA_TEST_MOD` names (a folder with its mod.toml) mounted over it as
/// the game mounts it, with its id.
fn game_and_mod() -> Option<(ta_vfs::Vfs, Option<String>)> {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return None;
    };
    let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).expect("TA_TEST_MOD: a mod's folder").expect("TA_TEST_MOD: a mod's folder"));
    if let Some(info) = &module {
        println!("mod: {} {}", info.manifest.name, info.manifest.version);
    }
    // (As the game mounts it: what it replaces of the install left out.)
    let mounted = ta_vfs::mods::mount(Some(&install.root), None, &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
    mounted.install.unwrap();
    mounted.mods.expect("the mod mounts");
    Some((mounted.vfs, module.map(|m| m.manifest.id)))
}

#[test]
#[ignore]
fn animation_scan() {
    let Some((vfs, mod_id)) = game_and_mod() else { return };
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(50_000), energy: Fx::from_int(500_000), ai: false })
        .collect();
    let target_type = types.id("CORGOL").or_else(|| types.id("ARMBULL")).unwrap();
    let mut models: BTreeMap<String, Option<Arc<Geo>>> = BTreeMap::new();
    for t in &types.types {
        let object = t.def.object.to_ascii_lowercase();
        models.entry(object.clone()).or_insert_with(|| vfs.read_opt(&format!("objects3d/{object}.3do")).and_then(|b| Model3do::parse(&b).ok()).map(|m| Arc::new(load_geo(&m))));
    }
    // (A mod's units: those whose script or model it brings.)
    let the_mods = |t: &UnitType| {
        let object = t.def.object.to_ascii_lowercase();
        let files = t.def.script_paths().into_iter().chain([format!("objects3d/{object}.3do")]);
        mod_id.as_ref().is_none_or(|id| files.into_iter().any(|p| vfs.origin(&p).is_some_and(|o| o.layer == *id)))
    };
    let picked: Vec<&UnitType> = types.types.iter().filter(|t| t.script.is_some() && the_mods(t) && only.as_ref().is_none_or(|o| o.contains(&t.name.to_ascii_uppercase()))).collect();
    let mismatches = std::sync::Mutex::new(Vec::<String>::new());
    let mut reports: Vec<(String, f64, Vec<Finding>, usize)> = picked
        .par_iter()
        .filter_map(|t| {
            let geo = models.get(&t.def.object.to_ascii_lowercase())?.clone()?;
            let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
            let (land, water) = spots(&sim);
            let wet = t.def.min_water_depth > 0.0 || t.locomotion == Locomotion::Naval;
            let at = if wet { water? } else { land };
            // The scan's piece transforms must be the simulation's.
            let mut check = |sim: &Sim, id: UnitId| {
                let Some(u) = sim.unit(id) else { return };
                let (pose, _) = snapshot(u, t);
                let xf = transforms(&geo, &pose, &OURS);
                let ux = unit_xf(u);
                for (i, x) in xf.iter().enumerate() {
                    let ours = ux.apply(x.t);
                    let sim_pos = ta_sim::piece_world_position(u, t, u.cob.as_ref(), i);
                    let theirs = [sim_pos.x.to_f64(), sim_pos.y.to_f64(), sim_pos.z.to_f64()];
                    let d = sub(ours, theirs);
                    // (The simulation's table trigonometry against f64: a piece a mod's script
                    // parks tens of thousands of pixels away is a third of a pixel out.)
                    let far = sub(ours, ux.t);
                    if dot(d, d).sqrt() > 0.1 + 1e-5 * dot(far, far).sqrt() {
                        mismatches.lock().unwrap().push(format!("{} {}: scan {:?} simulation {:?}", t.name, geo.parts[i].name, ours, theirs));
                    }
                }
            };
            let (samples, aims) = drive(&mut sim, t, at, target_type, &mut check)?;
            if std::env::var("TA_SCAN_DEBUG").is_ok() {
                for s in &samples {
                    let xf = transforms(&geo, &s.pose, &OURS);
                    let line: Vec<String> = (0..geo.parts.len())
                        .filter_map(|i| centroid(&geo, &xf, i).map(|c| format!("{} ({:.0},{:.0},{:.0}){}", geo.parts[i].name, c[0], c[1], c[2], if s.visible[i] { "" } else { "h" })))
                        .collect();
                    println!("{} {}: {}", t.name, s.stage, line.join(" "));
                    let moved: Vec<String> = (0..geo.parts.len())
                        .filter(|&i| s.pose[i].0 != [0.0; 3] || s.pose[i].1 != [0.0; 3])
                        .map(|i| format!("{} m{:?} t{:?}", geo.parts[i].name, s.pose[i].0, s.pose[i].1))
                        .collect();
                    println!("    {}", moved.join("; "));
                }
            }
            let found = findings(&geo, t, &samples, &aims);
            let score: f64 = found.iter().map(Finding::score).sum();
            Some((t.name.clone(), score, found, aims.len()))
        })
        .collect();
    let mismatches = mismatches.into_inner().unwrap();
    assert!(mismatches.is_empty(), "the scan places pieces where the simulation does: {:#?}", &mismatches[..mismatches.len().min(20)]);
    reports.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let flagged = reports.iter().filter(|r| r.1 > 0.0).count();
    let aims: usize = reports.iter().map(|r| r.3).sum();
    println!("{} units played ({aims} turret aims checked), {flagged} with findings", reports.len());
    let all = std::env::var("TA_SCAN_ALL").is_ok();
    for (name, score, found, _) in reports.iter().filter(|r| r.1 > 0.0).take(if all { usize::MAX } else { 60 }) {
        println!("{name:10} {score:7.1}");
        let mut found = found.clone();
        found.sort_by(|a, b| b.score().total_cmp(&a.score()));
        for f in found.iter().take(6) {
            println!("    {:?} {} ({}): {:.1}{}", f.kind, f.what, f.stage, f.amount, if accepted(name, f) { "  (as TA has it)" } else { "" });
        }
    }
    let mut by_kind: BTreeMap<Kind, usize> = BTreeMap::new();
    for (_, _, found, _) in &reports {
        for f in found {
            *by_kind.entry(f.kind).or_default() += 1;
        }
    }
    println!("findings by kind: {by_kind:?}");
    let unexpected: Vec<String> = reports
        .iter()
        .flat_map(|(name, _, found, _)| found.iter().filter(|f| !accepted(name, f)).map(move |f| format!("{name}: {:?} {} ({}): {:.1}", f.kind, f.what, f.stage, f.amount)))
        .collect();
    println!("{} findings not TA's own", unexpected.len());
    if scan_conv() == OURS && mod_id.is_none() {
        assert!(unexpected.is_empty(), "animations gone wrong (look at them in the game: --demo gallery with TA_GALLERY_UNITS): {unexpected:#?}");
    }
}

// ---------------------------------------------------------------------------------------
// Recoils: guns sliding back as they fire, and what they pass through.

/// What the functions the engine calls of one kind (`fire`: `FirePrimary`...; `aim`:
/// `AimPrimary`...), and those they call or start, do to each COB piece: the axes they move it
/// along (`moves`) or turn it about (bit 0 x, 1 y, 2 z).
fn script_axes(script: &CobScript, kind: &str, moves: bool) -> Vec<u8> {
    let code = &script.code;
    let mut starts: Vec<usize> = script.functions.iter().map(|f| f.offset as usize).collect();
    starts.sort_unstable();
    starts.dedup();
    let end_of = |start: usize| starts.iter().copied().find(|&o| o > start).unwrap_or(code.len()).min(code.len());
    let mut out = vec![0u8; script.pieces.len()];
    let mut todo: Vec<usize> = script.functions.iter().enumerate().filter(|(_, f)| f.name.to_ascii_lowercase().starts_with(kind)).map(|(i, _)| i).collect();
    let mut seen = vec![false; script.functions.len()];
    while let Some(fi) = todo.pop() {
        if std::mem::replace(&mut seen[fi], true) {
            continue;
        }
        let mut pc = script.functions[fi].offset as usize;
        let end = end_of(pc);
        while pc < end {
            let opc = code[pc];
            let Some(n) = operand_count(opc) else { break };
            let arg = |k: usize| code.get(pc + 1 + k).map(|&v| v as usize);
            let ours = if moves { matches!(opc, op::MOVE | op::MOVE_NOW) } else { matches!(opc, op::TURN | op::TURN_NOW) };
            if ours && let (Some(p), Some(a)) = (arg(0), arg(1)) && a < 3 && p < out.len() {
                out[p] |= 1 << a;
            }
            if matches!(opc, op::START_SCRIPT | op::CALL_SCRIPT) && let Some(f) = arg(0).filter(|&f| f < seen.len()) {
                todo.push(f);
            }
            pc += 1 + n;
        }
    }
    out
}

/// Per model piece, the axes a script's fire functions move it along (its recoil) or its aim
/// functions turn it about (a gun pointing itself).
fn gun_axes(t: &UnitType, kind: &str, moves: bool) -> Vec<u8> {
    let Some(p) = &t.script else { return vec![0; t.model.pieces.len()] };
    let axes = script_axes(&p.script, kind, moves);
    (0..t.model.pieces.len()).map(|i| t.model_to_cob.get(i).copied().flatten().and_then(|c| axes.get(c).copied()).unwrap_or(0)).collect()
}

/// Every piece's transform as `transforms` makes it, but for the moves along the axes `along`
/// marks, made in the piece's own turned frame (`T(offset) · R · T(move)`): a gun recoiling
/// along the line it points rather than level in its parent's frame.
fn transforms_along(geo: &Geo, pose: &Pose, c: &Conv, along: &[u8]) -> Vec<Xf> {
    let mut out: Vec<Xf> = Vec::with_capacity(geo.parts.len());
    for (i, p) in geo.parts.iter().enumerate() {
        let (m, t) = pose[i];
        let r = mm(&rot(1, c.t[1] * t[1]), &mm(&rot(0, c.t[0] * t[0]), &rot(2, c.t[2] * t[2])));
        let shift = [c.m[0] * m[0], c.m[1] * m[1], c.m[2] * m[2]];
        let mask = along.get(i).copied().unwrap_or(0);
        let own: V3 = std::array::from_fn(|k| if mask & 1 << k != 0 { shift[k] } else { 0.0 });
        let level: V3 = std::array::from_fn(|k| if mask & 1 << k != 0 { 0.0 } else { shift[k] });
        let local = Xf { r, t: add(add(p.offset, level), mv(&r, own)) };
        out.push(match p.parent.filter(|&q| q < out.len()) {
            Some(q) => Xf { r: mm(&out[q].r, &local.r), t: out[q].apply(local.t) },
            None => local,
        });
    }
    out
}

/// Edges of piece a through faces of piece b, and of b through a's: (b's edge through a's face,
/// the edge, the face's triangle).
fn crossing_list(geo: &Geo, w: &[Vec<V3>], a: usize, b: usize) -> Vec<(bool, usize, usize)> {
    let mut out = Vec::new();
    for (flip, (x, y)) in [(false, (a, b)), (true, (b, a))] {
        for (e, &(p, q)) in geo.parts[x].edges.iter().enumerate() {
            for (k, t) in geo.parts[y].tris.iter().enumerate() {
                if seg_tri(w[x][p], w[x][q], [w[y][t[0]], w[y][t[1]], w[y][t[2]]]) {
                    out.push((flip, e, k));
                }
            }
        }
    }
    out
}

/// Markers rather than parts of the body: muzzle flashes, fire points and the like.
fn marker(name: &str) -> bool {
    ["flare", "fire", "point", "spray", "emit", "flash"].iter().any(|m| name.contains(m))
}

/// What a pose's recoils pass through: for each gun recoiled (a piece its fire functions move,
/// off its rest along those axes), with what it carries, the crossings with the rest of the
/// model (its edges through their faces, theirs through its) that the same pose without the
/// recoil doesn't have, but for those of it backing head on into what it slides into (a barrel
/// into its turret through the turret's front: the face it meets faces it, within 60 degrees).
/// (gun, the piece passed through, crossings.) `along`: the moves made in the pieces' own
/// frames (see `transforms_along`).
fn recoil_through(geo: &Geo, pose: &Pose, visible: &[bool], recoil: &[u8], along: &[u8]) -> Vec<(usize, usize, usize)> {
    let n = geo.parts.len();
    let recoiled: Vec<usize> = (0..n).filter(|&i| (0..3).any(|k| recoil[i] & 1 << k != 0 && pose[i].0[k].abs() > 0.05)).collect();
    if recoiled.is_empty() {
        return Vec::new();
    }
    let mut still = pose.clone();
    for &i in &recoiled {
        for k in 0..3 {
            if recoil[i] & 1 << k != 0 {
                still[i].0[k] = 0.0;
            }
        }
    }
    let (xf, xf0) = (transforms_along(geo, pose, &OURS, along), transforms_along(geo, &still, &OURS, along));
    let (w, w0) = (world_verts(geo, &xf), world_verts(geo, &xf0));
    let shown = |i: usize| visible.get(i).copied().unwrap_or(true) && geo.parts[i].solid() && !marker(&geo.parts[i].name);
    let b: Vec<(V3, V3)> = w.iter().map(|v| bounds(v.iter().copied())).collect();
    let overlap = |a: (V3, V3), c: (V3, V3)| (0..3).all(|k| a.0[k] <= c.1[k] + 0.1 && c.0[k] <= a.1[k] + 0.1);
    let mut out = Vec::new();
    for &g in &recoiled {
        let moved = subtree(geo, g);
        let d = sub(xf[g].t, xf0[g].t);
        let len = dot(d, d).sqrt();
        if len < 0.05 {
            continue;
        }
        let d = scale(d, 1.0 / len);
        for i in (0..n).filter(|&i| moved[i] && shown(i)) {
            for j in (0..n).filter(|&j| !moved[j] && shown(j) && overlap(b[i], b[j])) {
                let now = crossing_list(geo, &w, i, j);
                if now.is_empty() {
                    continue;
                }
                let before = crossing_list(geo, &w0, i, j);
                let count = now
                    .iter()
                    .filter(|x| !before.contains(x))
                    .filter(|&&(flip, _, k)| {
                        // The face crossed (outward: TA's faces are clockwise seen from outside,
                        // z flipped), and the way the edge goes through it.
                        let (face_piece, rel) = if flip { (i, scale(d, -1.0)) } else { (j, d) };
                        let (ww, t) = (&w[face_piece], geo.parts[face_piece].tris[k]);
                        let normal = cross(sub(ww[t[2]], ww[t[0]]), sub(ww[t[1]], ww[t[0]]));
                        let nl = dot(normal, normal).sqrt();
                        nl < 1e-9 || dot(normal, rel) / nl > -0.5
                    })
                    .count();
                if count > 0 {
                    out.push((g, j, count));
                }
            }
        }
    }
    out
}

/// Plays a unit firing at targets straight ahead, near, at mid range and far (its guns pitched
/// low to high), sampled every tick.
fn fire_drive(sim: &mut Sim, t: &UnitType, at: FxVec3, target_type: usize) -> Option<Vec<Sample>> {
    let id = sim.spawn_unit(t.id, 0, at, 0, true)?;
    let mut out = Vec::new();
    play(sim, id, t, &[], 60, 60, "built", &mut out);
    let weapons: Vec<&ta_sim::WeaponType> = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).collect();
    let range = weapons.iter().map(|w| w.range.to_f64()).fold(0.0, f64::max);
    let reload = weapons.iter().map(|w| w.reload_ticks).max().unwrap_or(30);
    for (stage, share) in [("fire near", 0.25), ("fire mid", 0.55), ("fire far", 0.9)] {
        let Some(u) = sim.unit(id) else { break };
        let d = (range * share).clamp(48.0, 1200.0);
        let h = u.heading as f64 / 65536.0 * std::f64::consts::TAU;
        let spot = u.pos + FxVec3::new(Fx::from_f64(h.sin() * d), Fx::ZERO, Fx::from_f64(h.cos() * d));
        let Some(target) = sim.spawn_unit(target_type, 1, spot, 0, true) else { continue };
        let ticks = (reload * 2 + 90).clamp(150, 600);
        play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Attack(Target::Unit(target)), queue: false }], ticks, 1, stage, &mut out);
        sim.remove_unit(target);
        play(sim, id, t, &[Command::Order { player: 0, units: vec![id], order: Order::Stop, queue: false }], 30, 30, stage, &mut out);
    }
    Some(out)
}

/// `recoil_scan` (report only): every unit with guns that recoil (pieces its fire functions
/// move), firing at targets ahead near, at mid range and far, every tick: what each gun's
/// recoil passes through that it was clear of the moment before (see `recoil_through`), as
/// TA's frame has it (a move level in its parent's frame) and with recoils drawn along the gun
/// (`TA_RECOIL_ALONG=1`: in the gun's own turned frame). And how each recoiling gun is rigged:
/// turned by its aim itself (`self` + the axes), or carried by a piece its aim turns. Run with
/// `cargo test --release -p ta_sim --test animation_scan recoil_scan -- --ignored --nocapture`
/// (`TA_UNITS=CORRAID,ARMSTUMP` for some).
#[test]
#[ignore]
fn recoil_scan() {
    let Some((vfs, _)) = game_and_mod() else { return };
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
    let along_axis = std::env::var("TA_RECOIL_ALONG").is_ok_and(|v| v == "1");
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(50_000), energy: Fx::from_int(500_000), ai: false })
        .collect();
    let target_type = types.id("CORGOL").or_else(|| types.id("ARMBULL")).unwrap();
    let mut models: BTreeMap<String, Option<Arc<Geo>>> = BTreeMap::new();
    for t in &types.types {
        let object = t.def.object.to_ascii_lowercase();
        models.entry(object.clone()).or_insert_with(|| vfs.read_opt(&format!("objects3d/{object}.3do")).and_then(|b| Model3do::parse(&b).ok()).map(|m| Arc::new(load_geo(&m))));
    }
    let picked: Vec<&UnitType> = types
        .types
        .iter()
        .filter(|t| t.script.is_some() && t.locomotion != Locomotion::Air && t.weapons.iter().any(Option::is_some))
        .filter(|t| only.as_ref().is_none_or(|o| o.contains(&t.name.to_ascii_uppercase())))
        .filter(|t| gun_axes(t, "fire", true).iter().any(|&m| m != 0))
        .collect();
    type Report = (String, Vec<String>, BTreeMap<(usize, usize), (usize, &'static str, f64)>);
    let mut reports: Vec<Report> = picked
        .par_iter()
        .filter_map(|t| {
            let geo = models.get(&t.def.object.to_ascii_lowercase())?.clone()?;
            let recoil = gun_axes(t, "fire", true);
            let aim = gun_axes(t, "aim", false);
            let along: Vec<u8> = if along_axis { recoil.clone() } else { vec![0; recoil.len()] };
            // How each recoiling gun is rigged: what its aim turns of it and of what carries it.
            let rig: Vec<String> = (0..geo.parts.len())
                .filter(|&i| recoil[i] != 0 && geo.parts[i].solid() && !marker(&geo.parts[i].name))
                .map(|i| {
                    let axes = |m: u8| ["x", "y", "z"].iter().enumerate().filter(|(k, _)| m & 1 << k != 0).map(|(_, a)| *a).collect::<String>();
                    let mut carried = Vec::new();
                    let mut q = geo.parts[i].parent;
                    while let Some(p) = q {
                        if aim[p] != 0 {
                            carried.push(format!("{} {}", geo.parts[p].name, axes(aim[p])));
                        }
                        q = geo.parts[p].parent;
                    }
                    format!("{} moves {}{}{}", geo.parts[i].name, axes(recoil[i]), if aim[i] != 0 { format!(", self {}", axes(aim[i])) } else { String::new() }, if carried.is_empty() { String::new() } else { format!(", on {}", carried.join(", ")) })
                })
                .collect();
            let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
            let (land, water) = spots(&sim);
            let wet = t.def.min_water_depth > 0.0 || t.locomotion == Locomotion::Naval;
            let at = if wet { water? } else { land };
            let samples = fire_drive(&mut sim, t, at, target_type)?;
            // The worst of each gun and what it passes through: crossings, stage, the gun's pitch
            // (degrees up, from its turns and its carriers').
            let mut worst: BTreeMap<(usize, usize), (usize, &'static str, f64)> = BTreeMap::new();
            for s in &samples {
                for (g, j, count) in recoil_through(&geo, &s.pose, &s.visible, &recoil, &along) {
                    let pitch = mv(&chain_rotation(t, &s.pose, g), [0.0, 0.0, 1.0])[1].clamp(-1.0, 1.0).asin().to_degrees();
                    let e = worst.entry((g, j)).or_insert((0, s.stage, pitch));
                    if count > e.0 {
                        *e = (count, s.stage, pitch);
                    }
                }
            }
            Some((t.name.clone(), rig, worst))
        })
        .collect();
    reports.sort_by(|a, b| {
        let most = |r: &Report| r.2.values().map(|v| v.0).max().unwrap_or(0);
        most(b).cmp(&most(a)).then(a.0.cmp(&b.0))
    });
    println!("{} units with recoiling guns{}", reports.len(), if along_axis { ", recoils along the guns" } else { ", recoils level in their parents' frames (TA's)" });
    let mut clipping = 0;
    for (name, rig, worst) in &reports {
        let geo = models.get(&types.get(types.id(name).unwrap()).def.object.to_ascii_lowercase()).cloned().flatten().unwrap();
        let mut list: Vec<(&(usize, usize), &(usize, &'static str, f64))> = worst.iter().collect();
        list.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        let through: Vec<String> = list.iter().take(4).map(|((g, j), (n, stage, pitch))| format!("{} through {} {n} ({stage}, pitched {pitch:.0})", geo.parts[*g].name, geo.parts[*j].name)).collect();
        if !through.is_empty() {
            clipping += 1;
        }
        println!("{name:10} {}{}", rig.join("; "), if through.is_empty() { String::new() } else { format!("\n           {}", through.join(", ")) });
    }
    println!("{clipping} of {} units' recoils pass through something", reports.len());
}

// ---------------------------------------------------------------------------------------
// Joints that come apart.

/// Closest distance from a point to a triangle.
fn point_tri(p: V3, t: [V3; 3]) -> f64 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let closest = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        let cp = sub(p, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        let (vc, vb, va) = (d1 * d4 - d3 * d2, d5 * d2 - d1 * d6, d3 * d6 - d5 * d4);
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else if d6 >= 0.0 && d5 <= d6 {
            c
        } else if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            add(a, scale(ab, d1 / (d1 - d3)))
        } else if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            add(a, scale(ac, d2 / (d2 - d6)))
        } else if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            add(b, scale(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))))
        } else {
            let den = 1.0 / (va + vb + vc);
            add(a, add(scale(ab, vb * den), scale(ac, vc * den)))
        }
    };
    let d = sub(p, closest);
    dot(d, d).sqrt()
}

/// How far apart two pieces' surfaces are (0 where they cross or touch): the nearest of
/// either's corners, edge middles and face middles to the other's faces.
fn surface_gap(geo: &Geo, w: &[Vec<V3>], xf: &[Xf], i: usize, j: usize) -> f64 {
    if pair_crossings(geo, w, i, j) > 0 {
        return 0.0;
    }
    let near = |a: usize, b: usize| {
        let mut best = f64::MAX;
        for s in &geo.parts[a].samples {
            let p = xf[a].apply(*s);
            for t in &geo.parts[b].tris {
                best = best.min(point_tri(p, [w[b][t[0]], w[b][t[1]], w[b][t[2]]]));
            }
        }
        best
    };
    near(i, j).min(near(j, i))
}

/// Two pieces' bounds in the unit's frame overlap, given `slack` more room.
fn near_bounds(geo: &Geo, xf: &[Xf], i: usize, j: usize, slack: f64) -> bool {
    let b = |k: usize| bounds(geo.parts[k].samples.iter().map(|s| xf[k].apply(*s)));
    let ((alo, ahi), (blo, bhi)) = (b(i), b(j));
    (0..3).all(|k| alo[k] - slack <= bhi[k] && blo[k] - slack <= ahi[k])
}

/// Along the vertical line through piece b's pivot at rest, how far apart a's and b's surfaces
/// facing each other are (the connector between them would span it: see the renderer's
/// model/connectors.rs); none where the line misses either.
fn vertical_gap(geo: &Geo, xf: &[Xf], a: usize, b: usize) -> Option<f64> {
    let (x, z) = (xf[b].t[0], xf[b].t[2]);
    let heights = |k: usize| -> Vec<f64> {
        let w: Vec<V3> = geo.parts[k].verts.iter().map(|v| xf[k].apply(*v)).collect();
        geo.parts[k].tris.iter().filter_map(|t| {
            let (p, q, r) = (w[t[0]], w[t[1]], w[t[2]]);
            let den = (q[2] - r[2]) * (p[0] - r[0]) + (r[0] - q[0]) * (p[2] - r[2]);
            if den.abs() < 1e-9 {
                return None;
            }
            let u = ((q[2] - r[2]) * (x - r[0]) + (r[0] - q[0]) * (z - r[2])) / den;
            let v = ((r[2] - p[2]) * (x - r[0]) + (p[0] - r[0]) * (z - r[2])) / den;
            (u >= -1e-4 && v >= -1e-4 && u + v <= 1.0 + 1e-4).then(|| u * p[1] + v * q[1] + (1.0 - u - v) * r[1])
        }).collect()
    };
    let (ha, hb) = (heights(a), heights(b));
    if ha.is_empty() || hb.is_empty() {
        return None;
    }
    let (a_lo, a_hi) = (ha.iter().copied().fold(f64::MAX, f64::min), ha.iter().copied().fold(f64::MIN, f64::max));
    let (b_lo, b_hi) = (hb.iter().copied().fold(f64::MAX, f64::min), hb.iter().copied().fold(f64::MIN, f64::max));
    Some(if b_lo + b_hi > a_lo + a_hi { b_lo - a_hi } else { a_lo - b_hi })
}

/// A pair of pieces touching at rest and how far apart the animations took them.
#[derive(Clone, Debug)]
struct Apart {
    a: usize,
    b: usize,
    gap: f64,
    stage: &'static str,
}

/// `joint_scan` (report only): every unit's pieces that touch their parent at rest and come
/// apart from it as the unit stands, walks and stops (a walker's feet dropping away from its
/// body), by how far, and the solid pieces touching nothing even at rest (TA's high camera hid
/// the air round them). Doors, covers and wings opening to build, produce or fire are left
/// out: they are meant to part. Run with `cargo test --release -p ta_sim --test animation_scan
/// joint_scan -- --ignored --nocapture` (`TA_UNITS=ARMCK,CORKROG` for some;
/// `TA_JOINT_APART=1.0`, the gap that counts).
#[test]
#[ignore]
fn joint_scan() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
    let apart_at: f64 = std::env::var("TA_JOINT_APART").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(50_000), energy: Fx::from_int(500_000), ai: false })
        .collect();
    let target_type = types.id("CORGOL").or_else(|| types.id("ARMBULL")).unwrap();
    let mut models: BTreeMap<String, Option<Arc<Geo>>> = BTreeMap::new();
    for t in &types.types {
        let object = t.def.object.to_ascii_lowercase();
        models.entry(object.clone()).or_insert_with(|| vfs.read_opt(&format!("objects3d/{object}.3do")).and_then(|b| Model3do::parse(&b).ok()).map(|m| Arc::new(load_geo(&m))));
    }
    let picked: Vec<&UnitType> = types.types.iter().filter(|t| t.script.is_some() && only.as_ref().is_none_or(|o| o.contains(&t.name.to_ascii_uppercase()))).collect();
    let mut reports: Vec<(String, Vec<Apart>, Vec<(usize, f64)>, Vec<(String, String)>)> = picked
        .par_iter()
        .filter_map(|t| {
            let geo = models.get(&t.def.object.to_ascii_lowercase())?.clone()?;
            let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
            let (land, water) = spots(&sim);
            let wet = t.def.min_water_depth > 0.0 || t.locomotion == Locomotion::Naval;
            let at = if wet { water? } else { land };
            let (samples, _) = drive(&mut sim, t, at, target_type, &mut |_: &Sim, _: UnitId| {})?;
            // At rest: the pairs that touch, and the pieces that touch nothing.
            let rest = transforms(&geo, &rest_pose(&geo), &OURS);
            let w = world_verts(&geo, &rest);
            // (Muzzle flashes, fire points and the like are markers, not pieces of the body.)
            let marker = |name: &str| ["flare", "fire", "point", "spray", "emit", "flash"].iter().any(|m| name.contains(m));
            let solid: Vec<usize> = (0..geo.parts.len()).filter(|&i| geo.parts[i].solid() && !marker(&geo.parts[i].name) && samples.first().is_none_or(|s| s.visible[i])).collect();
            let mut touching = Vec::new();
            let mut nearest = vec![f64::MAX; geo.parts.len()];
            for (k, &i) in solid.iter().enumerate() {
                for &j in &solid[k + 1..] {
                    if !near_bounds(&geo, &rest, i, j, apart_at) {
                        continue;
                    }
                    let gap = surface_gap(&geo, &w, &rest, i, j);
                    nearest[i] = nearest[i].min(gap);
                    nearest[j] = nearest[j].min(gap);
                    let family = geo.parts[i].parent == Some(j) || geo.parts[j].parent == Some(i);
                    if gap <= 0.25 && family {
                        touching.push((i, j));
                    }
                }
            }
            let floating: Vec<(usize, f64)> = solid.iter().filter(|&&i| solid.len() > 1 && nearest[i] > apart_at).map(|&i| (i, nearest[i].min(999.0))).collect();
            // In the animations: how far each touching pair came apart, both shown.
            let mut apart: Vec<Apart> = Vec::new();
            for s in samples.iter().filter(|s| matches!(s.stage, "built" | "moving" | "stopped")) {
                let xf = transforms(&geo, &s.pose, &OURS);
                let w = world_verts(&geo, &xf);
                for &(i, j) in &touching {
                    if !s.visible[i] || !s.visible[j] {
                        continue;
                    }
                    let gap = surface_gap(&geo, &w, &xf, i, j);
                    if gap <= apart_at {
                        continue;
                    }
                    match apart.iter_mut().find(|a| a.a == i && a.b == j) {
                        Some(a) if a.gap >= gap => {}
                        Some(a) => (a.gap, a.stage) = (gap, s.stage),
                        None => apart.push(Apart { a: i, b: j, gap, stage: s.stage }),
                    }
                }
            }
            // What a connector could join: a joint coming apart; a piece touching nothing, with
            // its parent or a child above or below it (within 8).
            let mut joins: Vec<(usize, usize)> = apart.iter().map(|a| if geo.parts[a.b].parent == Some(a.a) { (a.a, a.b) } else { (a.b, a.a) }).collect();
            for &(p, _) in &floating {
                let family = geo.parts[p].parent.map(|q| (q, p)).into_iter().chain((0..geo.parts.len()).filter(|&c| geo.parts[c].parent == Some(p)).map(|c| (p, c)));
                for (a, b) in family {
                    if solid.contains(&a) && solid.contains(&b) && !joins.contains(&(a, b)) && vertical_gap(&geo, &rest, a, b).is_some_and(|g| g > -0.5 && g <= 8.0) {
                        joins.push((a, b));
                    }
                }
            }
            let joins: Vec<(String, String)> = joins.into_iter().map(|(a, b)| (geo.parts[a].name.clone(), geo.parts[b].name.clone())).collect();
            (!apart.is_empty() || !floating.is_empty()).then(|| (t.name.clone(), apart, floating, joins))
        })
        .collect();
    reports.sort_by(|a, b| {
        let worst = |r: &(String, Vec<Apart>, Vec<(usize, f64)>, Vec<(String, String)>)| r.1.iter().map(|a| a.gap).fold(0.0, f64::max);
        worst(b).total_cmp(&worst(a)).then(a.0.cmp(&b.0))
    });
    println!("{} of {} units with a joint coming apart (more than {apart_at}) or a piece touching nothing", reports.len(), picked.len());
    for (name, apart, floating, _) in &reports {
        let geo = models.get(&types.get(types.id(name).unwrap()).def.object.to_ascii_lowercase()).cloned().flatten().unwrap();
        let mut apart = apart.clone();
        apart.sort_by(|a, b| b.gap.total_cmp(&a.gap));
        let parts: Vec<String> = apart.iter().take(6).map(|a| format!("{}/{} {:.1} ({})", geo.parts[a.a].name, geo.parts[a.b].name, a.gap, a.stage)).collect();
        let alone: Vec<String> = floating.iter().map(|(i, d)| format!("{} {:.1}", geo.parts[*i].name, d)).collect();
        println!("{name:10} apart: {}{}", parts.join(", "), if alone.is_empty() { String::new() } else { format!("   alone at rest: {}", alone.join(", ")) });
    }
    // The connectors that could join them, as model/connectors.rs lists them (by eye first).
    println!("suggested joins:");
    for (name, _, _, joins) in reports.iter().filter(|r| !r.3.is_empty()) {
        let object = types.get(types.id(name).unwrap()).def.object.to_ascii_lowercase();
        let list: Vec<String> = joins.iter().map(|(a, b)| format!("Join::new(\"{a}\", \"{b}\")")).collect();
        println!("    (\"{object}\", &[{}]), // {name}", list.join(", "));
    }
}

// ---------------------------------------------------------------------------------------
// The sign each script axis takes in our frame, from the shipped scripts.

/// A pose a script holds (at a sleep or a wait, or where a path through it ends), as its code
/// reads: its constant moves and turns from rest, along one path through its branches.
struct Keyframe {
    func: String,
    /// In a loop, or in a function that walks or swims: a cycle, not a state.
    cycle: bool,
    /// Which path through the function (its keyframes in order along it).
    path: usize,
    pose: Pose,
}

/// A constant turn (not `turn-now`), from where the piece was on that path.
struct TurnStep {
    piece: usize,
    axis: usize,
    from: f64,
    to: f64,
    before: Pose,
}

/// A constant move in a fire function: a recoil.
struct Recoil {
    piece: usize,
    axis: usize,
    value: f64,
    before: Pose,
}

fn cyclic(name: &str) -> bool {
    let f = name.to_ascii_lowercase();
    ["walk", "swim", "crawl", "motion", "legs"].iter().any(|w| f.contains(w))
}

/// Reads every function's constant moves and turns along each path through its branches
/// (conditions on constants folded; a loop's body once).
fn read_script(script: &CobScript, cob_to_model: &[Option<usize>], n: usize) -> (Vec<Keyframe>, Vec<TurnStep>, Vec<Recoil>) {
    let code = &script.code;
    let mut loops: Vec<(usize, usize)> = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let Some(k) = operand_count(code[pc]) else { break };
        if code[pc] == op::JUMP && code.get(pc + 1).is_some_and(|&t| (t as usize) <= pc) {
            loops.push((code[pc + 1] as usize, pc));
        }
        pc += 1 + k;
    }
    let mut starts: Vec<(usize, usize)> = script.functions.iter().enumerate().map(|(i, f)| (f.offset as usize, i)).collect();
    starts.sort_unstable();
    let (mut keys, mut turns, mut recoils) = (Vec::new(), Vec::new(), Vec::new());
    let held = |pose: &Pose| pose.iter().any(|(m, t)| m.iter().chain(t).any(|v| *v != 0.0));
    let mut paths = 0;
    for (k, &(start, fi)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(code.len(), |s| s.0).min(code.len());
        let name = &script.functions[fi].name;
        let fire = name.to_ascii_lowercase().contains("fire");
        let mut stack: Vec<(usize, Pose)> = vec![(start, vec![([0.0; 3], [0.0; 3]); n])];
        let mut forks = 0;
        while let Some((mut pc, mut pose)) = stack.pop() {
            paths += 1;
            let (mut last, mut steps) = (None, 0);
            while pc < end && steps < 20_000 {
                steps += 1;
                let opc = code[pc];
                let Some(k) = operand_count(opc) else { break };
                let arg = |i: usize| code.get(pc + 1 + i).map(|&v| v as usize);
                let next = pc + 1 + k;
                let in_loop = loops.iter().any(|&(a, b)| pc >= a && pc <= b);
                match opc {
                    op::PUSH_CONSTANT => {
                        last = arg(0).map(|v| v as u32 as i32);
                        pc = next;
                        continue;
                    }
                    op::JUMP => match arg(0) {
                        Some(t) if t > pc => pc = t,
                        _ => break,
                    },
                    op::JUMP_IF_FALSE => {
                        let t = arg(0).unwrap_or(end);
                        match last {
                            Some(0) => pc = t,
                            Some(_) => pc = next,
                            None => {
                                forks += 1;
                                if t > pc && forks < 64 {
                                    stack.push((t, pose.clone()));
                                }
                                pc = next;
                            }
                        }
                    }
                    op::RETURN => break,
                    op::SLEEP | op::WAIT_TURN | op::WAIT_MOVE => {
                        if held(&pose) {
                            keys.push(Keyframe { func: name.clone(), cycle: in_loop || cyclic(name), path: paths, pose: pose.clone() });
                        }
                        pc = next;
                    }
                    op::MOVE | op::MOVE_NOW | op::TURN | op::TURN_NOW => {
                        if let (Some(v), Some(&Some(mp)), Some(axis)) = (last, arg(0).and_then(|c| cob_to_model.get(c)), arg(1).filter(|&a| a < 3)) {
                            if matches!(opc, op::TURN | op::TURN_NOW) {
                                if opc == op::TURN {
                                    turns.push(TurnStep { piece: mp, axis, from: pose[mp].1[axis], to: v as f64, before: pose.clone() });
                                }
                                pose[mp].1[axis] = v as f64;
                            } else {
                                if fire {
                                    recoils.push(Recoil { piece: mp, axis, value: v as f64 / 65536.0, before: pose.clone() });
                                }
                                pose[mp].0[axis] = v as f64 / 65536.0;
                            }
                        }
                        pc = next;
                    }
                    _ => pc = next,
                }
                last = None;
            }
            if held(&pose) {
                keys.push(Keyframe { func: name.clone(), cycle: cyclic(name), path: paths, pose: pose.clone() });
            }
        }
    }
    (keys, turns, recoils)
}

fn subtree(geo: &Geo, root: usize) -> Vec<bool> {
    let mut inside = vec![false; geo.parts.len()];
    inside[root] = true;
    for i in root + 1..geo.parts.len() {
        if geo.parts[i].parent.is_some_and(|p| inside[p]) {
            inside[i] = true;
        }
    }
    inside
}

/// How plausible a pose looks: surface crossings between pieces that clear each other at
/// rest, how far it sinks below its rest pose's lowest point, and points far out of its rest
/// bounds.
fn pose_score(geo: &Geo, xf: &[Xf]) -> (usize, f64, usize) {
    let xings = new_crossings(geo, xf, None).iter().map(|c| c.2).sum();
    let size = sub(geo.hi, geo.lo);
    let margin = |k: usize| (size[k] * 0.25).max(8.0);
    let (mut low, mut outside) = (f64::MAX, 0);
    for (i, p) in geo.parts.iter().enumerate() {
        for s in &p.samples {
            let q = xf[i].apply(*s);
            low = low.min(q[1]);
            if (0..3).any(|k| q[k] < geo.lo[k] - margin(k) || q[k] > geo.hi[k] + margin(k)) {
                outside += 1;
            }
        }
    }
    (xings, (geo.lo[1] - low).max(0.0), outside)
}

/// Crossings between a piece (with what it carries) and the rest of the model.
fn subtree_crossings(geo: &Geo, xf: &[Xf], moved: &[bool]) -> usize {
    let w = world_verts(geo, xf);
    let mut n = 0;
    for i in (0..geo.parts.len()).filter(|&i| moved[i]) {
        for j in (0..geo.parts.len()).filter(|&j| !moved[j]) {
            n += pair_crossings(geo, &w, i, j);
        }
    }
    n
}

#[derive(Default)]
struct Tally {
    /// Per hypothesis: ours more plausible, the flip more plausible, no difference.
    poses: [[usize; 3]; 7],
    /// Per axis: backwards, forwards.
    recoils: [[usize; 2]; 3],
    /// Ours, turns about x flipped, moves along z flipped: walkers whose feet slide backwards,
    /// forwards.
    walkers: [[usize; 2]; 3],
    /// Long turns: clearer the direct way, the short way, no difference.
    paths: [usize; 3],
}

#[test]
#[ignore]
fn script_conventions() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let mut objects: Vec<String> =
        vfs.list("scripts", Some("cob"), false).iter().filter_map(|p| p.rsplit('/').next()?.strip_suffix(".cob").map(str::to_ascii_lowercase)).collect();
    objects.sort();
    objects.dedup();
    // Each axis flipped alone, and the mirror image (x moves, y and z turns flipped together):
    // its name, its convention and the axes it flips (turn?, axis).
    type Hypothesis = (String, Conv, Vec<(bool, usize)>);
    let mut hyps: Vec<Hypothesis> = Vec::new();
    for turn in [false, true] {
        for axis in 0..3 {
            hyps.push((format!("{} {}", if turn { "turns about" } else { "moves along" }, ["x", "y", "z"][axis]), OURS.flip(turn, axis), vec![(turn, axis)]));
        }
    }
    hyps.push(("the mirror image".into(), OURS.flip(false, 0).flip(true, 1).flip(true, 2), vec![(false, 0), (true, 1), (true, 2)]));
    let tallies: Vec<Tally> = objects
        .par_iter()
        .filter_map(|o| {
            let script = CobScript::parse(&vfs.read_opt(&format!("scripts/{o}.cob"))?).ok()?;
            let geo = load_geo(&Model3do::parse(&vfs.read_opt(&format!("objects3d/{o}.3do"))?).ok()?);
            let cob_to_model: Vec<Option<usize>> = script.pieces.iter().map(|p| geo.parts.iter().position(|q| q.name.eq_ignore_ascii_case(p))).collect();
            let (keys, turns, recoils) = read_script(&script, &cob_to_model, geo.parts.len());
            let mut t = Tally::default();
            // 1. Every distinct state a script holds (not walk cycles, aims or recoils), read
            // with each flip: more surface crossings, sinking or straying is less plausible.
            let mut seen = HashSet::new();
            for k in keys.iter().filter(|k| !k.cycle && !k.func.to_ascii_lowercase().contains("fire") && !k.func.to_ascii_lowercase().contains("aim")) {
                if !seen.insert(format!("{:?}", k.pose.iter().map(|(m, r)| (m.map(|x| (x * 4.0) as i64), r.map(|x| (x / 16.0) as i64))).collect::<Vec<_>>())) {
                    continue;
                }
                let ours = pose_score(&geo, &transforms(&geo, &k.pose, &OURS));
                for (h, (_, conv, dofs)) in hyps.iter().enumerate() {
                    let uses = k.pose.iter().any(|(m, r)| dofs.iter().any(|&(turn, a)| if turn { r[a].abs() >= 1456.0 } else { m[a].abs() >= 1.0 }));
                    if !uses {
                        continue;
                    }
                    let alt = pose_score(&geo, &transforms(&geo, &k.pose, conv));
                    let mut score = 0;
                    for (d, thr) in [(ours.0 as f64 - alt.0 as f64, 4.0), (ours.1 - alt.1, 1.5), (ours.2 as f64 - alt.2 as f64, 2.0)] {
                        if d.abs() >= thr {
                            score -= d.signum() as i32;
                        }
                    }
                    t.poses[h][if score > 0 {
                        0
                    } else if score < 0 {
                        1
                    } else {
                        2
                    }] += 1;
                }
            }
            // 2. Recoils: a gun's moves in its fire functions go back from where it points
            // (from its pivot to the middle of what it carries, in its parent's frame).
            let mut seen = HashSet::new();
            for r in recoils.iter().filter(|r| r.value.abs() >= 0.5) {
                if !seen.insert((r.piece, r.axis, (r.value * 16.0) as i64)) {
                    continue;
                }
                let xf = transforms(&geo, &r.before, &OURS);
                let inside = subtree(&geo, r.piece);
                let pts: Vec<V3> = (0..geo.parts.len()).filter(|&i| inside[i]).flat_map(|i| geo.parts[i].samples.iter().map(move |s| (i, *s))).map(|(i, s)| xf[i].apply(s)).collect();
                if pts.is_empty() {
                    continue;
                }
                let c = scale(pts.iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / pts.len() as f64);
                let parent = geo.parts[r.piece].parent.map_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], |p| xf[p].r);
                let local = mtv(&parent, sub(c, xf[r.piece].t));
                let len = dot(local, local).sqrt();
                if len < 1.0 || local[r.axis].abs() < 0.5 * len {
                    continue;
                }
                let back = (OURS.m[r.axis] * r.value < 0.0) == (local[r.axis] > 0.0);
                t.recoils[r.axis][if back { 0 } else { 1 }] += 1;
            }
            // 3. Walk cycles: a foot on the ground slides backwards from one keyframe to the
            // next (the body walks on over it).
            let mut cycles: BTreeMap<usize, Vec<&Keyframe>> = BTreeMap::new();
            for k in keys.iter().filter(|k| k.func.to_ascii_lowercase().contains("walk")) {
                cycles.entry(k.path).or_default().push(k);
            }
            if let Some(cycle) = cycles.values().filter(|c| c.len() >= 3).max_by_key(|c| c.len()) {
                for (h, conv) in [OURS, OURS.flip(true, 0), OURS.flip(false, 2)].iter().enumerate() {
                    let frames: Vec<Vec<Xf>> = cycle.iter().map(|k| transforms(&geo, &k.pose, conv)).collect();
                    let mut slide = 0.0;
                    for (i, p) in geo.parts.iter().enumerate().filter(|(_, p)| !p.samples.is_empty()) {
                        let track: Vec<(f64, f64)> = frames
                            .iter()
                            .map(|f| {
                                let pts: Vec<V3> = p.samples.iter().map(|s| f[i].apply(*s)).collect();
                                (pts.iter().map(|q| q[1]).fold(f64::MAX, f64::min), pts.iter().map(|q| q[2]).sum::<f64>() / pts.len() as f64)
                            })
                            .collect();
                        for w in 0..track.len() {
                            let (a, b) = (track[w], track[(w + 1) % track.len()]);
                            if a.0 <= geo.lo[1] + 1.0 && b.0 <= geo.lo[1] + 1.0 && (b.1 - a.1).abs() > 0.25 {
                                slide += b.1 - a.1;
                            }
                        }
                    }
                    if slide != 0.0 {
                        t.walkers[h][if slide < 0.0 { 0 } else { 1 }] += 1;
                    }
                }
            }
            // 4. Turns of more than half a turn: the direct way (as numbers) and the short way
            // round go opposite ways; which keeps the piece clear of the rest? (Turns go the
            // short way, as the aircraft's barrel rolls need: 240°, 120°, 0° rolls them round
            // only so. The Core construction hovercraft's and advanced construction sub's
            // hatch doors, keyframed to -211°, would clear the hull only the direct way.)
            let mut seen = HashSet::new();
            for s in turns.iter().filter(|s| (s.to - s.from).abs() > 33_000.0) {
                if !seen.insert((s.piece, s.axis, s.from as i64, s.to as i64)) {
                    continue;
                }
                let moved = subtree(&geo, s.piece);
                let base = subtree_crossings(&geo, &transforms(&geo, &s.before, &OURS), &moved);
                let along = |to: f64| {
                    let mut pose = s.before.clone();
                    (1..=12)
                        .map(|k| {
                            pose[s.piece].1[s.axis] = s.from + (to - s.from) * k as f64 / 12.0;
                            subtree_crossings(&geo, &transforms(&geo, &pose, &OURS), &moved).saturating_sub(base)
                        })
                        .sum::<usize>()
                };
                let short = s.from + ((s.to - s.from + 32768.0).rem_euclid(65536.0) - 32768.0);
                let (direct, round) = (along(s.to), along(short));
                t.paths[if direct + 3 < round {
                    0
                } else if round + 3 < direct {
                    1
                } else {
                    2
                }] += 1;
            }
            Some(t)
        })
        .collect();
    let mut all = Tally::default();
    let add_up = |to: &mut [usize], from: &[usize]| to.iter_mut().zip(from).for_each(|(a, b)| *a += b);
    for t in &tallies {
        all.poses.iter_mut().zip(&t.poses).for_each(|(a, b)| add_up(a, b));
        all.recoils.iter_mut().zip(&t.recoils).for_each(|(a, b)| add_up(a, b));
        all.walkers.iter_mut().zip(&t.walkers).for_each(|(a, b)| add_up(a, b));
        add_up(&mut all.paths, &t.paths);
    }
    println!("{} scripts read", tallies.len());
    println!("poses the scripts hold, read our way and with an axis flipped (fewer crossings, less sinking or straying wins):");
    for (h, (name, _, _)) in hyps.iter().enumerate() {
        let p = all.poses[h];
        println!("    {name}: ours more plausible {}, flipped {}, no difference {}", p[0], p[1], p[2]);
    }
    for (axis, r) in all.recoils.iter().enumerate() {
        println!("recoils along {}: backwards {}, forwards {}", ["x", "y", "z"][axis], r[0], r[1]);
    }
    for (h, name) in ["ours", "turns about x flipped", "moves along z flipped"].iter().enumerate() {
        println!("walkers, {name}: feet on the ground slide backwards {}, forwards {}", all.walkers[h][0], all.walkers[h][1]);
    }
    println!("turns of more than half a turn: clearer the direct way {}, the short way {}, no difference {}", all.paths[0], all.paths[1], all.paths[2]);
    // Ours wins on every count it can be judged by.
    for (h, (name, _, _)) in hyps.iter().enumerate() {
        assert!(all.poses[h][0] > 2 * all.poses[h][1], "{name}: {:?}", all.poses[h]);
    }
    assert!(all.recoils[2][0] > 10 * all.recoils[2][1], "recoils go backwards: {:?}", all.recoils);
    assert!(all.walkers[0][0] > 4 * all.walkers[0][1] && all.walkers[1][1] > all.walkers[1][0], "walkers walk forwards: {:?}", all.walkers);
}
