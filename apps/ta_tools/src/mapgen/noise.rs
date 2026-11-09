//! Deterministic noise for the map generator: gradient noise, its fractal sums, ridges, and a
//! hash for per-cell choices. Pure functions of their inputs and seed, so a map comes out the
//! same on every computer.

/// A well-mixed 32-bit hash of a lattice point and a seed.
#[inline]
pub fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

/// A hash as a number in [0, 1).
#[inline]
pub fn unit(x: i32, y: i32, seed: u32) -> f32 {
    (hash(x, y, seed) >> 8) as f32 / (1u32 << 24) as f32
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad(h: u32, dx: f32, dy: f32) -> f32 {
    // Eight directions.
    match h & 7 {
        0 => dx + dy,
        1 => dx - dy,
        2 => -dx + dy,
        3 => -dx - dy,
        4 => dx * 1.414,
        5 => -dx * 1.414,
        6 => dy * 1.414,
        _ => -dy * 1.414,
    }
}

/// Gradient noise, about -1 to 1, with features a unit apart.
#[inline]
pub fn perlin(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let (u, v) = (fade(fx), fade(fy));
    let n00 = grad(hash(ix, iy, seed), fx, fy);
    let n10 = grad(hash(ix + 1, iy, seed), fx - 1.0, fy);
    let n01 = grad(hash(ix, iy + 1, seed), fx, fy - 1.0);
    let n11 = grad(hash(ix + 1, iy + 1, seed), fx - 1.0, fy - 1.0);
    let a = n00 + (n10 - n00) * u;
    let b = n01 + (n11 - n01) * u;
    (a + (b - a) * v) * 0.72
}

/// Smooth value noise in [0, 1].
#[inline]
pub fn value(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = unit(ix, iy, seed) + (unit(ix + 1, iy, seed) - unit(ix, iy, seed)) * u;
    let b = unit(ix, iy + 1, seed) + (unit(ix + 1, iy + 1, seed) - unit(ix, iy + 1, seed)) * u;
    a + (b - a) * v
}

/// Fractal sum of [`perlin`]: `octaves` layers, each twice as fine and half as strong. About
/// -1 to 1.
pub fn fbm(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        // (Each octave turned a little, so their grids never line up.)
        let (c, s) = ROT[o as usize % ROT.len()];
        let (rx, ry) = (x * c - y * s, x * s + y * c);
        sum += perlin(rx * freq, ry * freq, seed.wrapping_add(o * 1013)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm * 1.6
}

const ROT: [(f32, f32); 6] = [(1.0, 0.0), (0.866, 0.5), (0.5, 0.866), (0.0, 1.0), (-0.5, 0.866), (-0.866, 0.5)];

/// Ridges: sharp crests where [`perlin`] crosses zero, 0 to 1.
pub fn ridged(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm, mut weight) = (0.0, 1.0, 1.0, 0.0, 1.0f32);
    for o in 0..octaves {
        let (c, s) = ROT[o as usize % ROT.len()];
        let (rx, ry) = (x * c - y * s, x * s + y * c);
        let n = 1.0 - perlin(rx * freq, ry * freq, seed.wrapping_add(o * 7919)).abs() * 1.3;
        let n = n.clamp(0.0, 1.0).powi(2) * weight;
        weight = (n * 1.5).clamp(0.0, 1.0);
        sum += n * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.1;
    }
    sum / norm
}

/// Cellular noise: points scattered one to a unit cell (jittered by `jitter`, 0 to 1), and at
/// (x, y) the distances to the nearest (`f1`) and second nearest (`f2`) and the nearest one's
/// hash (a value per cell, for colouring each its own). `f2 - f1` is small along the edges
/// between cells: cracks, seams, the joints between slabs and stones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub f1: f32,
    pub f2: f32,
    pub id: u32,
    /// The nearest point's offset from (x, y).
    pub dx: f32,
    pub dy: f32,
}

#[allow(dead_code)]
pub fn worley(x: f32, y: f32, seed: u32, jitter: f32) -> Cell {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let mut best = Cell { f1: f32::MAX, f2: f32::MAX, id: 0, dx: 0.0, dy: 0.0 };
    for oy in -1..=1 {
        for ox in -1..=1 {
            let (cx, cy) = (ix + ox, iy + oy);
            let h = hash(cx, cy, seed);
            let px = cx as f32 + 0.5 + (((h & 0xFFFF) as f32 / 65535.0) - 0.5) * jitter;
            let py = cy as f32 + 0.5 + (((h >> 16) as f32 / 65535.0) - 0.5) * jitter;
            let (dx, dy) = (px - x, py - y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < best.f1 {
                best.f2 = best.f1;
                best = Cell { f1: d, f2: best.f2, id: h, dx, dy };
            } else if d < best.f2 {
                best.f2 = d;
            }
        }
    }
    best
}

#[inline]
pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_and_repeatable() {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..20_000 {
            let (x, y) = (i as f32 * 0.137, (i % 97) as f32 * 0.311);
            let n = perlin(x, y, 7);
            assert_eq!(n, perlin(x, y, 7));
            lo = lo.min(n);
            hi = hi.max(n);
            let r = ridged(x, y, 3, 4);
            assert!((0.0..=1.0).contains(&r));
            let v = value(x, y, 9);
            assert!((0.0..=1.0).contains(&v));
            let c = worley(x, y, 5, 0.9);
            assert!(c.f1 <= c.f2 && c.f1 < 1.2 && c == worley(x, y, 5, 0.9));
        }
        assert!(lo > -1.2 && hi < 1.2 && hi - lo > 1.0, "{lo} {hi}");
    }
}

/// The art's fine texture repeats every this many pixels across and down (in the art's own
/// rows, not the ground's z), a whole number of tiles: then every tile of a stretch of uniform
/// ground is one of `(PERIOD / 32)^2` alike tiles that meet seamlessly whichever way they are
/// laid, as TA's own ground tiles do, and the map's tiles are spent on what is its own (shores,
/// cliffs, tracks, the edges between grounds). See [`super::tiles::group`]'s phases.
pub const PERIOD: f32 = 256.0;

/// The hash of a lattice point wrapped to `n` points a period (so the noise built on it repeats).
#[inline]
fn whash(x: i32, y: i32, n: i32, seed: u32) -> u32 {
    hash(x.rem_euclid(n), y.rem_euclid(n), seed)
}

/// How many lattice cells a period holds for features about `size` pixels apart, and the
/// lattice coordinates of art pixel (x, y).
#[inline]
fn lattice(x: f32, y: f32, size: f32) -> (f32, f32, i32) {
    let n = (PERIOD / size).round().max(1.0);
    let k = n / PERIOD;
    (x * k, y * k, n as i32)
}

/// Value noise in [0, 1] at art pixel (x, y), features about `size` pixels apart, repeating
/// every [`PERIOD`].
pub fn tvalue(x: f32, y: f32, size: f32, seed: u32) -> f32 {
    let (x, y, n) = lattice(x, y, size);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let u = |h: u32| (h >> 8) as f32 / (1u32 << 24) as f32;
    let (a, b, c, d) = (u(whash(ix, iy, n, seed)), u(whash(ix + 1, iy, n, seed)), u(whash(ix, iy + 1, n, seed)), u(whash(ix + 1, iy + 1, n, seed)));
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// [`tvalue`] with features `sx` pixels apart across and `sy` down.
pub fn tvalue2(x: f32, y: f32, sx: f32, sy: f32, seed: u32) -> f32 {
    let (nx, ny) = ((PERIOD / sx).round().max(1.0), (PERIOD / sy).round().max(1.0));
    let (x, y) = (x * nx / PERIOD, y * ny / PERIOD);
    let (nx, ny) = (nx as i32, ny as i32);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let u = |a: i32, b: i32| (hash(a.rem_euclid(nx), b.rem_euclid(ny), seed) >> 8) as f32 / (1u32 << 24) as f32;
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = u(ix, iy) + (u(ix + 1, iy) - u(ix, iy)) * sx;
    let bottom = u(ix, iy + 1) + (u(ix + 1, iy + 1) - u(ix, iy + 1)) * sx;
    top + (bottom - top) * sy
}

/// Gradient noise, about -1 to 1, at art pixel (x, y), features about `size` pixels apart,
/// repeating every [`PERIOD`].
pub fn tperlin(x: f32, y: f32, size: f32, seed: u32) -> f32 {
    let (x, y, n) = lattice(x, y, size);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let (u, v) = (fade(fx), fade(fy));
    let n00 = grad(whash(ix, iy, n, seed), fx, fy);
    let n10 = grad(whash(ix + 1, iy, n, seed), fx - 1.0, fy);
    let n01 = grad(whash(ix, iy + 1, n, seed), fx, fy - 1.0);
    let n11 = grad(whash(ix + 1, iy + 1, n, seed), fx - 1.0, fy - 1.0);
    let a = n00 + (n10 - n00) * u;
    let b = n01 + (n11 - n01) * u;
    (a + (b - a) * v) * 0.72
}

/// Fractal sum of [`tperlin`]: `octaves` layers from features `size` pixels apart, each twice as
/// fine and half as strong, each shifted (a whole number of its lattice cells, so the sum still
/// repeats every [`PERIOD`]). About -1 to 1.
pub fn tfbm(x: f32, y: f32, size: f32, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut s, mut norm) = (0.0, 1.0, size, 0.0);
    for o in 0..octaves {
        let shift = (hash(o as i32, 7, seed) % 64) as f32 * s;
        sum += tperlin(x + shift, y - shift * 0.5, s, seed.wrapping_add(o * 1013)) * amp;
        norm += amp;
        amp *= 0.5;
        s *= 0.5;
    }
    sum / norm * 1.6
}

/// Cellular noise ([`worley`]) at art pixel (x, y), cells about `sx` by `sy` pixels, repeating
/// every [`PERIOD`].
pub fn tworley(x: f32, y: f32, sx: f32, sy: f32, seed: u32, jitter: f32) -> Cell {
    let (nx, ny) = ((PERIOD / sx).round().max(1.0), (PERIOD / sy).round().max(1.0));
    let (x, y) = (x * nx / PERIOD, y * ny / PERIOD);
    let (nx, ny) = (nx as i32, ny as i32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let mut best = Cell { f1: f32::MAX, f2: f32::MAX, id: 0, dx: 0.0, dy: 0.0 };
    for oy in -1..=1 {
        for ox in -1..=1 {
            let (cx, cy) = (ix + ox, iy + oy);
            let h = hash(cx.rem_euclid(nx), cy.rem_euclid(ny), seed);
            let px = cx as f32 + 0.5 + (((h & 0xFFFF) as f32 / 65535.0) - 0.5) * jitter;
            let py = cy as f32 + 0.5 + (((h >> 16) as f32 / 65535.0) - 0.5) * jitter;
            let (dx, dy) = (px - x, py - y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < best.f1 {
                best.f2 = best.f1;
                best = Cell { f1: d, f2: best.f2, id: h, dx, dy };
            } else if d < best.f2 {
                best.f2 = d;
            }
        }
    }
    best
}

/// A hash per art pixel, in [0, 1), repeating every [`PERIOD`].
#[inline]
pub fn tunit(x: f32, y: f32, seed: u32) -> f32 {
    let p = PERIOD as i32;
    unit((x.floor() as i32).rem_euclid(p), (y.floor() as i32).rem_euclid(p), seed)
}

/// A wave across art pixel (x, y), its crests `(a, b)` periods to the period along x and y
/// (whole numbers, so it repeats every [`PERIOD`]): the phase, in waves.
#[inline]
pub fn twave(x: f32, y: f32, a: f32, b: f32) -> f32 {
    (x * a + y * b) / PERIOD
}

#[cfg(test)]
mod periodic {
    use super::*;

    #[test]
    fn the_fine_texture_repeats_every_period() {
        for i in 0..400 {
            let (x, y) = (i as f32 * 3.7 + 0.5, (i % 37) as f32 * 7.1 + 0.5);
            for (dx, dy) in [(PERIOD, 0.0), (0.0, PERIOD), (-PERIOD, 2.0 * PERIOD)] {
                let (u, v) = (x + dx, y + dy);
                assert!((tvalue(x, y, 4.3, 9) - tvalue(u, v, 4.3, 9)).abs() < 1e-4);
                assert!((tperlin(x, y, 19.0, 9) - tperlin(u, v, 19.0, 9)).abs() < 1e-4);
                assert!((tfbm(x, y, 40.0, 9, 4) - tfbm(u, v, 40.0, 9, 4)).abs() < 1e-3);
                let (a, b) = (tworley(x, y, 6.5, 5.5, 9, 0.9), tworley(u, v, 6.5, 5.5, 9, 0.9));
                assert!((a.f1 - b.f1).abs() < 1e-3 && a.id == b.id);
                assert_eq!(tunit(x, y, 3), tunit(u, v, 3));
                let d = twave(u, v, 38.0, 15.0) - twave(x, y, 38.0, 15.0);
                assert!((d - d.round()).abs() < 1e-3);
                assert!((tvalue2(x, y, 9.0, 3.2, 9) - tvalue2(u, v, 9.0, 3.2, 9)).abs() < 1e-4);
            }
        }
    }
}
