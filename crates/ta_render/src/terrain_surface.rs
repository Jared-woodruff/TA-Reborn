//! The ground as it is drawn: a smooth surface over TA's heightmap.
//!
//! TA's heightmap is one whole-unit height a 16-unit cell. Drawn as it is, as flat triangles,
//! the ground shows facets: a ridge's crest is a sawtooth of straight segments with a spike at
//! every height sample, a single cell a few units higher than those round it is a perfect
//! pyramid (Dark Side's lone spikes, 18 units on a 32-unit base), a single cell lower a planar
//! V-notch. TA's own oblique camera never showed any of it.
//!
//! The drawn surface ([`display_heights`]) follows the heightmap within a tolerance: half a unit
//! (its rounding) where units walk, up to six units on ground steeper than any unit climbs. The
//! heights are smoothed within that tolerance, then taken as the control points of a cubic
//! B-spline (pre-sharpened on steep ground, so hills keep their peaks), whose values at the
//! vertices are the heights drawn. Where a vertex and everything round it lies above the sea
//! (or under it) it stays there, so coasts stay where they are. The simulation keeps TA's
//! heights; what stands on the ground is drawn on this one (`ta_game::scene::drawn_ground`).
//! In TA's oblique projection the ground at any height shows the art it covers, so the classic
//! camera's picture is unchanged by it.

use rayon::prelude::*;

/// Where a cell's corners are this many units apart or more on average round a vertex, it is
/// steep ground (fully so at twice that): the tolerance there grows to [`STEEP_TOLERANCE`].
const STEEP_FROM: f32 = 10.0;
/// How far (units) the drawn ground may leave TA's heights on level ground (their rounding)
/// and on the steepest.
const LEVEL_TOLERANCE: f32 = 0.5;
const STEEP_TOLERANCE: f32 = 6.0;
/// On metal and urban worlds (walls and terraces built square), the steepest's tolerance.
const BUILT_TOLERANCE: f32 = 1.5;
/// Smoothing passes within the tolerance (16 are within a unit of converged).
const PASSES: usize = 24;
/// How far (units) a vertex surrounded by land stays above the sea, or one surrounded by sea
/// below it.
const COAST_MARGIN: f32 = 0.25;

/// The drawn heights of the vertices of a `width` × `height` heightmap (`heights`, row-major,
/// whole units), `sea` its sea level; `built`: a metal or urban world, whose steep faces keep
/// closer to their squared-off heights.
pub fn display_heights(width: u32, height: u32, heights: &[u8], sea: f32, built: bool) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || heights.len() < w * h {
        return heights.iter().map(|&v| v as f32).collect();
    }
    let ta: Vec<f32> = heights[..w * h].iter().map(|&v| v as f32).collect();
    // How steep the ground is round each vertex (0 level .. 1 steep): the mean of its cells'
    // corner ranges.
    let range = |cx: usize, cz: usize| {
        let (x1, z1) = ((cx + 1).min(w - 1), (cz + 1).min(h - 1));
        let c = [ta[cz * w + cx], ta[cz * w + x1], ta[z1 * w + cx], ta[z1 * w + x1]];
        c.iter().copied().fold(f32::MIN, f32::max) - c.iter().copied().fold(f32::MAX, f32::min)
    };
    let steep: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, z) = (i % w, i / w);
            let mut sum = 0.0f32;
            let mut n = 0.0f32;
            for (cx, cz) in [(x.wrapping_sub(1), z.wrapping_sub(1)), (x, z.wrapping_sub(1)), (x.wrapping_sub(1), z), (x, z)] {
                if cx < w.saturating_sub(1).max(1) && cz < h.saturating_sub(1).max(1) {
                    sum += range(cx, cz);
                    n += 1.0;
                }
            }
            ((sum / n.max(1.0) - STEEP_FROM) / STEEP_FROM).clamp(0.0, 1.0)
        })
        .collect();
    let steepest = if built { BUILT_TOLERANCE } else { STEEP_TOLERANCE };
    let tolerance: Vec<f32> = steep.iter().map(|&k| LEVEL_TOLERANCE + (steepest - LEVEL_TOLERANCE) * k).collect();
    // Smoothed within the tolerance.
    let mut smooth = ta.clone();
    for _ in 0..PASSES {
        let blurred = filter(&smooth, w, h, [0.25, 0.5, 0.25]);
        smooth = blurred.par_iter().zip(&ta).zip(&tolerance).map(|((&b, &t), &tol)| b.clamp(t - tol, t + tol)).collect();
    }
    // The B-spline's control points: sharpened where the ground is steep, so the spline (which
    // smooths what it passes near) keeps the hills' peaks and the cliffs' lips; the spline at
    // the vertices. (Built worlds' walls stay as square as their tolerance: no spline.)
    let drawn = if built {
        smooth
    } else {
        let steep = filter(&steep, w, h, [0.25, 0.5, 0.25]);
        let spline = [1.0 / 6.0, 4.0 / 6.0, 1.0 / 6.0];
        let splined = filter(&smooth, w, h, spline);
        let controls: Vec<f32> = smooth.par_iter().zip(&splined).zip(&steep).map(|((&s, &b), &k)| s + k * (s - b)).collect();
        filter(&controls, w, h, spline)
    };
    // Land and sea away from the coast kept as they are.
    (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, z) = (i % w, i / w);
            let (mut wet, mut dry) = (true, true);
            for nz in z.saturating_sub(1)..=(z + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let under = ta[nz * w + nx] < sea;
                    wet &= under;
                    dry &= !under;
                }
            }
            let v = drawn[i];
            if dry {
                v.max(sea + COAST_MARGIN)
            } else if wet {
                v.min(sea - COAST_MARGIN)
            } else {
                v
            }
        })
        .collect()
}

/// `values` (a `w` × `h` grid) filtered by the 3-tap kernel `k` along both axes, clamped at
/// the edges.
fn filter(values: &[f32], w: usize, h: usize, k: [f32; 3]) -> Vec<f32> {
    let across: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, z) = (i % w, i / w);
            let row = &values[z * w..z * w + w];
            k[0] * row[x.saturating_sub(1)] + k[1] * row[x] + k[2] * row[(x + 1).min(w - 1)]
        })
        .collect();
    (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, z) = (i % w, i / w);
            k[0] * across[z.saturating_sub(1) * w + x] + k[1] * across[i] + k[2] * across[(z + 1).min(h - 1) * w + x]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..h).flat_map(|z| (0..w).map(move |x| (x, z))).map(|(x, z)| f(x, z)).collect()
    }

    #[test]
    fn level_ground_and_gentle_slopes_stay_where_units_walk() {
        let (w, h) = (24, 24);
        for heights in [grid(w, h, |_, _| 40), grid(w, h, |x, _| 30 + x as u8), grid(w, h, |x, z| 30 + ((x + z) / 3) as u8)] {
            let drawn = display_heights(w as u32, h as u32, &heights, 0.0, false);
            for (i, (&d, &t)) in drawn.iter().zip(&heights).enumerate() {
                let (x, z) = (i % w, i / w);
                if (2..w - 2).contains(&x) && (2..h - 2).contains(&z) {
                    assert!((d - t as f32).abs() <= 0.6, "({x}, {z}): {d} drawn for {t}");
                }
            }
        }
    }

    #[test]
    fn a_lone_spike_becomes_a_low_mound() {
        // Dark Side: single vertices 18 units above a plateau.
        let (w, h) = (16, 16);
        let heights = grid(w, h, |x, z| if (x, z) == (8, 8) { 73 } else { 55 });
        let drawn = display_heights(w as u32, h as u32, &heights, 0.0, false);
        let top = drawn[8 * w + 8];
        assert!(top > 57.0 && top < 64.0, "the spike drawn {top} high over 55");
        assert!(drawn[8 * w + 9] < top && drawn[8 * w + 9] > 55.0, "and its foot a slope");
    }

    #[test]
    fn coasts_keep_their_land_and_sea() {
        // A beach a unit above the sea and the sea bed a unit below it.
        let (w, h) = (20, 20);
        let sea = 40.0;
        let heights = grid(w, h, |x, _| if x < 10 { 39 } else if x < 12 { 41 } else { 41 + 4 * (x as u8 - 12) });
        let drawn = display_heights(w as u32, h as u32, &heights, sea, false);
        for (i, &d) in drawn.iter().enumerate() {
            let x = i % w;
            if x <= 8 {
                assert!(d < sea, "sea bed at {x} drawn dry ({d})");
            } else if x >= 11 {
                assert!(d > sea, "land at {x} drawn under the sea ({d})");
            }
        }
    }

    #[test]
    fn steep_faces_move_at_most_their_tolerance_and_keep_their_lip() {
        // A cliff 40 units high in one cell.
        let (w, h) = (20, 20);
        let heights = grid(w, h, |x, _| if x < 10 { 60 } else { 100 });
        for built in [false, true] {
            let drawn = display_heights(w as u32, h as u32, &heights, 0.0, built);
            for (i, (&d, &t)) in drawn.iter().zip(&heights).enumerate() {
                // (The spline adds its own rounding to the smoothing's tolerance.)
                assert!((d - t as f32).abs() < if built { BUILT_TOLERANCE + 0.01 } else { 9.0 }, "{i}: {d} for {t}");
            }
            // Away from the cliff the plateau and the plain are where they were.
            assert!((drawn[10 * w + 3] - 60.0).abs() < 0.6 && (drawn[10 * w + 16] - 100.0).abs() < 0.6);
        }
    }
}
