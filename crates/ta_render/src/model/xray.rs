//! Every installed model looked at from all round, as the game makes it whole (halves.rs,
//! caps.rs), for what a 3D camera sees of faces from behind: the insides of shells through
//! their holes, and sheets from their backs. The game draws faces from the front alone, so
//! wherever a face's back is the nearest thing in sight, one sees past it (through the unit, or
//! into it): a gap. (`TA_SHOW_BACKS=1` paints them in the game, see model.rs.)
//!
//! `cargo test --release -p ta_render --lib xray -- --ignored --nocapture`: the models showing
//! the most, each with the faces showing it and why the holes round them were left open, and
//! the area seen by those reasons. `TA_SCAN_MODELS=a,b` for those models alone, `TA_XRAY_OUT=<file>`
//! writes every model's area (JSON) for comparing two runs.

use std::collections::BTreeMap;

use glam::Vec3;
use rayon::prelude::*;

use super::caps::{self, Polygon};
use super::caps::tests::{install, model_faces};
use super::halves::tests::make_whole_as_the_game;
use super::sight;

/// What a triangle of the model made whole is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Source {
    /// A face of a piece (its number in the piece's faces, TA's own then those mirrored).
    Face(usize, usize),
    /// A cap of a piece.
    Cap(usize),
}

type Tri = sight::Tri<Source>;

/// The directions looked from: elevations from a little under the level to high over it.
fn directions() -> Vec<Vec3> {
    sight::directions(&[-15.0, 0.0, 15.0, 30.0, 50.0, 75.0], 16)
}

/// A model made whole as the game makes it: its triangles at rest (model space), each piece's
/// mending and halves, and its pieces' names.
struct Whole {
    tris: Vec<Tri>,
    mended: Vec<caps::Mending>,
    halves: Vec<super::halves::Halves>,
    own: Vec<usize>,
    names: Vec<String>,
}

fn whole(model: &ta_formats::model3do::Model3do) -> Whole {
    let (faces, rest, mendable) = model_faces(model);
    let own: Vec<usize> = faces.iter().map(Vec::len).collect();
    let (faces, halves, mended) = make_whole_as_the_game(faces, &rest, &mendable);
    let mut tris = Vec::new();
    for (pi, (pf, m)) in faces.iter().zip(&mended).enumerate() {
        if !mendable[pi] {
            continue;
        }
        let off = rest[pi];
        for (f, face) in pf.iter().enumerate() {
            if face.len() < 3 {
                continue;
            }
            // (As the game draws it: a face turned round, from its last corner.)
            let mut face: Polygon = face.iter().map(|&p| p + off).collect();
            if m.flipped.contains(&f) {
                face.reverse();
            }
            let normal = caps::plane(&face).0;
            for k in 1..face.len() - 1 {
                tris.push(Tri { p: [face[0], face[k], face[k + 1]], normal, of: Source::Face(pi, f) });
            }
        }
        for (c, cap) in m.caps.iter().enumerate() {
            for t in &cap.triangles {
                let p = t.map(|i| cap.corners[i as usize] + off);
                let normal = caps::plane(&p).0;
                tris.push(Tri { p, normal, of: Source::Cap(pi * 1000 + c) });
            }
        }
    }
    Whole { tris, mended, halves, own, names: model.pieces.iter().map(|p| p.name.clone()).collect() }
}

/// Why a face seen from behind is open: the verdicts of the loops round it, else of its shell
/// (halves.rs), else "shell" (seen into through a hole that isn't round it).
fn why(w: &Whole, source: Source) -> String {
    match source {
        Source::Cap(_) => "cap".into(),
        Source::Face(pi, f) => {
            let mut v: Vec<String> = w.mended[pi].holes.iter().filter(|h| h.faces.contains(&(f as u32))).map(|h| format!("{:?}", h.verdict)).collect();
            v.sort();
            v.dedup();
            if f >= w.own[pi] {
                v.insert(0, "mirrored".into());
            }
            if v.is_empty()
                && let Some(s) = w.halves[pi].shells.iter().find(|s| s.faces.contains(&f))
            {
                v.push(format!("shell {:?}", s.verdict));
            }
            if v.is_empty() { "closed".into() } else { v.join("+") }
        }
    }
}

/// A model's stem, what of it is seen from behind (square units, averaged over the directions)
/// and that as a share of what is seen of it, and by face (with why it is open), the most first.
type Seen = (String, f32, f32, Vec<(f32, String)>);

#[test]
#[ignore]
fn xray_every_installed_model() {
    let Some(vfs) = install() else { return };
    let chosen: Vec<String> = std::env::var("TA_SCAN_MODELS").map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()).collect()).unwrap_or_default();
    let paths: Vec<String> = vfs
        .list("objects3d", Some("3do"), true)
        .into_iter()
        .filter(|p| !p.contains("_dead"))
        .filter(|p| chosen.is_empty() || chosen.iter().any(|c| p.ends_with(&format!("/{c}.3do"))))
        .collect();
    let dirs = directions();
    let started = std::time::Instant::now();
    // Each model: its area seen from behind (square units, averaged over the directions), and
    // by face, with why.
    let results: Vec<Seen> = paths
        .par_iter()
        .filter_map(|path| {
            let model = ta_formats::model3do::Model3do::parse(&vfs.read(path).ok()?).ok()?;
            let w = whole(&model);
            if w.tris.is_empty() {
                return None;
            }
            let foot = w.tris.iter().flat_map(|t| t.p).map(|p| p.y).fold(f32::MAX, f32::min);
            let mut behind = vec![0.0f32; w.tris.len()];
            let mut covered = 0.0;
            for &d in &dirs {
                covered += sight::look(&w.tris, d, 0.35, 320, &mut behind);
            }
            // Not a plate lying on the ground (a pad, a hover's skirt: the ground's), nor what is
            // under the ground a building stands on.
            let mut seen: BTreeMap<Source, f32> = BTreeMap::new();
            for (t, &a) in w.tris.iter().zip(&behind) {
                let grounded = t.normal.y.abs() > 0.7 && t.p.iter().all(|p| p.y <= foot + 0.5);
                if a > 0.0 && !grounded && t.p.iter().any(|p| p.y > 0.0) {
                    *seen.entry(t.of).or_default() += a;
                }
            }
            let k = dirs.len() as f32;
            let total = seen.values().sum::<f32>() / k;
            let share = total / (covered / k).max(1.0);
            let mut faces: Vec<(f32, String)> = seen
                .iter()
                .map(|(&s, &a)| {
                    let name = match s {
                        Source::Face(pi, f) => format!("{} face {f}", w.names[pi]),
                        Source::Cap(c) => format!("{} cap {}", w.names[c / 1000], c % 1000),
                    };
                    (a / k, format!("{name} [{}]", why(&w, s)))
                })
                .collect();
            faces.sort_by(|a, b| b.0.total_cmp(&a.0));
            let stem = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".3do").to_ascii_lowercase();
            Some((stem, total, share, faces))
        })
        .collect();
    let mut results = results;
    // (By the share of what is seen: a small unit's gap shows as much as a big one's.)
    let by_share = std::env::var("TA_XRAY_SHARE").is_ok();
    results.sort_by(|a, b| if by_share { b.2.total_cmp(&a.2) } else { b.1.total_cmp(&a.1) });
    let mut by_why: BTreeMap<String, f32> = BTreeMap::new();
    for (_, _, _, faces) in &results {
        for (a, f) in faces {
            let reason = f.rsplit('[').next().unwrap_or("").trim_end_matches(']').to_string();
            *by_why.entry(reason).or_default() += a;
        }
    }
    let shown = if chosen.is_empty() { 60 } else { results.len() };
    for (stem, total, share, faces) in results.iter().take(shown) {
        let top: Vec<String> = faces.iter().take(if chosen.is_empty() { 4 } else { 30 }).map(|(a, f)| format!("{a:.1} {f}")).collect();
        eprintln!("{total:8.1} {:5.1}%  {stem:10}  {}", share * 100.0, top.join("; "));
    }
    let all: f32 = results.iter().map(|r| r.1).sum();
    eprintln!("{} models, {all:.0} square units seen from behind (averaged over {} directions), {:.1} s", results.len(), dirs.len(), started.elapsed().as_secs_f64());
    let mut reasons: Vec<(&String, &f32)> = by_why.iter().collect();
    reasons.sort_by(|a, b| b.1.total_cmp(a.1));
    for (r, a) in reasons.iter().take(25) {
        eprintln!("  {a:8.1}  {r}");
    }
    if let Ok(out) = std::env::var("TA_XRAY_OUT") {
        let json: BTreeMap<&str, f32> = results.iter().map(|(s, t, _, _)| (s.as_str(), *t)).collect();
        std::fs::write(out, format!("{json:?}")).unwrap();
    }
}
