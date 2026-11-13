//! TA's view of the ground: the map's art is painted as the game's camera saw it, from the
//! south and above, so a place at height `h` shows `h / 2` pixels higher up the picture than
//! its z (the renderer looks the art up at `(x, z - h / 2)`). Painting a map's art therefore
//! means, for each pixel of a column, finding which ground the camera saw there.

/// For each art row of the columns `x0..x0 + width` (pixels), the ground seen there in TA's
/// view: its z and height, given the ground's height at any point (`height(x, z)`, pixels).
/// Marching from north to south, nearer (more southern) ground drawn over farther; ground
/// rising faster than two units a pixel towards the south faces away and is hidden. Rows
/// `0..rows`.
pub fn columns(height: impl Fn(f32, f32) -> f32, x0: usize, width: usize, rows: usize) -> Vec<Vec<(f32, f32)>> {
    (x0..x0 + width)
        .map(|x| {
            let xf = x as f32 + 0.5;
            let mut out = vec![(f32::NAN, 0.0f32); rows];
            let step = 0.5;
            let mut z = -160.0f32;
            let mut prev: Option<(f32, f32, f32)> = None;
            let end = rows as f32 + 320.0;
            while z < end {
                let h = height(xf, z.max(0.0));
                let row = z - h / 2.0;
                if let Some((pz, ph, prow)) = prev
                    && row > prow
                {
                    let (r0, r1) = (prow.ceil().max(0.0) as i64, (row.floor() as i64).min(rows as i64 - 1));
                    for r in r0..=r1 {
                        let t = (r as f32 - prow) / (row - prow);
                        out[r as usize] = (pz + (z - pz) * t, ph + (h - ph) * t);
                    }
                }
                prev = Some((z, h, row));
                z += step;
            }
            // (Rows the march never reached take the ground below them.)
            for r in (0..rows.saturating_sub(1)).rev() {
                if out[r].0.is_nan() {
                    out[r] = out[r + 1];
                }
            }
            for r in 1..rows {
                if out[r].0.is_nan() {
                    out[r] = out[r - 1];
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground at height 40 is seen 20 rows up; a step up hides nothing and shows its face.
    #[test]
    fn the_ground_is_seen_half_its_height_up_the_picture() {
        let cols = columns(|_, _| 40.0, 0, 1, 64);
        assert!((cols[0][10].0 - 30.5).abs() < 0.5 || (cols[0][10].0 - 30.0).abs() < 0.6, "{:?}", cols[0][10]);
        assert_eq!(cols[0][10].1, 40.0);
        // A wall rising 60 at z = 100 (facing the camera): rows 70..100 show its face.
        let cols = columns(|_, z| if z < 100.0 { 0.0 } else { 60.0 }, 0, 1, 160);
        assert!(cols[0][60].1 < 1.0, "the low ground before the wall: {:?}", cols[0][60]);
        assert!(cols[0][120].1 > 59.0, "the top beyond it: {:?}", cols[0][120]);
    }
}
