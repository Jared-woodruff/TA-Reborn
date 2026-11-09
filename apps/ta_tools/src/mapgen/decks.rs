//! Built land (metal worlds): decks of plating at a few levels on a grid of blocks, dropping to
//! coolant channels at vertical walls, joined by straight ramps where neighbouring decks differ by
//! a level, raised bunkers on them; the heights square, as TA's metal worlds are built.
//!
//! A block's level comes from a smooth noise read at its middle (through the symmetry, so the
//! blocks of a symmetric map are twins); channels follow the lines where a second noise is high,
//! so they run in canals across the map rather than as scattered pools. Each block keeps a margin
//! at its own level; across its border with a neighbour the height steps in two cells (a wall)
//! or, along the middle of the border where a ramp is chosen, rises evenly over the ramp's
//! length.

use super::field::Field;
use super::noise::{fbm, ridged, smoothstep, unit};
use super::terrain::Sym;

/// How a built world's land is laid out.
#[derive(Clone)]
pub struct Decks {
    /// Cells across a block.
    pub block: usize,
    /// The decks' heights, lowest first.
    pub levels: Vec<f32>,
    /// The channels' floor (below the sea), and how much of the map they take (0 none).
    pub floor: f32,
    pub channels: f32,
    /// How many blocks carry a raised bunker (0 to 1) and how high it stands over its deck.
    pub bunkers: f32,
    pub bunker_h: f32,
    /// How many of the borders between decks a level apart carry a ramp (0 to 1); the others are
    /// walls. Borders more than a level apart are always walls.
    pub ramps: f32,
}

/// What the painter and placer read of a built world.
pub struct Built {
    /// Each cell's block (index into `block_level`), and each block's level (-1 a channel).
    pub block_of: Vec<u32>,
    pub block_level: Vec<i32>,
    /// Each block's bunker (x0, z0, x1, z1 in cells), if it has one.
    pub bunkers: Vec<Option<(usize, usize, usize, usize)>>,
    /// Cells on a ramp.
    pub ramp: Vec<bool>,
}

/// The heights of a built world `w` x `h` cells, and what was built.
pub fn build(d: &Decks, w: usize, h: usize, sea: f32, seed: u32, sym: &Sym) -> (Field, Built) {
    let b = d.block.max(8);
    let (bw, bh) = (w.div_ceil(b), h.div_ceil(b));
    let nf = w.max(h) as f32;
    // Each block's level: the noise at its middle, ranked into the levels' shares.
    let mid = |i: usize, j: usize| ((i * b + b / 2).min(w - 1) as f32, (j * b + b / 2).min(h - 1) as f32);
    let raw: Vec<f32> = (0..bw * bh)
        .map(|k| {
            let (x, z) = mid(k % bw, k / bw);
            sym.eval(x, z, |x, z| fbm(x / nf * 3.0, z / nf * 3.0, seed ^ 0xDE1, 4))
        })
        .collect();
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &c| raw[a].total_cmp(&raw[c]).then(a.cmp(&c)));
    let mut level = vec![0i32; bw * bh];
    let n = d.levels.len() as f32;
    for (r, &k) in order.iter().enumerate() {
        level[k] = ((r as f32 / raw.len() as f32) * n).floor().min(n - 1.0) as i32;
    }
    // Channels: blocks where canal lines run (ridges of a noise), the lowest ones first.
    if d.channels > 0.0 {
        let canal: Vec<f32> = (0..bw * bh)
            .map(|k| {
                let (x, z) = mid(k % bw, k / bw);
                sym.eval(x, z, |x, z| ridged(x / nf * 2.2, z / nf * 2.2, seed ^ 0xC4A, 3)) - level[k] as f32 * 0.08
            })
            .collect();
        let mut by: Vec<usize> = (0..canal.len()).collect();
        by.sort_by(|&a, &c| canal[c].total_cmp(&canal[a]).then(a.cmp(&c)));
        for &k in by.iter().take((d.channels * canal.len() as f32) as usize) {
            level[k] = -1;
        }
    }
    // Twins agree (the ranking above can split a tie across the halves).
    if sym.on {
        for k in 0..bw * bh {
            let (i, j) = (k % bw, k / bw);
            let t = (bh - 1 - j) * bw + (bw - 1 - i);
            if t > k {
                let v = level[k].max(level[t]);
                level[k] = v;
                level[t] = v;
            }
        }
    }
    let height_of = |lv: i32| if lv < 0 { d.floor } else { d.levels[lv as usize] };
    // Which borders carry a ramp: between decks a level apart, chosen by a hash of the pair
    // (the same for a border and its twin).
    let ramp_between = |a: usize, c: usize| -> bool {
        let (la, lc) = (level[a], level[c]);
        if la < 0 || lc < 0 || (la - lc).abs() != 1 {
            return false;
        }
        let twin = |k: usize| (bh - 1 - k / bw) * bw + (bw - 1 - k % bw);
        let pair = |p: usize, q: usize| (p.min(q), p.max(q));
        let key = if sym.on { pair(a, c).min(pair(twin(a), twin(c))) } else { pair(a, c) };
        unit(key.0 as i32, key.1 as i32, seed ^ 0x4A9) < d.ramps
    };
    let mut height = Field::new(w, h, 0.0);
    let mut ramp = vec![false; w * h];
    let mut block_of = vec![0u32; w * h];
    // (How far across a border a ramp runs, each side, and how wide it is along it.)
    let (run, half_wide) = (((d.levels.windows(2).map(|p| p[1] - p[0]).fold(0.0f32, f32::max)) / 7.0).ceil() as i64, (b as i64 / 4).max(3));
    for z in 0..h {
        for x in 0..w {
            let (i, j) = (x / b, z / b);
            let k = j * bw + i;
            block_of[z * w + x] = k as u32;
            let mut v = height_of(level[k]);
            let (lx, lz) = ((x % b) as i64, (z % b) as i64);
            let bi = b as i64;
            // The neighbour across the nearest border, and how far it is.
            let borders = [(-1i64, 0i64, lx), (1, 0, bi - 1 - lx), (0, -1, lz), (0, 1, bi - 1 - lz)];
            for &(di, dj, dist) in &borders {
                let (ni, nj) = (i as i64 + di, j as i64 + dj);
                if ni < 0 || nj < 0 || ni >= bw as i64 || nj >= bh as i64 {
                    continue;
                }
                let nk = nj as usize * bw + ni as usize;
                let nv = height_of(level[nk]);
                if (nv - height_of(level[k])).abs() < 0.5 {
                    continue;
                }
                // Along the border, how far from its middle (doubled, so a cell and its twin
                // measure alike).
                let along = if di != 0 { 2 * lz + 1 - bi } else { 2 * lx + 1 - bi };
                if ramp_between(k, nk) && along.abs() <= 2 * half_wide && dist < run {
                    // On the ramp: from this deck's level at `run` cells in to the middle of
                    // the rise at the border.
                    let t = 0.5 - (dist as f32 + 0.5) / (2.0 * run as f32);
                    let r = v + (nv - height_of(level[k])) * t.clamp(0.0, 0.5);
                    if (r - v).abs() > 0.01 {
                        v = r;
                        ramp[z * w + x] = true;
                    }
                } else if dist == 0 {
                    // The wall: the border's cell halfway down (or up), the step square.
                    let lower = nv.min(height_of(level[k]));
                    if height_of(level[k]) > nv {
                        v = v.min(lower + (height_of(level[k]) - lower) * 0.5);
                    }
                }
            }
            height.set(x, z, v);
        }
    }
    // Bunkers: a raised block with sloped sides inside some decks, clear of their borders.
    let mut bunkers = vec![None; bw * bh];
    if d.bunkers > 0.0 {
        for k in 0..bw * bh {
            let (i, j) = (k % bw, k / bw);
            if level[k] < 0 || unit(i as i32, j as i32, seed ^ 0xB4B) >= d.bunkers {
                continue;
            }
            if sym.on && !sym.first(mid(i, j).0, mid(i, j).1) {
                continue;
            }
            // Its size and proportions its own, and off the block's middle (clear of its border).
            let (u1, u2, u3, u4) = (unit(j as i32, i as i32, seed ^ 0xB4C), unit(i as i32, j as i32, seed ^ 0xB4D), unit(j as i32, i as i32, seed ^ 0xB4E), unit(i as i32, j as i32, seed ^ 0xB4F));
            let sw = (b as f32 * (0.2 + 0.22 * u1)) as usize;
            let sh = (sw as f32 * (0.55 + 0.9 * u2)).min(b as f32 * 0.45) as usize;
            let margin = 5;
            let (fx, fz) = (b.saturating_sub(sw + 2 * margin), b.saturating_sub(sh + 2 * margin));
            let (x0, z0) = (i * b + margin + (u3 * fx as f32) as usize, j * b + margin + (u4 * fz as f32) as usize);
            let r = (x0, z0, (x0 + sw).min(w - 1), (z0 + sh).min(h - 1));
            bunkers[k] = Some(r);
            if sym.on {
                let t = (bh - 1 - j) * bw + (bw - 1 - i);
                let (tx0, tz0) = (w - 1 - r.2, h - 1 - r.3);
                bunkers[t] = Some((tx0, tz0, tx0 + (r.2 - r.0), tz0 + (r.3 - r.1)));
            }
        }
        for (k, bk) in bunkers.iter().enumerate() {
            let Some((x0, z0, x1, z1)) = *bk else { continue };
            let base = height_of(level[k]);
            // (Its sides two cells wide: too steep to climb, cover to fight from.)
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let edge = (x - x0).min(x1 - x).min(z - z0).min(z1 - z) as f32;
                    let rise = d.bunker_h * smoothstep(0.0, 2.0, edge + 0.5);
                    let v = height.at(x, z).max(base + rise);
                    height.set(x, z, v);
                }
            }
        }
    }
    let _ = sea;
    (height, Built { block_of, block_level: level, bunkers, ramp })
}
