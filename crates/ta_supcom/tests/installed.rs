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
