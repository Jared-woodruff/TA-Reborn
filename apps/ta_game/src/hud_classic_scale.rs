//! Enlarging TA's 640×480 interface art and lettering for the screen (the classic interface,
//! `hud_classic.rs`). Done on the player's own files, at the size each piece is shown, and
//! kept only in memory.
//!
//! Smooth (the default): the art is doubled, once or more, by directional cubic convolution
//! (each new pixel interpolated along the direction the picture varies least in, so edges and
//! TA's one-pixel lines come out straight and smooth rather than in steps; Zhou, Shen and
//! Zeng's DCCI, here choosing a direction only at an edge, never in the grain of TA's metal),
//! resampled to the size drawn, and its edges steepened back to about a screen pixel where
//! they were found (interpolation alone leaves every one of TA's hard edges a ramp as wide as
//! its pixels). TA's one-bit letters are doubled the same way as a field of cover whose
//! half-way line is their new outline, drawn anti-aliased at the size shown.
//!
//! Original pixels: each of TA's pixels a square of the screen, the screen pixels that fall
//! on a seam between two shared by area, so that at 2.25 or 4.5 screen pixels to one of
//! TA's every square is as wide as the next.

use rayon::prelude::*;

/// A pixel: premultiplied RGBA, 0..1 each.
pub type Px = [f32; 4];

/// An image of premultiplied pixels, row by row.
#[derive(Clone, Debug)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub px: Vec<Px>,
}

impl Plane {
    /// From straight (not premultiplied) RGBA8.
    pub fn from_rgba8(rgba: &[u8], width: usize, height: usize) -> Plane {
        let px = rgba
            .chunks_exact(4)
            .take(width * height)
            .map(|c| {
                let a = c[3] as f32 / 255.0;
                [c[0] as f32 / 255.0 * a, c[1] as f32 / 255.0 * a, c[2] as f32 / 255.0 * a, a]
            })
            .collect();
        Plane { width, height, px }
    }

    /// White where `bits` is set, clear elsewhere (a letter).
    pub fn from_coverage(bits: &[bool], width: usize, height: usize) -> Plane {
        Plane { width, height, px: bits.iter().take(width * height).map(|&b| if b { [1.0; 4] } else { [0.0; 4] }).collect() }
    }

    /// Straight RGBA8 again.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3].clamp(0.0, 1.0);
            let un = |c: f32| if a > 1.0 / 512.0 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
            let byte = |v: f32| (v * 255.0 + 0.5) as u8;
            out.extend_from_slice(&[byte(un(p[0])), byte(un(p[1])), byte(un(p[2])), byte(a)]);
        }
        out
    }

    /// The pixel at `(x, y)`, the nearest edge pixel outside.
    #[inline]
    fn at(&self, x: isize, y: isize) -> Px {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.px[y * self.width + x]
    }

    /// `n` pixels more on every side: the edge's own (art goes on past its edges), or clear
    /// ones (a letter stands on nothing).
    fn padded(&self, n: usize, clear: bool) -> Plane {
        let (w, h) = (self.width + 2 * n, self.height + 2 * n);
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (x as isize - n as isize, y as isize - n as isize);
                let inside = sx >= 0 && sy >= 0 && (sx as usize) < self.width && (sy as usize) < self.height;
                px.push(if clear && !inside { [0.0; 4] } else { self.at(sx, sy) });
            }
        }
        Plane { width: w, height: h, px }
    }
}

/// How far apart two pixels are, to find edges by: colour (as bright as it looks) and cover.
#[inline]
fn diff(a: Px, b: Px) -> f32 {
    0.30 * (a[0] - b[0]).abs() + 0.59 * (a[1] - b[1]).abs() + 0.11 * (a[2] - b[2]).abs() + 0.5 * (a[3] - b[3]).abs()
}

/// The cubic half-way between `b` and `c`, with `a` and `d` beyond them.
#[inline]
fn cubic4(a: Px, b: Px, c: Px, d: Px) -> Px {
    std::array::from_fn(|i| (9.0 * (b[i] + c[i]) - a[i] - d[i]) / 16.0)
}

/// No channel past the nearest pixels' (the cubic's overshoot would ring round edges).
#[inline]
fn within(p: Px, near: [Px; 4]) -> Px {
    std::array::from_fn(|i| {
        let lo = near.iter().map(|n| n[i]).fold(f32::INFINITY, f32::min);
        let hi = near.iter().map(|n| n[i]).fold(f32::NEG_INFINITY, f32::max);
        p[i].clamp(lo, hi)
    })
}

/// DCCI's choice between interpolating along one direction (`p1`, the picture varying by `g1`
/// along it) or the other (`p2`, `g2`): the one it varies much less along, or a blend
/// leaning to it. Only an edge chooses: where the two differ little in themselves (the grain
/// of TA's metal), both evenly, since choosing by chance there draws worms in the grain.
#[inline]
fn directional(g1: f32, g2: f32, p1: Px, p2: Px) -> Px {
    // (`diff`s summed over nine pairs: a difference of this is no edge, twice it one.)
    const EDGE: f32 = 0.6;
    // (The paper's thresholds are for 0-255 values.)
    const T: f32 = 1.15;
    let firm = ((g1 - g2).abs() / EDGE - 1.0).clamp(0.0, 1.0);
    if firm <= 0.0 {
        return std::array::from_fn(|i| (p1[i] + p2[i]) * 0.5);
    }
    let (s1, s2) = (g1 * 255.0, g2 * 255.0);
    let chosen = if 1.0 + s2 > T * (1.0 + s1) {
        p1
    } else if 1.0 + s1 > T * (1.0 + s2) {
        p2
    } else {
        let (w1, w2) = (1.0 / (1.0 + s1.powi(5)), 1.0 / (1.0 + s2.powi(5)));
        std::array::from_fn(|i| (w1 * p1[i] + w2 * p2[i]) / (w1 + w2))
    };
    std::array::from_fn(|i| (p1[i] + p2[i]) * 0.5 * (1.0 - firm) + chosen[i] * firm)
}

/// Twice the size, less one: the pixels kept where they were (every other one, every
/// other row) and the new ones between them interpolated along edges. `clear` edges for
/// letters (nothing past them), else the edge pixels go on.
pub fn double(src: &Plane, clear: bool) -> Plane {
    const PAD: usize = 2;
    if src.width == 0 || src.height == 0 {
        return src.clone();
    }
    let p = src.padded(PAD, clear);
    let (w, h) = (p.width, p.height);
    // The new pixels in the middle of four old ones: along the diagonal the picture varies
    // least along.
    let mut centres = vec![[0.0f32; 4]; (w - 1) * (h - 1)];
    centres.par_chunks_mut(w - 1).with_min_len(16).enumerate().for_each(|(y, row)| {
        let y = y as isize;
        let at = |x: isize, y: isize| p.at(x, y);
        for (x, out) in row.iter_mut().enumerate() {
            let x = x as isize;
            let (mut down, mut up) = (0.0, 0.0);
            for j in y - 1..=y + 1 {
                for i in x - 1..=x + 1 {
                    down += diff(at(i, j), at(i + 1, j + 1));
                    up += diff(at(i, j + 1), at(i + 1, j));
                }
            }
            let along_down = cubic4(at(x - 1, y - 1), at(x, y), at(x + 1, y + 1), at(x + 2, y + 2));
            let along_up = cubic4(at(x + 2, y - 1), at(x + 1, y), at(x, y + 1), at(x - 1, y + 2));
            *out = within(directional(down, up, along_down, along_up), [at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)]);
        }
    });
    // The doubled picture's known pixels (old ones and centres); the rest, between two old
    // and two centres, along the row or the column, the same way.
    let known = |x: isize, y: isize| -> Px {
        let (x, y) = (x as usize, y as usize);
        if x % 2 == 0 { p.px[(y / 2) * w + x / 2] } else { centres[(y / 2) * (w - 1) + x / 2] }
    };
    let (ow, oh) = (2 * src.width - 1, 2 * src.height - 1);
    let off = 2 * PAD as isize;
    let mut out = vec![[0.0f32; 4]; ow * oh];
    out.par_chunks_mut(ow).with_min_len(16).enumerate().for_each(|(oy, row)| {
        let y = oy as isize + off;
        for (ox, o) in row.iter_mut().enumerate() {
            let x = ox as isize + off;
            if (x + y) % 2 == 0 {
                *o = known(x, y);
                continue;
            }
            // (Pairs two apart of known pixels round this one, along the row; the same turned
            // for the column.)
            const PAIRS: [(isize, isize); 9] = [(-3, 0), (-1, 0), (1, 0), (-2, -1), (0, -1), (-2, 1), (0, 1), (-1, -2), (-1, 2)];
            let (mut across, mut down) = (0.0, 0.0);
            for (a, b) in PAIRS {
                across += diff(known(x + a, y + b), known(x + a + 2, y + b));
                down += diff(known(x + b, y + a), known(x + b, y + a + 2));
            }
            let along_row = cubic4(known(x - 3, y), known(x - 1, y), known(x + 1, y), known(x + 3, y));
            let along_column = cubic4(known(x, y - 3), known(x, y - 1), known(x, y + 1), known(x, y + 3));
            *o = within(directional(across, down, along_row, along_column), [known(x - 1, y), known(x + 1, y), known(x, y - 1), known(x, y + 1)]);
        }
    });
    Plane { width: ow, height: oh, px: out }
}

/// How an output pixel gathers the pixels it falls over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    /// Each by how much of the output pixel it covers (squares, as TA's pixels are).
    Area,
    /// Catmull-Rom's cubic through the nearest four, kept within the nearest two (for
    /// enlarging a little a picture already smooth).
    Cubic,
}

/// For each output pixel along one axis: the first source pixel, its weights, and the two
/// source pixels it may not go past (a cubic's).
struct Taps {
    first: Vec<usize>,
    weights: Vec<Vec<f32>>,
    near: Vec<Option<(usize, usize)>>,
}

/// Output pixel `j` lies at TA's `(j + 0.5) / scale - offset` (TA pixels); a source of `m`
/// pixels to TA's one, its first on TA's first pixel's middle (`double`'s), at `m = 1` TA's
/// own.
fn taps(out_n: usize, src_n: usize, scale: f32, offset: f32, m: f32, filter: Filter) -> Taps {
    let mut t = Taps { first: Vec::with_capacity(out_n), weights: Vec::with_capacity(out_n), near: Vec::with_capacity(out_n) };
    let last = src_n as isize - 1;
    let clampi = |i: isize| i.clamp(0, last) as usize;
    for j in 0..out_n {
        let u = (j as f32 + 0.5) / scale - offset;
        let c = (u - 0.5) * m;
        match filter {
            Filter::Area => {
                // The output pixel's span in source pixels, each source pixel `i` spanning
                // `i - 0.5 .. i + 0.5`.
                let half = 0.5 * m / scale;
                let (lo, hi) = (c - half, c + half);
                let (i0, i1) = ((lo + 0.5).floor() as isize, (hi + 0.5).ceil() as isize - 1);
                let first = clampi(i0);
                let mut w = vec![0.0f32; (clampi(i1) - first + 1).max(1)];
                for i in i0..=i1 {
                    let cover = (hi.min(i as f32 + 0.5) - lo.max(i as f32 - 0.5)).max(0.0);
                    w[clampi(i) - first] += cover;
                }
                let sum: f32 = w.iter().sum();
                if sum > 0.0 {
                    w.iter_mut().for_each(|v| *v /= sum);
                } else {
                    w[0] = 1.0;
                }
                t.first.push(first);
                t.weights.push(w);
                t.near.push(None);
            }
            Filter::Cubic => {
                let i0 = c.floor();
                let f = c - i0;
                let i0 = i0 as isize;
                // Catmull-Rom at -1, 0, 1, 2 from `i0`.
                let k = [
                    ((-f + 2.0) * f - 1.0) * f * 0.5,
                    ((3.0 * f - 5.0) * f * f + 2.0) * 0.5,
                    ((-3.0 * f + 4.0) * f + 1.0) * f * 0.5,
                    ((f - 1.0) * f * f) * 0.5,
                ];
                let first = clampi(i0 - 1);
                let end = clampi(i0 + 2);
                let mut w = vec![0.0f32; end - first + 1];
                for (d, kd) in k.iter().enumerate() {
                    w[clampi(i0 - 1 + d as isize) - first] += kd;
                }
                t.first.push(first);
                t.weights.push(w);
                t.near.push(Some((clampi(i0), clampi(i0 + 1))));
            }
        }
    }
    t
}

#[inline]
fn gather(src: &[Px], stride: usize, first: usize, w: &[f32], near: Option<(usize, usize)>) -> Px {
    let mut o = [0.0f32; 4];
    for (k, &wk) in w.iter().enumerate() {
        let p = src[(first + k) * stride];
        for c in 0..4 {
            o[c] += wk * p[c];
        }
    }
    if let Some((a, b)) = near {
        let (a, b) = (src[a * stride], src[b * stride]);
        for c in 0..4 {
            o[c] = o[c].clamp(a[c].min(b[c]), a[c].max(b[c]));
        }
    }
    o
}

/// `src` (`m` pixels to TA's one) to `out_w` × `out_h`, `scale` output pixels to TA's one,
/// `offset` TA pixels in from the output's top-left corner.
pub fn resample(src: &Plane, m: f32, out_w: usize, out_h: usize, scale: (f32, f32), offset: (f32, f32), filter: Filter) -> Plane {
    if src.width == 0 || src.height == 0 || out_w == 0 || out_h == 0 {
        return Plane { width: out_w, height: out_h, px: vec![[0.0; 4]; out_w * out_h] };
    }
    let tx = taps(out_w, src.width, scale.0, offset.0, m, filter);
    let ty = taps(out_h, src.height, scale.1, offset.1, m, filter);
    // Along the rows, then down the columns.
    let mut mid = vec![[0.0f32; 4]; out_w * src.height];
    mid.par_chunks_mut(out_w).with_min_len(16).enumerate().for_each(|(y, row)| {
        let line = &src.px[y * src.width..(y + 1) * src.width];
        for (x, o) in row.iter_mut().enumerate() {
            *o = gather(line, 1, tx.first[x], &tx.weights[x], tx.near[x]);
        }
    });
    let mut px = vec![[0.0f32; 4]; out_w * out_h];
    px.par_chunks_mut(out_w).with_min_len(16).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let p = gather(&mid[x..], out_w, ty.first[y], &ty.weights[y], ty.near[y]);
            // (Premultiplied: no colour past the cover.)
            let a = p[3].clamp(0.0, 1.0);
            *o = [p[0].clamp(0.0, a), p[1].clamp(0.0, a), p[2].clamp(0.0, a), a];
        }
    });
    Plane { width: out_w, height: out_h, px }
}

/// Interpolation makes a pixel-sharp edge of TA's a ramp as wide as one of its pixels (two
/// to five of the screen's): this steepens it back to about one screen pixel, where it was
/// smoothly found. Each output pixel's colour is taken along the line between the two most
/// different of the four TA pixels round it and moved towards the nearer end, by how much
/// they differ (lettering, bevels and outlines; not the grain of TA's metal). `scale` and
/// `offset` as [`resample`]'s.
fn steepen(out: &mut Plane, src: &Plane, scale: (f32, f32), offset: (f32, f32), strength: f32) {
    // (Contrasts, in `diff`'s measure, from which edges are steepened, and fully from.)
    const SOFT: f32 = 0.25;
    const HARD: f32 = 0.5;
    // Screen pixels a steepened edge takes.
    const EDGE: f32 = 1.2;
    let slope = scale.0.max(scale.1) / EDGE;
    let width = out.width;
    out.px.par_chunks_mut(width).with_min_len(16).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) / scale.1 - offset.1;
        let j0 = (v - 0.5).floor() as isize;
        for (x, p) in row.iter_mut().enumerate() {
            let u = (x as f32 + 0.5) / scale.0 - offset.0;
            let i0 = (u - 0.5).floor() as isize;
            let near = [src.at(i0, j0), src.at(i0 + 1, j0), src.at(i0, j0 + 1), src.at(i0 + 1, j0 + 1)];
            // The two most different.
            let (mut a, mut b, mut most) = (near[0], near[0], 0.0);
            for m in 0..4 {
                for n in m + 1..4 {
                    let d = diff(near[m], near[n]);
                    if d > most {
                        (a, b, most) = (near[m], near[n], d);
                    }
                }
            }
            let g = strength * ((most - SOFT) / (HARD - SOFT)).clamp(0.0, 1.0);
            if g <= 0.0 {
                continue;
            }
            let axis: Px = std::array::from_fn(|c| b[c] - a[c]);
            let len2: f32 = axis.iter().map(|v| v * v).sum();
            if len2 < 1e-6 {
                continue;
            }
            let t = (0..4).map(|c| (p[c] - a[c]) * axis[c]).sum::<f32>() / len2;
            let steep = ((t - 0.5) * slope + 0.5).clamp(0.0, 1.0);
            let shift = g * (steep - t.clamp(0.0, 1.0));
            let q = within(std::array::from_fn(|c| p[c] + shift * axis[c]), near);
            let alpha = q[3].clamp(0.0, 1.0);
            *p = [q[0].clamp(0.0, alpha), q[1].clamp(0.0, alpha), q[2].clamp(0.0, alpha), alpha];
        }
    });
}

/// How the classic interface's art is enlarged.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Look {
    /// Edges and lettering smooth at any size.
    #[default]
    Smooth,
    /// TA's own square pixels, evenly sized.
    Pixels,
}

/// What a picture is, for how hard its edges are made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Kind {
    /// The interface's own art: panels, buttons, lettering, bevels, titles, emblems.
    #[default]
    Interface,
    /// A unit's picture (a render, its detail speckled): its edges kept softer, or its speckle
    /// would be cut into blotches.
    Picture,
}

impl Kind {
    /// How far its edges are steepened (see `steepen`).
    fn edges(self) -> f32 {
        match self {
            Kind::Interface => 1.0,
            Kind::Picture => 0.35,
        }
    }
}

/// Doublings for a picture shown `scale` screen pixels to one of its own: enough that it is
/// at most a little enlarged after them (a little: the cubic keeps it sharp).
fn doublings(scale: f32) -> u32 {
    let mut n = 0;
    while (1u32 << n) as f32 * 1.15 < scale && n < 3 {
        n += 1;
    }
    n
}

/// Art `src` shown at `out_w` × `out_h` screen pixels.
pub fn enlarge(src: &Plane, out_w: usize, out_h: usize, look: Look, kind: Kind) -> Plane {
    let scale = (out_w as f32 / src.width.max(1) as f32, out_h as f32 / src.height.max(1) as f32);
    match look {
        Look::Pixels => resample(src, 1.0, out_w, out_h, scale, (0.0, 0.0), Filter::Area),
        Look::Smooth => {
            let n = doublings(scale.0.max(scale.1));
            let mut p = src.clone();
            for _ in 0..n {
                p = double(&p, false);
            }
            let m = (1u32 << n) as f32;
            let filter = if n == 0 || m > scale.0.min(scale.1) * 1.01 { Filter::Area } else { Filter::Cubic };
            let mut out = resample(&p, m, out_w, out_h, scale, (0.0, 0.0), filter);
            if scale.0.min(scale.1) > 1.5 {
                steepen(&mut out, src, scale, (0.0, 0.0), kind.edges());
            }
            out
        }
    }
}

/// A letter of TA's (`bits`, `w` × `h`) at `scale` screen pixels to its one, in a cell `pad`
/// pixels larger on every side, the letter `shift` screen pixels right of the cell's grid (for
/// letters that fall between whole pixels): its cover, 0..255 per pixel, and the cell's size.
pub fn letter(bits: &[bool], w: usize, h: usize, scale: f32, pad: usize, shift: f32, look: Look) -> (Vec<u8>, usize, usize) {
    let (cw, ch) = (((w as f32 * scale).ceil() as usize).max(1) + 2 * pad, ((h as f32 * scale).ceil() as usize).max(1) + 2 * pad);
    let src = Plane::from_coverage(bits, w, h);
    // (The letter's first pixel, padded with one of nothing, in TA pixels from the cell's corner.)
    let offset = ((pad as f32 + shift) / scale - 1.0, pad as f32 / scale - 1.0);
    if look == Look::Pixels || w == 0 || h == 0 {
        // Its squares.
        let p = resample(&src.padded(1, true), 1.0, cw, ch, (scale, scale), offset, Filter::Area);
        return (p.px.iter().map(|p| (p[3] * 255.0 + 0.5) as u8).collect(), cw, ch);
    }
    // The cover's field, doubled twice along its edges, then looked at four by four within
    // each screen pixel: inside where over half.
    const SUB: usize = 4;
    let field = double(&double(&src.padded(1, true), true), true);
    let fine = resample(&field, 4.0, cw * SUB, ch * SUB, (scale * SUB as f32, scale * SUB as f32), offset, Filter::Cubic);
    let mut cover = vec![0u8; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            let mut n = 0;
            for sy in 0..SUB {
                let row = &fine.px[(y * SUB + sy) * fine.width..];
                for sx in 0..SUB {
                    n += (row[x * SUB + sx][3] > 0.5) as u32;
                }
            }
            cover[y * cw + x] = ((n * 255 + (SUB * SUB) as u32 / 2) / (SUB * SUB) as u32) as u8;
        }
    }
    (cover, cw, ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Plane {
        Plane { width: w, height: h, px: (0..w * h).map(|i| [f(i % w, i / w); 4]).collect() }
    }

    /// Opaque greys.
    fn grey(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Plane {
        Plane { width: w, height: h, px: (0..w * h).map(|i| { let v = f(i % w, i / w); [v, v, v, 1.0] }).collect() }
    }

    #[test]
    fn doubling_keeps_the_old_pixels_and_flat_colour() {
        let p = plane(5, 4, |x, y| ((x * 7 + y * 3) % 5) as f32 / 4.0);
        let d = double(&p, false);
        assert_eq!((d.width, d.height), (9, 7));
        for y in 0..4 {
            for x in 0..5 {
                assert_eq!(d.px[2 * y * 9 + 2 * x], p.px[y * 5 + x], "{x},{y}");
            }
        }
        let flat = double(&plane(6, 6, |_, _| 0.4), false);
        assert!(flat.px.iter().all(|p| (p[0] - 0.4).abs() < 1e-6));
    }

    #[test]
    fn a_diagonal_line_is_joined_up() {
        // One pixel wide, corner to corner: the pixels between are filled in along it, not
        // left half-grey like a stair's corners.
        let p = plane(8, 8, |x, y| if x == y { 1.0 } else { 0.0 });
        let d = double(&p, true);
        for i in 1..6 {
            let c = d.px[(2 * i + 1) * d.width + 2 * i + 1][3];
            assert!(c > 0.9, "between ({i},{i}) and the next: {c}");
        }
    }

    #[test]
    fn grain_is_not_streaked() {
        // Faint noise (TA's metal) has no edge to follow: the new pixels are the even blend
        // of both diagonals, not one chosen by chance.
        let p = grey(9, 9, |x, y| 0.4 + 0.06 * (((x * 5 + y * 3) % 7) as f32 / 6.0));
        let d = double(&p, false);
        for (x, y) in [(3, 3), (4, 5), (5, 2)] {
            let at = |x: usize, y: usize| p.px[y * 9 + x][0];
            let even = (9.0 * (at(x, y) + at(x + 1, y + 1) + at(x + 1, y) + at(x, y + 1)) - at(x - 1, y - 1) - at(x + 2, y + 2) - at(x + 2, y - 1) - at(x - 1, y + 2)) / 32.0;
            let lo = [at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)].into_iter().fold(1.0, f32::min);
            let hi = [at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)].into_iter().fold(0.0, f32::max);
            assert!((d.px[(2 * y + 1) * d.width + 2 * x + 1][0] - even.clamp(lo, hi)).abs() < 1e-5);
        }
    }

    #[test]
    fn the_cubic_stays_within_its_neighbours() {
        // A hard edge: no ringing above white or below black.
        let p = plane(8, 3, |x, _| if x < 4 { 0.0 } else { 1.0 });
        for (w, h) in [(37, 14), (17, 7), (24, 9)] {
            let out = enlarge(&p, w, h, Look::Smooth, Kind::Interface);
            assert!(out.px.iter().all(|p| p[0] >= -1e-6 && p[0] <= 1.0 + 1e-6));
        }
    }

    #[test]
    fn hard_edges_stay_hard() {
        // Black to white at 4.5 screen pixels to TA's one: smooth would make a ramp of four or
        // five pixels; steepened, the edge is crossed in two, where it was.
        let p = grey(8, 4, |x, _| if x < 4 { 0.0 } else { 1.0 });
        let out = enlarge(&p, 36, 18, Look::Smooth, Kind::Interface);
        let row: Vec<f32> = (0..36).map(|x| out.px[9 * 36 + x][0]).collect();
        let grey = row.iter().filter(|&&v| v > 0.05 && v < 0.95).count();
        assert!(grey <= 2, "{row:?}");
        assert!(row[17] < 0.5 && row[18] > 0.5, "the edge at TA's x = 4: {row:?}");
    }

    #[test]
    fn original_pixels_are_even_squares() {
        // Two and a quarter screen pixels to each: every column of squares as wide, the seams
        // shared.
        let p = plane(4, 1, |x, _| (x % 2) as f32);
        let out = enlarge(&p, 9, 1, Look::Pixels, Kind::Interface);
        let v: Vec<f32> = out.px.iter().map(|p| p[0]).collect();
        let total: f32 = v.iter().sum();
        assert!((total - 4.5).abs() < 1e-4, "{v:?}");
        assert_eq!(v[0], 0.0);
        assert!((v[2] - 0.75).abs() < 1e-4, "{v:?}");
        // Whole multiples are exact copies.
        let out = enlarge(&p, 12, 3, Look::Pixels, Kind::Interface);
        let want = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert!(out.px[..12].iter().zip(want).all(|(p, w)| (p[0] - w).abs() < 1e-5), "{:?}", &out.px[..12]);
    }

    #[test]
    fn letters_keep_their_place_and_weight() {
        // A one-pixel stroke three tall, at three screen pixels to one: about three wide,
        // where it was; shifted half a pixel, half a pixel right.
        let bits = [false, true, false, false, true, false, false, true, false];
        let ink = |shift: f32, look: Look| {
            let (cover, w, h) = letter(&bits, 3, 3, 3.0, 2, shift, look);
            assert_eq!((w, h), (13, 13));
            let row: Vec<f32> = (0..w).map(|x| cover[6 * w + x] as f32 / 255.0).collect();
            let total: f32 = row.iter().sum();
            let middle = row.iter().enumerate().map(|(x, c)| (x as f32 + 0.5) * c).sum::<f32>() / total;
            (total, middle)
        };
        for look in [Look::Smooth, Look::Pixels] {
            let (total, middle) = ink(0.0, look);
            assert!((total - 3.0).abs() < 0.8, "{look:?}: {total}");
            // (The stroke spans TA's x 1..2: screen 2 + 3..6.)
            assert!((middle - 6.5).abs() < 0.3, "{look:?}: {middle}");
            let (_, moved) = ink(0.5, look);
            assert!((moved - middle - 0.5).abs() < 0.3, "{look:?}: {middle} -> {moved}");
        }
    }
}
