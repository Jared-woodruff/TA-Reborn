//! `ta-tools supcom`: Supreme Commander: Forged Alliance's maps made into TA maps, as the launcher
//! makes them, into a folder holding `maps/` (to mount as a mod, or look at), with pictures to
//! judge them by: the art, and the game's own preview of the map beside it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use ta_formats::image::Palette;
use ta_formats::tnt::{TILE_SIZE, TntMap};
use ta_supcom::convert::{self, Options, Stage, kit::Templates};
use ta_supcom::files::Files;
use ta_supcom::install::{self, Install, Store};
use ta_supcom::map::{self, Map};

pub fn run(out: &Path, only: Option<&str>, game: Option<&Path>, preview: Option<&Path>, budget: usize, max_cells: usize) -> Result<()> {
    // TA's own features and palette: the original game alone, as every player has it.
    let ta = ta_vfs::install::discover(None)?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&ta.root, "base")?;
    let palette = Palette::parse(&vfs.read("palettes/palette.pal")?)?;
    let templates = Templates::load(&vfs);
    let install = match game {
        Some(g) => Install { root: install::validate_picked(g).context("no copy of Forged Alliance there")?, store: Store::Folder },
        None => install::discover().into_iter().next().context("no copy of Forged Alliance found (give its folder with --game)")?,
    };
    eprintln!("Forged Alliance: {} ({:?})", install.root.display(), install.store);
    let user = install::user_map_folders();
    let files = Files::open(&install, &user)?;
    let wanted: Option<Vec<String>> = only.map(|o| o.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect());
    let mut folders: Vec<(PathBuf, bool)> = Vec::new();
    for dir in files.map_dirs() {
        let players_own = user.iter().any(|u| u == dir);
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && map::scenario_file(p).is_some()).collect();
        found.sort();
        folders.extend(found.into_iter().map(|f| (f, players_own)));
    }
    let maps_out = out.join("maps");
    std::fs::create_dir_all(&maps_out)?;
    if let Some(p) = preview {
        std::fs::create_dir_all(p)?;
    }
    let options = Options { budget, max_cells };
    let mut failed = 0;
    for (folder, players_own) in folders {
        let name = folder.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let scenario = match map::read_scenario(&folder) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{name}: {e}");
                continue;
            }
        };
        match &wanted {
            Some(w) if !w.iter().any(|x| *x == name || scenario.name.eq_ignore_ascii_case(x)) => continue,
            None if !scenario.skirmish() => continue,
            _ => {}
        }
        let t = Instant::now();
        let result = Map::read(&files, &folder).map_err(anyhow::Error::from).and_then(|m| {
            // How long each stage took (`TA_SUPCOM_TIMES=1` prints them).
            let marks = std::sync::Mutex::new(vec![("read", t.elapsed().as_secs_f32())]);
            let c = convert::convert(&m, &files, &templates, &palette, players_own, &options, &|s| {
                let name = match s { Stage::Land => "land", Stage::Features => "features", Stage::Painting(_) => "painting", Stage::Writing => "writing" };
                let mut marks = marks.lock().unwrap();
                if marks.last().is_none_or(|(n, _)| *n != name) {
                    marks.push((name, t.elapsed().as_secs_f32()));
                }
            })?;
            if std::env::var_os("TA_SUPCOM_TIMES").is_some() {
                let marks = marks.into_inner().unwrap();
                eprintln!("  {}", marks.iter().map(|(n, at)| format!("{n} {at:.1}s")).collect::<Vec<_>>().join(", "));
            }
            Ok((m, c))
        });
        match result {
            Ok((m, c)) => {
                std::fs::write(maps_out.join(format!("{}.tnt", c.stem)), &c.tnt)?;
                std::fs::write(maps_out.join(format!("{}.ota", c.stem)), &c.ota)?;
                if let Some(f) = &c.features {
                    std::fs::write(maps_out.join(format!("{}.features", c.stem)), f)?;
                }
                let k = &c.counts;
                // (How much the tiles step at their edges against inside them: TA Reborn's own maps
                // are held to 1.33.)
                let tnt = TntMap::parse(&c.tnt)?;
                let [bl, ba, bb] = ta_mapart::tiles::blockiness(&tnt.tile_map, &tnt.tiles, &palette.colors, tnt.width as usize / 2);
                eprintln!(
                    "{:<10} {:<34} {:>4}x{:<4} {:<7} tiles {:>5} steps {bl:.2} {ba:.2} {bb:.2} players {} deposits {:>3} vents {} rocks {:>4} trees {:>5} walls {:>3} opened {:>4}{} {:.1} MB [{:.1}s]",
                    m.scenario.folder, c.stem, c.cells.0, c.cells.1, c.kit, c.tiles, c.players, k.deposits, k.vents, k.rocks, k.trees, k.walls, k.opened,
                    if c.cut_off.is_empty() { String::new() } else { format!(" CUT OFF {:?}", c.cut_off) },
                    c.tnt.len() as f64 / 1e6, t.elapsed().as_secs_f32()
                );
                if let Some(p) = preview {
                    pictures(p, &c.stem, &c.tnt, &m, &palette)?;
                }
            }
            Err(e) => {
                eprintln!("{name}: {e:#}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        anyhow::bail!("{failed} maps did not convert");
    }
    Ok(())
}

/// The map's art (at most 2048 pixels across), and the game's own preview of it beside, to
/// compare.
fn pictures(dir: &Path, stem: &str, tnt: &[u8], m: &Map, palette: &Palette) -> Result<()> {
    let map = TntMap::parse(tnt)?;
    let (w, h) = (map.width_px() as usize, map.height_px() as usize);
    let tw = w / TILE_SIZE;
    let step = w.max(h).div_ceil(2048).max(1);
    let (ow, oh) = (w / step, h / step);
    let mut img = image::RgbImage::new(ow as u32, oh as u32);
    for y in 0..oh {
        for x in 0..ow {
            let mut acc = [0u32; 3];
            for dy in 0..step {
                for dx in 0..step {
                    let (px, py) = (x * step + dx, y * step + dy);
                    let t = map.tile_map[(py / TILE_SIZE) * tw + px / TILE_SIZE] as usize;
                    let c = palette.colors[map.tiles[t][(py % TILE_SIZE) * TILE_SIZE + px % TILE_SIZE] as usize];
                    for k in 0..3 {
                        acc[k] += c[k] as u32;
                    }
                }
            }
            let n = (step * step) as u32;
            img.put_pixel(x as u32, y as u32, image::Rgb([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8]));
        }
    }
    let file = stem.replace([' ', '\''], "_");
    img.save(dir.join(format!("{file}.png")))?;
    // The game's own preview (its top-down picture of the map).
    if let Ok(p) = ta_supcom::dds::decode(&m.scmap.preview, 0) {
        let rgb: Vec<u8> = p.rgba.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
        if let Some(i) = image::RgbImage::from_raw(p.width, p.height, rgb) {
            i.save(dir.join(format!("{file}_game.png")))?;
        }
    }
    // A detail at full size: a 1024-pixel square from the middle of the map.
    let (dw, dh) = (1024.min(w), 1024.min(h));
    let (ox, oy) = ((w - dw) / 2, (h - dh) / 2);
    let mut detail = image::RgbImage::new(dw as u32, dh as u32);
    for y in 0..dh {
        for x in 0..dw {
            let (px, py) = (ox + x, oy + y);
            let t = map.tile_map[(py / TILE_SIZE) * tw + px / TILE_SIZE] as usize;
            detail.put_pixel(x as u32, y as u32, image::Rgb(palette.colors[map.tiles[t][(py % TILE_SIZE) * TILE_SIZE + px % TILE_SIZE] as usize]));
        }
    }
    detail.save(dir.join(format!("{file}_detail.png")))?;
    Ok(())
}
