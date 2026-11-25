//! The player's own copy of Forged Alliance, its units read whole: every blueprint, and for every
//! unit a player can make, its meshes, textures and animations, and the bones its weapons and
//! build beams name; the conventions the conversion rests on; every unit made a TA model the
//! same, byte for byte, twice; and the sides' emblems (in TA's palette: TA's copy found too).
//! Ignored (it needs the game installed, and takes a while):
//! `cargo test --release -p ta_supcom --test installed_units -- --ignored --nocapture`
//! (`TA_SUPCOM_DIR=<folder>` a copy of the game elsewhere).

use std::collections::BTreeMap;
use std::path::PathBuf;

use ta_supcom::files::Files;
use ta_supcom::install::{self, Install, Store};
use ta_formats::surface::Material;
use ta_supcom::factions::{anims, enhance, faction, make, model, pictures, textures, units, work};
use ta_supcom::unit::{self, Faction, Tech};
use ta_supcom::{sca, scm};

fn the_game() -> Option<Install> {
    if let Some(dir) = std::env::var_os("TA_SUPCOM_DIR") {
        return Some(Install { root: PathBuf::from(dir), store: Store::Folder });
    }
    install::discover().into_iter().next()
}

/// TA's own palette, from the copy of TA found (the sides' emblems are made in it): none without
/// one.
fn ta_palette() -> Option<ta_formats::image::Palette> {
    let ta = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&ta.root, "base").ok()?;
    ta_formats::image::Palette::parse(&vfs.read_opt("palettes/palette.pal")?).ok()
}

/// Every unit blueprint reads, and the four factions have what a player may make of them.
#[test]
#[ignore]
fn every_unit_blueprint_reads() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let listed = files.list("units/").iter().filter(|p| p.ends_with("_unit.bp")).count();
    eprintln!("{} of {listed} unit blueprints read", all.len());
    assert_eq!(all.len(), listed, "every blueprint reads");
    let playable: Vec<_> = all.iter().filter(|u| u.playable()).collect();
    let mut by_faction: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for u in &playable {
        *by_faction.entry(u.faction.name().to_string()).or_default().entry(format!("{:?}", u.tech)).or_default() += 1;
    }
    for (faction, tiers) in &by_faction {
        eprintln!("{faction}: {} units {tiers:?}", tiers.values().sum::<usize>());
    }
    for faction in Faction::PLAYABLE {
        let theirs: Vec<_> = playable.iter().filter(|u| u.faction == faction).collect();
        assert!(theirs.len() > 50, "{}: {} units", faction.name(), theirs.len());
        assert_eq!(theirs.iter().filter(|u| u.tech == Some(Tech::Commander) && u.is("COMMAND")).count(), 1, "{}: one commander", faction.name());
    }
    let nameless: Vec<&str> = playable.iter().filter(|u| u.name.is_empty() && u.description.is_empty()).map(|u| u.id.as_str()).collect();
    eprintln!("playable units with neither a name nor a description: {nameless:?}");
    // What each builder makes, by its lists' categories, and units nobody makes.
    let mut made = std::collections::BTreeSet::new();
    for builder in playable.iter().filter(|u| !u.buildable.is_empty()) {
        for u in &playable {
            if builder.buildable.iter().any(|e| u.matches(e)) {
                made.insert(u.id.clone());
            }
        }
    }
    let orphans: Vec<&str> = playable.iter().filter(|u| !made.contains(&u.id) && !u.is("COMMAND")).map(|u| u.id.as_str()).collect();
    eprintln!("{} playable units made by some builder; made by none: {orphans:?}", made.len());
}

/// Every unit a player can make: its meshes read, its textures are there, the bones its weapons
/// and build beams name are its mesh's, and its walk and deaths read and move its mesh's bones.
#[test]
#[ignore]
fn every_playable_units_meshes_and_animations_read() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let playable: Vec<_> = all.into_iter().filter(|u| u.playable()).collect();
    let (mut meshes, mut animations, mut vertices, mut problems) = (0usize, 0usize, 0usize, Vec::new());
    let mut largest = (0usize, String::new());
    for u in &playable {
        let Some(lod0) = u.display.lods.first() else {
            problems.push(format!("{}: no mesh", u.id));
            continue;
        };
        let mesh = match files.read(&lod0.mesh).unwrap() {
            Some(bytes) => scm::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", lod0.mesh)),
            None => {
                problems.push(format!("{}: no {}", u.id, lod0.mesh));
                continue;
            }
        };
        meshes += 1;
        vertices += mesh.vertices.len();
        if mesh.vertices.len() > largest.0 {
            largest = (mesh.vertices.len(), u.id.clone());
        }
        for lod in &u.display.lods[1..] {
            if let Some(bytes) = files.read(&lod.mesh).unwrap() {
                scm::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", lod.mesh));
                meshes += 1;
            }
        }
        for texture in [&lod0.albedo, &lod0.specular, &lod0.normals] {
            if files.read(texture).unwrap().is_none() {
                problems.push(format!("{}: no {texture}", u.id));
            }
        }
        let mut bones: Vec<(String, &str)> = Vec::new();
        for w in &u.weapons {
            for b in [&w.yaw_bone, &w.pitch_bone, &w.muzzle_bone].into_iter().flatten() {
                bones.push((b.clone(), "weapon"));
            }
            for rack in &w.racks {
                if !rack.rack.is_empty() {
                    bones.push((rack.rack.clone(), "rack"));
                }
                bones.extend(rack.muzzles.iter().map(|m| (m.clone(), "muzzle")));
            }
        }
        if let Some((yaw, pitch, effects)) = &u.build_bones {
            bones.extend([yaw, pitch].into_iter().flatten().map(|b| (b.clone(), "build")));
            bones.extend(effects.iter().map(|b| (b.clone(), "build effect")));
        }
        for (bone, what) in bones {
            if mesh.bone(&bone).is_none() {
                problems.push(format!("{}: its {what} bone {bone} is not its mesh's", u.id));
            }
        }
        let anims = u.display.walk.iter().map(|(a, _)| a).chain(&u.display.deaths);
        for path in anims {
            let Some(bytes) = files.read(path).unwrap() else {
                problems.push(format!("{}: no {path}", u.id));
                continue;
            };
            let anim = sca::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
            animations += 1;
            let strangers: Vec<&str> = anim.bones.iter().filter(|b| mesh.bone(b).is_none()).map(String::as_str).collect();
            if !strangers.is_empty() {
                problems.push(format!("{}: {path} moves bones its mesh has not: {strangers:?}", u.id));
            }
        }
    }
    eprintln!("{} playable units: {meshes} meshes ({vertices} vertices in their first levels; the largest {} with {}), {animations} animations", playable.len(), largest.1, largest.0);
    eprintln!("{} problems:", problems.len());
    for p in &problems {
        eprintln!("  {p}");
    }
}

/// What the conversion takes the meshes to be (docs/SUPCOM_UNITS.md), held over every unit a
/// player can make: the first vector of a vertex's frame is its normal (the triangles' corners
/// in order wind anticlockwise about it), the next two run against the texture's u and v; a
/// bone's bind pose (the inverse of the file's matrix) is its rest pose (its place and turn
/// composed down the skeleton); the units face +z, their right at -x.
#[test]
#[ignore]
fn the_meshes_are_as_the_conversion_takes_them() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let (mut wound, mut against, mut triangles, mut apart) = (0usize, 0usize, 0usize, Vec::new());
    let mut meshes = BTreeMap::new();
    for u in all.iter().filter(|u| u.playable()) {
        let Some(lod0) = u.display.lods.first() else { continue };
        let Some(bytes) = files.read(&lod0.mesh).unwrap() else { continue };
        let mesh = scm::parse(&bytes).unwrap();
        for t in &mesh.triangles {
            let p = |i: u32| mesh.vertices[i as usize].position;
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let v = &mesh.vertices[t[0] as usize];
            if n.iter().map(|x| x * x).sum::<f32>() < 1e-12 {
                continue;
            }
            triangles += 1;
            wound += (n[0] * v.normal[0] + n[1] * v.normal[1] + n[2] * v.normal[2] > 0.0) as usize;
            // (The tangent against u: along the corners' u, the place goes the other way.)
            let (du1, du2) = (mesh.vertices[t[1] as usize].uv0[0] - v.uv0[0], mesh.vertices[t[2] as usize].uv0[0] - v.uv0[0]);
            let (dv1, dv2) = (mesh.vertices[t[1] as usize].uv0[1] - v.uv0[1], mesh.vertices[t[2] as usize].uv0[1] - v.uv0[1]);
            let det = du1 * dv2 - du2 * dv1;
            if det.abs() > 1e-9 {
                let dpdu = [0, 1, 2].map(|k| (e1[k] * dv2 - e2[k] * dv1) / det);
                against += (dpdu[0] * v.tangent[0] + dpdu[1] * v.tangent[1] + dpdu[2] * v.tangent[2] < 0.0) as usize;
            }
        }
        let rest = mesh.rest_poses();
        let size = mesh.vertices.iter().fold(1e-6f32, |m, v| m.max(v.position.iter().fold(0f32, |a, c| a.max(c.abs()))));
        for (b, bone) in mesh.bones.iter().enumerate() {
            let m = bone.rest_inverse;
            let t = [m[12], m[13], m[14]];
            let bind = [0, 1, 2].map(|k| -(t[0] * m[4 * k] + t[1] * m[4 * k + 1] + t[2] * m[4 * k + 2]));
            let d = (0..3).map(|k| (bind[k] - rest[b].0[k]).powi(2)).sum::<f32>().sqrt();
            if d > size * 0.02 && mesh.vertices.iter().any(|v| v.bone == b) {
                apart.push(format!("{} {}", u.id, bone.name));
            }
        }
        meshes.insert(u.id.clone(), mesh);
    }
    eprintln!("{wound} of {triangles} triangles wound about their normals, {against} with tangents against u");
    assert!(wound * 100 >= triangles * 99, "the normal comes first");
    assert!(against * 100 >= triangles * 90, "the tangent runs against u");
    assert!(apart.is_empty(), "bones bound away from their rest: {apart:?}");
    // The commanders' right arms at -x, their left at +x; the Striker's gun forward of its hull.
    let place = |id: &str, bone: &str| -> [f32; 3] {
        let mesh = &meshes[id];
        mesh.rest_poses()[mesh.bone(bone).unwrap_or_else(|| panic!("{id} has no {bone}"))].0
    };
    assert!(place("XSL0001", "Right_Arm_B01")[0] < 0.0 && place("XSL0001", "Left_Arm_B01")[0] > 0.0);
    assert!(place("UAL0001", "Right_Turret_Muzzle")[0] < 0.0 && place("UAL0001", "Left_Turret_Muzzle")[0] > 0.0);
    assert!(place("UEL0201", "Turret_Muzzle")[2] > 0.0 && place("UEL0201", "Exhaust_Left")[2] < 0.0);
}

/// Every unit a player can make made a TA model with its surface, its paint, its model for far
/// off and its build picture, as `ta-tools supcom-units` makes them: each surface fits its model
/// as written and read back, the far model has the near one's pieces, every unit has its icon,
/// and making them again gives the same bytes (a mod of units is compared byte for byte before a
/// multiplayer battle).
#[test]
#[ignore]
fn every_unit_is_made_a_model_the_same_twice() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    // (Triangles whose texture coordinates were moved into the texture, and those left straddling
    // its edge; the units with a far model.)
    let mut uvs = (0usize, 0usize);
    let mut fars = 0usize;
    // (The vertices of the meshes with a far one: the near meshes' and the far ones'.)
    let mut far_vertices = (0usize, 0usize);
    let mut far_uvs = (0usize, 0usize);
    let mut make = |u: &unit::UnitBlueprint| -> Vec<Vec<u8>> {
        let lod0 = &u.display.lods[0];
        let mesh = scm::parse(&files.read(&lod0.mesh).unwrap().unwrap()).unwrap();
        let paint: Vec<Vec<u8>> = [&lod0.albedo, &lod0.specular, &lod0.normals].iter().map(|p| files.read(p).unwrap().map(|b| textures::texture(&b).unwrap()).unwrap_or_default()).collect();
        let material = Material { name: u.id.to_ascii_lowercase(), albedo: "a".into(), specular: "s".into(), normals: "n".into(), mean: textures::mean(&paint[0]) };
        let near = model::convert(&mesh, u.display.uniform_scale, material.clone()).unwrap();
        uvs.0 += near.uvs_moved;
        uvs.1 += near.uvs_straddling;
        let mut out = vec![near.model.to_bytes(), near.surface.to_bytes()];
        let back = ta_formats::model3do::Model3do::parse(&out[0]).unwrap();
        assert!(near.surface.fits(&back), "{}: its surface fits its model as read back", u.id);
        if let Some(bytes) = u.display.lods.get(1).and_then(|l| files.read(&l.mesh).unwrap()) {
            let far_mesh = scm::parse(&bytes).unwrap();
            let far = model::convert_onto(&far_mesh, &mesh, &near, u.display.uniform_scale, material).unwrap();
            fars += 1;
            far_vertices.0 += mesh.vertices.len();
            far_vertices.1 += far_mesh.vertices.len();
            far_uvs.0 += far.uvs_moved;
            far_uvs.1 += far.uvs_straddling;
            let far_back = ta_formats::model3do::Model3do::parse(&far.model.to_bytes()).unwrap();
            assert!(far.surface.fits(&far_back), "{}: the far surface fits", u.id);
            assert_eq!(far_back.pieces.iter().map(|p| &p.name).collect::<Vec<_>>(), back.pieces.iter().map(|p| &p.name).collect::<Vec<_>>(), "{}: the far model's pieces", u.id);
            out.extend([far.model.to_bytes(), far.surface.to_bytes()]);
        }
        out.extend(paint);
        let icon = files.read(&pictures::icon_path(u)).unwrap().unwrap_or_else(|| panic!("{}: no icon", u.id));
        let frame = pictures::frame_path(u).and_then(|p| files.read(&p).unwrap());
        assert!(frame.is_some(), "{}: no frame for its kind", u.id);
        out.push(pictures::picture(&icon, frame.as_deref()).unwrap());
        out
    };
    let playable: Vec<_> = all.iter().filter(|u| u.playable() && !u.display.lods.is_empty()).collect();
    let mut bytes = 0usize;
    for u in &playable {
        let first = make(u);
        bytes += first.iter().map(Vec::len).sum::<usize>();
        assert!(first == make(u), "{}: made again, the same bytes", u.id);
    }
    eprintln!("{} units made twice the same, {} MB each time", playable.len(), bytes >> 20);
    eprintln!("{} with a far model ({} vertices near, {} far); {} triangles moved into their texture, {} straddling its edge (far models: {} and {})", fars / 2, far_vertices.0 / 2, far_vertices.1 / 2, uvs.0 / 2, uvs.1 / 2, far_uvs.0 / 2, far_uvs.1 / 2);
}



/// The whole mod, as `ta-tools supcom-units` and the launcher make it, made twice from the
/// player's copy: every unit made, every file the same both times (each file's path, size and
/// hash compared in the order they come), each unit with its model, surface, paint, picture,
/// unit file and script, and its factions' side, menus, wrecks and weapons; their sides' emblems
/// too, where a copy of TA is found (they are made in its palette).
#[test]
#[ignore]
fn the_mod_is_made_the_same_twice() {
    use std::hash::{Hash, Hasher};
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let palette = ta_palette();
    // (Its sounds too, from the copy's own banks.)
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds");
    let run = || {
        let mut out: Vec<(String, usize, u64)> = Vec::new();
        let made = make::make(&files, &units, palette.as_ref(), sounds.as_deref(), "test", &mut |path, bytes| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut h);
            out.push((path.to_string(), bytes.len(), h.finish()));
            Ok(())
        }, &mut |_, _| true)
        .unwrap();
        (made, out)
    };
    let (made, first) = run();
    assert_eq!(made.units, units.len(), "every unit made: {:?}", made.skipped);
    let (_, second) = run();
    assert!(first == second, "made again, the same files");
    // Stopped by its watch before the third unit: two made, and said so.
    let stopped = make::make(&files, &units, None, None, "test", &mut |_, _| Ok(()), &mut |i, _| i < 2).unwrap();
    assert!(stopped.stopped && stopped.units == 2, "{stopped:?}");
    for u in &units {
        let id = u.id.to_ascii_lowercase();
        for path in [format!("objects3d/{id}.3do"), format!("objects3d/{id}.surface"), format!("units/{id}.fbi"), format!("scripts/{id}.cob"), format!("unitpics/{id}.pcx")] {
            assert!(first.iter().any(|(p, ..)| *p == path), "{path}");
        }
    }
    for path in ["gamedata/sidedata_supcom.tdf", "download/supcom_uef.tdf", "features/corpses/supcom_cybran.tdf", "weapons/supcom_seraphim.tdf", "mod.toml"] {
        assert!(first.iter().any(|(p, ..)| p == path), "{path}");
    }
    // (The four sides' emblems in the classic interface, made in TA's palette where TA is found.)
    if palette.is_some() {
        assert!(first.iter().any(|(p, ..)| *p == ta_supcom::factions::emblem::path()), "the emblems");
    } else {
        eprintln!("no copy of TA found: the emblems not made");
    }
    eprintln!("{} files, {} MB, the same twice", first.len(), first.iter().map(|f| f.1).sum::<usize>() >> 20);
}

/// The four sides' emblems in the classic interface, made of the game's own in TA's palette: an
/// entry a side in the game's order, ten frames of 32 × 32 each, the shape the same in every
/// player's colour (no other pixel of that colour) and across a good part of the frame.
#[test]
#[ignore]
fn the_sides_emblems_are_made_of_the_games_own() {
    use ta_supcom::factions::emblem;
    let (Some(game), Some(palette)) = (the_game(), ta_palette()) else {
        eprintln!("no copy of Forged Alliance, or of TA, found: nothing to make");
        return;
    };
    let files = Files::own(&game).unwrap();
    let sides = ["UEF", "CYBRAN", "AEON", "SERAPHIM"];
    let gaf = ta_formats::gaf::Gaf::parse(&emblem::gaf(&files, &sides, &palette).unwrap().expect("made")).unwrap();
    assert_eq!(gaf.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), sides.map(emblem::entry));
    for e in &gaf.entries {
        assert_eq!(e.frames.len(), 10, "{}", e.name);
        let shapes: Vec<Vec<bool>> = e.frames.iter().enumerate().map(|(p, f)| f.image.pixels.iter().map(|&i| i == emblem::PLAYERS[p]).collect()).collect();
        let filled = shapes[0].iter().filter(|&&s| s).count();
        eprintln!("{}: {filled} pixels of the player's colour", e.name);
        assert!(e.frames.iter().all(|f| (f.image.width, f.image.height) == (emblem::SIDE, emblem::SIDE)), "{}", e.name);
        assert!(shapes.iter().all(|s| *s == shapes[0]), "{}: the same shape in every colour", e.name);
        assert!(filled * 7 > 32 * 32, "{}: {filled} pixels", e.name);
    }
}

/// Every unit a player can make is reached from its faction's commander: what each builder
/// builds (its lists' categories), what each upgrades into in place (the tech 2 and 3 factories,
/// which nothing builds outright), what its enhancements add to its menu (the engineering
/// suites'), the drones each keeps (a station's, a commander's pods), and what those build and
/// upgrade into, all the way down.
#[test]
#[ignore]
fn every_playable_unit_is_reached_from_its_commander() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    for f in Faction::PLAYABLE {
        let theirs: Vec<&unit::UnitBlueprint> = all.iter().filter(|u| u.playable() && u.faction == f).collect();
        let lists = faction::all_builds(&theirs);
        let commander = faction::commander(&theirs).expect("a commander");
        let mut reached: Vec<String> = vec![commander.id.clone()];
        let mut upgrades = 0;
        let mut by_enhancements: Vec<String> = Vec::new();
        let mut k = 0;
        while k < reached.len() {
            if let Some((_, built)) = lists.iter().find(|(b, _)| b.id == reached[k]) {
                for u in built {
                    if !reached.contains(&u.id) {
                        reached.push(u.id.clone());
                    }
                }
            }
            if let Some(u) = theirs.iter().find(|u| u.id == reached[k]) {
                // (Its drones, made by it: a station's, TA Reborn's `[DRONES]`.)
                for d in units::station_drones(u) {
                    if let Some(b) = theirs.iter().find(|b| b.id.eq_ignore_ascii_case(&d.id))
                        && !reached.contains(&b.id)
                    {
                        reached.push(b.id.clone());
                    }
                }
                let own = units::weapons(u);
                let extra = enhance::extra_weapons(u, &own);
                for e in enhance::enhancements(u, &theirs, &own, &extra).0 {
                    // (The drones it gives: a commander's pods.)
                    for d in &e.drones {
                        if let Some(b) = theirs.iter().find(|b| b.id.eq_ignore_ascii_case(&d.id))
                            && !reached.contains(&b.id)
                        {
                            reached.push(b.id.clone());
                        }
                    }
                    for o in e.build_options {
                        if let Some(b) = theirs.iter().find(|b| b.id.eq_ignore_ascii_case(&o))
                            && !reached.contains(&b.id)
                        {
                            reached.push(b.id.clone());
                            by_enhancements.push(b.id.clone());
                        }
                    }
                }
            }
            if let Some(u) = theirs.iter().find(|u| u.id == reached[k]).and_then(|u| faction::upgrade_of(u, &theirs)) {
                upgrades += 1;
                if !reached.contains(&u.id) {
                    reached.push(u.id.clone());
                }
            }
            k += 1;
        }
        let missing: Vec<&str> = theirs.iter().filter(|u| !reached.contains(&u.id)).map(|u| u.id.as_str()).collect();
        eprintln!("{}: {} of {} reached from {} ({upgrades} upgrade in place; {} first through its enhancements' menus)", f.name(), reached.len(), theirs.len(), commander.id, by_enhancements.len());
        // Nothing builds what is made by upgrading alone: the factories of tech 2 and 3.
        for (b, built) in &lists {
            for u in built {
                assert!(!(u.is("FACTORY") && u.motion == unit::Motion::None && matches!(u.tech, Some(unit::Tech::T2 | unit::Tech::T3)) && u.upgrades_from.is_some()), "{}: {} builds {}", f.name(), b.id, u.id);
            }
        }
        assert!(missing.is_empty(), "{}: not reached: {missing:?}", f.name());
        // The commander builds its own tier: not what its engineering enhancements bring.
        let own = faction::builds(commander, &theirs);
        assert!(!own.iter().any(|u| u.is("BUILTBYTIER2COMMANDER") || u.is("BUILTBYTIER3COMMANDER")), "{}: {:?}", f.name(), own.iter().map(|u| &u.id).collect::<Vec<_>>());
    }
}

/// The commanders' and support commanders' enhancements as TA Reborn plays them: every commander
/// has its engineering suites (tech 2, then tech 3 over it, units on their menus that FA keeps
/// from the commander until then) and a gun upgrade; every prerequisite is among them, in the
/// same slot where it is a suite; each has a price, and an effect.
#[test]
#[ignore]
fn the_commanders_enhancements_are_played() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let mut total = 0;
    for f in Faction::PLAYABLE {
        let theirs: Vec<&unit::UnitBlueprint> = all.iter().filter(|u| u.playable() && u.faction == f).collect();
        let commander = faction::commander(&theirs).expect("a commander");
        for u in theirs.iter().filter(|u| u.table.path(&["Enhancements"]).is_some()) {
            let own = units::weapons(u);
            let extra = enhance::extra_weapons(u, &own);
            let (list, variants) = enhance::enhancements(u, &theirs, &own, &extra);
            total += list.len();
            let line: Vec<String> = list
                .iter()
                .map(|e| format!("{} [{}{}] {}m {}e {}t{}{}{}{}", e.id, e.slot, e.prerequisite.as_ref().map(|p| format!(" after {p}")).unwrap_or_default(), e.cost_mass, e.cost_energy, e.build_time,
                    if e.build_options.is_empty() { String::new() } else { format!(" +{} units", e.build_options.len()) },
                    if e.weapons.is_empty() { String::new() } else { format!(" weapons {:?}", e.weapons) },
                    if e.max_damage > 0.0 { format!(" +{} hp", e.max_damage) } else { String::new() },
                    if e.worker_time > 0.0 { format!(" +{} build", e.worker_time) } else { String::new() }))
                .collect();
            eprintln!("{} {}: {} enhancements, {} weapon variants\n  {}", f.name(), u.id, list.len(), variants.sections.len(), line.join("\n  "));
            for e in &list {
                assert!(e.cost_mass > 0.0 || e.cost_energy > 0.0, "{}: {} has a price", u.id, e.id);
                assert!(!e.slot.is_empty(), "{}: {} has a slot", u.id, e.id);
                if let Some(p) = &e.prerequisite {
                    assert!(list.iter().any(|o| o.id.eq_ignore_ascii_case(p)), "{}: {} needs {p}, which is played", u.id, e.id);
                }
            }
            if u.id == commander.id {
                let suites: Vec<_> = list.iter().filter(|e| !e.build_options.is_empty()).collect();
                assert!(suites.len() >= 2, "{}: its engineering suites", u.id);
                let tech2 = theirs.iter().filter(|b| b.is("BUILTBYTIER2COMMANDER")).count();
                assert!(tech2 == 0 || suites.iter().any(|e| e.build_options.len() >= tech2.min(1)), "{}: the suites' units", u.id);
                let main = own.slots.iter().position(|w| units::kind(w) != units::Kind::Overcharge).expect("a gun");
                assert!(list.iter().any(|e| e.weapons.iter().any(|(slot, _)| *slot == main)), "{}: a gun upgrade", u.id);
            }
        }
    }
    eprintln!("{total} enhancements in all");
}

/// A weapon section as TA Reborn reads it: its keys.
fn keys(section: &str) -> ta_formats::tdf::TdfSection {
    ta_formats::tdf::parse(section).sections.remove(0)
}

/// Every weapon slot of every unit a player can make, by Forged Alliance's firing rules: its
/// damage a second in TA's keys Forged Alliance's (within 2%, its reload in whole ticks); a missile
/// its launcher stores is TA's stockpile at its projectile's price and build time; missile
/// defences take their own class (an anti-nuke the strategic missiles, the rest the tactical) and
/// missiles are of theirs.
#[test]
#[ignore]
fn the_weapons_follow_forged_alliances_rules() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let (mut slots, mut stocked, mut defences, mut off) = (0usize, Vec::new(), Vec::new(), Vec::new());
    let mut worst: Vec<(f64, String)> = Vec::new();
    for u in all.iter().filter(|u| u.playable()) {
        let w = units::weapons(u);
        for slot in &w.slots {
            let section = units::weapon_section(u, slot);
            let k = keys(&section);
            let name = units::weapon_name(u, slot);
            let num = |key: &str| k.get_f64_or(key, 0.0);
            if units::stockpiled(slot) {
                let shot = slot.shot.as_ref().expect("a stored missile's projectile");
                let seconds = shot.build_time / u.build_rate.max(1e-9);
                assert!(k.get_bool_or("stockpile", false), "{name}");
                assert_eq!((num("metalpershot"), num("energypershot")), (shot.cost_mass.round(), shot.cost_energy.round()), "{name}");
                assert!((num("reloadtime") - seconds).abs() <= 1.0 / 30.0 + 1e-6, "{name}: {} against {seconds}", num("reloadtime"));
                stocked.push(format!("{name} {}+{} {:.0}s", shot.cost_mass, shot.cost_energy, seconds));
            }
            let class = |key: &str| k.get_i64_or(key, 0) as u32;
            if units::kind(slot) == units::Kind::AntiMissile {
                let want = if slot.only_allow.to_ascii_uppercase().contains("STRATEGIC") { units::STRATEGIC } else { units::TACTICAL };
                assert_eq!(class("interceptor"), want, "{name}: {}", slot.only_allow);
                // (A tactical defence's shot leaves its muzzle at the speed to meet a missile coming
                // down within its range: the zapper's beam and the flares too.)
                if want == units::TACTICAL {
                    assert!(num("weaponvelocity") >= 2880.0 && num("startvelocity") == num("weaponvelocity"), "{name}: {section}");
                }
                defences.push(format!("{name}={want} at {}", num("weaponvelocity")));
                continue;
            }
            match slot.shot.as_ref() {
                Some(s) if s.strategic_missile() => assert_eq!(class("targetable"), units::STRATEGIC, "{name}"),
                Some(s) if s.tactical_missile() => assert_eq!(class("targetable"), units::TACTICAL, "{name}"),
                _ => assert_eq!(class("targetable"), u32::from(slot.nuke.is_some()), "{name}"),
            }
            if units::stockpiled(slot) || slot.nuke.is_some() {
                continue;
            }
            slots += 1;
            let fa = units::fa_dps(slot);
            let ticks = (num("reloadtime") * 30.0).round().max(1.0);
            let ta = k.section("DAMAGE").map_or(0.0, |d| d.get_f64_or("default", 0.0)) * k.get_i64_or("burst", 1).max(1) as f64 / (ticks / 30.0);
            let ratio = if fa > 0.0 { ta / fa } else { 1.0 };
            worst.push(((ratio - 1.0).abs(), format!("{name}: FA {fa:.1}, TA {ta:.1}")));
            if (ratio - 1.0).abs() > 0.02 {
                off.push(format!("{name}: FA {fa:.1}, TA {ta:.1}"));
            }
        }
        // What its script switches off as it is made is not in its slots (but the Overcharge).
        for slot in &w.slots {
            assert!(units::kind(slot) == units::Kind::Overcharge || !u.birth.weapon_off(&slot.label), "{}: {} is off at birth", u.id, slot.label);
        }
    }
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    eprintln!("{slots} weapon slots by damage a second; the furthest from Forged Alliance's: {:?}", worst.iter().take(5).map(|w| &w.1).collect::<Vec<_>>());
    eprintln!("{} stored missiles: {stocked:?}", stocked.len());
    eprintln!("{} missile defences: {defences:?}", defences.len());
    assert!(off.is_empty(), "{} slots off Forged Alliance's damage a second by more than 2%: {off:?}", off.len());
}

/// What the units' scripts switch off as they are made: weapons (a second, toggled gun; a missile
/// an enhancement brings) and intel (the Cybran commander's stealth and sonar), listed.
#[test]
#[ignore]
fn what_the_scripts_switch_off_at_birth() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    for u in all.iter().filter(|u| u.playable()) {
        let b = &u.birth;
        if !b.weapons_off.is_empty() || !b.intel_off.is_empty() || !b.by_layer.is_empty() {
            eprintln!("{}: weapons off {:?}, by layer {:?}, intel off {:?}", u.id, b.weapons_off, b.by_layer, b.intel_off);
        }
    }
    let xsl0001 = all.iter().find(|u| u.id == "XSL0001").unwrap();
    assert!(xsl0001.birth.weapon_off("Missile"), "{:?}", xsl0001.birth);
    let url0001 = all.iter().find(|u| u.id == "URL0001").unwrap();
    assert!(url0001.birth.intel_off("RadarStealth") && url0001.birth.intel_off("Sonar"), "{:?}", url0001.birth);
    let url0203 = all.iter().find(|u| u.id == "URL0203").unwrap();
    assert!(!url0203.birth.weapon_off("Torpedo") && !url0203.birth.weapon_off("Bolter"), "the Wagner's by its layer: {:?}", url0203.birth);
}

/// Every walk clip is a cycle: where its last frame ends it (at its length), that frame is its
/// first, the difference Forged Alliance's own walks leave (UEL0106's knees, URL0107's leg) eased
/// in over the cycle's last quarter.
#[test]
#[ignore]
fn walks_end_where_they_start() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let (mut walks, mut closed, mut wrapping) = (0, 0, 0);
    let mut out_of_step: Vec<String> = Vec::new();
    for u in all.iter().filter(|u| u.playable()) {
        let Some((path, _)) = &u.display.walk else { continue };
        let Some(bytes) = files.read(path).unwrap() else { continue };
        let Ok(anim) = sca::parse(&bytes) else { continue };
        let lod0 = &u.display.lods[0];
        let mesh = scm::parse(&files.read(&lod0.mesh).unwrap().unwrap()).unwrap();
        // How far Forged Alliance's own walk ends from where it starts: the most any bone's turn
        // differs (degrees).
        let (a, z) = (&anim.frames[0], anim.frames.last().unwrap());
        let apart = a.keys.iter().zip(&z.keys).map(|(p, q)| {
            let dot = (p.1[0] * q.1[0] + p.1[1] * q.1[1] + p.1[2] * q.1[2] + p.1[3] * q.1[3]).abs().min(1.0) as f64;
            2.0 * dot.acos().to_degrees()
        }).fold(0.0, f64::max);
        if apart > 5.0 {
            out_of_step.push(format!("{} {apart:.0}°", u.id));
        }
        let material = Material { name: "m".into(), albedo: "a".into(), specular: "s".into(), normals: "n".into(), mean: [0; 3] };
        let near = model::convert(&mesh, u.display.uniform_scale, material).unwrap();
        let clips = ta_supcom::factions::anims::anims(&mesh, &near.bones, u.display.uniform_scale * 32.0, Some(&anim), &[], &[]);
        let Some(walk) = clips.clip("walk") else { continue };
        walks += 1;
        let last = walk.times.len() - 1;
        if walk.times[last] + 1e-4 < walk.seconds {
            wrapping += 1;
            continue;
        }
        closed += 1;
        assert_eq!(walk.frame(0), walk.frame(last), "{}: its walk's last frame its first", u.id);
    }
    eprintln!("{walks} walks: {closed} ending at their length (their last frame their first), {wrapping} wrapping back over the time left; Forged Alliance's own ending out of step: {out_of_step:?}");
}

/// Every building at work, as the conversion makes it (`factions::work`): the clips its model
/// plays (`active`, `open`, `upgrade`: each made of the animation its classes or its script play,
/// those the game has), and what its script spins, swings and slides, listed; then held: every
/// extractor pumps, every building upgrading in place with an animation for it plays it, the
/// power generators, fabricators and radars whose scripts spin their parts spin them, and nothing
/// a script turns is left out unaccounted for.
#[test]
#[ignore]
fn buildings_at_work() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let all = unit::catalogue(&files).unwrap();
    let playable: Vec<&unit::UnitBlueprint> = all.iter().filter(|u| u.playable() && !u.display.lods.is_empty()).collect();
    let mut clips_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut spins_of: BTreeMap<String, usize> = BTreeMap::new();
    let (mut missing, mut fighting, mut left_out) = (Vec::new(), Vec::new(), Vec::new());
    let (mut buildings, mut with_clips, mut with_motions) = (0, 0, 0);
    for u in playable.iter().filter(|u| u.motion == unit::Motion::None) {
        buildings += 1;
        let faction: Vec<&unit::UnitBlueprint> = playable.iter().copied().filter(|o| o.faction == u.faction).collect();
        let upgrades = faction::upgrade_of(u, &faction).is_some();
        let mesh = scm::parse(&files.read(&u.display.lods[0].mesh).unwrap().unwrap()).unwrap();
        let material = Material { name: "m".into(), albedo: "a".into(), specular: "s".into(), normals: "n".into(), mean: [0; 3] };
        let near = model::convert(&mesh, u.display.uniform_scale, material).unwrap();
        let pixels = u.display.uniform_scale * 32.0;
        let mut read = Vec::new();
        for s in work::clips(u, upgrades) {
            match files.read(&s.path).unwrap().map(|b| sca::parse(&b)) {
                Some(Ok(a)) => read.push((s, a)),
                Some(Err(e)) => missing.push(format!("{} {}: {e}", u.id, s.path)),
                None => missing.push(format!("{} {} ({})", u.id, s.path, s.name)),
            }
        }
        let at_work: Vec<anims::Work> = read.iter().map(|(s, a)| anims::Work { name: s.name, animation: a, rate: s.rate, cycle: s.cycle }).collect();
        let clips = anims::anims(&mesh, &near.bones, pixels, None, &[], &at_work);
        let (mut kept, mut names) = (Vec::new(), Vec::new());
        for c in &clips.clips {
            if matches!(c.name.as_str(), "active" | "open") && work::turns_an_aim(u, &mesh, &near.bones, c) {
                fighting.push(format!("{} {}", u.id, c.name));
                continue;
            }
            assert!(c.fits(near.model.pieces.len()), "{}: its {} fits its model", u.id, c.name);
            kept.push(format!("{} {:.2} s, {} pieces", c.name, c.seconds, c.pieces.len()));
            names.push(c.name.clone());
        }
        let pieces: Vec<String> = near.model.pieces.iter().map(|p| p.name.clone()).collect();
        let motions = work::motions(u, &mesh, &pieces, pixels);
        left_out.extend(motions.left_out.iter().map(|w| format!("{}: {w}", u.id)));
        if !kept.is_empty() {
            with_clips += 1;
        }
        if !motions.spins.is_empty() || !motions.slides.is_empty() {
            with_motions += 1;
        }
        let described: Vec<String> = motions
            .spins
            .iter()
            .map(|s| format!("{} {:?}{} about {:?}", pieces[s.piece as usize], s.when, if s.sway.is_some() { " swinging" } else { " spinning" }, s.axes))
            .chain(motions.slides.iter().map(|s| format!("{} {:?}{} to {:?} (made at {:?})", pieces[s.piece as usize], s.when, if s.shuttle { " shuttling" } else { " sliding" }, s.goals, s.made)))
            .collect();
        if !kept.is_empty() || !described.is_empty() {
            eprintln!("{} ({}): {}{}{}", u.id, u.description, kept.join("; "), if kept.is_empty() || described.is_empty() { "" } else { " | " }, described.join("; "));
        }
        clips_of.insert(u.id.clone(), names);
        spins_of.insert(u.id.clone(), motions.spins.len());
    }
    let count = |name: &str| clips_of.values().filter(|c| c.iter().any(|n| n == name)).count();
    eprintln!("{buildings} buildings: {with_clips} with clips ({} active, {} open, {} upgrade), {with_motions} with parts their scripts move ({} spinning or swinging)", count("active"), count("open"), count("upgrade"), spins_of.values().filter(|&&n| n > 0).count());
    eprintln!("animations named and not in the game: {missing:?}");
    eprintln!("clips turning what a weapon aims, not made: {fighting:?}");
    eprintln!("left out: {left_out:?}");
    let has = |id: &str, clip: &str| clips_of.get(id).is_some_and(|c| c.iter().any(|n| n == clip));
    // Every extractor pumps (its loop), and those upgrading with an animation for it play it.
    for u in playable.iter().filter(|u| u.is("MASSEXTRACTION") && u.motion == unit::Motion::None) {
        assert!(has(&u.id, "active"), "{}: it pumps", u.id);
    }
    for id in ["UEB1103", "UEB1202", "UAB1103", "UAB1202", "URB1103", "URB1202", "UEB0101", "URB0102", "XSB0201", "UEB3101", "UAB3102"] {
        assert!(has(id, "upgrade"), "{id}: it upgrades as its animation shows");
    }
    // Power generators, fabricators and radars spin what their scripts spin.
    for (id, spins) in [("UEB1101", 2), ("UAB1301", 3), ("XSB1301", 1), ("XAB1401", 1), ("UEB1303", 1), ("URB1104", 1), ("XSB1303", 3), ("UEB3201", 1), ("UAB3104", 3), ("URB3101", 3), ("XSB3104", 3)] {
        assert_eq!(spins_of.get(id).copied(), Some(spins), "{id}");
    }
    assert!(has("XSB1102", "active") && has("URB0101", "active") && has("UAB1102", "open"), "the Seraphim hydrocarbon plant's loop, the Cybran factory's arms, the Aeon hydrocarbon plant's opening");
    assert!(!has("UAB2301", "open"), "a turret unfolding before it aims is left as it rests");
    assert!(left_out.iter().all(|w| w.contains("BuildAttachBone")), "{left_out:?}");
}

/// Every factory, every weapon that unpacks and every anti-air weapon, as the mod has them (made
/// whole once): each factory's ways out (`[EXITS]`) one for each of its roll-off points, each past
/// its footprint, its ramp within its way, the units built facing them (`UnitSpin` and the build
/// bone's own turn: the Cybran quantum gateway's rests turned half round); each weapon whose
/// unpack animation the game has unpacks in its script (`UnpackWeapon<n>`, `PackWeapon<n>`, its
/// aim waiting for it), those that lock their unit's motion holding it still (`StartMoving`); every
/// anti-air weapon that takes on aircraft alone takes no other target (`OnlyTargetCategory<n>=VTOL`):
/// the anti-air towers (2104, 2204, 2304 of each faction), the interceptors' guns, an enhancement's
/// missiles; not the missile defences. `--nocapture` lists them.
#[test]
#[ignore]
fn factories_roll_off_weapons_unpack_and_anti_air_shoots_aircraft_alone() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut scripts: BTreeMap<String, ta_formats::cob::CobScript> = BTreeMap::new();
    make::make(&files, &units, None, None, "test", &mut |path, bytes| {
        if let Some(id) = path.strip_prefix("units/").and_then(|p| p.strip_suffix(".fbi")) {
            texts.insert(id.to_ascii_uppercase(), String::from_utf8_lossy(bytes).into_owned());
        } else if let Some(id) = path.strip_prefix("scripts/").and_then(|p| p.strip_suffix(".cob")) {
            scripts.insert(id.to_ascii_uppercase(), ta_formats::cob::CobScript::parse(bytes).unwrap());
        }
        Ok(())
    }, &mut |_, _| true)
    .unwrap();
    let info = |id: &str| ta_formats::tdf::parse(&texts[&id.to_ascii_uppercase()]).section("UNITINFO").cloned().unwrap();
    // Factories: their ways out.
    let mut factories = 0;
    for u in units.iter().filter(|u| units::builds(u) == units::Builds::Factory) {
        let fbi = info(&u.id);
        let exits = ta_data::ExitDef::of_unit(&fbi);
        assert_eq!(exits.len(), u.roll_off.len(), "{}: a way out for each roll-off point", u.id);
        if exits.is_empty() {
            continue;
        }
        factories += 1;
        let (fx, fz) = (fbi.get_f64_or("FootprintX", 1.0) * 8.0, fbi.get_f64_or("FootprintZ", 1.0) * 8.0);
        for e in &exits {
            assert!(e.x.abs() >= fx + units::CLEAR - 1.0 || e.z.abs() >= fz + units::CLEAR - 1.0, "{}: {e:?} past its footprint", u.id);
            assert!(e.ramp.0 <= e.ramp.1, "{}: {e:?}", u.id);
        }
        assert!(fbi.get("YardMap").is_some_and(|y| !y.contains('c')), "{}: solid", u.id);
        let ways: Vec<String> = exits.iter().map(|e| format!("({}, {}) facing {} ramp {}-{}", e.x, e.z, e.heading, e.ramp.0, e.ramp.1)).collect();
        eprintln!("{} ({}): {}", u.id, u.description, ways.join("; "));
    }
    assert!(factories >= 40, "{factories} factories with ways out");
    let heading = |id: &str| ta_data::ExitDef::of_unit(&info(id)).iter().map(|e| e.heading as i64).collect::<Vec<_>>();
    assert_eq!(heading("UEB0101"), [90, -90], "out to either side, facing the way");
    assert_eq!(heading("URB0304"), [180, 0], "its build bone rests turned half round");
    let ramp = |id: &str| ta_data::ExitDef::of_unit(&info(id))[0].ramp;
    assert!(ramp("UEB0101").0 >= 24.0 && ramp("UEB0101").1 <= 72.0, "the UEF land factory's platform and ramp: {:?}", ramp("UEB0101"));
    assert_eq!(ramp("UEB0103"), (0.0, 0.0), "a naval factory's ships are built at the water");
    // Weapons that unpack.
    let mut unpacking = 0;
    for u in &units {
        let own = units::weapons(u);
        for (slot, w) in own.slots.iter().enumerate() {
            let Some(up) = &w.unpack else { continue };
            let Some(path) = &up.animation else { continue };
            let Some(bytes) = files.read(path).unwrap() else {
                eprintln!("{} {}: its unpack animation {path} is not in the game", u.id, w.label);
                continue;
            };
            let animation = sca::parse(&bytes).unwrap();
            let mesh = scm::parse(&files.read(&u.display.lods[0].mesh).unwrap().unwrap()).unwrap();
            let material = Material { name: "m".into(), albedo: "a".into(), specular: "s".into(), normals: "n".into(), mean: [0; 3] };
            let near = model::convert(&mesh, u.display.uniform_scale, material).unwrap();
            let Some(made) = make::unpacking(w, &[(path.clone(), animation.clone())], &mesh, &near.bones, u.display.uniform_scale * 32.0) else {
                eprintln!("{} {}: its unpack animation moves nothing", u.id, w.label);
                continue;
            };
            unpacking += 1;
            let script = &scripts[&u.id.to_ascii_uppercase()];
            for f in [format!("UnpackWeapon{}", slot + 1), format!("PackWeapon{}", slot + 1), "TargetCleared".to_string()] {
                assert!(script.function(&f).is_some(), "{}: {f}", u.id);
            }
            let aim = ["AimPrimary", "AimSecondary", "AimTertiary"].get(slot).map_or(format!("AimWeapon{}", slot + 1), |s| s.to_string());
            assert!(script.function(&aim).is_some(), "{}: it aims ({aim})", u.id);
            if up.locks_motion {
                assert!(script.function("StartMoving").is_some() && script.function("StopMoving").is_some(), "{}: held still", u.id);
            }
            // (What it moves, and whether that is what a weapon of the unit aims.)
            let names: Vec<String> = made.clip.pieces.iter().map(|&p| near.model.pieces[p as usize].name.clone()).collect();
            let aimed: Vec<&str> = u.weapons.iter().flat_map(|w| [w.yaw_bone.as_deref(), w.pitch_bone.as_deref()]).flatten().collect();
            let aims: Vec<&String> = names.iter().filter(|n| aimed.iter().any(|a| a.eq_ignore_ascii_case(n))).collect();
            let kept = ta_supcom::factions::script::kept_frames(&made.clip);
            eprintln!(
                "{} {} (slot {}): {} over {:.2} s ({} of {} frames), repacks after {} ms{}{}: {}{}",
                u.id,
                w.label,
                slot + 1,
                path,
                made.clip.seconds,
                kept.len(),
                made.clip.times.len(),
                made.repack_ms,
                if up.locks_motion { ", holding its unit still" } else { "" },
                if made.after_launch { ", after each launch" } else { "" },
                names.join(", "),
                if aims.is_empty() { String::new() } else { format!(" (aimed: {aims:?})") },
            );
        }
    }
    assert!(unpacking >= 14, "{unpacking} weapons unpack");
    for id in ["UEL0111", "XEL0306"] {
        assert!(scripts[id].function("UnpackWeapon1").is_some() && scripts[id].function("AimPrimary").is_some(), "{id}: its launcher raised before it fires");
    }
    // Anti-air: aircraft alone.
    let mut air_only = 0;
    for u in &units {
        let fbi = info(&u.id);
        for slot in 0..units::MAX_WEAPONS {
            if fbi.get(&format!("OnlyTargetCategory{}", slot + 1)) == Some("VTOL") {
                air_only += 1;
            }
        }
    }
    for faction in ["UEB", "URB", "UAB", "XSB"] {
        for tower in ["2104", "2204", "2304"] {
            let id = format!("{faction}{tower}");
            assert_eq!(info(&id).get("OnlyTargetCategory1"), Some("VTOL"), "{id}: aircraft alone");
        }
    }
    // The interceptors' guns (their layers name the land too, but none but `AIR` units), and the
    // Cybran support commander's anti-air missiles in the slot its enhancement fills; not the
    // missile defences, nor the Striker's gun.
    for id in ["UEA0102", "URA0102", "UAA0102", "XSA0102"] {
        assert_eq!(info(id).get("OnlyTargetCategory1"), Some("VTOL"), "{id}: aircraft alone");
    }
    assert_eq!(info("URL0301").get("OnlyTargetCategory2"), Some("VTOL"), "URL0301: its enhancement's missiles");
    for id in ["UEB4201", "UEB4302", "UEL0201"] {
        assert!((1..=units::MAX_WEAPONS).all(|slot| info(id).get(&format!("OnlyTargetCategory{slot}")).is_none()), "{id}");
    }
    eprintln!("{factories} factories with ways out, {unpacking} weapons unpacking, {air_only} weapons shooting aircraft alone");
}

/// The engineering stations, the units Forged Alliance makes only by upgrading, and the
/// structures' adjacency, as the mod is made from the player's copy: every engineering station
/// works round it by itself (`AssistPatrol`, with build power and a reach); the Kennel upgrades
/// into the tech 3 Kennel and the Hive into that of tech 2 and that into tech 3's, which no menu
/// offers; every structure with a size class says it (`AdjacencySize`), what moves neither gives
/// nor takes, and each bonus given changes something TA Reborn plays, by a share.
#[test]
#[ignore]
fn stations_upgrades_and_adjacency_are_made() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    make::make(&files, &units, None, None, "test", &mut |path, bytes| {
        if path.starts_with("units/") || path.starts_with("download/") {
            texts.insert(path.to_string(), String::from_utf8_lossy(bytes).into_owned());
        }
        Ok(())
    }, &mut |_, _| true)
    .unwrap();
    let info = |id: &str| ta_formats::tdf::parse(&texts[&format!("units/{}.fbi", id.to_ascii_lowercase())]).sections.remove(0);
    let menus: String = texts.iter().filter(|(p, _)| p.starts_with("download/")).map(|(_, t)| t.as_str()).collect();
    for (from, to) in [("XEB0104", "XEB0204"), ("XRB0104", "XRB0204"), ("XRB0204", "XRB0304")] {
        assert_eq!(info(from).get("UpgradesTo"), Some(to), "{from} upgrades into {to}");
        assert!(units.iter().any(|u| u.id == to && u.upgrade_only), "{to}: made by upgrading alone");
        assert!(!menus.contains(&format!("UNITNAME={to};")), "{to} is on no menu");
    }
    let stations: Vec<&unit::UnitBlueprint> = units.iter().filter(|u| u.is("ENGINEERSTATION")).collect();
    for s in &stations {
        let f = info(&s.id);
        assert_eq!(f.get("DefaultMissionType"), Some("AssistPatrol"), "{}", s.id);
        assert!(f.get_i64_or("WorkerTime", 0) > 0 && f.get_i64_or("BuildDistance", 0) >= 64 && f.get("CanReclamate") == Some("1"), "{}: build power, a reach, reclaiming", s.id);
    }
    assert!(stations.len() >= 5, "the Kennels and the Hives: {:?}", stations.iter().map(|s| &s.id).collect::<Vec<_>>());
    let (mut sized, mut givers, mut bonuses) = (0, 0, 0);
    for u in &units {
        let f = info(&u.id);
        let size = f.get("AdjacencySize").map(|s| s.parse::<u32>().expect("a number"));
        let given = f.sections.iter().find(|s| s.name == "ADJACENCY");
        if u.motion != unit::Motion::None {
            assert!(size.is_none() && given.is_none(), "{}: what moves gives and takes nothing", u.id);
            continue;
        }
        assert_eq!(size, u.size_class(), "{}", u.id);
        sized += size.is_some() as usize;
        if let Some(given) = given {
            givers += 1;
            for b in &given.sections {
                bonuses += 1;
                assert!(b.get("Affects").and_then(ta_sim::AdjacencyAffect::of).is_some(), "{}: {} changes what is played", u.id, b.name);
                assert!(b.get_f64_or("Add", 0.0) != 0.0 && b.get("Size").is_some(), "{}: {} adds a share to a size", u.id, b.name);
            }
        }
    }
    eprintln!("{} engineering stations; {sized} structures with a size class; {givers} giving {bonuses} bonuses", stations.len());
    assert!(givers >= 30 && sized >= 150, "the generators, extractors, fabricators and storage give; the structures have their sizes");
    // Every faction's tech 1 power generator cuts what the factories beside it pay to build.
    for f in Faction::PLAYABLE {
        let generator = units.iter().find(|u| u.faction == f && u.is("ENERGYPRODUCTION") && u.tech == Some(Tech::T1) && u.adjacency.is_some()).expect("a tech 1 generator with adjacency");
        let given = info(&generator.id);
        let given = given.sections.iter().find(|s| s.name == "ADJACENCY").expect("its bonuses");
        assert!(given.sections.iter().any(|b| b.get("Affects") == Some("BuildEnergy") && b.get("Size") == Some("16")), "{}: build energy to size 16", generator.id);
    }
}

/// The copy's sound banks and the units' plan of sounds (every playable unit's category and
/// weapons, the announcer).
fn sound_plan(game: &Install, files: &Files, units: &[unit::UnitBlueprint]) -> Option<(ta_supcom::factions::sounds::Banks, ta_supcom::factions::sounds::Plan)> {
    use ta_supcom::factions::sounds;
    let dir = ta_vfs::install::find_dir_ci(&game.root, "sounds")?;
    let banks = sounds::Banks::read(&dir).unwrap();
    let mut plan = sounds::Plan::default();
    for u in units {
        for w in u.weapons.iter().filter(|w| w.unpack.is_some()) {
            plan.unpacking(&banks, u, w);
        }
        plan.unit(&banks, u, units::ta_sound_category(u, &units::weapons(u)));
    }
    plan.commander_alert(&banks, files).unwrap();
    Some((banks, plan))
}

/// Every sound the playable units' blueprints name (their own, their weapons' and their
/// projectiles') is a set of waves the copy has, but those the game's own data names and does not
/// have (listed: a cue no bank of that name holds); every set's files are made; the announcer's two.
#[test]
#[ignore]
fn every_sound_the_units_name_is_made() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let Some((banks, plan)) = sound_plan(&game, &files, &units) else {
        eprintln!("no sounds folder in the copy");
        return;
    };
    let (sound_banks, wave_banks, settings) = banks.counts();
    eprintln!("{sound_banks} sound banks, {wave_banks} wave banks, global settings read: {settings}");
    assert!(settings, "the global settings");
    let mut not_in_the_game = Vec::new();
    for (bank, cue) in &plan.missing {
        assert!(!banks.has_cue(bank, cue), "{bank}/{cue}: the copy has the cue, but not its sound as read");
        not_in_the_game.push(format!("{bank}/{cue}"));
    }
    eprintln!("named by the blueprints but not in the copy's banks ({}): {not_in_the_game:?}", not_in_the_game.len());
    let made: std::collections::BTreeSet<String> = plan.files().collect();
    let sets: Vec<_> = plan.sets().collect();
    for (name, variants, _) in &sets {
        assert!(!variants.is_empty(), "{name}");
        for v in variants {
            assert!(made.contains(&format!("{v}.wav")), "{name}: {v}");
        }
    }
    let table = plan.table(&["UEF"]);
    assert!(table.contains("commanderattacked=FA_") && table.contains("nuclearlaunch=FA_"), "the announcer's two");
    eprintln!("{} sets, {} loops, {} files, {} weapons with sounds of their own", sets.len(), sets.iter().filter(|s| s.2).count(), made.len(), plan.weapons.len());
}

/// The units' sounds as the game decodes them, against the copy's own waves: as long, never
/// clipping where the wave does not (three samples running at full scale), the noise IMA ADPCM
/// leaves well under the sound, its level the wave's block by block (no jumps), no click at either
/// end.
#[test]
#[ignore]
fn the_sounds_are_clean() {
    use ta_supcom::factions::sounds;
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let Some((banks, plan)) = sound_plan(&game, &files, &units) else { return };
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    plan.write(&banks, 64, &mut |path, bytes| {
        written.insert(path.to_string(), bytes.to_vec());
        Ok(())
    })
    .unwrap();
    let mut snrs: Vec<(f64, String)> = Vec::new();
    let (mut clipped, mut rails, mut jumps, mut edges, mut bytes) = (0, 0, 0, 0, 0usize);
    for (path, source) in plan.sources() {
        let data = &written[&path];
        bytes += data.len();
        let (sound, _) = ta_audio::decode::wav::decode(data).unwrap_or_else(|e| panic!("{path}: {e}"));
        let decoded = sound.samples();
        let first = decoded.first().copied().unwrap_or(0.0).abs();
        let last = decoded.last().copied().unwrap_or(0.0).abs();
        if first > 0.25 || last > 0.25 {
            edges += 1;
        }
        let Some((bank, wave, delay)) = source else { continue };
        let (pcm, rate) = sounds::samples(&banks, bank, wave).unwrap();
        let lead = (delay as u64 * rate as u64 / 1000) as usize;
        assert!(decoded.len() >= lead + pcm.len() && decoded.len() < lead + pcm.len() + 8, "{path}: {} for {}", decoded.len(), lead + pcm.len());
        let (mut signal, mut noise) = (0f64, 0f64);
        // (Full scale where the wave is not: a sample now and then is the codec's error, three
        // running a clip.)
        let mut run = 0;
        for (i, &a) in pcm.iter().enumerate() {
            let at_rail = (decoded[lead + i] as f64 * 32768.0).abs() >= 32767.0 && (a as f64).abs() < 32000.0;
            rails += at_rail as usize;
            run = if at_rail { run + 1 } else { 0 };
            clipped += (run == 3) as usize;
        }
        let block = (rate as usize / 20).max(1);
        for (k, chunk) in pcm.chunks(block).enumerate() {
            let (mut s, mut n) = (0f64, 0f64);
            for (i, &a) in chunk.iter().enumerate() {
                let b = decoded[lead + k * block + i] as f64 * 32768.0;
                s += (a as f64).powi(2);
                n += (a as f64 - b).powi(2);
            }
            // (A block a twentieth of a second: the decoded one's level within a decibel of the
            // wave's where the wave is to be heard.)
            let level = |e: f64| 10.0 * (e / chunk.len() as f64).max(1e-9).log10();
            let decoded_energy: f64 = chunk.iter().enumerate().map(|(i, _)| (decoded[lead + k * block + i] as f64 * 32768.0).powi(2)).sum();
            if level(s) > 40.0 && (level(s) - level(decoded_energy)).abs() > 1.0 {
                jumps += 1;
            }
            signal += s;
            noise += n;
        }
        snrs.push((10.0 * (signal / noise.max(1.0)).log10(), path));
    }
    snrs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let median = snrs[snrs.len() / 2].0;
    eprintln!("{} files, {:.1} MB; IMA ADPCM's noise under the sound: median {median:.1} dB, the worst {:?}", written.len(), bytes as f64 / 1048576.0, &snrs[..snrs.len().min(5)]);
    eprintln!("clips where the wave has none: {clipped} (samples at full scale where the wave is not: {rails}); blocks a decibel off their wave's level: {jumps}; files starting or ending loud: {edges}");
    assert_eq!(clipped, 0);
    assert!(median > 20.0, "{median}");
    assert!(jumps * 1000 < snrs.len() * 50, "{jumps} blocks");
}

/// Every weapon's effects, as Forged Alliance's scripts and emitters name them, made TA Reborn's:
/// each weapon's muzzle, its shot's trail and impacts, its shot's or beam's colours, and its
/// projectile's own model where TA Reborn draws it as one and the game has its mesh; each
/// faction's effects in TA Reborn's terms (every burst of a kind it plays, its numbers within its
/// bounds), made the same twice. `TA_SUPCOM_FX_OUT=<dir>` writes the effects files there.
#[test]
#[ignore]
fn every_weapons_projectile_and_effects_are_made() {
    use ta_supcom::factions::effects;
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let started = std::time::Instant::now();
    let made = effects::make(&files, &units).unwrap();
    eprintln!("effects made in {} ms", started.elapsed().as_millis());
    let again = effects::make(&files, &units).unwrap();
    assert!(made.files == again.files && made.models == again.models, "made again, the same");
    if let Some(dir) = std::env::var_os("TA_SUPCOM_FX_OUT") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (path, text) in &made.files {
            std::fs::write(dir.join(path.rsplit('/').next().unwrap()), text).unwrap();
        }
    }
    // Every faction's effects read as the game reads them, each burst of a kind it draws.
    let mut effect_names = std::collections::BTreeSet::new();
    for (path, text) in &made.files {
        let doc = ta_formats::tdf::parse(text);
        let mut bursts = 0;
        for s in &doc.sections {
            let (e, problems) = ta_data::EffectDef::from_tdf(s);
            assert!(problems.is_empty(), "{path} [{}]: {problems:?}", s.name);
            assert!(!e.bursts.is_empty() && e.bursts.len() <= ta_data::effect::MAX_BURSTS, "{path} [{}]", s.name);
            bursts += e.bursts.len();
            effect_names.insert(e.name.clone());
        }
        eprintln!("{path}: {} effects, {bursts} bursts, {} KB", doc.sections.len(), text.len() >> 10);
    }
    assert_eq!(made.files.len(), 4, "a file a faction");
    // Each weapon (but the death and suicide blasts) and what it was given.
    let (mut weapons, mut fired, mut hits, mut trails, mut coloured, mut modelled, mut want_models) = (0, 0, 0, 0, 0, 0, 0);
    let mut by_kind: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut unmade: Vec<String> = Vec::new();
    for u in &units {
        let dressed = made.dressed(u);
        for w in &dressed.weapons {
            if w.category.eq_ignore_ascii_case("death") || w.category.eq_ignore_ascii_case("kamikaze") || w.damage <= 0.0 && w.nuke.is_none() {
                continue;
            }
            weapons += 1;
            let look = w.look.clone().unwrap_or_default();
            let has = |key: &str| look.keys.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
            for key in ["fireeffect", "traileffect", "hiteffect", "waterhiteffect"] {
                if let Some(name) = has(key) {
                    assert!(effect_names.contains(&name.to_ascii_uppercase()), "{} {}: its {key} {name} is made", u.id, w.label);
                }
            }
            fired += has("fireeffect").is_some() as usize;
            hits += has("hiteffect").is_some() as usize;
            trails += has("traileffect").is_some() as usize;
            coloured += has("rgbcolor").is_some() as usize;
            let kind = format!("{:?}", units::kind(w));
            let e = by_kind.entry(kind).or_default();
            e.0 += 1;
            e.1 += has("fireeffect").is_some() as usize;
            e.2 += has("hiteffect").is_some() as usize;
            if has("fireeffect").is_none() && has("hiteffect").is_none() {
                unmade.push(format!("{} {} ({:?})", u.id, w.label, units::kind(w)));
            }
            // Drawn as a model with a mesh of its own: that mesh, made.
            if effects::drawn_as_a_model(w)
                && let Some(path) = w.projectile.as_deref().map(ta_supcom::projectile::normalise)
                && files.read(&format!("{}_lod0.scm", path.trim_end_matches("_proj.bp"))).unwrap().is_some()
            {
                want_models += 1;
                assert_eq!(look.model.as_deref(), Some(effects::model_name(&path).as_str()), "{} {}: its projectile's own model", u.id, w.label);
                assert!(made.models.contains(&path), "{path}");
                modelled += 1;
            }
        }
    }
    eprintln!("{weapons} weapons: {fired} with a muzzle effect, {hits} with a hit effect, {trails} with a trail, {coloured} in their own colours, {modelled} of {want_models} drawn with their projectile's own model");
    for (kind, (n, f, h)) in &by_kind {
        eprintln!("  {kind}: {n} weapons, {f} muzzles, {h} hits");
    }
    eprintln!("weapons with neither a muzzle nor a hit effect: {unmade:?}");
    eprintln!("{} projectile models", made.models.len());
    assert!(fired * 10 >= weapons * 8 && hits * 10 >= weapons * 7, "most weapons have their muzzles and their hits");
    // Every projectile model made: its model, surface and paint, reading as the game reads them.
    for path in &made.models {
        let shot = units.iter().flat_map(|u| &u.weapons).find_map(|w| w.shot.as_ref().filter(|s| s.path == *path)).unwrap();
        let out = effects::projectile_model(&files, path, shot).unwrap().unwrap_or_else(|| panic!("{path}: its model"));
        let name = effects::model_name(path);
        let model = out.iter().find(|(p, _)| *p == format!("objects3d/{name}.3do")).map(|(_, b)| ta_formats::model3do::Model3do::parse(b).unwrap()).unwrap();
        let surface = out.iter().find(|(p, _)| *p == format!("objects3d/{name}.surface")).map(|(_, b)| ta_formats::surface::Surface::parse(b).unwrap()).unwrap();
        assert!(surface.fits(&model), "{path}: its surface fits");
        assert_eq!(out.iter().filter(|(p, _)| p.starts_with("textures/supcom/")).count(), 3, "{path}: its paint");
    }
}

/// TA Reborn's logistics as the factions' units are made with them, read as the game reads them:
/// every air transport that carries does so in slots, each a piece of its model, the smaller
/// first, of a passenger's class; every commander's Personal Teleporter teleports; the
/// Kennels and the UEF's pods keep their drones (themselves on no menu); every factory that moves
/// is a factory, with a build piece of its model and a menu.
#[test]
#[ignore]
fn logistics_are_made() {
    let Some(game) = the_game() else {
        eprintln!("no copy of Forged Alliance found: nothing to read");
        return;
    };
    let files = Files::own(&game).unwrap();
    let units: Vec<_> = unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut models: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    make::make(&files, &units, None, None, "test", &mut |path, bytes| {
        if path.starts_with("units/") || path.starts_with("download/") {
            texts.insert(path.to_string(), String::from_utf8_lossy(bytes).into_owned());
        } else if path.starts_with("objects3d/") && path.ends_with(".3do") && !path.contains("_lod") {
            models.insert(path.to_string(), bytes.to_vec());
        }
        Ok(())
    }, &mut |_, _| true)
    .unwrap();
    let info = |id: &str| ta_formats::tdf::parse(&texts[&format!("units/{}.fbi", id.to_ascii_lowercase())]).sections.remove(0);
    let def = |id: &str| ta_data::UnitDef::from_tdf(&info(id)).expect("a unit");
    let pieces = |id: &str| -> Vec<String> {
        let model = ta_formats::model3do::Model3do::parse(&models[&format!("objects3d/{}.3do", id.to_ascii_lowercase())]).expect("its model");
        model.pieces.iter().map(|p| p.name.to_ascii_lowercase()).collect()
    };
    let menus: String = texts.iter().filter(|(p, _)| p.starts_with("download/")).map(|(_, t)| t.as_str()).collect();
    let on_menu = |id: &str| menus.contains(&format!("UNITNAME={};", id.to_ascii_uppercase()));
    let has_menu = |id: &str| menus.contains(&format!("UNITMENU={};", id.to_ascii_uppercase()));

    // The air transports: slots of their models' pieces, the small first, and one in each.
    let mut slotted = Vec::new();
    for u in units.iter().filter(|u| u.motion == unit::Motion::Air && u.is("TRANSPORTATION")) {
        let d = def(&u.id);
        if d.transport_capacity == 0 {
            continue;
        }
        assert!(!d.transport_slots.is_empty(), "{}: carries in slots", u.id);
        let own = pieces(&u.id);
        for s in &d.transport_slots {
            assert!(own.contains(&s.piece.to_ascii_lowercase()), "{}: its slot {} is a piece of its model", u.id, s.piece);
            assert!((1..=3).contains(&s.class), "{}: {} of a passenger's class", u.id, s.piece);
        }
        assert!(d.transport_slots.windows(2).all(|w| w[0].class <= w[1].class), "{}: the small slots first, by class", u.id);
        slotted.push(format!("{} {}", u.id, d.transport_slots.len()));
    }
    // A passenger's class is one of the slots' (the tech 1 land units the small).
    for u in units.iter().filter(|u| matches!(u.motion, unit::Motion::Land | unit::Motion::Hover | unit::Motion::Amphibious)) {
        assert!((1..=10).contains(&def(&u.id).transport_class), "{}: a class", u.id);
    }
    // Every commander's Personal Teleporter.
    let mut teleports = Vec::new();
    for f in Faction::PLAYABLE {
        let theirs: Vec<&unit::UnitBlueprint> = units.iter().filter(|u| u.faction == f).collect();
        let commander = faction::commander(&theirs).expect("a commander");
        let d = def(&commander.id);
        let teleporter = d.enhancements.iter().find(|e| e.teleport.is_some()).unwrap_or_else(|| panic!("{}: a teleporter", commander.id));
        let t = teleporter.teleport.unwrap();
        assert!(t.energy > 0.0 && t.time > 0.0 && t.energy_per_px == 0.0, "{}: a charge, the same anywhere", commander.id);
        teleports.push(format!("{} {}", commander.id, teleporter.id));
    }
    // The Kennels' drones, and the UEF's pods.
    for (station, count) in [("XEB0104", 1), ("XEB0204", 2)] {
        let d = def(station);
        assert_eq!(d.drones.len(), 1, "{station}: its drone");
        assert_eq!(d.drones[0].count, count, "{station}: as many as it keeps");
        assert!(!on_menu(&d.drones[0].unit), "{station}: its drone is made by it alone");
        assert!(pieces(station).contains(&d.drones[0].piece.to_ascii_lowercase()), "{station}: made at a piece of its model");
    }
    let uef: Vec<&unit::UnitBlueprint> = units.iter().filter(|u| u.faction == Faction::Uef).collect();
    let commander = def(&faction::commander(&uef).unwrap().id);
    let pods: Vec<_> = commander.enhancements.iter().filter(|e| !e.drones.is_empty()).collect();
    assert!(pods.len() >= 2, "the UEF commander's two shoulder drones");
    for e in &pods {
        // (The second, installed over the first in its slot, keeps both.)
        let count = if e.prerequisite.is_empty() { 1 } else { 2 };
        assert!(!on_menu(&e.drones[0].unit) && e.drones[0].count == count, "{}: its pods, of their own", e.id);
    }
    // The factories that move.
    let mobile: Vec<&unit::UnitBlueprint> = units.iter().filter(|u| units::mobile_factory(u)).collect();
    for u in &mobile {
        let d = def(&u.id);
        assert!(d.mobile_factory && d.worker_time > 0.0 && has_menu(&u.id), "{}: a factory that moves, its menu", u.id);
        assert!(pieces(&u.id).contains(&d.build_piece.to_ascii_lowercase()), "{}: built on a piece of its model ({})", u.id, d.build_piece);
    }
    assert!(mobile.len() >= 7, "the Fatboy, the carriers, the Atlantis, the Tempest, the CZAR: {:?}", mobile.iter().map(|u| &u.id).collect::<Vec<_>>());
    eprintln!("slotted: {}\nteleporters: {}\nfactories that move: {}", slotted.join(", "), teleports.join(", "), mobile.iter().map(|u| u.id.as_str()).collect::<Vec<_>>().join(", "));
}
