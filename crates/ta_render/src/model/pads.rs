//! Pads at a building's foot (runways, landing pads, aprons) lifted clear of the ground. TA
//! drew buildings over a flat picture of the map, so a pad modelled a little below the
//! building's base still showed: the Aircraft Plant's runway lies 0.18 below it. On the 3D
//! ground, where a building stands on the highest ground under its footprint, such a pad is
//! buried, and one at the base itself flickers with the ground.

use ta_formats::model3do::{FIXED_ONE, Model3do};

/// Pads end at least this far above the base...
const ABOVE: f32 = 0.06;
/// ...and their layers (a marking on a runway) are kept in order within this much more.
const SPAN: f32 = 0.1;
/// Deeper than this under the base is a foundation, meant to go into the ground.
const DEEPEST: f32 = 0.75;
/// A face this level (the difference of its corners' heights) is a pad.
const LEVEL: f32 = 0.3;

/// Lifts the level faces lying between `DEEPEST` below a model's base and `ABOVE` it to just
/// above the base, keeping their order and staying under the next level face above them.
/// Returns how many corners moved. For buildings and wrecks, which stand on the ground.
pub fn lift_ground_pads(model: &mut Model3do) -> usize {
    // Each piece's origin at rest, in model units.
    let mut rest_y = Vec::with_capacity(model.pieces.len());
    for p in &model.pieces {
        let own = p.offset[1] as f32 / FIXED_ONE;
        rest_y.push(p.parent.map_or(0.0, |parent| rest_y[parent]) + own);
    }
    let height = |pi: usize, v: [i32; 3]| rest_y[pi] + v[1] as f32 / FIXED_ONE;
    let (mut corners, mut lowest, mut ceiling) = (Vec::new(), f32::MAX, f32::MAX);
    for (pi, p) in model.pieces.iter().enumerate() {
        for prim in p.primitives.iter().filter(|prim| prim.indices.len() >= 3) {
            let ys: Vec<f32> = prim.indices.iter().filter_map(|&i| p.vertices.get(i as usize)).map(|&v| height(pi, v)).collect();
            let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &y| (lo.min(y), hi.max(y)));
            if ys.len() < 3 || hi - lo > LEVEL {
                continue;
            }
            if lo >= -DEEPEST && hi < ABOVE {
                lowest = lowest.min(lo);
                corners.extend(prim.indices.iter().map(|&i| (pi, i as usize)));
            } else if lo >= ABOVE {
                ceiling = ceiling.min(lo);
            }
        }
    }
    if corners.is_empty() {
        return 0;
    }
    corners.sort_unstable();
    corners.dedup();
    let top = (ABOVE + SPAN).min((ABOVE + ceiling) * 0.5);
    let scale = (top - ABOVE) / (ABOVE - lowest).max(1e-3);
    for &(pi, i) in &corners {
        let Some(v) = model.pieces[pi].vertices.get_mut(i) else { continue };
        let y = rest_y[pi] + v[1] as f32 / FIXED_ONE;
        let lifted = ABOVE + (y - lowest) * scale;
        v[1] = ((lifted - rest_y[pi]) * FIXED_ONE).round() as i32;
    }
    corners.len()
}

#[cfg(test)]
mod tests {
    use ta_formats::model3do::{Piece, Primitive};

    use super::*;

    fn quad(y: f32, x0: f32, x1: f32) -> (Vec<[i32; 3]>, Vec<u16>) {
        let f = |v: f32| (v * FIXED_ONE) as i32;
        (vec![[f(x0), f(y), f(-5.0)], [f(x1), f(y), f(-5.0)], [f(x1), f(y), f(5.0)], [f(x0), f(y), f(5.0)]], vec![0, 1, 2, 3])
    }

    fn piece(name: &str, parent: Option<usize>, offset_y: f32, faces: &[(f32, f32, f32)]) -> Piece {
        let mut vertices = Vec::new();
        let mut primitives = Vec::new();
        for &(y, x0, x1) in faces {
            let (v, idx) = quad(y, x0, x1);
            let first = vertices.len() as u16;
            vertices.extend(v);
            primitives.push(Primitive { color_index: 0, indices: idx.iter().map(|i| first + i).collect(), texture: None });
        }
        Piece { name: name.into(), parent, offset: [0, (offset_y * FIXED_ONE) as i32, 0], vertices, primitives, selection_primitive: None, children: Vec::new() }
    }

    #[test]
    fn a_runway_under_the_base_comes_up_in_order_under_the_pad_over_it() {
        // The Aircraft Plant's shape: a runway 0.18 under the base, a marking on it at the base,
        // a landing pad (its own piece) 0.24 up, a wall rising from the base, and a foundation
        // going 4 into the ground.
        let mut model = Model3do {
            pieces: vec![
                piece("base", None, 0.0, &[(-0.18, -60.0, 60.0), (0.0, -10.0, 10.0), (-4.0, -2.0, 2.0)]),
                piece("pad", Some(0), 0.24, &[(0.0, -20.0, 20.0)]),
            ],
        };
        let wall_top = (20.0 * FIXED_ONE) as i32;
        model.pieces[0].vertices.push([0, wall_top, 0]);
        assert_eq!(lift_ground_pads(&mut model), 8, "the runway's and the marking's corners");
        let y = |pi: usize, vi: usize| model.pieces[pi].vertices[vi][1] as f32 / FIXED_ONE;
        let (runway, marking, foundation) = (y(0, 0), y(0, 4), y(0, 8));
        let pad = 0.24 + y(1, 0);
        assert!(runway >= ABOVE - 1e-4, "the runway is above the base: {runway}");
        assert!(marking > runway + 0.01, "the marking stays on the runway: {marking} over {runway}");
        assert!(pad > marking + 0.01, "the pad stays over both: {pad} over {marking}");
        assert_eq!(foundation, -4.0, "a foundation stays in the ground");
        assert_eq!(model.pieces[0].vertices[12][1], wall_top, "nothing above moves");
    }

    #[test]
    fn a_model_without_pads_is_left_alone() {
        let mut model = Model3do { pieces: vec![piece("base", None, 0.0, &[(3.0, -5.0, 5.0)])] };
        assert_eq!(lift_ground_pads(&mut model), 0);
    }
}
