//! VFS-level inspection commands: format validation, image/map/model/script dumps.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;
use ta_formats::cob::{self, CobScript};
use ta_formats::gaf::{Gaf, GafReader};
use ta_formats::image::Palette;
use ta_formats::model3do::Model3do;
use ta_formats::pcx::PcxImage;
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_vfs::Vfs;

pub fn palette(vfs: &Vfs) -> Result<Palette> {
    Ok(Palette::parse(&vfs.read("palettes/palette.pal")?)?)
}

/// Parses every file of each known binary format and reports failures.
pub fn validate(vfs: &Vfs) -> Result<()> {
    let all = vfs.list("", None, true);
    let by_ext = |ext: &str| -> Vec<&String> { all.iter().filter(|f| f.ends_with(ext)).collect() };

    fn run<T: Send>(
        label: &str,
        files: &[&String],
        vfs: &Vfs,
        parse: impl Fn(&[u8]) -> std::result::Result<T, String> + Sync,
    ) -> Vec<(String, T)> {
        let results: Vec<(String, std::result::Result<T, String>)> = files
            .par_iter()
            .map(|f| {
                let r = vfs.read(f).map_err(|e| e.to_string()).and_then(|d| parse(&d));
                (f.to_string(), r)
            })
            .collect();
        let mut ok = Vec::new();
        let mut failed = 0;
        for (f, r) in results {
            match r {
                Ok(v) => ok.push((f, v)),
                Err(e) => {
                    failed += 1;
                    if failed <= 8 {
                        println!("    FAIL {f}: {e}");
                    }
                }
            }
        }
        println!("{label:<6} {:>5} ok  {:>4} failed", ok.len(), failed);
        ok
    }

    // (A frame at a time, as the game reads a weapon's explosion: (entries, frames, whether
    // `Gaf::parse` could decode it whole, as the game does any other GAF).)
    let gafs = run("gaf", &by_ext(".gaf"), vfs, |d| {
        let reader = GafReader::open(d).map_err(|e| e.to_string())?;
        let costs = reader.entries.iter().map(|e| e.frames.iter().map(|f| reader.check(f)).collect::<std::result::Result<Vec<u64>, _>>()).collect::<std::result::Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        Ok((reader.entries.len(), reader.entries.iter().map(|e| e.frames.len()).sum::<usize>(), reader.whole(&costs)))
    });
    let frames: usize = gafs.iter().map(|(_, g)| g.1).sum();
    println!("         {} entries, {frames} frames", gafs.iter().map(|(_, g)| g.0).sum::<usize>());
    let big: Vec<&str> = gafs.iter().filter(|(_, g)| !g.2).map(|(f, _)| f.as_str()).collect();
    if !big.is_empty() {
        println!("         {} too large to decode whole (read a frame at a time, as explosions): {}", big.len(), big.join(", "));
    }

    let models = run("3do", &by_ext(".3do"), vfs, |d| Model3do::parse(d).map_err(|e| e.to_string()));
    let mut prim_sizes: BTreeMap<usize, usize> = BTreeMap::new();
    for (_, m) in &models {
        for p in &m.pieces {
            for pr in &p.primitives {
                *prim_sizes.entry(pr.indices.len()).or_default() += 1;
            }
        }
    }
    println!("         primitive vertex counts: {prim_sizes:?}");

    let scripts = run("cob", &by_ext(".cob"), vfs, |d| CobScript::parse(d).map_err(|e| e.to_string()));
    let mut unknown: BTreeMap<u32, usize> = BTreeMap::new();
    for (_, s) in &scripts {
        let mut pc = 0;
        while pc < s.code.len() {
            match cob::operand_count(s.code[pc]) {
                Some(n) => pc += 1 + n,
                None => {
                    *unknown.entry(s.code[pc]).or_default() += 1;
                    pc += 1;
                }
            }
        }
    }
    println!("         unknown opcodes: {unknown:x?}");

    let maps = run("tnt", &by_ext(".tnt"), vfs, |d| TntMap::parse(d).map_err(|e| e.to_string()));
    let largest = maps.iter().max_by_key(|(_, m)| m.width * m.height);
    if let Some((name, m)) = largest {
        println!("         largest map: {name} ({}x{} px)", m.width_px(), m.height_px());
    }
    run("pcx", &by_ext(".pcx"), vfs, |d| PcxImage::parse(d).map_err(|e| e.to_string()));

    let text: Vec<&String> =
        all.iter().filter(|f| [".tdf", ".fbi", ".gui", ".ota"].iter().any(|e| f.ends_with(e))).collect();
    let docs = run("tdf", &text, vfs, |d| Ok::<_, String>(tdf::parse_bytes(d)));
    let empty: Vec<&String> = docs.iter().filter(|(_, d)| d.sections.is_empty()).map(|(f, _)| f).collect();
    println!("         {} text files with no sections: {:?}", empty.len(), &empty[..empty.len().min(10)]);
    Ok(())
}

/// Every frame of a GAF to a PNG, a frame at a time (a mod's explosions may be too large to
/// decode whole).
pub fn dump_gaf(vfs: &Vfs, path: &str, out: &Path) -> Result<()> {
    let pal = palette(vfs)?;
    let data = vfs.read(path)?;
    let reader = GafReader::open(&data)?;
    std::fs::create_dir_all(out)?;
    for e in &reader.entries {
        for (i, f) in e.frames.iter().enumerate() {
            if f.width == 0 || f.height == 0 {
                continue;
            }
            let img = reader.frame(f)?.image;
            let rgba = img.to_rgba(&pal);
            let file = out.join(format!("{}_{i:02}.png", sanitize(&e.name)));
            image::save_buffer(&file, &rgba, img.width, img.height, image::ColorType::Rgba8)?;
        }
        let first = e.frames.first();
        println!(
            "{:<32} {} frames, first {}x{} origin ({},{}) transp {}",
            e.name,
            e.frames.len(),
            first.map_or(0, |f| f.width),
            first.map_or(0, |f| f.height),
            first.map_or(0, |f| f.origin_x),
            first.map_or(0, |f| f.origin_y),
            first.map_or(0, |f| f.transparency_index),
        );
    }
    Ok(())
}

pub fn dump_map(vfs: &Vfs, name: &str, out: &Path) -> Result<()> {
    let pal = palette(vfs)?;
    let path = format!("maps/{name}.tnt");
    let map = TntMap::parse(&vfs.read(&path).with_context(|| path.clone())?)?;
    std::fs::create_dir_all(out)?;
    println!(
        "{name}: {}x{} cells ({}x{} px), {} tiles, sea level {}, {} feature types, minimap {}x{}",
        map.width,
        map.height,
        map.width_px(),
        map.height_px(),
        map.tiles.len(),
        map.sea_level,
        map.feature_names.len(),
        map.minimap.width,
        map.minimap.height
    );
    let idx = map.render_indices();
    let rgba: Vec<u8> = idx.iter().flat_map(|&p| pal.rgba(p)).collect();
    image::save_buffer(out.join(format!("{name}.png")), &rgba, map.width_px(), map.height_px(), image::ColorType::Rgba8)?;
    image::save_buffer(out.join(format!("{name}_height.png")), &map.heights, map.width, map.height, image::ColorType::L8)?;
    // Feature mask: features green, 0xFFFE red, 0xFFFC (void) blue, others dark.
    let mask: Vec<u8> = map
        .features
        .iter()
        .flat_map(|&f| match f {
            0xFFFF => [0, 0, 0],
            0xFFFE => [255, 0, 0],
            0xFFFC => [0, 0, 255],
            f if (f as usize) < map.feature_names.len() => [0, 255, 0],
            _ => [255, 255, 0],
        })
        .collect();
    image::save_buffer(out.join(format!("{name}_features.png")), &mask, map.width, map.height, image::ColorType::Rgb8)?;
    let mm = map.minimap.to_rgba(&pal);
    image::save_buffer(out.join(format!("{name}_mini.png")), &mm, map.minimap.width, map.minimap.height, image::ColorType::Rgba8)?;
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    for &f in &map.features {
        *counts.entry(f).or_default() += 1;
    }
    for (f, n) in counts {
        let label = map.feature_names.get(f as usize).cloned().unwrap_or_else(|| format!("special {f:#06x}"));
        println!("  feature {label:<24} x{n}");
    }
    if let Some(ota) = vfs.read_opt(&format!("maps/{name}.ota")) {
        println!("{}", String::from_utf8_lossy(&ota));
    }
    Ok(())
}

pub fn dump_model(vfs: &Vfs, path: &str) -> Result<()> {
    let m = Model3do::parse(&vfs.read(path)?)?;
    for (i, p) in m.pieces.iter().enumerate() {
        let depth = std::iter::successors(p.parent, |&q| m.pieces[q].parent).count();
        let tex: Vec<String> = p.primitives.iter().filter_map(|pr| pr.texture.clone()).collect();
        println!(
            "{i:3} {}{:<16} off=({:.2},{:.2},{:.2}) verts={} prims={} sel={:?} tex={:?}",
            "  ".repeat(depth),
            p.name,
            p.offset[0] as f32 / 65536.0,
            p.offset[1] as f32 / 65536.0,
            p.offset[2] as f32 / 65536.0,
            p.vertices.len(),
            p.primitives.len(),
            p.selection_primitive,
            {
                let mut t = tex.clone();
                t.dedup();
                t
            }
        );
        if std::env::var_os("TA_PRIMS").is_some() {
            for (k, pr) in p.primitives.iter().enumerate() {
                let pts: Vec<[f32; 3]> = pr.indices.iter().filter_map(|&i| p.vertices.get(i as usize)).map(|v| [v[0] as f32 / 65536.0, v[1] as f32 / 65536.0, v[2] as f32 / 65536.0]).collect();
                let lo = pts.iter().fold([f32::MAX; 3], |a, v| [a[0].min(v[0]), a[1].min(v[1]), a[2].min(v[2])]);
                let hi = pts.iter().fold([f32::MIN; 3], |a, v| [a[0].max(v[0]), a[1].max(v[1]), a[2].max(v[2])]);
                println!(
                    "      prim {k:2}: {} verts, {} colour {:3}, x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}{}",
                    pr.indices.len(),
                    pr.texture.as_deref().unwrap_or("-"),
                    pr.color_index,
                    lo[0], hi[0], lo[1], hi[1], lo[2], hi[2],
                    if Some(k) == p.selection_primitive { "  (selection)" } else { "" }
                );
            }
        }
    }
    Ok(())
}

pub fn dump_script(vfs: &Vfs, path: &str) -> Result<()> {
    let s = CobScript::parse(&vfs.read(path)?)?;
    print!("{}", cob::disassemble(&s));
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

pub fn check_data(vfs: &Vfs, unit: Option<&str>, menus: bool) -> Result<()> {
    let data = ta_data::GameData::load(vfs);
    if let Some(name) = unit {
        let u = data.unit(name).context("unknown unit")?;
        println!("{u:#?}");
        for w in u.weapons.iter().flatten() {
            println!("{:#?}", data.weapon(w));
        }
        return Ok(());
    }
    println!(
        "{} units, {} weapons, {} features, {} move classes, sides: {:?}",
        data.units.len(),
        data.weapons.len(),
        data.features.len(),
        data.move_classes.len(),
        data.sides.iter().map(|s| format!("{} (commander {})", s.name, s.commander)).collect::<Vec<_>>()
    );
    let mut problems = Vec::new();
    for u in &data.units {
        for w in u.weapons.iter().flatten() {
            if data.weapon(w).is_none() {
                problems.push(format!("{}: unknown weapon {w}", u.name));
            }
        }
        if u.can_move && !u.can_fly && !u.movement_class.is_empty() && data.move_class(&u.movement_class).is_none() {
            problems.push(format!("{}: unknown movement class {}", u.name, u.movement_class));
        }
        if !vfs.exists(&format!("objects3d/{}.3do", u.object)) {
            problems.push(format!("{}: missing model {}", u.name, u.object));
        }
        if !u.script_paths().iter().any(|p| vfs.exists(p)) {
            problems.push(format!("{}: missing script {}", u.name, u.script_path(|_| false)));
        }
        for e in [&u.explode_as, &u.self_destruct_as] {
            if !e.is_empty() && data.weapon(e).is_none() {
                problems.push(format!("{}: unknown explosion weapon {e}", u.name));
            }
        }
        if !u.corpse.is_empty() && data.feature(&u.corpse).is_none() {
            problems.push(format!("{}: unknown corpse feature {}", u.name, u.corpse));
        }
    }
    for f in &data.features {
        if !f.object.is_empty() && !vfs.exists(&format!("objects3d/{}.3do", f.object)) {
            problems.push(format!("feature {}: missing model {}", f.name, f.object));
        }
        if f.is_sprite() && !vfs.exists(&format!("anims/{}.gaf", f.filename)) {
            problems.push(format!("feature {}: missing anims/{}.gaf", f.name, f.filename));
        }
    }
    println!("{} reference problems", problems.len());
    for p in problems.iter().take(40) {
        println!("  {p}");
    }
    let builders: Vec<&ta_data::UnitDef> = data.units.iter().filter(|u| u.builder).collect();
    if menus {
        // (Every unit with a menu: one with build power and no Builder flag has one too. Then
        // what the computer players' lists add to it, `[CANBUILD]`.)
        for u in data.units.iter().filter(|u| u.builder || !u.build_options.is_empty() || !u.ai_build_options.is_empty()) {
            println!("menu {} {}: {}", u.name, u.build_options.len(), u.build_options.join(" "));
            if !u.ai_build_options.is_empty() {
                println!("ai menu {} {}: {}", u.name, u.ai_build_options.len(), u.ai_build_options.join(" "));
            }
        }
    }
    let empty: Vec<&str> = builders.iter().filter(|u| u.build_options.is_empty()).map(|u| u.name.as_str()).collect();
    println!("{} builders, {} without build options: {:?}", builders.len(), empty.len(), empty);
    for name in ["ARMCOM", "CORCOM", "ARMLAB", "CORVP"] {
        if let Some(u) = data.unit(name) {
            println!("{name} builds {}: {:?}", u.build_options.len(), u.build_options);
        }
    }
    // What each side's Commander reaches through the build menus, and what nobody reaches.
    let mut anyone = std::collections::BTreeSet::new();
    for side in &data.sides {
        let mut seen = std::collections::BTreeSet::new();
        let mut queue = vec![side.commander.clone()];
        while let Some(name) = queue.pop() {
            let Some(u) = data.unit(&name) else { continue };
            if seen.insert(u.name.clone()) {
                queue.extend(u.build_options.iter().chain(&u.ai_build_options).cloned());
            }
        }
        println!("{} reaches {} of {} units", side.name, seen.len(), data.units.len());
        anyone.extend(seen);
    }
    let unreached: Vec<&str> = data.units.iter().map(|u| u.name.as_str()).filter(|n| !anyone.contains(*n)).collect();
    println!("{} units no Commander reaches: {:?}", unreached.len(), unreached);
    let mut render: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for w in &data.weapons {
        render.entry(w.raw_render_type).or_default().push(w.name.clone());
    }
    for (rt, names) in render {
        println!("rendertype {rt}: {} weapons, e.g. {:?}", names.len(), &names[..names.len().min(6)]);
    }
    Ok(())
}

/// Lists every campaign and validates its missions against the game data.
pub fn check_missions(vfs: &ta_vfs::Vfs) -> anyhow::Result<()> {
    use ta_data::mission::{self, MissionCommand};
    let data = ta_data::GameData::load(vfs);
    let mut problems = 0;
    // Script pieces that are no command (TA's own slips: a missing comma or letter).
    let (mut unread, mut unread_missions, mut unmade) = (0, 0, 0);
    // Script names that stand for no unit (nor group, nor type where one may be named).
    let mut unnamed_total = 0;
    // Units carrying each of the editor's unit keys, in all missions and schemas.
    let mut key_totals: BTreeMap<&str, usize> = BTreeMap::new();
    for c in mission::campaigns(vfs) {
        println!("{} ({}, {} missions)", c.title, c.side, c.missions.len());
        for (i, entry) in c.missions.iter().enumerate() {
            let Some(m) = mission::Mission::load(vfs, &entry.file) else {
                println!("  {i:2}. {} — MISSING {}", entry.name, entry.file);
                problems += 1;
                continue;
            };
            let map_ok = vfs.exists(&m.map_path());
            let brief_ok = mission::briefing(vfs, &m.brief).is_some();
            let use_only = mission::use_only_units(vfs, &m.use_only);
            let mut unknown: Vec<String> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            // Builds a script asks of a unit that can't make that type (left out).
            let mut unbuildable: Vec<String> = Vec::new();
            let mut unnamed: Vec<String> = Vec::new();
            let mut keys: BTreeMap<&str, usize> = BTreeMap::new();
            let mut scripts = 0;
            for s in &m.schemas {
                // A name in a script: a unit's ident or group in this schema, or (`a`, `g`,
                // `i`) a unit type.
                let names: std::collections::HashSet<String> = s.units.iter().flat_map(|u| [u.ident.trim().to_ascii_uppercase(), u.group.trim().to_ascii_uppercase()]).filter(|n| !n.is_empty()).collect();
                let names_one = |n: &str| names.contains(&n.trim().to_ascii_uppercase());
                for u in &s.units {
                    for (key, on) in [
                        ("AiIgnore", u.ai_ignore),
                        ("AiPriorityTarget", u.ai_priority),
                        ("MissionCriticalUnit", u.critical),
                        ("InitialGroup", !u.group.is_empty()),
                        ("BuildPriority", !u.build_priority.is_empty()),
                        ("OffMapUnit", u.off_map),
                        ("CreationCountdown", u.creation_countdown > 0.0),
                    ] {
                        *keys.entry(key).or_default() += on as usize;
                    }
                    for c in &u.script {
                        let (word, name, typed) = match c {
                            MissionCommand::Attack(n) => ("a", n, true),
                            MissionCommand::Guard(n) => ("g", n, true),
                            MissionCommand::Load(n) => ("i", n, true),
                            MissionCommand::WaitAttacked(Some(n)) => ("wa", n, false),
                            _ => continue,
                        };
                        if !names_one(name) && !(typed && data.unit_id(name.trim()).is_some()) {
                            unnamed.push(format!("{word} {name}"));
                        }
                    }
                    if data.unit_id(&u.name).is_none() {
                        unknown.push(u.name.clone());
                    }
                    scripts += u.script.iter().filter(|c| !matches!(c, MissionCommand::Load(_))).count();
                    skipped.extend(u.unknown.iter().cloned());
                    // (As a mission's computer player builds: its menu and its `[CANBUILD]` list.)
                    let options = data.unit(&u.name).map(|d| d.build_options.iter().chain(&d.ai_build_options).cloned().collect::<Vec<_>>()).unwrap_or_default();
                    for c in &u.script {
                        if let MissionCommand::Build { unit, .. } = c
                            && !options.iter().any(|o| o.eq_ignore_ascii_case(unit))
                        {
                            unbuildable.push(format!("{}:{}", u.name.to_ascii_uppercase(), unit.to_ascii_uppercase()));
                        }
                    }
                }
            }
            unknown.sort();
            unknown.dedup();
            unbuildable.sort();
            unbuildable.dedup();
            unread += skipped.len();
            unread_missions += (!skipped.is_empty()) as usize;
            unmade += unbuildable.len();
            skipped.sort();
            skipped.dedup();
            unnamed.sort();
            unnamed.dedup();
            unnamed_total += unnamed.len();
            keys.retain(|_, n| *n > 0);
            for (k, n) in &keys {
                *key_totals.entry(k).or_default() += n;
            }
            let units: Vec<usize> = m.schemas.iter().map(|s| s.units.len()).collect();
            println!(
                "  {i:2}. {:40} {} units {:?} scripts {scripts} win {:?} lose {:?}{}{}{}{}{}{}{}{}{}",
                entry.name,
                entry.file,
                units,
                m.victory,
                m.defeat,
                m.max_units.map_or(String::new(), |n| format!(" MAXUNITS {n}")),
                if keys.is_empty() { String::new() } else { format!(" KEYS{keys:?}") },
                if map_ok { "" } else { " NO-MAP" },
                if brief_ok { "" } else { " NO-BRIEF" },
                if use_only.is_some() || m.use_only.is_empty() { String::new() } else { format!(" NO-USEONLY({})", m.use_only) },
                if unknown.is_empty() { String::new() } else { format!(" UNKNOWN-UNITS{unknown:?}") },
                if skipped.is_empty() { String::new() } else { format!(" UNREAD-SCRIPT{skipped:?}") },
                if unbuildable.is_empty() { String::new() } else { format!(" UNBUILDABLE{unbuildable:?}") },
                if unnamed.is_empty() { String::new() } else { format!(" UNNAMED{unnamed:?}") },
            );
            problems += (!map_ok) as usize + unknown.len();
        }
    }
    println!("{unread} script pieces in {unread_missions} missions are no command (left out)");
    println!("{unmade} kinds of build a script asks of a unit that can't make it (left out)");
    println!("{unnamed_total} kinds of script name stand for no unit, group or type (`wa`: the unit itself waits; else left out)");
    println!("units carrying the editor's keys: {key_totals:?} (OffMapUnit and CreationCountdown are not acted on)");
    println!("{problems} problems");
    Ok(())
}

/// Writes a 3DO model as Wavefront OBJ for a 3D program: one group per piece at its rest
/// position (the engine's axes: y up, z negated), with an MTL file and the model's textures
/// as PNG (team textures in the first player's colour). The files are made from the
/// player's own install, for looking at; they stay on this computer.
pub fn export_model_obj(vfs: &Vfs, path: &str, out: &Path) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    use std::fmt::Write as _;
    let model = Model3do::parse(&vfs.read(path)?)?;
    let pal = palette(vfs)?;
    let name = Path::new(path).file_stem().map_or_else(|| "model".to_string(), |s| s.to_string_lossy().to_ascii_lowercase());
    std::fs::create_dir_all(out)?;
    // The textures it uses, from textures/*.gaf (the first file holding a name wins).
    let wanted: HashSet<String> = model.texture_names().iter().map(|t| t.to_ascii_lowercase()).collect();
    let mut textures: HashMap<String, String> = HashMap::new();
    for file in vfs.list("textures", Some("gaf"), false) {
        let Ok(gaf) = Gaf::parse(&vfs.read(&file)?) else { continue };
        for entry in &gaf.entries {
            let key = entry.name.to_ascii_lowercase();
            let Some(frame) = entry.frames.first() else { continue };
            if !wanted.contains(&key) || textures.contains_key(&key) || frame.image.width == 0 {
                continue;
            }
            let png = format!("{}.png", sanitize(&entry.name));
            let rgba = frame.image.to_rgba_opaque(&pal);
            image::save_buffer(out.join(&png), &rgba, frame.image.width, frame.image.height, image::ColorType::Rgba8)?;
            textures.insert(key, png);
        }
    }
    // Each piece's place: its offset plus its parents'.
    let mut place = vec![[0f32; 3]; model.pieces.len()];
    for (i, p) in model.pieces.iter().enumerate() {
        let base = p.parent.map_or([0.0; 3], |q| place[q]);
        place[i] = [0, 1, 2].map(|k| base[k] + p.offset[k] as f32 / 65536.0);
    }
    let (mut obj, mut mtl) = (format!("mtllib {name}.mtl\n"), String::new());
    let mut materials: HashSet<String> = HashSet::new();
    let (mut v_base, mut vt_base) = (1usize, 1usize);
    for (i, p) in model.pieces.iter().enumerate() {
        let _ = writeln!(obj, "o {}", sanitize(&p.name));
        for v in &p.vertices {
            let w = [0, 1, 2].map(|k| place[i][k] + v[k] as f32 / 65536.0);
            let _ = writeln!(obj, "v {} {} {}", w[0], w[1], -w[2]);
        }
        for (k, prim) in p.primitives.iter().enumerate() {
            if prim.indices.len() < 3 || Some(k) == p.selection_primitive {
                continue;
            }
            let material = match prim.texture.as_ref().map(|t| t.to_ascii_lowercase()) {
                Some(t) if textures.contains_key(&t) => {
                    if materials.insert(t.clone()) {
                        let _ = writeln!(mtl, "newmtl {t}\nKd 1 1 1\nmap_Kd {}\n", textures[&t]);
                    }
                    t
                }
                _ => {
                    let c = pal.rgb(prim.color_index.min(255) as u8);
                    let m = format!("color{}", prim.color_index);
                    if materials.insert(m.clone()) {
                        let _ = writeln!(mtl, "newmtl {m}\nKd {} {} {}\n", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
                    }
                    m
                }
            };
            // TA lays a texture's corners on a polygon's first four corners (`polygon_uvs`).
            let n = prim.indices.len();
            for c in 0..n {
                let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]][c.min(3)];
                let _ = writeln!(obj, "vt {} {}", uv[0], uv[1]);
            }
            let _ = writeln!(obj, "usemtl {material}");
            // (z negated: the winding flips back to counter-clockwise.)
            let corners: Vec<String> = (0..n).rev().map(|c| format!("{}/{}", v_base + prim.indices[c] as usize, vt_base + c)).collect();
            let _ = writeln!(obj, "f {}", corners.join(" "));
            vt_base += n;
        }
        v_base += p.vertices.len();
    }
    std::fs::write(out.join(format!("{name}.obj")), obj)?;
    std::fs::write(out.join(format!("{name}.mtl")), mtl)?;
    println!("{} pieces, {} textures -> {}", model.pieces.len(), textures.len(), out.join(format!("{name}.obj")).display());
    Ok(())
}
