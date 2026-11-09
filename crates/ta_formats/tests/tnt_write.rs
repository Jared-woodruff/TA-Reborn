//! The TNT and TDF writers against the real game data: every map read, written and read
//! again gives the same map, byte for byte where TA's layout is the writer's. Skipped when no
//! TA install is found.

use ta_formats::tdf;
use ta_formats::tnt::TntMap;

fn open_install() -> Option<ta_vfs::Vfs> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    Some(vfs)
}

fn i32_at(data: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
}

fn assert_same(a: &TntMap, b: &TntMap, what: &str) {
    assert_eq!((a.width, a.height, a.sea_level), (b.width, b.height, b.sea_level), "{what}: size or sea level");
    assert_eq!(a.tile_map, b.tile_map, "{what}: tile map");
    assert_eq!(a.heights, b.heights, "{what}: heights");
    assert_eq!(a.features, b.features, "{what}: features");
    assert_eq!(a.feature_names, b.feature_names, "{what}: feature names");
    assert_eq!(a.tiles.len(), b.tiles.len(), "{what}: tile count");
    assert!(a.tiles.iter().zip(&b.tiles).all(|(x, y)| x[..] == y[..]), "{what}: tile graphics");
    assert_eq!(a.minimap, b.minimap, "{what}: minimap");
}

#[test]
fn every_map_reads_back_the_same_after_writing() {
    let Some(vfs) = open_install() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let maps = vfs.list("maps", Some("tnt"), false);
    assert!(maps.len() > 50, "the install has its maps");
    let mut same_bytes = 0;
    for path in &maps {
        let bytes = vfs.read(path).unwrap();
        let map = TntMap::parse(&bytes).unwrap();
        assert_eq!(TntMap::preview(&bytes).unwrap(), map.minimap, "{path}: lightweight preview");
        let written = map.write().unwrap_or_else(|e| panic!("{path}: {e}"));
        let again = TntMap::parse(&written).unwrap_or_else(|e| panic!("{path} written: {e}"));
        assert_same(&map, &again, path);
        // Writing what was read back gives the same file.
        assert_eq!(again.write().unwrap(), written, "{path}: a second write differs");
        // The layout is TA's: the same header (every section at the same offset) and the
        // cell attributes and tile graphics byte for byte. TA's editor left stray bytes in the
        // padding before the attributes and after the feature names' NULs, where we write
        // zeros, and some campaign maps declare a 252×256 minimap.
        assert_eq!(&written[..64], &bytes[..64], "{path}: header");
        let (attributes, names) = (i32_at(&bytes, 16) as usize, i32_at(&bytes, 32) as usize);
        assert!(written[attributes..names] == bytes[attributes..names], "{path}: attributes or tile graphics");
        if written == bytes {
            same_bytes += 1;
        }
    }
    eprintln!("{} maps written and read back; {same_bytes} byte-identical to the original", maps.len());
}

#[test]
fn every_ota_reads_back_the_same_after_writing() {
    let Some(vfs) = open_install() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let otas = vfs.list("maps", Some("ota"), false);
    for path in &otas {
        let doc = tdf::parse_bytes(&vfs.read(path).unwrap());
        let written = doc.to_bytes();
        assert_eq!(tdf::parse_bytes(&written), doc, "{path}");
    }
    eprintln!("{} OTA files written and read back", otas.len());
}

#[test]
fn built_minimaps_fill_the_canvas_like_ta_s() {
    let Some(vfs) = open_install() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let palette = ta_formats::image::Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
    for name in ["Coast To Coast", "Painted Desert", "Metal Heck"] {
        let Some(bytes) = vfs.read_opt(&format!("maps/{name}.tnt")) else { continue };
        let mut map = TntMap::parse(&bytes).unwrap();
        let built = map.build_minimap(&palette);
        let (w, h) = ta_formats::tnt::minimap_size(map.width, map.height);
        assert_eq!((built.width as usize, built.height as usize), (w, h), "{name}");
        // The shipped picture and ours cover the same part of the canvas, and ours reads back
        // through the filler crop unchanged.
        assert_eq!((map.minimap.width as usize, map.minimap.height as usize), (w, h), "{name}: shipped and built sizes");
        map.minimap = built.clone();
        let again = TntMap::parse(&map.write().unwrap()).unwrap();
        assert_eq!(again.minimap, built, "{name}");
    }
}

/// Which part of the art the shipped minimaps show (layout facts for FORMATS.md): the mean
/// colour difference between each map's minimap and its art box-filtered from a few candidate
/// windows of (width − 2) × (height − 8) cells.
#[test]
#[ignore]
fn show_minimap_window() {
    let Some(vfs) = open_install() else { return };
    let palette = ta_formats::image::Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
    let mut wins = std::collections::BTreeMap::new();
    for path in vfs.list("maps", Some("tnt"), false).iter().step_by(7) {
        let map = TntMap::parse(&vfs.read(path).unwrap()).unwrap();
        let art = map.render_indices();
        let aw = map.width_px() as usize;
        let (mw, mh) = (map.minimap.width as usize, map.minimap.height as usize);
        let mut scores = Vec::new();
        for (dx, dy) in [(0, 0), (16, 0), (32, 0), (0, 64), (16, 64), (0, 128), (16, 128), (8, 32), (16, 32)] {
            let (rw, rh) = ((map.width as usize - 2) * 16, (map.height as usize - 8) * 16);
            let mut diff = 0u64;
            for my in 0..mh {
                for mx in 0..mw {
                    let x = dx + (mx * 2 + 1) * rw / (mw * 2);
                    let y = dy + (my * 2 + 1) * rh / (mh * 2);
                    let a = palette.rgb(art[y.min(map.height_px() as usize - 1) * aw + x.min(aw - 1)]);
                    let b = palette.rgb(map.minimap.pixels[my * mw + mx]);
                    diff += (0..3).map(|k| (a[k] as i32 - b[k] as i32).unsigned_abs() as u64).sum::<u64>();
                }
            }
            scores.push((diff / (mw * mh).max(1) as u64, (dx, dy)));
        }
        scores.sort();
        println!("{path}: best {:?}, then {:?}", scores[0], &scores[1..3]);
        *wins.entry(scores[0].1).or_insert(0) += 1;
    }
    println!("{wins:?}");
}

/// How multi-cell features mark the cells they cover (layout facts for FORMATS.md).
#[test]
#[ignore]
fn show_covered_cells() {
    let Some(vfs) = open_install() else { return };
    let mut footprints = std::collections::HashMap::new();
    for p in vfs.list("features", Some("tdf"), true) {
        for s in tdf::parse_bytes(&vfs.read(&p).unwrap()).sections {
            let fp = (s.get_i32_or("footprintx", 1).max(1), s.get_i32_or("footprintz", 1).max(1));
            footprints.insert(s.name.to_ascii_lowercase(), fp);
        }
    }
    let (mut marked, mut unmarked, mut stray) = (0, 0, 0);
    let mut examples = Vec::new();
    for path in vfs.list("maps", Some("tnt"), false) {
        let map = TntMap::parse(&vfs.read(&path).unwrap()).unwrap();
        let (w, h) = (map.width as i32, map.height as i32);
        let mut explained = vec![false; map.features.len()];
        for z in 0..h {
            for x in 0..w {
                let f = map.features[(z * w + x) as usize];
                let Some(name) = map.feature_names.get(f as usize) else { continue };
                let (fx, fz) = footprints.get(&name.to_ascii_lowercase()).copied().unwrap_or((1, 1));
                if fx * fz == 1 {
                    continue;
                }
                let mut all = true;
                for dz in 0..fz {
                    for dx in 0..fx {
                        let (cx, cz) = (x + dx, z + dz);
                        if (dx, dz) == (0, 0) || cx >= w || cz >= h {
                            continue;
                        }
                        let i = (cz * w + cx) as usize;
                        explained[i] = true;
                        all &= map.features[i] == 0xFFFE;
                    }
                }
                if all { marked += 1 } else { unmarked += 1 }
                if examples.len() < 12 && !all {
                    examples.push(format!("{path} {name} {fx}x{fz} at {x},{z}"));
                }
            }
        }
        stray += map.features.iter().zip(&explained).filter(|(f, e)| **f == 0xFFFE && !**e).count();
    }
    println!("multi-cell features: {marked} with covered cells marked 0xFFFE, {unmarked} without; {stray} 0xFFFE cells under no footprint");
    for e in examples {
        println!("  {e}");
    }
}

/// Where written maps first differ from the shipped files, by section.
#[test]
#[ignore]
fn show_differences() {
    let Some(vfs) = open_install() else { return };
    let mut by_section = std::collections::BTreeMap::new();
    for path in vfs.list("maps", Some("tnt"), false) {
        let d = vfs.read(&path).unwrap();
        let w = TntMap::parse(&d).unwrap().write().unwrap();
        let sections = ["header", "tile map", "attributes", "tiles", "feature names", "minimap"];
        let starts: Vec<usize> = [0usize, 12, 16, 20, 32, 40].iter().map(|&o| if o == 0 { 0 } else { i32_at(&d, o) as usize }).collect();
        for (k, name) in sections.iter().enumerate() {
            let (a, b) = (starts[k], starts.get(k + 1).copied().unwrap_or(d.len().min(w.len())));
            let (x, y) = (&d[a..b.min(d.len())], &w[a..b.min(w.len())]);
            if x != y {
                let first = x.iter().zip(y).position(|(p, q)| p != q).unwrap_or(0);
                let n = x.iter().zip(y).filter(|(p, q)| p != q).count();
                *by_section.entry(*name).or_insert(0) += 1;
                if by_section[name] <= 3 {
                    println!("{path}: {name} differs from byte {first} of {} ({n} bytes)", b - a);
                }
            }
        }
    }
    println!("{by_section:?}");
}

/// Prints the header of a few maps (layout facts for FORMATS.md).
#[test]
#[ignore]
fn show_headers() {
    let Some(vfs) = open_install() else { return };
    for path in vfs.list("maps", Some("tnt"), false).iter().take(400) {
        let d = vfs.read(path).unwrap();
        let fields: Vec<i32> = (0..16).map(|k| i32_at(&d, k * 4)).collect();
        let map = TntMap::parse(&d).unwrap();
        let mm = i32_at(&d, 40) as usize;
        let (mw, mh) = (i32_at(&d, mm), i32_at(&d, mm + 4));
        let pads: std::collections::BTreeSet<u8> = (0..(map.width * map.height) as usize).map(|i| d[fields[4] as usize + i * 4 + 3]).collect();
        let covered = map.features.iter().filter(|&&f| f == 0xFFFE).count();
        let void = map.features.iter().filter(|&&f| f == 0xFFFC).count();
        println!(
            "{path}: {fields:?} len {} end {} mm {mw}x{mh} crop {}x{} pads {pads:?} covered {covered} void {void}",
            d.len(),
            mm + 8 + 252 * 252,
            map.minimap.width,
            map.minimap.height
        );
    }
}
