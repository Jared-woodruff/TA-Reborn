//! Smooth corners: TA's models share no vertices (every face has its own corners) and are lit
//! flat. Where faces meet at a shallow angle (the facets of a barrel, a dome, a rounded hull)
//! their corners take the faces' normals averaged, so the surface reads as the curve it was
//! drawn as; folds sharper than 45 degrees stay hard.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use super::ModelVertex;

/// A face of a piece with its corners as they will be drawn.
#[derive(Clone)]
pub(super) struct SourceFace {
    pub vertices: Vec<ModelVertex>,
    /// Outwards.
    pub normal: Vec3,
    /// Added to make a form whole (see `FaceSource::late` in model.rs): TA's own faces are
    /// smoothed and shaded as if it weren't there.
    pub late: bool,
}

type Key = [i32; 3];

/// Positions to a 4096th of a unit: corners there are the same corner.
fn key(p: Vec3) -> Key {
    [p.x, p.y, p.z].map(|x| (x * 4096.0).round() as i32)
}

fn position(v: &ModelVertex) -> Vec3 {
    Vec3::from_array(v.pos)
}

fn edge(a: Vec3, b: Vec3) -> (Key, Key) {
    let (a, b) = (key(a), key(b));
    if a < b { (a, b) } else { (b, a) }
}

fn face_area(f: &SourceFace) -> f32 {
    if f.vertices.len() < 3 {
        return 0.0;
    }
    let p = position(&f.vertices[0]);
    (1..f.vertices.len() - 1)
        .map(|i| (position(&f.vertices[i]) - p).cross(position(&f.vertices[i + 1]) - p).length() * 0.5)
        .sum()
}

/// Smooths the corners of faces that meet shallowly (normals within 45 degrees across an edge
/// they share), weighting each face by its angle at the corner and its size. Returns how many
/// corners changed.
/// Faces meeting at a fold shallower than this (the cosine between their normals) are smoothed
/// together (45 degrees).
pub(super) const SHALLOW: f32 = 0.7072;

pub(super) fn smooth_normals(faces: &mut [SourceFace]) -> usize {
    // TA's own faces as if the late ones weren't there; the late ones with all of them.
    if faces.iter().any(|f| f.late) {
        let own: Vec<usize> = (0..faces.len()).filter(|&f| !faces[f].late).collect();
        let mut alone: Vec<SourceFace> = own.iter().map(|&f| faces[f].clone()).collect();
        let mut all: Vec<SourceFace> = faces.to_vec();
        let late: Vec<bool> = faces.iter().map(|f| f.late).collect();
        let changed = smooth_all(&mut alone, None) + smooth_all(&mut all, Some(&late));
        for (k, &f) in own.iter().enumerate() {
            faces[f] = alone[k].clone();
        }
        for (f, face) in all.into_iter().enumerate() {
            if faces[f].late {
                faces[f] = face;
            }
        }
        return changed;
    }
    smooth_all(faces, None)
}

/// Smooths the corners of these faces (of those `only` names, where it does).
fn smooth_all(faces: &mut [SourceFace], only: Option<&[bool]>) -> usize {
    if faces.len() > 4096 || faces.iter().map(|f| f.vertices.len()).sum::<usize>() > 40_000 {
        return 0;
    }
    let mut corners: BTreeMap<Key, Vec<(usize, usize)>> = BTreeMap::new();
    for (f, face) in faces.iter().enumerate() {
        for (v, vertex) in face.vertices.iter().enumerate() {
            corners.entry(key(position(vertex))).or_default().push((f, v));
        }
    }
    let areas: Vec<f32> = faces.iter().map(face_area).collect();
    let normals: Vec<Vec3> = faces.iter().map(|f| f.normal).collect();
    // A shared position alone is not a smooth fan: two shells may touch at a point. Corners
    // join only through an edge exactly two faces share.
    let mut edges: BTreeMap<(Key, Key), Vec<(usize, usize)>> = BTreeMap::new();
    for (f, face) in faces.iter().enumerate() {
        let n = face.vertices.len();
        for i in 0..n {
            edges.entry(edge(position(&face.vertices[i]), position(&face.vertices[(i + 1) % n]))).or_default().push((f, i));
        }
    }
    let mut adjacent: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for sharing in edges.values().filter(|v| v.len() == 2) {
        let ((a, i), (b, j)) = (sharing[0], sharing[1]);
        if a == b || normals[a].dot(normals[b]) < SHALLOW {
            continue;
        }
        for ia in [i, (i + 1) % faces[a].vertices.len()] {
            let at = key(position(&faces[a].vertices[ia]));
            let jb = if at == key(position(&faces[b].vertices[j])) { j } else { (j + 1) % faces[b].vertices.len() };
            adjacent.entry((a, ia)).or_default().push((b, jb));
            adjacent.entry((b, jb)).or_default().push((a, ia));
        }
    }
    let mut changed = 0;
    for shared in corners.values() {
        if shared.len() > 64 {
            continue;
        }
        for &(f, v) in shared {
            if only.is_some_and(|only| !only[f]) {
                continue;
            }
            let mut normal = Vec3::ZERO;
            let mut connected = BTreeSet::new();
            let mut pending = vec![(f, v)];
            while let Some(node) = pending.pop() {
                if normals[f].dot(normals[node.0]) >= SHALLOW && connected.insert(node) && let Some(next) = adjacent.get(&node) {
                    pending.extend(next);
                }
            }
            for &(other, corner) in &connected {
                let polygon = &faces[other].vertices;
                let p = position(&polygon[corner]);
                let before = (position(&polygon[(corner + polygon.len() - 1) % polygon.len()]) - p).normalize_or_zero();
                let after = (position(&polygon[(corner + 1) % polygon.len()]) - p).normalize_or_zero();
                let angle = before.dot(after).clamp(-1.0, 1.0).acos();
                normal += normals[other] * angle * areas[other].sqrt();
            }
            let smoothed = normal.normalize_or(normals[f]);
            changed += usize::from(smoothed.dot(normals[f]) < 0.9999);
            let vertex = &mut faces[f].vertices[v];
            vertex.normal = [(smoothed.x * 127.0).round() as i8, (smoothed.y * 127.0).round() as i8, (smoothed.z * 127.0).round() as i8, vertex.normal[3]];
        }
    }
    changed
}

/// TA shaded some curved parts by hand, each facet in its own shade of a palette ramp (a fusion
/// reactor's globe, a dish's back): lit by Reborn as well, they would show every facet. The faces
/// in TA's palette colours (`colour`) joined smoothly (as `smooth_normals` joins them, three or
/// more together, across the model's pieces at rest: a globe can be two half globes) take one
/// colour, the mean of the lit half of their shades (Reborn's light shades the rest), so the light
/// falls on one surface. `pieces`: each piece's faces and its place in the model at rest (the late
/// ones left out: see `SourceFace::late`). Returns how many faces changed.
pub(super) fn even_hand_shading(pieces: &mut [(&mut Vec<SourceFace>, Vec3)], colour: impl Fn(&ModelVertex) -> bool) -> usize {
    let all: Vec<(usize, usize)> = pieces
        .iter()
        .enumerate()
        .flat_map(|(pi, (faces, _))| (0..faces.len()).filter(|&f| !faces[f].late && faces[f].vertices.first().is_some_and(&colour)).map(move |f| (pi, f)))
        .collect();
    if all.is_empty() || all.len() > 4096 {
        return 0;
    }
    let face = |k: usize| &pieces[all[k].0].0[all[k].1];
    let at = |k: usize, v: &ModelVertex| position(v) + pieces[all[k].0].1;
    let mut edges: BTreeMap<(Key, Key), Vec<usize>> = BTreeMap::new();
    for k in 0..all.len() {
        let f = face(k);
        let n = f.vertices.len();
        for i in 0..n {
            edges.entry(edge(at(k, &f.vertices[i]), at(k, &f.vertices[(i + 1) % n]))).or_default().push(k);
        }
    }
    // The groups of faces joined smoothly (union-find).
    let mut parent: Vec<usize> = (0..all.len()).collect();
    fn root(parent: &mut [usize], mut k: usize) -> usize {
        while parent[k] != k {
            parent[k] = parent[parent[k]];
            k = parent[k];
        }
        k
    }
    for sharing in edges.values().filter(|v| v.len() == 2) {
        let (a, b) = (sharing[0], sharing[1]);
        if a != b && face(a).normal.dot(face(b).normal) >= SHALLOW {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            parent[rb] = ra;
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for k in 0..all.len() {
        let r = root(&mut parent, k);
        groups.entry(r).or_default().push(k);
    }
    let lut = crate::util::srgb_to_linear_lut();
    // Each group's colour, then each face of it given it.
    let mut colours: Vec<(Vec<usize>, [u8; 3])> = Vec::new();
    for members in groups.values().filter(|m| m.len() >= 3) {
        // Each face's colour (linear), its brightness and its area; the lit half by area.
        let mut shades: Vec<([f32; 3], f32, f32)> = members
            .iter()
            .map(|&k| {
                let c = face(k).vertices[0].color;
                let rgb = [0, 1, 2].map(|i| lut[c[i] as usize]);
                (rgb, 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2], face_area(face(k)))
            })
            .collect();
        shades.sort_by(|a, b| b.1.total_cmp(&a.1));
        let total: f32 = shades.iter().map(|s| s.2).sum();
        if total <= 0.0 {
            continue;
        }
        let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
        for (rgb, _, area) in &shades {
            if weight >= total * 0.5 {
                break;
            }
            for i in 0..3 {
                sum[i] += rgb[i] * area;
            }
            weight += area;
        }
        colours.push((members.clone(), sum.map(|v| crate::util::linear_to_srgb(v / weight.max(1e-6)))));
    }
    let mut changed = 0;
    for (members, mean) in colours {
        for k in members {
            let (pi, f) = all[k];
            let face = &mut pieces[pi].0[f];
            changed += usize::from(face.vertices[0].color[..3] != mean);
            for v in &mut face.vertices {
                v.color[..3].copy_from_slice(&mean);
            }
        }
    }
    changed
}
