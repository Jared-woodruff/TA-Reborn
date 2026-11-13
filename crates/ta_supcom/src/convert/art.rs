//! The map's art in TA's form: the ground painted ([`super::paint`]) as TA's camera saw it
//! (`ta_mapart::slant`), cut into 32-pixel tiles, the alike ones shared where the map has more
//! places than the budget allows tiles (`ta_mapart::tiles`, as TA Reborn's own maps are made),
//! and each tile dithered into TA's palette. Colours go to OKLab and back with
//! [`ta_mapart::tiles::Portable`]: the same tiles on every computer.

use rayon::prelude::*;
use ta_formats::tnt::TILE_SIZE;
use ta_mapart::field::Field;
use ta_mapart::tiles::{self, Desc, PaletteLut, Portable};

use super::land::Land;
use super::paint::Painter;

/// How many times the places choose their tiles and the tiles settle to the places.
const SETTLE_ROUNDS: usize = 4;
/// The share of the budget tiles of cliff may keep whole.
const WHOLE_SHARE: f32 = 0.3;

/// The art: its tiles (palette indices) and the tile each place shows.
pub struct Art {
    pub tiles: Vec<Box<[u8; 1024]>>,
    pub tile_map: Vec<u16>,
}

/// The ground's colours at a tile (sRGB, 0 to 1), from its strip's view columns.
fn tile_pixels(p: &Painter, land: &Land, cols: &[Vec<(f32, f32)>], tx: usize, tz: usize) -> Vec<[f32; 3]> {
    let f = land.frame;
    let mut px = Vec::with_capacity(1024);
    for j in 0..TILE_SIZE {
        let row = tz * TILE_SIZE + j;
        for (i, col) in cols.iter().enumerate() {
            let (z, _) = col[row];
            let (u, v) = f.world((tx * TILE_SIZE + i) as f64 + 0.5, z as f64);
            px.push(p.colour(u, v));
        }
    }
    px
}

/// The art of `land` painted by `p`, at most `budget` tiles; `progress` is told the share done.
pub fn paint(p: &Painter, land: &Land, budget: usize, palette: &[[u8; 3]; 256], progress: &(dyn Fn(f32) + Sync)) -> Art {
    let (w, h) = (land.frame.w, land.frame.h);
    let (tw, th) = (w / 2, h / 2);
    let rows = h * 16;
    // The heights as the renderer draws them, which the art is seen over.
    let field = Field { w, h, data: land.heights.iter().map(|&v| v as f32).collect() };
    let columns = |tx: usize| ta_mapart::slant::columns(|x, z| field.sample_tri(x / 16.0, z / 16.0), tx * TILE_SIZE, TILE_SIZE, rows);
    let lut = PaletteLut::new_in::<Portable>(*palette, |i| (16..240).contains(&i));
    let done = std::sync::atomic::AtomicUsize::new(0);
    let tick = |share_of: f32, base: f32| {
        let d = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        progress(base + share_of * d as f32 / tw as f32);
    };
    if tw * th <= budget {
        // Room for every place's own tile: each dithered as it is painted.
        let strips: Vec<Vec<Box<[u8; 1024]>>> = (0..tw).into_par_iter().map(|tx| {
            let cols = columns(tx);
            let strip = (0..th).map(|tz| Box::new(lut.dither(&tile_pixels(p, land, &cols, tx, tz)))).collect();
            tick(1.0, 0.0);
            strip
        }).collect();
        let mut tiles = vec![Box::new([0u8; 1024]); tw * th];
        for (tx, strip) in strips.into_iter().enumerate() {
            for (tz, t) in strip.into_iter().enumerate() {
                tiles[tz * tw + tx] = t;
            }
        }
        // (Places whose art came out the same share one tile.)
        return share_identical(tiles);
    }
    // More places than tiles: each place described, the alike shared.
    let cliff_share = |tx: usize, tz: usize| -> f32 {
        let mut n = 0;
        for dz in 0..2 {
            for dx in 0..2 {
                n += land.cliff[(tz * 2 + dz) * w + tx * 2 + dx] as u32;
            }
        }
        n as f32 / 4.0
    };
    let strips: Vec<Vec<(Desc, f32)>> = (0..tw).into_par_iter().map(|tx| {
        let cols = columns(tx);
        let strip = (0..th).map(|tz| (tiles::describe_in::<Portable>(&tile_pixels(p, land, &cols, tx, tz)), cliff_share(tx, tz))).collect();
        tick(0.6, 0.0);
        strip
    }).collect();
    let times = std::env::var_os("TA_SUPCOM_TIMES").is_some();
    let t0 = std::time::Instant::now();
    let mut descs = vec![Desc { mean: [0.0; 3], layout: [0.0; 48], edge: [0.0; 48], phase: 0 }; tw * th];
    let mut rock = vec![0.0f32; tw * th];
    for (tx, strip) in strips.into_iter().enumerate() {
        for (tz, (d, r)) in strip.into_iter().enumerate() {
            descs[tz * tw + tx] = d;
            rock[tz * tw + tx] = r;
        }
    }
    let (mut assign, reps, k) = tiles::group(&descs, &rock, budget, WHOLE_SHARE);
    if times {
        eprintln!("    grouped in {:.1}s", t0.elapsed().as_secs_f32());
    }
    // The tiles kept, painted again in full.
    let mut by_strip: Vec<Vec<(usize, usize)>> = vec![Vec::new(); tw];
    for (g, &t) in reps.iter().enumerate() {
        by_strip[t as usize % tw].push((t as usize / tw, g));
    }
    done.store(0, std::sync::atomic::Ordering::Relaxed);
    let repainted: Vec<Vec<(usize, Vec<[f32; 3]>)>> = by_strip.par_iter().enumerate().map(|(tx, list)| {
        let out = if list.is_empty() { Vec::new() } else {
            let cols = columns(tx);
            list.iter().map(|&(tz, g)| (g, tile_pixels(p, land, &cols, tx, tz))).collect()
        };
        tick(0.3, 0.6);
        out
    }).collect();
    let mut kept: Vec<Vec<[f32; 3]>> = vec![Vec::new(); reps.len()];
    for (g, px) in repainted.into_iter().flatten() {
        kept[g] = px;
    }
    if times {
        eprintln!("    repainted by {:.1}s", t0.elapsed().as_secs_f32());
    }
    let rock_shown: Vec<f32> = reps.iter().map(|&t| rock[t as usize]).collect();
    for _ in 0..SETTLE_ROUNDS {
        let shown: Vec<Desc> = kept.par_iter().zip(&reps).map(|(px, &t)| Desc { phase: descs[t as usize].phase, ..tiles::describe_in::<Portable>(px) }).collect();
        tiles::stitch(&descs, &shown, &rock, &rock_shown, &mut assign, tw, k);
        let deltas = tiles::settle(&descs, &shown, &assign);
        kept.par_iter_mut().zip(&deltas).for_each(|(px, d)| tiles::shift_in::<Portable>(px, d));
    }
    if times {
        eprintln!("    settled by {:.1}s", t0.elapsed().as_secs_f32());
    }
    progress(0.95);
    let order = tiles::compact(&mut assign, kept.len());
    let tiles: Vec<Box<[u8; 1024]>> = order.par_iter().map(|&g| Box::new(lut.dither(&kept[g as usize]))).collect();
    Art { tiles, tile_map: assign.iter().map(|&g| g as u16).collect() }
}

/// Tiles one to a place, the identical ones (sea, uniform ground) shared. (Boxed tiles: a map
/// holds its tiles so, `TntMap::tiles`.)
#[allow(clippy::vec_box)]
fn share_identical(tiles: Vec<Box<[u8; 1024]>>) -> Art {
    let mut index: std::collections::BTreeMap<&[u8; 1024], u16> = std::collections::BTreeMap::new();
    let mut tile_map = Vec::with_capacity(tiles.len());
    let mut order: Vec<usize> = Vec::new();
    for (i, t) in tiles.iter().enumerate() {
        let n = index.len() as u16;
        let slot = *index.entry(&**t).or_insert_with(|| {
            order.push(i);
            n
        });
        tile_map.push(slot);
    }
    drop(index);
    let mut tiles = tiles;
    let kept = order.into_iter().map(|i| std::mem::replace(&mut tiles[i], Box::new([0u8; 1024]))).collect();
    Art { tiles: kept, tile_map }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_tiles_are_kept_once() {
        let a = Box::new([1u8; 1024]);
        let b = Box::new([2u8; 1024]);
        let art = share_identical(vec![a.clone(), b.clone(), a.clone(), b.clone(), a]);
        assert_eq!(art.tiles.len(), 2);
        assert_eq!(art.tile_map, [0, 1, 0, 1, 0]);
        assert_eq!(*art.tiles[1], *b);
    }
}
