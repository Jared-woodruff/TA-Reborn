//! A grid of numbers over the map's cells, and what the generator asks of one: values between
//! cells (as the game interpolates heights), slopes, and distances.

use rayon::prelude::*;

#[derive(Clone)]
pub struct Field {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Field {
    pub fn new(w: usize, h: usize, v: f32) -> Field {
        Field { w, h, data: vec![v; w * h] }
    }

    /// A field from `f(x, z)` at every cell, computed in parallel.
    pub fn from_fn(w: usize, h: usize, f: impl Fn(usize, usize) -> f32 + Sync) -> Field {
        let mut data = vec![0.0; w * h];
        data.par_chunks_mut(w).enumerate().for_each(|(z, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = f(x, z);
            }
        });
        Field { w, h, data }
    }

    #[inline]
    pub fn get(&self, x: i64, z: i64) -> f32 {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let z = z.clamp(0, self.h as i64 - 1) as usize;
        self.data[z * self.w + x]
    }

    #[inline]
    pub fn at(&self, x: usize, z: usize) -> f32 {
        self.data[z * self.w + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, z: usize, v: f32) {
        self.data[z * self.w + x] = v;
    }

    /// The value at a point in cell units, as the game draws a cell's corner heights: two
    /// triangles split along the diagonal from the cell's top left to its bottom right, or on a
    /// steep cell (corners 16 or more apart) along the diagonal whose ends differ less. (The
    /// renderer's rule, `ta_render::terrain::split_flipped`: the art lines up with the ground.)
    #[inline]
    pub fn sample_tri(&self, x: f32, z: f32) -> f32 {
        let (cx, cz) = (x.floor(), z.floor());
        let (tx, tz) = (x - cx, z - cz);
        let (cx, cz) = (cx as i64, cz as i64);
        let (h00, h10, h01, h11) = (self.get(cx, cz), self.get(cx + 1, cz), self.get(cx, cz + 1), self.get(cx + 1, cz + 1));
        let range = h00.max(h10).max(h01).max(h11) - h00.min(h10).min(h01).min(h11);
        if range >= 16.0 && (h10 - h01).abs() < (h00 - h11).abs() {
            if tx + tz <= 1.0 {
                h00 + (h10 - h00) * tx + (h01 - h00) * tz
            } else {
                h11 + (h01 - h11) * (1.0 - tx) + (h10 - h11) * (1.0 - tz)
            }
        } else if tx >= tz {
            h00 + (h10 - h00) * tx + (h11 - h10) * tz
        } else {
            h00 + (h11 - h01) * tx + (h01 - h00) * tz
        }
    }

    /// The steepness of the field at each cell (change per cell, from its neighbours).
    pub fn gradient(&self) -> Field {
        Field::from_fn(self.w, self.h, |x, z| {
            let (x, z) = (x as i64, z as i64);
            let gx = (self.get(x + 1, z) - self.get(x - 1, z)) * 0.5;
            let gz = (self.get(x, z + 1) - self.get(x, z - 1)) * 0.5;
            (gx * gx + gz * gz).sqrt()
        })
    }

    /// The value at a point in cell units, interpolated bilinearly.
    #[inline]
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let (cx, cz) = (x.floor(), z.floor());
        let (tx, tz) = (x - cx, z - cz);
        let (cx, cz) = (cx as i64, cz as i64);
        let a = self.get(cx, cz) + (self.get(cx + 1, cz) - self.get(cx, cz)) * tx;
        let b = self.get(cx, cz + 1) + (self.get(cx + 1, cz + 1) - self.get(cx, cz + 1)) * tx;
        a + (b - a) * tz
    }

    /// TA's own slope: a cell's highest corner less its lowest (what movement classes limit).
    pub fn ta_slope(&self) -> Field {
        Field::from_fn(self.w, self.h, |x, z| {
            let (x, z) = (x as i64, z as i64);
            let c = [self.get(x, z), self.get(x + 1, z), self.get(x, z + 1), self.get(x + 1, z + 1)];
            c.iter().fold(f32::MIN, |a, &b| a.max(b)) - c.iter().fold(f32::MAX, |a, &b| a.min(b))
        })
    }

    /// A box blur of radius `r`, three times over (near enough a gaussian).
    pub fn blurred(&self, r: usize) -> Field {
        let mut f = self.clone();
        for _ in 0..3 {
            f = f.box_pass(r, true).box_pass(r, false);
        }
        f
    }

    fn box_pass(&self, r: usize, horizontal: bool) -> Field {
        let (w, h) = (self.w, self.h);
        let k = (2 * r + 1) as f32;
        Field::from_fn(w, h, |x, z| {
            let mut s = 0.0;
            for d in -(r as i64)..=(r as i64) {
                s += if horizontal { self.get(x as i64 + d, z as i64) } else { self.get(x as i64, z as i64 + d) };
            }
            s / k
        })
    }

    pub fn map(&self, f: impl Fn(f32) -> f32 + Sync) -> Field {
        Field { w: self.w, h: self.h, data: self.data.par_iter().map(|&v| f(v)).collect() }
    }
}

/// The distance in cells from each cell to the nearest cell where `seed` is true (exact
/// Euclidean, Felzenszwalb and Huttenlocher's two passes).
pub fn distance(w: usize, h: usize, seed: &[bool]) -> Field {
    const INF: f32 = 1e12;
    let mut d: Vec<f32> = seed.iter().map(|&s| if s { 0.0 } else { INF }).collect();
    // Columns, then rows.
    let mut cols: Vec<Vec<f32>> = (0..w).into_par_iter().map(|x| edt_1d(&(0..h).map(|z| d[z * w + x]).collect::<Vec<_>>())).collect();
    for (x, col) in cols.iter_mut().enumerate() {
        for (z, v) in col.iter().enumerate() {
            d[z * w + x] = *v;
        }
    }
    let rows: Vec<Vec<f32>> = (0..h).into_par_iter().map(|z| edt_1d(&d[z * w..(z + 1) * w])).collect();
    let data = rows.into_iter().flatten().map(|v| v.sqrt()).collect();
    Field { w, h, data }
}

/// Squared distance transform of a sampled function (one dimension): the lower envelope of
/// the parabolas rooted at the finite samples.
fn edt_1d(f: &[f32]) -> Vec<f32> {
    const INF: f64 = 1e30;
    let n = f.len();
    let roots: Vec<usize> = (0..n).filter(|&q| f[q] < 1e11).collect();
    if roots.is_empty() {
        return vec![1e12; n];
    }
    let g = |q: usize| f[q] as f64 + (q * q) as f64;
    let cross = |q: usize, p: usize| (g(q) - g(p)) / (2.0 * (q as f64 - p as f64));
    let mut v: Vec<usize> = Vec::with_capacity(roots.len());
    let mut z: Vec<f64> = Vec::with_capacity(roots.len() + 1);
    v.push(roots[0]);
    z.push(-INF);
    z.push(INF);
    for &q in &roots[1..] {
        let mut s = cross(q, *v.last().unwrap());
        while s <= z[v.len() - 1] {
            v.pop();
            z.pop();
            s = cross(q, *v.last().unwrap());
        }
        let k = v.len();
        v.push(q);
        z[k] = s;
        z.push(INF);
    }
    let mut k = 0;
    (0..n)
        .map(|q| {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            ((q as f64 - p as f64).powi(2) + f[p] as f64) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_euclidean() {
        let (w, h) = (40, 30);
        let mut seed = vec![false; w * h];
        seed[10 * w + 12] = true;
        seed[25 * w + 30] = true;
        let d = distance(w, h, &seed);
        for z in 0..h {
            for x in 0..w {
                let a = (((x as f32 - 12.0).powi(2) + (z as f32 - 10.0).powi(2)).sqrt()).min(((x as f32 - 30.0).powi(2) + (z as f32 - 25.0).powi(2)).sqrt());
                assert!((d.at(x, z) - a).abs() < 1e-3, "{x},{z}: {} vs {a}", d.at(x, z));
            }
        }
    }

    #[test]
    fn triangles_meet_the_corners() {
        let f = Field { w: 2, h: 2, data: vec![0.0, 10.0, 20.0, 30.0] };
        assert_eq!(f.sample_tri(0.0, 0.0), 0.0);
        assert_eq!(f.sample_tri(1.0, 0.0), 10.0);
        assert_eq!(f.sample_tri(0.0, 1.0), 20.0);
        assert_eq!(f.sample_tri(1.0, 1.0), 30.0);
        // A steep cell rising from its north-west corner to its south-east one (a lip running
        // south-west to north-east) is split along the other diagonal: the middle of the slope
        // is level along it.
        let lip = Field { w: 2, h: 2, data: vec![0.0, 20.0, 20.0, 40.0] };
        assert_eq!(lip.sample_tri(0.5, 0.5), 20.0);
        assert_eq!(lip.sample_tri(0.75, 0.25), 20.0);
        // A gentle one keeps the first split.
        let gentle = Field { w: 2, h: 2, data: vec![0.0, 4.0, 4.0, 14.0] };
        assert_eq!(gentle.sample_tri(0.5, 0.5), 7.0);
    }
}
