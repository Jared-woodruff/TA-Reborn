//! The player's own copy of Forged Alliance, read whole: every map's scenario, save file and map
//! file, and every texture its strata name. Ignored (it needs the game installed, and takes a
//! while): `cargo test --release -p ta_supcom --test installed -- --ignored --nocapture`
//! (`TA_SUPCOM_DIR=<folder>` a copy of the game elsewhere).

use std::path::PathBuf;

use ta_supcom::files::Files;
use ta_supcom::install::{self, Install, Store};
use ta_supcom::map::{self, Map};

fn the_game() -> Option<Install> {
    if let Some(dir) = std::env::var_os("TA_SUPCOM_DIR") {
        return Some(Install { root: PathBuf::from(dir), store: Store::Folder });
    }
    install::discover().into_iter().next()
}

/// Every copy of the game found on this computer, in the order looked for (GOG's, Steam's, the
/// boxed copy's...), and that each holds the same maps.
#[test]
#[ignore]
fn every_copy_found() {
    let copies = install::discover();
    for copy in &copies {
        let maps = std::fs::read_dir(copy.maps()).map_or(0, |d| d.count());
        eprintln!("{:?}: {} ({maps} map folders)", copy.store, copy.root.display());
    }
    if copies.is_empty() {
        eprintln!("no copy of Forged Alliance found");
    }
}

#[test]
#[ignore]
fn every_installed_map_reads() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    eprintln!("the game: {} ({:?})", game.root.display(), game.store);
    let user = install::user_map_folders();
    eprintln!("players' maps: {user:?}");
    let files = Files::open(&game, &user).unwrap();
    let mut folders: Vec<PathBuf> = Vec::new();
    for dir in files.map_dirs() {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && map::scenario_file(p).is_some()).collect();
        found.sort();
        folders.extend(found);
    }
    assert!(!folders.is_empty(), "no maps");
    let mut failed = Vec::new();
    let mut missing_textures = std::collections::BTreeSet::new();
    for folder in &folders {
        let t = std::time::Instant::now();
        match Map::read(&files, folder) {
            Ok(m) => {
                let s = &m.scenario;
                for layer in m.scmap.albedo.iter().chain(&m.scmap.normals) {
                    if !layer.path.is_empty() && files.read(&layer.path).unwrap().is_none() {
                        missing_textures.insert(layer.path.clone());
                    }
                }
                eprintln!(
                    "{:<10} {:<26} {:<9} {:>4}x{:<4} v{} players {} mass {:>3} hydro {} civilians {:>4} props {:>5} decals {:>5}{} [{:.2}s]",
                    s.folder, s.name, s.kind, m.scmap.width, m.scmap.height, m.scmap.version, m.starts.len(), m.mass.len(), m.hydrocarbons.len(), m.civilians.len(),
                    m.scmap.props.len(), m.scmap.decals.len(), m.playable.map_or(String::new(), |p| format!(" area {p:?}")), t.elapsed().as_secs_f32()
                );
                if s.skirmish() {
                    assert!(m.starts.len() >= 2, "{}: {} starts", s.folder, m.starts.len());
                    assert_eq!((m.scmap.width, m.scmap.height), s.size, "{}", s.folder);
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", folder.display());
                failed.push(folder.clone());
            }
        }
    }
    eprintln!("textures the strata name that the game lacks: {missing_textures:?}");
    assert!(failed.is_empty(), "{} maps did not read: {failed:?}", failed.len());
}

/// Every skirmish map of the player's copy converted as the launcher converts them: each deposit
/// takes one of TA's extractors (on land dry, no steeper than 10 and no cliff; under water at
/// least 19 deep), the starts the game's land units drive between are joined for a tank, what
/// the map's own features leave is never worth more nor blocks more than they did, and the maps
/// of 5 km come out the same, byte for byte, on three threads as on all. Ignored (it needs both
/// games installed, and takes about twenty minutes): `cargo test --release -p ta_supcom --test
/// installed every_installed_map_converts -- --ignored --nocapture` (`TA_SUPCOM_ONLY=scmp_009`
/// for some).
#[test]
#[ignore]
fn every_installed_map_converts() {
    use ta_formats::tnt::{CELL_CLIFF, TntMap};
    use ta_supcom::convert::{self, Options, kit::Templates};
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to convert");
        return;
    };
    let Ok(ta) = ta_vfs::install::discover(None) else {
        eprintln!("no copy of Total Annihilation found: its features and palette are needed");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&ta.root, "base").unwrap();
    let palette = ta_formats::image::Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
    let templates = Templates::load(&vfs);
    let files = Files::open(&game, &[]).unwrap();
    let only: Option<Vec<String>> = std::env::var("TA_SUPCOM_ONLY").ok().map(|o| o.split(',').map(|s| s.trim().to_ascii_lowercase()).collect());
    let mut folders: Vec<PathBuf> = std::fs::read_dir(game.maps()).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir() && map::scenario_file(p).is_some()).collect();
    folders.sort();
    let options = Options::default();
    let few = rayon::ThreadPoolBuilder::new().num_threads(3).build().unwrap();
    let mut problems: Vec<String> = Vec::new();
    let mut seen = 0;
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
        if only.as_ref().is_some_and(|o| !o.contains(&name)) {
            continue;
        }
        let Ok(m) = Map::read(&files, &folder) else { continue };
        if !m.scenario.skirmish() {
            continue;
        }
        let t = std::time::Instant::now();
        let c = match convert::convert(&m, &files, &templates, &palette, false, &options, &|_| {}) {
            Ok(c) => c,
            Err(e) => {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let tnt = TntMap::parse(&c.tnt).unwrap();
        let (w, h) = (tnt.width as usize, tnt.height as usize);
        let depth = |x: usize, z: usize| tnt.sea_level - tnt.heights[z * w + x] as i32;
        // (A map without a cliff has no marks written: no flags at all.)
        let slope = |x: usize, z: usize| -> i32 {
            if tnt.flags.get(z * w + x).is_some_and(|f| f & CELL_CLIFF != 0) {
                return 255;
            }
            let at = |x: usize, z: usize| tnt.heights[z.min(h - 1) * w + x.min(w - 1)] as i32;
            let c = [at(x, z), at(x + 1, z), at(x, z + 1), at(x + 1, z + 1)];
            c.iter().max().unwrap() - c.iter().min().unwrap()
        };
        // Every deposit takes an extractor.
        let mut deposits = 0;
        for i in 0..w * h {
            let Some(feature) = tnt.feature_names.get(tnt.features[i] as usize) else { continue };
            let Some((_, s)) = templates.get(feature) else { continue };
            let deposit = s.get("category").is_some_and(|c| c.eq_ignore_ascii_case("metal")) || (s.get_f64_or("metal", 0.0) > 0.0 && s.get_bool_or("indestructible", false) && s.get_f64_or("height", 0.0) <= 8.0);
            if !deposit {
                continue;
            }
            deposits += 1;
            let fp = templates.footprint(feature) as usize;
            let cells: Vec<(usize, usize)> = (i / w..(i / w + fp).min(h)).flat_map(|z| (i % w..(i % w + fp).min(w)).map(move |x| (x, z))).collect();
            let land = cells.iter().all(|&(x, z)| depth(x, z) <= 0 && slope(x, z) <= 10);
            let sea = cells.iter().all(|&(x, z)| depth(x, z) >= 19);
            if !land && !sea {
                problems.push(format!("{name}: no extractor stands on the deposit at ({}, {}): depths {:?}, slopes {:?}", i % w, i / w, cells.iter().map(|&(x, z)| depth(x, z)).collect::<Vec<_>>(), cells.iter().map(|&(x, z)| slope(x, z)).collect::<Vec<_>>()));
            }
        }
        // The starts the game joined, joined.
        if !c.cut_off.is_empty() {
            problems.push(format!("{name}: starts the game's land units drive between that a tank cannot: {:?}", c.cut_off));
        }
        // What the map's own features leave.
        if let Some(bytes) = &c.features {
            let doc = ta_formats::tdf::parse_bytes(bytes);
            for s in &doc.sections {
                let next = s.get("featuredead").unwrap_or("");
                match doc.section(next) {
                    Some(n) => {
                        let more = n.get_f64_or("metal", 0.0) > s.get_f64_or("metal", 0.0) || n.get_f64_or("energy", 0.0) > s.get_f64_or("energy", 0.0);
                        let blocks = n.get_bool_or("blocking", true) && (!s.get_bool_or("blocking", true) || n.get_i64_or("footprintx", 1) > s.get_i64_or("footprintx", 1) || n.get_i64_or("footprintz", 1) > s.get_i64_or("footprintz", 1));
                        if more || blocks {
                            problems.push(format!("{name}: {} leaves {}, worth more or blocking more", s.name, n.name));
                        }
                    }
                    None if next.is_empty() => {}
                    None => {
                        let harmless = templates.get(next).is_some_and(|(_, d)| !d.get_bool_or("blocking", true) && d.get_f64_or("metal", 0.0) <= 0.0 && d.get_f64_or("energy", 0.0) <= 0.0);
                        if !harmless {
                            problems.push(format!("{name}: {} leaves TA's {next}", s.name));
                        }
                    }
                }
            }
        }
        // The same on another number of threads (5 km maps: a few seconds each).
        let mut same = "";
        if m.scmap.width <= 256 && m.scmap.height <= 256 {
            let again = few.install(|| convert::convert(&m, &files, &templates, &palette, false, &options, &|_| {})).unwrap();
            same = if again.tnt == c.tnt && again.ota == c.ota && again.features == c.features { ", the same on 3 threads" } else {
                problems.push(format!("{name}: other bytes on 3 threads"));
                ", OTHER BYTES ON 3 THREADS"
            };
        }
        eprintln!("{name:<9} {:<26} {deposits:>3} deposits, {} tiles{same} [{:.1}s]", m.scenario.name, tnt.tiles.len(), t.elapsed().as_secs_f32());
        for p in &problems[seen..] {
            eprintln!("  PROBLEM: {p}");
        }
        seen = problems.len();
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// How each map's colours match the game's own picture of it: the painting lit as the game's
/// terrain shader lights it, against the preview, over the land, and the gain that took (`wanted`
/// past the limits shows where the converter's light and the picture still differ):
/// `cargo test --release -p ta_supcom --test installed how_each_map_matches_its_preview --
/// --ignored --nocapture` (`TA_SUPCOM_ONLY=scmp_024,scmp_029` for some).
#[test]
#[ignore]
fn how_each_map_matches_its_preview() {
    let Some(game) = the_game() else { return };
    let files = Files::open(&game, &[]).unwrap();
    let only: Option<Vec<String>> = std::env::var("TA_SUPCOM_ONLY").ok().map(|o| o.split(',').map(|s| s.trim().to_ascii_lowercase()).collect());
    let mut folders: Vec<PathBuf> = std::fs::read_dir(game.maps()).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    folders.sort();
    let round = |v: [f64; 3], k: f64| v.map(|x| (x * k).round() / k);
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
        if only.as_ref().is_some_and(|o| !o.contains(&name)) {
            continue;
        }
        let Ok(m) = Map::read(&files, &folder) else { continue };
        if !m.scenario.skirmish() {
            continue;
        }
        let frame = ta_supcom::convert::land::Frame::of([0.0, 0.0, m.scmap.width as f64, m.scmap.height as f64]);
        let p = ta_supcom::convert::paint::Painter::new(&m.scmap, &files, frame).unwrap();
        let l = &m.scmap.lighting;
        match p.matched() {
            Some(c) => eprintln!(
                "{name:<9} {:<24} mult {:.2} bloom {:.2} land {:>5}: ours {:?} picture {:?} wanted {:?} gain {:?}",
                m.scenario.name, l.multiplier, l.bloom, c.places, round(c.ours.map(|v| v * 255.0), 1.0), round(c.theirs.map(|v| v * 255.0), 1.0), round(c.wanted, 100.0), round(c.gain, 100.0)
            ),
            None => eprintln!("{name:<9} {:<24} not matched", m.scenario.name),
        }
    }
}

/// What painting each map costs: its albedo decals, and how many lie over a square of the world
/// (`TA_SUPCOM_ONLY=scmp_040,scmp_009` for some).
#[test]
#[ignore]
fn the_decals_each_map_paints() {
    let Some(game) = the_game() else { return };
    let files = Files::open(&game, &[]).unwrap();
    let only: Option<Vec<String>> = std::env::var("TA_SUPCOM_ONLY").ok().map(|o| o.split(',').map(|s| s.trim().to_ascii_lowercase()).collect());
    let mut folders: Vec<PathBuf> = std::fs::read_dir(game.maps()).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    folders.sort();
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
        if only.as_ref().is_some_and(|o| !o.contains(&name)) {
            continue;
        }
        let Ok(m) = Map::read(&files, &folder) else { continue };
        let frame = ta_supcom::convert::land::Frame::of([0.0, 0.0, m.scmap.width as f64, m.scmap.height as f64]);
        let t = std::time::Instant::now();
        let p = ta_supcom::convert::paint::Painter::new(&m.scmap, &files, frame).unwrap();
        let (n, mean, max) = p.decal_load();
        let kinds = m.scmap.decals.iter().fold(std::collections::BTreeMap::<i32, usize>::new(), |mut k, d| { *k.entry(d.kind).or_default() += 1; k });
        eprintln!("{name}: {n} albedo decals of {kinds:?}, {mean:.1} a square, at most {max} [{:.1}s]", t.elapsed().as_secs_f32());
    }
}

/// The Supreme Commander mods' banners, made from this computer's copy as the imports make them
/// (the maps' of every map of the game's own), written to `TA_BANNER_OUT` (a folder; else one
/// in the system's temporary folder) to be looked at; each made twice, the same.
#[test]
#[ignore]
fn the_banners_of_the_installed_copy() {
    use ta_supcom::{banner, soundtrack};
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: no banners");
        return;
    };
    let out = std::env::var_os("TA_BANNER_OUT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("ta-supcom-banners"));
    std::fs::create_dir_all(&out).unwrap();
    let files = Files::own(&game).unwrap();
    let started = std::time::Instant::now();
    let factions = banner::factions(&files, &[]).expect("the factions' hall and emblems");
    let two_factions = banner::factions(&files, &["UEF", "SERAPHIM"]).unwrap();
    let sources: Vec<String> = std::fs::read_dir(game.maps()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect();
    let previews: Vec<_> = banner::map_order(&sources).iter().filter_map(|s| ta_vfs::install::find_dir_ci(&game.maps(), s)).filter_map(|folder| banner::map_preview(&folder)).take(banner::MOST_MAPS).collect();
    assert_eq!(previews.len(), banner::MOST_MAPS, "{sources:?}");
    let maps = banner::maps(&previews, Some(&files)).expect("the maps' pictures");
    // (A pack of two maps, and of one.)
    let two = banner::maps(&previews[..2], Some(&files)).unwrap();
    let one = banner::maps(&previews[..1], Some(&files)).unwrap();
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds").expect("the copy's sounds");
    let loudness = soundtrack::menu_loudness(&sounds).unwrap().expect("the menu theme");
    let music = banner::music(Some(&files), &loudness);
    let menus = banner::menus(&files).expect("the menus' pictures");
    eprintln!("four banners made in {:.2} s", started.elapsed().as_secs_f32());
    for (name, png) in [("factions", &factions), ("factions_two", &two_factions), ("maps", &maps), ("maps_two", &two), ("maps_one", &one), ("music", &music), ("menus", &menus)] {
        std::fs::write(out.join(format!("{name}.png")), png).unwrap();
        eprintln!("  {name}: {} KB", png.len() >> 10);
    }
    assert_eq!(banner::factions(&files, &[]).as_ref(), Some(&factions), "the same twice");
    assert_eq!(banner::music(Some(&files), &loudness), music, "the same twice");
    eprintln!("written to {}", out.display());
}
