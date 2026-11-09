//! `ta-tools mapgen`: TA Reborn's own maps, grown from a seed. Original land, art and layout (no
//! tile, picture or height of TA's is used); the features are the game's own, named. The maps
//! come in every size from duels to wide battlefields, and are made to show what the renderer does
//! with relief, water, forests, sand and rock.
//!
//! A map is made in steps: the land ([`terrain`]), the tracks worn across it ([`tracks`]), what
//! stands on it ([`place`]), its art painted in TA's oblique view ([`paint`]), the art cut into
//! TA's 32-pixel tiles and the alike ones shared ([`tiles`]), then written as TA writes maps
//! (`TntMap::write`, an OTA). The maps there are, and how each is made: [`recipes`].

pub mod coasts;
pub mod decks;
pub mod field;
pub mod geo;
pub mod noise;
pub mod paint;
pub mod place;
#[cfg(test)]
mod qa;
pub mod recipes;
pub mod terrain;
pub mod tiles;
pub mod tracks;

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use ta_formats::image::Palette;
use ta_formats::tdf::{TdfDocument, TdfSection};
use ta_formats::tnt::{TILE_SIZE, TntMap};

pub use recipes::{IDS, Recipe, recipe};

use paint::Ground;
use tracks::Tracks;

/// How many times the places choose their tiles and the tiles settle to the places (see
/// [`tiles::stitch`], [`tiles::settle`]).
const SETTLE_ROUNDS: usize = 6;

/// How little a tile's layout may vary (the sum of its blocks' squared departures from its
/// mean colour, OKLab), and how close its mean colour must be to its neighbours' (lightness,
/// colour), for it to count as plain ground: ground of one look, its fine texture repeating, so
/// it shares only with tiles of its own phase, seamlessly. Tiles with more to them (an edge, a
/// crest, a feature, or a gradient across tiles, which the few tiles of one phase would cut into
/// steps) share with any alike tile, whatever its phase.
const PLAIN: f32 = 0.004;
const PLAIN_STEP: [f32; 2] = [0.02, 0.015];

/// Makes map `id` into `out` (a folder holding `maps/`), with pictures of it in `preview`.
pub fn run(id: &str, out: &Path, preview: Option<&Path>, palette: &Palette) -> Result<()> {
    let Some(r) = recipe(id) else { bail!("no map {id:?}: there are {}", IDS.join(", ")) };
    let t0 = Instant::now();
    let seed = r.shape.seed;
    eprintln!("{}: land", r.name);
    let mut land = terrain::build(&r.shape);
    // The heights as the map stores them (whole units), which everything after follows.
    land.height = land.height.map(|h| h.round().clamp(0.0, 255.0));
    let (w, h) = (land.w, land.h);
    eprintln!("  {w} x {h} cells ({} x {} px), starts {:?}  [{:.1}s]", w * 16, h * 16, land.starts.iter().map(|s| (s.0 as u32 * 16, s.1 as u32 * 16)).collect::<Vec<_>>(), t0.elapsed().as_secs_f32());
    let mut forest = place::forest_field(&land, &r.kit, seed);
    // (A real place's forests where its biomes have them, their edges as short as the forests'
    // own: a forest thinned evenly over a wide stretch painted it in a gradient the tiles cut
    // into squares.)
    if let Some(share) = geo::forest_share(&land) {
        for (f, k) in forest.data.iter_mut().zip(&share.data) {
            *f *= noise::smoothstep(0.3, 0.42, *k);
        }
    }
    let tracks = if r.tracks { tracks::tracks(&land, &forest, seed) } else { Tracks::none(w, h) };
    // What features keep clear of: the tracks, and on maps that ask for it a way a tank drives
    // between the bases.
    let avoid = if r.kit.corridor {
        let way = terrain::corridor(&land.height, land.sea, &land.starts, (!land.void.is_empty()).then_some(&land.void[..]));
        let mut d = if r.tracks { tracks.dist.clone() } else { field::Field::new(w, h, tracks::FAR) };
        for (i, &c) in way.iter().enumerate() {
            if c {
                d.data[i] = 0.0;
                // (And its twin: the features' twins keep clear of it too.)
                if land.sym.on {
                    d.data[land.sym.twin_index(i)] = 0.0;
                }
            }
        }
        Some(d)
    } else {
        r.tracks.then(|| tracks.dist.clone())
    };
    let mut placed = place::place(&land, &r.kit, forest, avoid.as_ref(), seed);
    if let Some(earth) = &r.kit.earth {
        place::dress(&mut placed, &land, &r.kit, earth);
    }
    let earth_names = |pick: fn(&place::EarthKit) -> Vec<&'static str>| r.kit.earth.as_ref().map(pick).unwrap_or_default();
    let (more_metal, more_vents) = (earth_names(|e| e.metal()), earth_names(|e| e.vents()));
    eprintln!("  {} features of {} kinds  [{:.1}s]", placed.count, placed.names.len(), t0.elapsed().as_secs_f32());
    // How much metal each base has to hand (deposits within 40 cells), and the map in all.
    let deposits: Vec<(f32, f32)> = placed
        .cells
        .iter()
        .enumerate()
        .filter(|&(_, &f)| (f as usize) < placed.names.len() && r.kit.metal.iter().chain(&more_metal).any(|m| m.eq_ignore_ascii_case(&placed.names[f as usize])))
        .map(|(i, _)| ((i % w) as f32 + 1.5, (i / w) as f32 + 1.5))
        .collect();
    let near: Vec<usize> = land.starts.iter().map(|s| deposits.iter().filter(|d| (d.0 - s.0).hypot(d.1 - s.1) < 40.0).count()).collect();
    eprintln!("  metal: {} deposits, by the bases {near:?}", deposits.len());
    // A land map plays only if a tank can drive from every base to every other, round the
    // forests and the rocks as well as the cliffs.
    if r.shape.connect {
        let blocked: Vec<bool> = if land.void.is_empty() { placed.blocked.clone() } else { placed.blocked.iter().zip(&land.void).map(|(&b, &v)| b || v).collect() };
        let reach = terrain::reachable(&land.height, land.sea, &land.starts, Some(&blocked));
        if let Some(k) = reach.iter().position(|&r| !r) {
            // (`MAPGEN_NOCHECK=1` writes it all the same, to look at.)
            if std::env::var("MAPGEN_NOCHECK").is_ok_and(|v| v == "1") {
                eprintln!("  NO WAY for a tank from start 1 to start {} ({:?}): written anyway", k + 1, land.starts[k]);
                // What stands in the way a tank would take over the ground alone.
                let way = terrain::corridor(&land.height, land.sea, &[land.starts[0], land.starts[k]], (!land.void.is_empty()).then_some(&land.void[..]));
                let mut seen = std::collections::BTreeMap::<String, usize>::new();
                for (i, &c) in way.iter().enumerate() {
                    if c && placed.blocked[i] {
                        let (x, z) = (i % w, i / w);
                        // (The feature whose footprint covers it: look up and left a little.)
                        'find: for dz in 0..12 {
                            for dx in 0..12 {
                                if x >= dx && z >= dz {
                                    let f = placed.cells[(z - dz) * w + x - dx];
                                    if (f as usize) < placed.names.len() {
                                        *seen.entry(format!("{} at ({}, {})", placed.names[f as usize], x - dx, z - dz)).or_default() += 1;
                                        break 'find;
                                    }
                                }
                            }
                        }
                    }
                }
                eprintln!("  on the way: {seen:?}");
            } else {
                bail!("{}: no way for a tank from start 1 to start {} ({:?})", r.name, k + 1, land.starts[k]);
            }
        }
    }
    // Two coasts across an ocean: a tank drives from every base on a coast to every other there,
    // round the features too (across the ocean, by sea and air only).
    if land.halves_apart {
        for west in [true, false] {
            let mine: Vec<(f32, f32)> = land.starts.iter().copied().filter(|&(x, z)| land.sym.first(x, z) == west).collect();
            if mine.len() < 2 {
                continue;
            }
            let reach = terrain::reachable(&land.height, land.sea, &mine, Some(&placed.blocked));
            if let Some(k) = reach.iter().position(|&r| !r) {
                if std::env::var("MAPGEN_NOCHECK").is_ok_and(|v| v == "1") {
                    eprintln!("  NO WAY for a tank between the bases at {:?} and {:?}: written anyway", mine[0], mine[k]);
                } else {
                    bail!("{}: no way for a tank between the bases at {:?} and {:?} on their coast", r.name, mine[0], mine[k]);
                }
            }
        }
    }
    // How far each cell lies from a vent (the painter crusts the ground round them).
    let vents: Vec<bool> = placed.cells.iter().map(|&f| (f as usize) < placed.names.len() && r.kit.vents.iter().chain(&more_vents).any(|v| v.eq_ignore_ascii_case(&placed.names[f as usize]))).collect();
    let hot = if vents.iter().any(|&v| v) { field::distance(w, h, &vents) } else { field::Field::new(w, h, 1e6) };
    let mut ground = Ground::new(&land, &placed.forest, &tracks, &hot, r.paint.clone(), seed);
    ground.squares.clone_from(&placed.squares);
    // The art, tile by tile: first each tile's description, to group them.
    let (tw, th) = (w / 2, h / 2);
    let rows = h * 16;
    let strips: Vec<Vec<(tiles::Desc, f32)>> = (0..tw)
        .into_par_iter()
        .map(|tx| {
            let cols = paint::view_columns(&ground, tx * TILE_SIZE, TILE_SIZE, rows);
            (0..th).map(|tz| (tiles::describe(&tile_pixels(&ground, &cols, tx, tz)), tile_rock(&ground, &cols, tx, tz))).collect()
        })
        .collect();
    let mut descs = vec![tiles::Desc { mean: [0.0; 3], layout: [0.0; 48], edge: [0.0; 48], phase: 0 }; tw * th];
    let mut rock = vec![0.0f32; tw * th];
    // (Where each place lies in the repeat of the art's fine texture: only tiles of one phase
    // share. The first maps' art does not repeat.)
    let period = if r.paint.periodic() { (noise::PERIOD as usize / TILE_SIZE).max(1) } else { 1 };
    for (tx, strip) in strips.into_iter().enumerate() {
        for (tz, (d, rk)) in strip.into_iter().enumerate() {
            descs[tz * tw + tx] = d;
            rock[tz * tw + tx] = rk;
        }
    }
    if period > 1 {
        let phases: Vec<u16> = (0..tw * th)
            .into_par_iter()
            .map(|i| {
                let (tx, tz) = (i % tw, i / tw);
                let d = &descs[i];
                let energy: f32 = d.layout.iter().map(|v| v * v).sum();
                let even = [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)].iter().all(|&(dx, dz)| {
                    let (nx, nz) = (tx as i64 + dx, tz as i64 + dz);
                    if nx < 0 || nz < 0 || nx >= tw as i64 || nz >= th as i64 {
                        return true;
                    }
                    let n = &descs[nz as usize * tw + nx as usize];
                    (d.mean[0] - n.mean[0]).abs() < PLAIN_STEP[0] && (d.mean[1] - n.mean[1]).abs().max((d.mean[2] - n.mean[2]).abs()) < PLAIN_STEP[1]
                });
                if energy < PLAIN && even { 1 + ((tz % period) * period + tx % period) as u16 } else { 0 }
            })
            .collect();
        let plain = phases.iter().filter(|&&p| p > 0).count();
        for (d, p) in descs.iter_mut().zip(phases) {
            d.phase = p;
        }
        eprintln!("  plain ground: {:.0}% of the places", plain as f32 * 100.0 / (tw * th) as f32);
    }
    eprintln!("  painted {} tiles  [{:.1}s]", tw * th, t0.elapsed().as_secs_f32());
    let (mut assign, reps, k) = tiles::group(&descs, &rock, r.budget, r.whole_rock);
    eprintln!("  grouped  [{:.1}s]", t0.elapsed().as_secs_f32());
    // Then the tiles kept, painted again in full.
    let mut by_strip: Vec<Vec<(usize, usize)>> = vec![Vec::new(); tw];
    for (g, &t) in reps.iter().enumerate() {
        let t = t as usize;
        by_strip[t % tw].push((t / tw, g));
    }
    let done: Vec<Vec<(usize, Vec<[f32; 3]>)>> = by_strip
        .par_iter()
        .enumerate()
        .map(|(tx, list)| {
            if list.is_empty() {
                return Vec::new();
            }
            let cols = paint::view_columns(&ground, tx * TILE_SIZE, TILE_SIZE, rows);
            list.iter().map(|&(tz, g)| (g, tile_pixels(&ground, &cols, tx, tz))).collect()
        })
        .collect();
    let mut kept: Vec<Vec<[f32; 3]>> = vec![Vec::new(); reps.len()];
    for (g, px) in done.into_iter().flatten() {
        kept[g] = px;
    }
    // Every place takes the tile that meets its neighbours' as its painting does, and each tile's
    // edges settle to the middle of the places showing it; a few times over.
    // (How much rock in strata each tile kept shows: no place takes one with much more than its own.)
    let rock_shown: Vec<f32> = reps.iter().map(|&t| rock[t as usize]).collect();
    for _ in 0..SETTLE_ROUNDS {
        let shown: Vec<tiles::Desc> = kept.par_iter().zip(&reps).map(|(px, &t)| tiles::Desc { phase: descs[t as usize].phase, ..tiles::describe(px) }).collect();
        let changed = tiles::stitch(&descs, &shown, &rock, &rock_shown, &mut assign, tw, k);
        let deltas = tiles::settle(&descs, &shown, &assign);
        kept.par_iter_mut().zip(&deltas).for_each(|(px, d)| tiles::shift(px, d));
        eprintln!("  stitched: {changed} places took another tile  [{:.1}s]", t0.elapsed().as_secs_f32());
    }
    drop(descs);
    let order = tiles::compact(&mut assign, kept.len());
    // Dithered into the palette.
    let lut = tiles::PaletteLut::new(palette.colors, |i| (16..240).contains(&i));
    let tile_art: Vec<Box<[u8; 1024]>> = order.par_iter().map(|&g| Box::new(lut.dither(&kept[g as usize]))).collect();
    eprintln!("  dithered {} tiles  [{:.1}s]", tile_art.len(), t0.elapsed().as_secs_f32());
    // In space the void's cells are TA's void (nothing there, aircraft cross it).
    let mut features = placed.cells.clone();
    for (f, &v) in features.iter_mut().zip(&land.void) {
        if v && *f == 0xFFFF {
            *f = ta_formats::tnt::FEATURE_VOID;
        }
    }
    let mut map = TntMap {
        width: w as u32,
        height: h as u32,
        sea_level: land.sea as i32,
        tiles: tile_art,
        tile_map: assign.iter().map(|&g| g as u16).collect(),
        heights: land.height.data.iter().map(|&h| h as u8).collect(),
        features,
        feature_names: placed.names.clone(),
        minimap: Default::default(),
    };
    map.minimap = map.build_minimap(palette);
    let [bl, ba, bb] = tiles::blockiness(&map.tile_map, &map.tiles, &palette.colors, tw);
    eprintln!("  steps at tile edges over steps within tiles: L {bl:.2}, a {ba:.2}, b {bb:.2}");
    let dir = out.join("maps");
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let tnt = map.write()?;
    std::fs::write(dir.join(format!("{}.tnt", r.name)), &tnt)?;
    std::fs::write(dir.join(format!("{}.ota", r.name)), ota(&r, &land).to_bytes())?;
    eprintln!("  wrote {}.tnt ({:.1} MB) and .ota  [{:.1}s]", r.name, tnt.len() as f32 / 1e6, t0.elapsed().as_secs_f32());
    if let Some(p) = preview {
        write_previews(p, &r, &map, palette, &land, &tracks)?;
        // The same stretch as `_art_detail`, as painted (before its tiles were shared), to tell
        // the painting's faults from the sharing's.
        let (dw, dh) = (2048.min(w * 16), 1280.min(h * 16));
        let (ox, oy) = detail_origin(w * 16, h * 16, dw, dh);
        let (tx0, tx1) = (ox / TILE_SIZE, (ox + dw).div_ceil(TILE_SIZE));
        let cols: Vec<Vec<Vec<(f32, f32)>>> = (tx0..tx1).into_par_iter().map(|tx| paint::view_columns(&ground, tx * TILE_SIZE, TILE_SIZE, rows)).collect();
        let mut img = image::RgbImage::new(dw as u32, dh as u32);
        for y in 0..dh {
            for x in 0..dw {
                let (px, row) = (ox + x, oy + y);
                let (tx, i) = (px / TILE_SIZE - tx0, px % TILE_SIZE);
                let (z, hh) = cols[tx][i][row];
                let c = ground.color(px as f32 + 0.5, z, hh, row as f32 + 0.5);
                img.put_pixel(x as u32, y as u32, image::Rgb(c.map(|v| (v * 255.0).round() as u8)));
            }
        }
        img.save(p.join(format!("{}_painted_detail.png", r.name.replace(' ', "_"))))?;
    }
    Ok(())
}

/// Where the detail previews' stretch of art starts (pixels): about the middle, or about
/// `MAPGEN_DETAIL=x,y` (art pixels).
fn detail_origin(w: usize, h: usize, dw: usize, dh: usize) -> (usize, usize) {
    let at = std::env::var("MAPGEN_DETAIL").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<usize>().ok()?, y.trim().parse::<usize>().ok()?))
    });
    let (cx, cy) = at.unwrap_or((w / 2, h / 2));
    (cx.saturating_sub(dw / 2).min(w - dw), cy.saturating_sub(dh / 2).min(h - dh))
}

/// How much of a tile is painted as rock in strata (0..1), from the strip's view columns
/// (sampled every 4 pixels); a tile a track crosses counts as more than half rock (kept whole
/// before all but the rockiest, and never shown where the painting has no such line).
fn tile_rock(ground: &Ground, cols: &[Vec<(f32, f32)>], tx: usize, tz: usize) -> f32 {
    let (mut sum, mut track) = (0.0, 0.0);
    for j in (0..TILE_SIZE).step_by(4) {
        let row = tz * TILE_SIZE + j;
        for (i, col) in cols.iter().enumerate().step_by(4) {
            let (z, h) = col[row];
            let x = (tx * TILE_SIZE + i) as f32 + 0.5;
            sum += ground.rock(x, z, h);
            if ground.track(x, z) < 0.7 {
                track += 1.0;
            }
        }
    }
    let rock = sum / 64.0;
    if track > 0.0 { rock.max(0.62 + track / 64.0 * 0.3) } else { rock }
}

/// A tile's colours (sRGB, 0 to 1), from the strip's view columns.
fn tile_pixels(ground: &Ground, cols: &[Vec<(f32, f32)>], tx: usize, tz: usize) -> Vec<[f32; 3]> {
    let mut px = Vec::with_capacity(1024);
    for j in 0..TILE_SIZE {
        let row = tz * TILE_SIZE + j;
        for (i, col) in cols.iter().enumerate() {
            let (z, h) = col[row];
            px.push(ground.color((tx * TILE_SIZE + i) as f32 + 0.5, z, h, row as f32 + 0.5));
        }
    }
    px
}

/// The map's OTA, laid out as TA's skirmish maps' are.
fn ota(r: &Recipe, land: &terrain::Land) -> TdfDocument {
    let mut head = TdfSection::new("GlobalHeader");
    let players = land.starts.len();
    let counts: Vec<String> = (2..=players).step_by(2).map(|p| p.to_string()).collect();
    for (k, v) in [
        ("missionname", r.name.to_string()),
        ("missiondescription", r.description.clone()),
        ("planet", r.planet.to_string()),
        ("missionhint", String::new()),
        ("brief", String::new()),
        ("narration", String::new()),
        ("glamour", String::new()),
        ("lineofsight", "1".into()),
        ("mapping", "1".into()),
        ("tidalstrength", r.tidal.to_string()),
        ("solarstrength", r.solar.to_string()),
        ("lavaworld", if r.paint.molten() { "1" } else { "0" }.into()),
        ("killmul", "50".into()),
        ("timemul", "0".into()),
        ("minwindspeed", r.wind.0.to_string()),
        ("maxwindspeed", r.wind.1.to_string()),
        ("gravity", r.gravity.to_string()),
        ("numplayers", counts.join(", ")),
        ("size", format!("{} x {}", land.w * 16 / 512, land.h * 16 / 512)),
        ("memory", "32 mb".into()),
        ("DestroyAllUnits", "1".into()),
        ("AllUnitsKilled", "1".into()),
        ("SCHEMACOUNT", "1".into()),
    ] {
        head.set(k, v);
    }
    let mut schema = TdfSection::new("Schema 0");
    for (k, v) in [
        ("Type", "Network 1".to_string()),
        ("aiprofile", "DEFAULT".into()),
        ("SurfaceMetal", r.surface_metal.to_string()),
        ("MohoMetal", r.moho_metal.to_string()),
        ("HumanMetal", "1000".into()),
        ("ComputerMetal", "1000".into()),
        ("HumanEnergy", "1000".into()),
        ("ComputerEnergy", "1000".into()),
        ("MeteorWeapon", String::new()),
        ("MeteorRadius", "0".into()),
        ("MeteorDensity", "0".into()),
        ("MeteorDuration", "0".into()),
        ("MeteorInterval", "0".into()),
    ] {
        schema.set(k, v);
    }
    let mut specials = TdfSection::new("specials");
    for (k, &(x, z)) in land.starts.iter().enumerate() {
        let mut s = TdfSection::new(format!("special{k}"));
        s.set("specialwhat", format!("StartPos{}", k + 1));
        s.set("XPos", (x as u32 * 16 + 8).to_string());
        s.set("ZPos", (z as u32 * 16 + 8).to_string());
        specials.sections.push(s);
    }
    schema.sections.push(specials);
    head.sections.push(schema);
    TdfDocument { sections: vec![head] }
}

/// Pictures to judge a map by: the art (a pixel every 8), a stretch of it at full size, the
/// heights, the features and tracks.
fn write_previews(dir: &Path, r: &Recipe, map: &TntMap, palette: &Palette, land: &terrain::Land, tracks: &Tracks) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let (w, h) = (map.width_px() as usize, map.height_px() as usize);
    let tw = w / TILE_SIZE;
    let px = |x: usize, y: usize| -> [u8; 3] {
        let t = map.tile_map[(y / TILE_SIZE) * tw + x / TILE_SIZE] as usize;
        palette.colors[map.tiles[t][(y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE] as usize]
    };
    let stem = r.name.replace(' ', "_");
    let scale = if w.max(h) > 12_000 { 8 } else { 4 };
    let (pw, ph) = (w / scale, h / scale);
    let mut img = image::RgbImage::new(pw as u32, ph as u32);
    for y in 0..ph {
        for x in 0..pw {
            // (The mean of the pixels it stands for: the dither's whole pattern.)
            let mut acc = [0u32; 3];
            let mut n = 0;
            for dy in 0..scale {
                for dx in 0..scale {
                    let c = px(x * scale + dx, y * scale + dy);
                    for k in 0..3 {
                        acc[k] += c[k] as u32;
                    }
                    n += 1;
                }
            }
            img.put_pixel(x as u32, y as u32, image::Rgb(acc.map(|v| (v / n) as u8)));
        }
    }
    img.save(dir.join(format!("{stem}_art.png")))?;
    let (dw, dh) = (2048.min(w), 1280.min(h));
    let (ox, oy) = detail_origin(w, h, dw, dh);
    let mut img = image::RgbImage::new(dw as u32, dh as u32);
    for y in 0..dh {
        for x in 0..dw {
            img.put_pixel(x as u32, y as u32, image::Rgb(px(ox + x, oy + y)));
        }
    }
    img.save(dir.join(format!("{stem}_art_detail.png")))?;
    let (cw, ch) = (land.w, land.h);
    let mut img = image::RgbImage::new(cw as u32, ch as u32);
    for z in 0..ch {
        for x in 0..cw {
            let v = land.height.at(x, z);
            // (The water in the bands that matter to what crosses it: wading (a tank, 12 deep at
            // most), shallow (a light ship), 15 deep (battleships) and 30 (a shipyard).)
            let depth = land.sea - v;
            let mut c = if depth > 0.0 {
                if depth <= 12.0 {
                    [90, 160, 210]
                } else if depth < 15.0 {
                    [50, 110, 190]
                } else if depth < 30.0 {
                    [30, 60, 150]
                } else {
                    [12, 24, 92]
                }
            } else {
                [v as u8, v as u8, v as u8]
            };
            if tracks.dist.at(x, z) < 0.7 {
                c = [200, 120, 60];
            }
            let f = map.features[z * cw + x];
            let c = if f != 0xFFFF && (f as usize) < map.feature_names.len() {
                let name = map.feature_names[f as usize].to_ascii_lowercase();
                if name.contains("metal") { [255, 220, 0] } else if name.contains("tree") || name.contains("palm") || name.contains("lush") { [30, 140, 40] } else if name.contains("vent") || name.contains("geo") { [255, 60, 30] } else { [150, 110, 80] }
            } else {
                c
            };
            img.put_pixel(x as u32, z as u32, image::Rgb(c));
        }
    }
    for &(sx, sz) in &land.starts {
        for d in -6i32..=6 {
            for (x, z) in [(sx as i32 + d, sz as i32), (sx as i32, sz as i32 + d)] {
                if x >= 0 && z >= 0 && (x as usize) < cw && (z as usize) < ch {
                    img.put_pixel(x as u32, z as u32, image::Rgb([255, 0, 255]));
                }
            }
        }
    }
    img.save(dir.join(format!("{stem}_layout.png")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use ta_formats::tdf;
    use ta_formats::tnt::TntMap;

    /// The maps as committed (`assets/reborn/maps`): each there, whole, with its starts on dry,
    /// level ground and its features ones the game has (when the install is there to say).
    #[test]
    fn the_built_in_maps_are_whole() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn/maps");
        let features = ta_vfs::install::discover(None).ok().map(|install| {
            let mut vfs = ta_vfs::Vfs::new();
            vfs.mount_game_dir(&install.root, "base").unwrap();
            // (And TA Reborn's own features, the festive ones of the winter maps.)
            vfs.mount_builtin();
            ta_data::GameData::load(&vfs).features.iter().map(|f| f.name.to_ascii_lowercase()).collect::<std::collections::HashSet<_>>()
        });
        for id in super::IDS {
            let r = super::recipe(id).unwrap();
            let tnt = std::fs::read(dir.join(format!("{}.tnt", r.name))).unwrap_or_else(|e| panic!("{}: {e}", r.name));
            let map = TntMap::parse(&tnt).unwrap();
            assert_eq!((map.width as usize, map.height as usize), (r.shape.w, r.shape.h), "{}", r.name);
            assert!(map.tiles.len() <= r.budget, "{}: {} tiles", r.name, map.tiles.len());
            let ota = tdf::parse_bytes(&std::fs::read(dir.join(format!("{}.ota", r.name))).unwrap());
            let info = ta_data::MapInfo::from_ota(&ota);
            let starts = &info.schemas[0].start_positions;
            assert_eq!(starts.len(), r.shape.players, "{}", r.name);
            for &(_, x, z) in starts {
                let s = (x, z);
                let (cx, cz) = ((x / 16.0) as u32, (z / 16.0) as u32);
                let h = map.height_at(cx, cz) as i32;
                assert!(h > map.sea_level + 4, "{}: start {s:?} is at {h}, the sea at {}", r.name, map.sea_level);
                let around: Vec<i32> = (0..9).map(|k| map.height_at(cx + k % 3 * 4 - 4, cz + k / 3 * 4 - 4) as i32).collect();
                assert!(around.iter().max().unwrap() - around.iter().min().unwrap() <= 8, "{}: start {s:?} is not level: {around:?}", r.name);
            }
            if let Some(known) = &features {
                for name in &map.feature_names {
                    assert!(known.contains(&name.to_ascii_lowercase()), "{}: no feature {name}", r.name);
                }
            }
        }
    }

    /// The maps as committed are the generator's own: each map's land (every cell's height) and
    /// its OTA (starts, weather, description) as its recipe makes them now. A change to the
    /// generator that moves a map's land fails here until that map is made again
    /// (`ta-tools mapgen <id> assets/reborn`, docs/MAPS.md): on 2026-11-07 the harbour channels
    /// were deepened from 24 to 30 for the submarines, and four of the maps with harbours were left
    /// as they were. Checked where the maps are made, on Windows x64 with the install (not on the build
    /// machines, whose maths libraries may round a cell differently), and the land alone: the
    /// painting is slow (`mapgen all` takes ten minutes).
    #[test]
    #[cfg_attr(not(all(windows, target_arch = "x86_64")), ignore = "the maps are made on Windows x64; another maths library may round a cell differently")]
    fn the_built_in_maps_are_the_generators() {
        use rayon::prelude::*;
        if ta_vfs::install::discover(None).is_err() {
            eprintln!("TA install not found; skipping");
            return;
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn/maps");
        let stale: Vec<String> = super::IDS
            .par_iter()
            .filter_map(|id| {
                let r = super::recipe(id).unwrap();
                let land = super::terrain::build(&r.shape);
                let map = TntMap::parse(&std::fs::read(dir.join(format!("{}.tnt", r.name))).unwrap()).unwrap();
                // (Heights as `run` stores them: whole units.)
                let moved = land.height.data.iter().zip(&map.heights).filter(|&(&h, &stored)| h.round().clamp(0.0, 255.0) as u8 != stored).count();
                let ota = super::ota(&r, &land).to_bytes() == std::fs::read(dir.join(format!("{}.ota", r.name))).unwrap();
                (moved > 0 || !ota).then(|| format!("{} ({id}): {moved} cells' heights differ{}", r.name, if ota { "" } else { ", and its OTA" }))
            })
            .collect();
        assert!(stale.is_empty(), "maps no longer as the generator makes them; make them again with `ta-tools mapgen <id> assets/reborn` (docs/MAPS.md):\n{}", stale.join("\n"));
    }

    /// The symmetric maps as committed: each half the other turned round the middle, heights and
    /// features (a feature's footprint turned with it), so no start is better placed.
    #[test]
    fn the_symmetric_maps_are_fair() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn/maps");
        for id in super::IDS {
            let r = super::recipe(id).unwrap();
            if !r.shape.symmetric {
                continue;
            }
            let map = TntMap::parse(&std::fs::read(dir.join(format!("{}.tnt", r.name))).unwrap()).unwrap();
            let (w, h) = (map.width as usize, map.height as usize);
            for z in 0..h {
                for x in 0..w {
                    assert_eq!(map.heights[z * w + x], map.heights[(h - 1 - z) * w + (w - 1 - x)], "{}: ({x}, {z})", r.name);
                }
            }
            let mut count = std::collections::HashMap::<u16, usize>::new();
            for &f in &map.features {
                if (f as usize) < map.feature_names.len() {
                    *count.entry(f).or_default() += 1;
                }
            }
            // (The middle village's centrepiece, its footprint centred between the halves, is its
            // own twin.)
            if let Some((name, fw, fz)) = r.kit.village.as_ref().filter(|v| v.middle).map(|v| v.centre) {
                let at = ((h - fz) / 2) * w + (w - fw) / 2;
                let slot = map.features[at];
                assert!(map.feature_names.get(slot as usize).is_some_and(|n| n.eq_ignore_ascii_case(name)), "{}: no {name} in the middle", r.name);
                *count.get_mut(&slot).unwrap() -= 1;
            }
            assert!(count.values().all(|&c| c % 2 == 0), "{}: features without twins {count:?}", r.name);
        }
    }

    /// How TA's own maps measure by [`super::tiles::blockiness`] (`TA_MAPS=a,b` for others):
    /// `cargo test --release -p ta_tools tas_own_blockiness -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn tas_own_blockiness() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = ta_data::GameData::load(&vfs).palette;
        let names = std::env::var("TA_MAPS").unwrap_or_else(|_| "Metal Heck,Core Prime Industrial Area,Metal Gridlock,Metal Maze,Metal Isles,Coast To Coast,Full Moon,Dark Side".into());
        for name in names.split(',') {
            let Ok(data) = vfs.read(&format!("maps/{}.tnt", name.trim())) else {
                eprintln!("{name}: not found");
                continue;
            };
            let map = TntMap::parse(&data).unwrap();
            let steps = super::tiles::blockiness(&map.tile_map, &map.tiles, &palette.colors, map.width as usize / 2);
            eprintln!("{name}: {steps:?} ({} tiles)", map.tiles.len());
        }
    }

    /// The committed maps' tiles meet as their painting runs: the steps in colour across tile
    /// edges at most a third bigger than those inside the tiles (TA's own maps measure 0.92 to
    /// 1.12; tiles shared without regard to their neighbours measured 1.3 to 2.3). With the
    /// install's palette, when it is there.
    #[test]
    fn the_built_in_maps_show_no_tile_grid() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = ta_data::GameData::load(&vfs).palette;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn/maps");
        for id in super::IDS {
            let r = super::recipe(id).unwrap();
            let map = TntMap::parse(&std::fs::read(dir.join(format!("{}.tnt", r.name))).unwrap()).unwrap();
            let steps = super::tiles::blockiness(&map.tile_map, &map.tiles, &palette.colors, map.width as usize / 2);
            assert!(steps.iter().all(|&s| s <= 1.33), "{}: tile edges step {steps:?} times as much as inside the tiles", r.name);
        }
    }
}
