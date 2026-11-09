//! Which walkers stand level (`ta_sim::defs::big_walker`, `UnitType::level`), against how far the
//! ground would tilt them: every walking unit of the game (and of the mod `TA_TEST_MOD=<mod folder>`
//! names, its own units), its height and its footprint, how much its model reaches out beyond it,
//! the tilt the slope under its footprint gives it over a rough map's dry land (`TA_TEST_MAP`,
//! Cinder Basin by default: at every 40 px, facing four ways) and how far that swings its top. A
//! report, skipped without a TA install: `cargo test --release -p ta_sim --test big_walkers --
//! --ignored --nocapture`.

use std::sync::Arc;

use ta_data::GameData;
use ta_formats::model3do::{FIXED_ONE, Model3do};
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, Locomotion, UnitTypes};

fn game_and_mod() -> Option<(ta_vfs::Vfs, Option<String>)> {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return None;
    };
    let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).expect("TA_TEST_MOD: a mod's folder").expect("TA_TEST_MOD: a mod's folder"));
    if let Some(info) = &module {
        println!("mod: {} {}", info.manifest.name, info.manifest.version);
    }
    // (As the game mounts it: what it replaces of the install left out.)
    let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
    mounted.install.unwrap();
    mounted.mods.expect("the mod mounts");
    Some((mounted.vfs, module.map(|m| m.manifest.id)))
}

/// How far out a model's soles reach at rest, across and along (its drawn faces' vertices within
/// 2 px of the ground; flashes and fire points left out).
fn stance(m: &Model3do) -> (f64, f64) {
    let mut at: Vec<[f64; 3]> = Vec::new();
    let (mut across, mut along) = (0.0f64, 0.0f64);
    let one = FIXED_ONE as f64;
    for p in &m.pieces {
        let base = p.parent.map_or([0.0; 3], |q| at[q]);
        let o = [base[0] + p.offset[0] as f64 / one, base[1] + p.offset[1] as f64 / one, base[2] - p.offset[2] as f64 / one];
        at.push(o);
        let n = p.name.to_ascii_lowercase();
        if ["flare", "flash", "fire", "muzzle", "emit", "beam", "spray", "smoke", "puff"].iter().any(|k| n.contains(k)) {
            continue;
        }
        for (j, prim) in p.primitives.iter().enumerate() {
            if Some(j) == p.selection_primitive || prim.indices.len() < 3 {
                continue;
            }
            for v in prim.indices.iter().filter_map(|&i| p.vertices.get(i as usize)) {
                if (o[1] + v[1] as f64 / one).abs() <= 2.0 {
                    across = across.max((o[0] + v[0] as f64 / one).abs());
                    along = along.max((o[2] - v[2] as f64 / one).abs());
                }
            }
        }
    }
    (across, along)
}

#[test]
#[ignore]
fn big_walkers() {
    let Some((vfs, mod_id)) = game_and_mod() else { return };
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map_name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Cinder Basin".into());
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).expect("the map")).unwrap();
    let world = ta_sim::world::World::new(&map);
    let (w, h) = world.size_px();
    let (w, h) = (w.to_int(), h.to_int());
    // A footprint's tilt (degrees) as `Sim::settle` gives it, over the map's dry land: the 95th
    // percentile and the most.
    let mut tilts: std::collections::BTreeMap<(u32, u32), (f64, f64)> = std::collections::BTreeMap::new();
    let mut tilt = |fp: (u32, u32)| {
        *tilts.entry(fp).or_insert_with(|| {
            let (half_l, half_w) = (fp.1.clamp(1, 8) as f64 * 8.0, fp.0.clamp(1, 8) as f64 * 8.0);
            let ground = |x: f64, z: f64| world.height_at(Fx::from_f64(x), Fx::from_f64(z)).to_f64();
            let mut all = Vec::new();
            for z in (200..h - 200).step_by(40) {
                for x in (200..w - 200).step_by(40) {
                    let (x, z) = (x as f64, z as f64);
                    if ground(x, z) <= world.sea_level.to_f64() {
                        continue;
                    }
                    for k in 0..4 {
                        let (s, c) = (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos();
                        let pitch = (ground(x + s * half_l, z + c * half_l) - ground(x - s * half_l, z - c * half_l)).atan2(half_l * 2.0);
                        let roll = (ground(x + c * half_w, z - s * half_w) - ground(x - c * half_w, z + s * half_w)).atan2(half_w * 2.0);
                        all.push(pitch.hypot(roll).to_degrees());
                    }
                }
            }
            all.sort_by(|a, b| a.total_cmp(b));
            (all[(all.len() - 1) * 95 / 100], all[all.len() - 1])
        })
    };
    let ours = |t: &ta_sim::UnitType| {
        let object = t.def.object.to_ascii_lowercase();
        let files = [format!("units/{}.fbi", t.name.to_ascii_lowercase()), format!("objects3d/{object}.3do")].into_iter().chain(t.def.script_paths());
        mod_id.as_ref().is_none_or(|id| files.into_iter().any(|p| vfs.origin(&p).is_some_and(|o| o.layer == *id)))
    };
    let mut rows = Vec::new();
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Ground && ours(t)) {
        let walks = matches!(t.def.tedclass.as_str(), "KBOT" | "COMMANDER") || t.def.has_category("KBOT") || t.move_class.is_some_and(|c| data.move_classes[c].max_slope >= 255.0);
        if !walks {
            continue;
        }
        let Some(m) = vfs.read_opt(&format!("objects3d/{}.3do", t.def.object.to_ascii_lowercase())).and_then(|b| Model3do::parse(&b).ok()) else { continue };
        let (across, along) = stance(&m);
        let (p95, most) = tilt(t.footprint);
        let height = t.model.height.to_f64();
        let sway = height * p95.to_radians().sin();
        let verdict = match (t.big_walker, t.def.upright) {
            (true, false) => "BIG: stands level now",
            (true, true) => "big, upright already",
            (false, true) => "upright",
            (false, false) => "tilts with the ground",
        };
        rows.push((
            (!t.big_walker, -height as i64),
            format!(
                "{:<12} {:<24.24} {:<9} {:<18.18} fp {}x{} h {:>4.0} soles +-{:>3.0} x +-{:>3.0}  tilt p95 {:>4.1} max {:>4.1}  top swings {:>5.1}  {verdict}",
                t.name, t.def.display_name, t.def.tedclass, t.def.movement_class, t.footprint.0, t.footprint.1, height, across, along, p95, most, sway
            ),
        ));
    }
    rows.sort_by_key(|r| r.0);
    for (_, r) in &rows {
        println!("{r}");
    }
    let big: Vec<&str> = types.types.iter().filter(|t| t.big_walker && !t.def.upright && ours(t)).map(|t| t.name.as_str()).collect();
    println!("{} walkers; stand level now: {big:?}", rows.len());
}
