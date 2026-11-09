//! Ore on the metal deposits (modern camera): a cluster of metal crystals standing on each
//! deposit. The clusters are made here, as 3DO models of hexagonal shards with pointed tips
//! (a few variants per map, in the palette's steel colours), so the renderer lights them like
//! any model: shining metal, shadows, rain sheen.

use std::f32::consts::TAU;

use glam::Vec3;
use ta_formats::image::Palette;
use ta_formats::model3do::{FIXED_ONE, Model3do, Piece, Primitive};

/// Cluster variants made per map.
pub const VARIANTS: u32 = 6;

/// A cluster of five to nine crystals about 15 units across, seeded by `seed`.
pub fn cluster(seed: u32, palette: &Palette) -> Model3do {
    let mut state = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453) | 1;
    let mut rand = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % 10_000) as f32 / 10_000.0
    };
    let steel = [nearest(palette, [178, 188, 206]), nearest(palette, [132, 142, 162]), nearest(palette, [90, 98, 116])];
    let shards = 5 + (rand() * 4.0) as usize;
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut polygons: Vec<(Vec<u16>, u32)> = Vec::new();
    for s in 0..shards {
        let a = rand() * TAU;
        let main = s == 0;
        let dist = if main { 0.0 } else { 2.0 + rand() * 4.5 };
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let base = out * dist - Vec3::Y * 1.2;
        let axis = (Vec3::Y + out * (0.12 + rand() * 0.4) * if main { 0.4 } else { 1.0 }).normalize();
        let length = if main { 8.0 + rand() * 4.0 } else { 3.5 + rand() * 5.0 };
        let width = if main { 1.9 } else { 0.9 + rand() * 0.9 };
        let u = axis.cross(Vec3::Z).normalize_or(Vec3::X);
        let v = axis.cross(u);
        let twist = rand() * TAU;
        let first = vertices.len() as u16;
        let ring = |k: usize, scale: f32| {
            let t = twist + k as f32 * TAU / 6.0;
            (u * t.cos() + v * t.sin()) * width * scale
        };
        for k in 0..6 {
            vertices.push(base + ring(k, 1.0));
        }
        for k in 0..6 {
            vertices.push(base + axis * (length * 0.72) + ring(k, 0.85));
        }
        vertices.push(base + axis * length);
        let centre = base + axis * (length * 0.45);
        for k in 0..6u16 {
            let k1 = (k + 1) % 6;
            let side = steel[((rand() * 3.0) as usize).min(2)];
            let facet = steel[(k as usize) % 2];
            for (poly, color) in [(vec![first + k, first + k1, first + 6 + k1, first + 6 + k], side), (vec![first + 6 + k, first + 6 + k1, first + 12], facet)] {
                polygons.push((outward(poly, &vertices, centre), color));
            }
        }
    }
    let fixed = |v: Vec3| [(v.x * FIXED_ONE) as i32, (v.y * FIXED_ONE) as i32, (-v.z * FIXED_ONE) as i32];
    Model3do {
        pieces: vec![Piece {
            name: "ore".into(),
            parent: None,
            offset: [0; 3],
            vertices: vertices.iter().map(|&v| fixed(v)).collect(),
            primitives: polygons.into_iter().map(|(indices, color_index)| Primitive { color_index, indices, texture: None }).collect(),
            selection_primitive: None,
            children: Vec::new(),
        }],
    }
}

/// `poly` ordered as TA's models order their polygons: clockwise seen from outside (from
/// `centre` outwards) in world space; the model library turns them into front faces.
fn outward(mut poly: Vec<u16>, vertices: &[Vec3], centre: Vec3) -> Vec<u16> {
    let corners: Vec<Vec3> = poly.iter().map(|&i| vertices[i as usize]).collect();
    let mut n = Vec3::ZERO;
    for i in 0..corners.len() {
        let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
        n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    let middle = corners.iter().copied().sum::<Vec3>() / corners.len() as f32;
    if n.dot(middle - centre) > 0.0 {
        poly.reverse();
    }
    poly
}

/// The palette index whose colour is nearest `rgb`.
fn nearest(palette: &Palette, rgb: [u8; 3]) -> u32 {
    (0..=255u8)
        .min_by_key(|&i| {
            let c = palette.rgb(i);
            (0..3).map(|k| (c[k] as i32 - rgb[k] as i32).pow(2)).sum::<i32>()
        })
        .unwrap_or(0) as u32
}

/// The metal deposits the renderer lays in the ground as ore beds (modern camera, terrain.wgsl;
/// TA's painted patches are not drawn there): per heightmap cell, how much of it the bed of a
/// deposit richer than the map's plain ground covers (0..255), a disc round the deposit's
/// middle about as wide as TA paints a deposit (two fifths of its footprint across each way:
/// a field of deposits side by side stays a field of beds, not one sheet of ore).
pub fn deposits(sim: &ta_sim::Sim) -> (u32, u32, Vec<u8>) {
    let w = &sim.world;
    let (cw, ch) = (w.width as usize, w.height as usize);
    let mut cover = vec![0.0f32; cw * ch];
    for f in &sim.features {
        let def = &sim.data.features[f.def];
        let fp = (def.footprint_x, def.footprint_z);
        if !def.is_metal_deposit() || !w.ore_under(f.origin, fp) {
            continue;
        }
        // (In cells: its middle, and the bed's radius.)
        let centre = (f.origin.0 as f32 + fp.0 as f32 * 0.5, f.origin.1 as f32 + fp.1 as f32 * 0.5);
        let radius = 0.42 * fp.0.min(fp.1).max(1) as f32;
        let (x0, x1) = ((centre.0 - radius - 1.0).floor().max(0.0) as usize, ((centre.0 + radius + 1.0).ceil() as usize).min(cw));
        let (z0, z1) = ((centre.1 - radius - 1.0).floor().max(0.0) as usize, ((centre.1 + radius + 1.0).ceil() as usize).min(ch));
        for z in z0..z1 {
            for x in x0..x1 {
                // (The share of the cell inside the disc, from 4 x 4 points in it.)
                let inside = (0..16)
                    .filter(|k| {
                        let (px, pz) = (x as f32 + (k % 4) as f32 * 0.25 + 0.125, z as f32 + (k / 4) as f32 * 0.25 + 0.125);
                        (px - centre.0).hypot(pz - centre.1) <= radius
                    })
                    .count() as f32
                    / 16.0;
                let c = &mut cover[z * cw + x];
                *c = c.max(inside);
            }
        }
    }
    (w.width, w.height, cover.iter().map(|&c| (c * 255.0).round() as u8).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_are_closed_crystals_facing_out() {
        let palette = Palette::parse(&[128u8; 768]).expect("palette");
        for seed in 0..VARIANTS {
            let m = cluster(seed, &palette);
            let p = &m.pieces[0];
            assert!((5..=9).contains(&(p.vertices.len() / 13)), "shards: {}", p.vertices.len() / 13);
            assert_eq!(p.primitives.len(), p.vertices.len() / 13 * 12);
            assert!(p.primitives.iter().all(|pr| pr.indices.iter().all(|&i| (i as usize) < p.vertices.len())));
        }
    }
}
