use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

fn game() -> Sim {
    game_of(&[
        ("GUN", "Weapon1=LASER; BadTargetCategory=DECOY;"),
        ("TARGET", "CanMove=1; MaxVelocity=1; Category=TARGET;"),
        ("DECOY", "CanMove=1; MaxVelocity=1; Category=DECOY;"),
    ])
}

/// A game of made-up units (`UnitName` and their FBI keys) armed with LASER, on flat ground.
fn game_of(units: &[(&str, &str)]) -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ta-target-ranking-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::create_dir_all(dir.join("weapons")).unwrap();
    for &(name, keys) in units {
        std::fs::write(
            dir.join(format!("units/{name}.fbi")),
            format!("[UNITINFO] {{ UnitName={name}; MaxDamage=1000; SightDistance=600; {keys} }}"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("weapons/laser.tdf"),
        "[LASER] { range=400; reloadtime=0.1; weaponvelocity=400; rendertype=0; [DAMAGE] { default=1; } }
         [AAGUN] { range=400; reloadtime=0.1; weaponvelocity=400; rendertype=0; toairweapon=1; [DAMAGE] { default=1; } }",
    )
    .unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(crate::UnitTypes::compile(&data, &|_| None));
    std::fs::remove_dir_all(dir).unwrap();
    let map = ta_formats::tnt::TntMap {
        width: 64,
        height: 64,
        sea_level: 0,
        tiles: Vec::new(),
        tile_map: Vec::new(),
        heights: vec![60; 64 * 64],
        features: vec![ta_formats::tnt::FEATURE_NONE; 64 * 64],
        feature_names: Vec::new(),
        minimap: Default::default(),
    };
    let players: Vec<_> = (0..2)
        .map(|i| crate::PlayerSetup {
            side: "ARM".into(),
            color: i,
            start: FxVec3::ZERO,
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: i == 0,
        })
        .collect();
    Sim::new(
        types,
        data,
        &map,
        &ta_data::MapInfo::default(),
        0,
        &players,
        7,
    )
}

#[test]
fn ranking_keeps_visibility_categories_priority_retaliation_and_id_ties() {
    for case in 0..6 {
        let mut sim = game();
        let spawn = |sim: &mut Sim, name, owner, x, z| {
            sim.spawn_unit(
                sim.types.id(name).unwrap(),
                owner,
                FxVec3::from_ints(x, 0, z),
                0,
                true,
            )
            .unwrap()
        };
        let gun = spawn(&mut sim, "GUN", 0, 300, 300);
        let decoy = spawn(&mut sim, "DECOY", 1, 332, 300);
        let first = spawn(&mut sim, "TARGET", 1, 440, 300);
        spawn(&mut sim, "TARGET", 1, 300, 440); // Equal distance; the lower ID wins.
        let hidden = spawn(&mut sim, "TARGET", 1, 380, 300);
        let attacker = spawn(&mut sim, "TARGET", 1, 500, 300);
        let priority = spawn(&mut sim, "TARGET", 1, 300, 540);
        sim.unit_mut(hidden).unwrap().cloaked = true;
        let rules = crate::mission::MissionRules {
            priority_targets: match case {
                2 | 4 => vec![priority],
                3 => vec![decoy],
                5 => vec![hidden],
                _ => Vec::new(),
            },
            ai_ignored: if case == 4 {
                vec![priority]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        sim.mission = Some(rules);
        if matches!(case, 1 | 2 | 4) {
            let u = sim.unit_mut(gun).unwrap();
            u.last_attacker = Some(attacker);
            u.last_hit_tick = 0;
        }
        sim.grid.rebuild(sim.units.iter().flatten());
        sim.vision.set_all(0, crate::vision::LOS);
        let weapons = sim.weapon_types.clone();
        let weapon = sim.unit_type(sim.unit(gun).unwrap()).weapons[0].unwrap();
        let expected = match case {
            1 | 4 => attacker,
            2 => priority,
            _ => first,
        };
        assert_eq!(
            sim.find_target(gun, 0, weapons.get(weapon), 0, None),
            Some(Target::Unit(expected)),
            "target ranking case {case}"
        );
    }
}

fn spawn(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 0, z), 0, true).unwrap()
}

/// TA's units never lock on to a bad target of their own accord: not when it is the only enemy
/// in reach (an AA K-bot beside a tank), not when it shoots at them; only an attack order sends
/// them at one.
#[test]
fn a_bad_target_is_shot_at_only_by_order() {
    let mut sim = game_of(&[
        ("GUN", "Weapon1=LASER; BadTargetCategory=DECOY; Category=GUN;"),
        ("DECOY", "Weapon1=LASER; CanMove=1; MaxVelocity=1; Category=DECOY;"),
    ]);
    sim.vision.set_all(0, crate::vision::LOS);
    sim.vision.set_all(1, crate::vision::LOS);
    let gun = spawn(&mut sim, "GUN", 0, 300, 300);
    let decoy = spawn(&mut sim, "DECOY", 1, 360, 300);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert!(sim.unit(decoy).unwrap().weapons[0].target == Some(Target::Unit(gun)), "the decoy takes the gun on");
    assert!(sim.unit(gun).unwrap().health < sim.types.get(sim.unit(gun).unwrap().type_id).max_health, "and hits it");
    assert_eq!(sim.unit(gun).unwrap().weapons[0].target, None, "the only enemy near, and shooting at it: a bad target all the same");
    sim.step(&[crate::Command::Order { player: 0, units: vec![gun], order: Order::Attack(Target::Unit(decoy)), queue: false }]);
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(decoy)), "ordered, it fires at it");
}

/// Each weapon keeps its own (TA's `wpri_`, `wsec_badTargetCategory`): a cruiser's guns never
/// pick an aircraft, its depth charges nothing but a submarine.
#[test]
fn each_weapon_keeps_its_own_bad_targets() {
    let mut sim = game_of(&[
        ("TWIN", "Weapon1=LASER; Weapon2=LASER; wpri_badTargetCategory=DECOY; wsec_badTargetCategory=TARGET; BadTargetCategory=DECOY;"),
        ("TARGET", "CanMove=1; MaxVelocity=1; Category=TARGET;"),
        ("DECOY", "CanMove=1; MaxVelocity=1; Category=DECOY;"),
    ]);
    let twin = spawn(&mut sim, "TWIN", 0, 300, 300);
    let decoy = spawn(&mut sim, "DECOY", 1, 340, 300);
    let target = spawn(&mut sim, "TARGET", 1, 380, 300);
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.vision.set_all(0, crate::vision::LOS);
    let weapons = sim.weapon_types.clone();
    let w = |sim: &Sim, k: usize| sim.unit_type(sim.unit(twin).unwrap()).weapons[k].unwrap();
    let (w0, w1) = (w(&sim, 0), w(&sim, 1));
    assert_eq!(sim.find_target(twin, 0, weapons.get(w0), 0, None), Some(Target::Unit(target)), "the first leaves the decoy");
    assert_eq!(sim.find_target(twin, 1, weapons.get(w1), 0, None), Some(Target::Unit(decoy)), "the second leaves the target");
}

/// A weapon whose bad targets are every unit it could hit would never fire of its own accord,
/// which mods' tags of their own make (Total Mayhem's flak towers list `ALL`, which all its units
/// carry; Escalation's anti-air missiles a tag all its aircraft carry): its bad targets are
/// lifted, and it takes aircraft on. One with a unit left to take on keeps to its bad targets.
#[test]
fn a_weapon_whose_bad_targets_are_all_it_could_hit_takes_on_what_it_can() {
    let mut sim = game_of(&[
        ("FLAKKER", "Weapon1=AAGUN; BadTargetCategory=ALL; Category=ALL NOTAIR;"),
        ("PICKY", "Weapon1=AAGUN; wpri_badTargetCategory=FAST; Category=ALL NOTAIR;"),
        ("JET", "CanFly=1; CanMove=1; MaxVelocity=5; Category=ALL VTOL FAST;"),
        ("BLIMP", "CanFly=1; CanMove=1; MaxVelocity=1; Category=ALL VTOL;"),
    ]);
    let types = sim.types.clone();
    assert!(types.by_name("FLAKKER").unwrap().weapon_bad_masks[0].is_empty(), "every unit lists ALL: lifted");
    assert!(!types.by_name("PICKY").unwrap().weapon_bad_masks[0].is_empty(), "the blimp is left to it: kept");
    let flakker = spawn(&mut sim, "FLAKKER", 0, 300, 300);
    let picky = spawn(&mut sim, "PICKY", 0, 300, 500);
    let jet = spawn(&mut sim, "JET", 1, 360, 400);
    {
        let u = sim.unit_mut(jet).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    }
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.vision.set_all(0, crate::vision::LOS);
    let weapons = sim.weapon_types.clone();
    let w = |sim: &Sim, id: UnitId| sim.unit_type(sim.unit(id).unwrap()).weapons[0].unwrap();
    let (flak, aa) = (w(&sim, flakker), w(&sim, picky));
    assert_eq!(sim.find_target(flakker, 0, weapons.get(flak), 0, None), Some(Target::Unit(jet)), "the flak takes the jet on");
    assert_eq!(sim.find_target(picky, 0, weapons.get(aa), 0, None), None, "the jet is FAST: a bad target still");
}

/// Only the units a player can field count: a weapon whose bad targets are every unit the sides
/// can build is lifted, though a unit nobody builds lacks them (Escalation's joke unit and
/// mission-only units lack the `ALL` its weapons list: the Raptor's Starburst Missiles never
/// fired). Once a side can build that unit, the weapon keeps to its bad targets.
#[test]
fn only_the_units_a_player_can_field_count_for_lifting_bad_targets() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let compile = |oddity_built: bool| {
        let dir = std::env::temp_dir().join(format!("ta-fieldable-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        for sub in ["units", "weapons", "gamedata"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for (name, keys) in [
            ("BOSS", "Builder=1; WorkerTime=100; Category=ALL NOTAIR;"),
            ("FLAKKER", "Weapon1=AAGUN; BadTargetCategory=ALL; Category=ALL NOTAIR;"),
            ("JET", "CanFly=1; CanMove=1; MaxVelocity=5; Category=ALL VTOL;"),
            ("ODDITY", "CanFly=1; CanMove=1; MaxVelocity=5; Category=VTOL;"),
        ] {
            std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO] {{ UnitName={name}; MaxDamage=1000; {keys} }}")).unwrap();
        }
        std::fs::write(
            dir.join("weapons/laser.tdf"),
            "[AAGUN] { range=400; reloadtime=0.1; weaponvelocity=400; rendertype=0; toairweapon=1; [DAMAGE] { default=1; } }",
        )
        .unwrap();
        let extra = if oddity_built { " canbuild3=ODDITY;" } else { "" };
        std::fs::write(
            dir.join("gamedata/sidedata.tdf"),
            format!("[SIDE0] {{ name=ARM; commander=BOSS; }} [CANBUILD] {{ [BOSS] {{ canbuild1=FLAKKER; canbuild2=JET;{extra} }} }}"),
        )
        .unwrap();
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        let types = crate::UnitTypes::compile(&data, &|_| None);
        let _ = std::fs::remove_dir_all(&dir);
        types.by_name("FLAKKER").unwrap().weapon_bad_masks[0].is_empty()
    };
    assert!(compile(false), "nobody builds the oddity: every aircraft a player can field lists ALL, so the flak's ALL is lifted");
    assert!(!compile(true), "the oddity built: an aircraft left to the flak, so it keeps to its bad targets");
}

/// TA's own units, every one: what each of its weapons takes on of its own accord, of an aircraft
/// in flight, a tank, a building, a boat and a submarine (`--nocapture` lists them). Held to TA:
/// its anti-air units (the AA K-bots, missile trucks and towers, flak, fighters) never pick what
/// is on the ground or the water, its other armed units never an aircraft, submarines' torpedoes
/// only what is in the water, the cruisers' depth charges only submarines. (The anti-air
/// hovercraft as anti-air, past the slip in their files: see `defs::target_tags`.)
#[test]
fn every_units_weapons_pick_what_tas_rules_let_them() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(crate::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    // (No weapon of TA's has bad targets that are every unit it could hit: none is lifted.)
    for t in &types.types {
        for k in (0..3).filter(|&k| t.weapons[k].is_some()) {
            let tags = match t.def.weapon_bad_target_category[k].trim() { "" => t.def.bad_target_category.as_str(), list => list };
            assert!(types.categories(tags).is_empty() || !t.weapon_bad_masks[k].is_empty(), "{} weapon {}: its bad targets lifted", t.name, k + 1);
        }
    }
    let map = ta_formats::tnt::TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let players: Vec<_> = (0..2)
        .map(|i| crate::PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 3);
    // (Coast To Coast: land in the west, a sea 85 deep in the middle.)
    let (land, sea) = (FxVec3::from_ints(400, 0, 1000), FxVec3::from_ints(1600, 0, 1000));
    assert!(sim.world.depth_at(land.x, land.z) <= Fx::ZERO && sim.world.depth_at(sea.x, sea.z) > Fx::from_int(40));
    let put = |sim: &mut Sim, name: &str, at: FxVec3| sim.spawn_unit(types.id(name).unwrap(), 1, at, 0, true).unwrap();
    let aircraft = put(&mut sim, "ARMBRAWL", land);
    {
        let u = sim.unit_mut(aircraft).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    }
    let targets = [
        ("air", aircraft),
        ("tank", put(&mut sim, "ARMSTUMP", land + FxVec3::from_ints(100, 0, 0))),
        ("building", put(&mut sim, "ARMLLT", land + FxVec3::from_ints(0, 0, 100))),
        ("boat", put(&mut sim, "ARMPT", sea)),
        ("sub", put(&mut sim, "ARMSUB", sea + FxVec3::from_ints(150, 0, 0))),
    ];
    assert!(sim.airborne(sim.unit(aircraft).unwrap()), "the Brawler is flying");
    assert!(sim.in_water(sim.unit(targets[3].1).unwrap()) && !sim.submerged(sim.unit(targets[3].1).unwrap()), "the PT boat is on the water");
    assert!(sim.submerged(sim.unit(targets[4].1).unwrap()), "the submarine is under it");
    let weapons = sim.weapon_types.clone();
    // What a unit's weapon in `slot` (by TA's names) takes on of its own accord.
    let picks = |sim: &Sim, unit: &str, slot: usize| -> Vec<&'static str> {
        let t = types.by_name(unit).unwrap_or_else(|| panic!("{unit}"));
        let w = weapons.get(t.weapons[slot].unwrap_or_else(|| panic!("{unit} has no weapon {}", slot + 1)));
        targets.iter().filter(|&&(_, id)| sim.engages(t.id, slot, w, sim.unit(id).unwrap())).map(|&(kind, _)| kind).collect()
    };
    let mut table = Vec::new();
    for t in &types.types {
        for slot in 0..3 {
            let Some(wid) = t.weapons[slot] else { continue };
            let w = weapons.get(wid);
            // (Only by order: D-guns, bombs and nukes; anti-missiles take on missiles alone.)
            if w.command_fire || w.def.interceptor {
                continue;
            }
            let kinds = picks(&sim, &t.name, slot);
            table.push(format!("{:<9} weapon {} {:<20} {}", t.name, slot + 1, w.name, if kinds.is_empty() { "nothing".to_string() } else { kinds.join(", ") }));
        }
    }
    println!("{}", table.join("\n"));
    let mut wrong = Vec::new();
    let mut expect = |unit: &str, slot: usize, want: &[&str]| {
        let got = picks(&sim, unit, slot);
        if got != want {
            wrong.push(format!("{unit} weapon {}: picks {got:?}, TA {want:?}", slot + 1));
        }
    };
    // Anti-air: aircraft only.
    for unit in ["ARMJETH", "CORCRASH", "ARMSAM", "CORMIST", "ARMRL", "CORRL", "ARMFLAK", "CORFLAK", "ARMYORK", "CORSENT", "ARMFIG", "CORVENG", "ARMHAWK", "CORVAMP"] {
        expect(unit, 0, &["air"]);
    }
    // (The anti-air hovercraft too, though TA's files give them the ground hovercraft's tags.)
    for unit in ["ARMAH", "CORAH"] {
        expect(unit, 0, &["air"]);
    }
    // The ground's: never an aircraft.
    for unit in ["ARMPW", "ARMFLASH", "CORGATOR", "ARMLLT", "CORLLT", "ARMHLT", "CORHLT", "ARMCOM", "CORCOM", "ARMZEUS", "CORPYRO"] {
        expect(unit, 0, &["tank", "building", "boat"]);
    }
    // (TA's A.K. lists no bad targets, the Peewee VTOL: the A.K. takes on aircraft too.)
    expect("CORAK", 0, &["air", "tank", "building", "boat"]);
    // Shells: never an aircraft (they cannot reach one) nor under the water.
    for unit in ["ARMSTUMP", "CORRAID", "ARMBULL", "CORREAP", "ARMGUARD", "CORPUN"] {
        expect(unit, 0, &["tank", "building", "boat"]);
    }
    // Submarines: what is in the water.
    for unit in ["ARMSUB", "CORSUB"] {
        expect(unit, 0, &["boat", "sub"]);
    }
    // The cruisers' depth charges: submarines.
    for unit in ["ARMCRUS", "CORCRUS"] {
        expect(unit, 1, &["sub"]);
    }
    assert!(wrong.is_empty(), "{} units against TA's rules:\n{}", wrong.len(), wrong.join("\n"));
}
