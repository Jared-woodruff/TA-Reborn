//! Installed units that can be switched on and off (`OnOffable`); skip without an original
//! install.

use super::*;
use super::air_original::{field, original};

/// Ticks a unit is given to come to rest after being switched: a Core metal maker's plug turns
/// shut in under four seconds; a Core extractor's arms slow down, turn back round and fold away
/// in about eight.
const SETTLE: usize = 600;

/// Units whose scripts keep a piece turning whatever their switch says, as TA's own scripts
/// have it: the carriers spin their radar dish from `Create` and have no `Activate` or
/// `Deactivate`, the Core underwater extractor spins its fan from `Create`.
const TURNING_ANYWAY: [&str; 3] = ["ARMCARRY", "CORCARRY", "CORUWMEX"];

/// A unit's script pieces as they stand: offset, rotation, shown.
fn pose(sim: &Sim, id: UnitId) -> Vec<([i32; 3], [i32; 3], bool)> {
    sim.unit(id).and_then(|u| u.cob.as_ref()).map(|c| c.pieces.iter().map(|p| (p.offset, p.rotation, p.visible)).collect()).unwrap_or_default()
}

/// How many of a unit's script pieces move over the next `ticks` ticks.
fn moving(sim: &mut Sim, id: UnitId, ticks: usize) -> usize {
    let mut last = pose(sim, id);
    let mut moved = vec![false; last.len()];
    for _ in 0..ticks {
        sim.step(&[]);
        let now = pose(sim, id);
        for (m, (a, b)) in moved.iter_mut().zip(last.iter().zip(&now)) {
            *m |= a != b;
        }
        last = now;
    }
    moved.iter().filter(|&&m| m).count()
}

/// Switches `id` on or off, as the player does, and gives it time to come to rest.
fn switch(sim: &mut Sim, id: UnitId, on: bool) {
    sim.step(&[Command::SetActive { player: 0, units: vec![id], on }]);
    for _ in 0..SETTLE {
        sim.step(&[]);
    }
}

/// A field for `t`: water under what floats or dives, metal under the extractors (so that they
/// turn as they work), and `t` finished at its middle.
fn standing(data: &Arc<ta_data::GameData>, types: &Arc<UnitTypes>, t: &UnitType) -> (Sim, UnitId) {
    let water = t.def.min_water_depth > 0.0 || t.locomotion == Locomotion::Naval;
    let mut sim = field(data, types, water);
    for z in 40..60 {
        for x in 40..60 {
            sim.world.set_metal(x, z, 50);
        }
    }
    let id = sim.spawn_unit(t.id, 0, FxVec3::from_ints(800, 60, 800), 0, true).unwrap();
    for _ in 0..SETTLE {
        sim.step(&[]);
    }
    (sim, id)
}

#[test]
fn original_on_off_units_rest_while_off_and_work_again_when_on() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.def.on_offable) {
        let (mut sim, id) = standing(data, types, t);
        let u = sim.unit(id).unwrap();
        assert_eq!(u.activated, t.def.activate_when_built, "{} is switched on when built as its definition says", t.name);
        if !u.activated {
            switch(&mut sim, id, true);
        }
        assert!(sim.working(sim.unit(id).unwrap()), "{} works while on", t.name);
        let (on_moving, on_playing) = (moving(&mut sim, id, 60), sim.unit(id).unwrap().textures_playing(t));
        switch(&mut sim, id, false);
        let u = sim.unit(id).unwrap();
        assert!(!u.activated && !sim.working(u) && !sim.sensors_active(u), "{} stops working when switched off", t.name);
        if !TURNING_ANYWAY.contains(&t.name.as_str()) {
            assert_eq!(moving(&mut sim, id, 60), 0, "{} still animates while switched off", t.name);
        }
        switch(&mut sim, id, true);
        let u = sim.unit(id).unwrap();
        assert!(u.activated && sim.working(u), "{} works again when switched on", t.name);
        assert_eq!(u.textures_playing(t), on_playing, "{}'s lamps and glows play again as they did", t.name);
        assert_eq!(moving(&mut sim, id, 60), on_moving, "{} animates again as it did", t.name);
        checked += 1;
    }
    assert!(checked >= 30, "expected the original on/off roster, checked {checked}");
    println!("{checked} original on/off units rest while off and work again when on");
}

/// TA's metal makers show their work by an animated texture, not a moving piece: their `Go`
/// lets the glow play (`dont-cache`) and their `Stop` holds it (`cache`). The moho makers and
/// the Core floating maker close their lids, flaps or doors over a glow that plays on.
#[test]
fn original_metal_makers_hold_their_glow_while_switched_off() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    for (name, glow, held) in [
        ("ARMMAKR", "sphere", true),
        ("CORMAKR", "glow", true),
        ("ARMFMKR", "dome", true),
        ("ARMMMKR", "glowingthing", false),
        ("CORMMKR", "glowingthing", false),
        ("CORFMKR", "glow", false),
    ] {
        let t = types.get(types.id(name).unwrap());
        let piece = t.model.pieces.iter().position(|p| p.name.eq_ignore_ascii_case(glow)).unwrap();
        let (mut sim, id) = standing(data, types, t);
        let plays = |sim: &Sim| sim.unit(id).unwrap().textures_playing(t) >> piece & 1 != 0;
        assert!(plays(&sim), "{name}'s {glow} plays while it works");
        switch(&mut sim, id, false);
        assert_eq!(plays(&sim), !held, "{name}'s {glow} {} while switched off", if held { "holds" } else { "plays on (under its lids)" });
        switch(&mut sim, id, true);
        assert!(plays(&sim), "{name}'s {glow} plays again");
    }
}

/// The simulation does not switch a maker off when energy runs short: it stays on, makes and
/// uses nothing (`Sim::working` is false, the scene puts its lights out), and works again when
/// there is energy.
#[test]
fn original_metal_maker_short_of_energy_stays_on_but_stops_working() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let t = types.get(types.id("ARMMAKR").unwrap());
    let (mut sim, id) = standing(data, types, t);
    assert!(sim.working(sim.unit(id).unwrap()));
    for _ in 0..30 {
        sim.players[0].economy.energy = Fx::ZERO;
        sim.step(&[]);
    }
    let u = sim.unit(id).unwrap();
    assert!(u.activated && !u.upkeep_paid && !sim.working(u), "an unpaid maker stays on but does not work");
    assert_eq!(sim.unit_flows(u).metal_make, Fx::ZERO, "and makes nothing");
    sim.players[0].economy.energy = Fx::from_int(10_000);
    sim.step(&[]);
    sim.step(&[]);
    assert!(sim.working(sim.unit(id).unwrap()), "it works again when there is energy");
}

/// A copy of the game (a replay's seek, the highlights' snapshots) keeps what is switched off,
/// its scripts' held textures with it, and plays on as the original.
#[test]
fn a_copied_game_keeps_its_switched_off_makers() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let t = types.get(types.id("ARMMAKR").unwrap());
    let (mut sim, id) = standing(data, types, t);
    switch(&mut sim, id, false);
    let mut copy = sim.clone();
    let state = |sim: &Sim| sim.unit(id).map(|u| (u.activated, u.textures_playing(t), pose(sim, id)));
    assert_eq!(state(&copy), state(&sim));
    for s in [&mut sim, &mut copy] {
        switch(s, id, true);
    }
    assert_eq!(state(&copy), state(&sim));
    assert_eq!(copy.checksum(), sim.checksum());
}

/// Every on/off unit of the install: what it is, what its script does when switched off and
/// on again (pieces moving, effects given off), as a table. A report, not a check.
#[test]
#[ignore]
fn survey_on_off_units() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    for t in types.types.iter().filter(|t| t.def.on_offable) {
        let (mut sim, id) = standing(data, types, t);
        let count = |sim: &mut Sim| {
            sim.take_events();
            let moved = moving(sim, id, 90);
            let sfx = sim.take_events().iter().filter(|e| matches!(e, SimEvent::Sfx { unit, .. } if *unit == id)).count();
            (moved, sfx, sim.unit(id).unwrap().textures_playing(t))
        };
        let built = count(&mut sim);
        switch(&mut sim, id, false);
        let off = count(&mut sim);
        switch(&mut sim, id, true);
        let on = count(&mut sim);
        let script = sim.unit(id).and_then(|u| u.cob.as_ref());
        let has = |f: &str| script.is_some_and(|c| c.has_function(f)) as u8;
        println!(
            "{:9} {:?} built-on {} E {:.1} makes {:.0} radar {} sonar {} jam {} sjam {} Activate {} Deactivate {} | on: {} moving, {} sfx, playing {:#x} | off: {} moving, {} sfx, playing {:#x} | on again: {} moving, {} sfx, playing {:#x}",
            t.name, t.locomotion, t.def.activate_when_built as u8, t.def.energy_use, t.def.makes_metal, t.def.radar_distance, t.def.sonar_distance, t.def.radar_jam, t.def.sonar_jam,
            has("Activate"), has("Deactivate"), built.0, built.1, built.2, off.0, off.1, off.2, on.0, on.1, on.2,
        );
    }
}

/// Every `cache` and `dont-cache` in a script: (function, op, COB piece).
fn cache_ops(program: &crate::cob::CobProgram) -> Vec<(String, u32, usize)> {
    use ta_formats::cob::{op, operand_count};
    let code = &program.script.code;
    let mut out = Vec::new();
    for f in &program.script.functions {
        let mut pc = f.offset as usize;
        while pc < code.len() {
            let opcode = code[pc];
            let Some(n) = operand_count(opcode) else { break };
            if (opcode == op::CACHE || opcode == op::DONT_CACHE) && pc + 1 < code.len() {
                out.push((f.name.clone(), opcode, code[pc + 1] as usize));
            }
            if opcode == op::RETURN {
                break;
            }
            pc += 1 + n;
        }
    }
    out
}

/// Every unit with an animated texture (frames other than one or a team's ten) and every
/// `cache`/`dont-cache` of its script, by piece: which lamps and glows play when. A report.
#[test]
#[ignore]
fn survey_animated_textures() {
    let Some(install) = ta_vfs::install::discover(None).ok() else { eprintln!("TA install not found; skipping"); return };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let mut frames: std::collections::BTreeMap<String, usize> = Default::default();
    for path in vfs.list("textures", Some("gaf"), true) {
        let Some(bytes) = vfs.read_opt(&path) else { continue };
        let Ok(gaf) = ta_formats::gaf::GafReader::open(&bytes) else { continue };
        for e in &gaf.entries {
            frames.insert(e.name.to_ascii_lowercase(), e.frames.len());
        }
    }
    let animated: Vec<_> = frames.iter().filter(|(_, n)| **n > 1 && **n != 10).collect();
    println!("animated textures: {animated:?}");
    let data = ta_data::GameData::load(&vfs);
    let types = UnitTypes::compile(&data, &|p| vfs.read_opt(p));
    for t in &types.types {
        let object = t.def.object.to_ascii_lowercase();
        let Some(model) = vfs.read_opt(&format!("objects3d/{object}.3do")).and_then(|b| ta_formats::model3do::Model3do::parse(&b).ok()) else { continue };
        let lit: Vec<String> = model
            .pieces
            .iter()
            .filter_map(|p| {
                let texture = p.primitives.iter().filter_map(|q| q.texture.as_deref()).find(|n| frames.get(&n.to_ascii_lowercase()).is_some_and(|&k| k > 1 && k != 10))?;
                Some(format!("{}:{texture}", p.name))
            })
            .collect();
        let ops = t.script.as_ref().map(|s| cache_ops(s)).unwrap_or_default();
        if lit.is_empty() && ops.is_empty() {
            continue;
        }
        let program = t.script.as_ref();
        let ops: Vec<String> = ops
            .iter()
            .map(|(f, o, p)| {
                let piece = program.and_then(|s| s.piece_map.get(*p).copied().flatten()).and_then(|m| model.pieces.get(m)).map_or("?", |x| x.name.as_str());
                format!("{f}:{}{piece}", if *o == ta_formats::cob::op::CACHE { "cache " } else { "dont-cache " })
            })
            .collect();
        println!("{:9} onoff={} {:?} lit={lit:?} ops={ops:?}", t.name, t.def.on_offable as u8, t.locomotion);
    }
}

/// Every `spin ... speed <0>` about one axis of a piece the same script spins about another
/// (which stops that spin when the axis is idle, see `cob.rs`): in the install, only the radar
/// vehicles' `Deactivate`.
#[test]
fn only_the_radar_vehicles_stop_a_spin_about_another_axis() {
    use ta_formats::cob::{op, operand_count};
    let Some((_, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut found = Vec::new();
    for t in &types.types {
        let Some(program) = t.script.as_ref() else { continue };
        let code = &program.script.code;
        // Every spin in the script: (function, piece, axis, its speed when a constant).
        let mut spins: Vec<(&str, u32, u32, Option<i32>)> = Vec::new();
        for f in &program.script.functions {
            let (mut pc, mut pushed) = (f.offset as usize, None);
            while pc + 2 < code.len() {
                let opcode = code[pc];
                let Some(n) = operand_count(opcode) else { break };
                if opcode == op::SPIN {
                    spins.push((f.name.as_str(), code[pc + 1], code[pc + 2], pushed));
                }
                pushed = (opcode == op::PUSH_CONSTANT).then(|| code[pc + 1] as i32);
                if opcode == op::RETURN {
                    break;
                }
                pc += 1 + n;
            }
        }
        for &(f, piece, axis, speed) in &spins {
            if speed == Some(0) && spins.iter().any(|s| s.1 == piece && s.2 != axis && s.3 != Some(0)) {
                found.push(format!("{} {f}", t.name));
            }
        }
    }
    found.sort();
    assert_eq!(found, ["ARMSEER Deactivate", "CORVRAD Deactivate"]);
}
