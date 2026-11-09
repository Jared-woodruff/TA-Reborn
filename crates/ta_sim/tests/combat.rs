//! Combat integration test against the real game data (skipped without a TA install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, PlayerSetup, Sim, SimEvent, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: if i == 0 { "ARM".into() } else { "CORE".into() },
            color: i as u8,
            start: FxVec3::ZERO,
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: false,
        })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
}

/// Finds dry, nearly flat ground: an east-west strip varying by at most 10 px.
fn flat_spot(sim: &Sim, len: i64) -> Option<FxVec3> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    for z in (64..h.to_int() - 64).step_by(32) {
        for x in (64..w.to_int() - len - 64).step_by(32) {
            let hs: Vec<Fx> = (0..=len).step_by(16).map(|dx| sim.world.height_at(Fx::from_int(x + dx), Fx::from_int(z))).collect();
            let (lo, hi) = (hs.iter().min()?, hs.iter().max()?);
            if *lo > sea && *hi - *lo <= Fx::from_int(3) {
                return Some(FxVec3::from_ints(x + 20, 0, z));
            }
        }
    }
    None
}

/// Towers (a laser and a plasma battery, whose shells arc clear of the ground) and a tank told
/// to attack ground shell it, and stop when told to: the ground is no target of their own
/// once the order is gone, and their guns let go of it.
#[test]
fn stop_ends_an_attack_on_ground() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let laser = sim.spawn_unit(sim.types.id("ARMLLT").unwrap(), 0, a, 0, true).unwrap();
    let battery = sim.spawn_unit(sim.types.id("ARMGUARD").unwrap(), 0, a + FxVec3::from_ints(0, 0, -90), 0, true).unwrap();
    let tank = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, a + FxVec3::from_ints(40, 0, 60), 0, true).unwrap();
    let units = [laser, battery, tank];
    let ground = a + FxVec3::from_ints(200, 0, 0);
    let shots = |sim: &mut Sim, ticks: u32| {
        let mut n = [0u32; 3];
        for _ in 0..ticks {
            sim.step(&[]);
            for e in sim.take_events() {
                if let SimEvent::WeaponFired { unit, .. } = e {
                    for (k, &u) in units.iter().enumerate() {
                        n[k] += (unit == u) as u32;
                    }
                }
            }
        }
        n
    };
    let order = |order: ta_sim::Order| ta_sim::Command::Order { player: 0, units: units.to_vec(), order, queue: false };
    sim.step(&[order(ta_sim::Order::Attack(ta_sim::Target::Ground(ground)))]);
    let shelling = shots(&mut sim, 300);
    assert!(shelling.iter().all(|&n| n > 0), "all shell the ground: {shelling:?}");
    sim.step(&[order(ta_sim::Order::Stop)]);
    // (A shot on its way or a burst's tail may still go.)
    shots(&mut sim, 30);
    assert_eq!(shots(&mut sim, 300), [0, 0, 0], "Stop ends it");
    for id in units {
        let u = sim.unit(id).unwrap();
        assert!(u.weapons.iter().all(|w| w.target.is_none()), "{}'s guns let go of the ground", sim.unit_type(u).name);
    }
}

/// A shot goes off where it meets what it hits, on the near side of it, not where the tick's
/// flight would have taken it on to (through it, a tick of travel beyond); and the client is
/// told how far through the tick that was, and given the projectile to draw to its end.
#[test]
fn shots_go_off_where_they_meet_their_target() {
    use ta_sim::{Command, Order, Target};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 260).expect("no flat land on the map");
    // A heavy laser tower (800 px a second: 27 px a tick) at a Goliath 200 px east.
    let tower = sim.spawn_unit(sim.types.id("ARMHLT").unwrap(), 0, a, 0, true).unwrap();
    let target = sim.spawn_unit(sim.types.id("CORGOL").unwrap(), 1, a + FxVec3::from_ints(200, 0, 0), -16384, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![tower], order: Order::Attack(Target::Unit(target)), queue: false }]);
    let centre = sim.unit(target).unwrap().pos;
    let mut checked = 0;
    for _ in 0..30 * 12 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::Explosion { pos, when, .. } = e {
                // (The projectile that went off there: kept for the client, at that moment.)
                let (spent, _) = sim
                    .spent
                    .iter()
                    .find(|(p, at)| *at == when && (p.prev_pos + (p.pos - p.prev_pos).scale(when)).distance_xz(pos) < Fx::ONE)
                    .expect("the shot that went off is kept for the client")
                    .clone();
                assert!(when >= Fx::ZERO && when <= Fx::ONE, "{when:?}");
                if spent.owner != tower {
                    continue; // (The Goliath firing back.)
                }
                // On the near side: no further from the tower than the target's middle, and
                // within the target's reach (it is some 40 px across).
                let from_tower = pos.distance_xz(a).to_f32();
                assert!(from_tower <= centre.distance_xz(a).to_f32() + 1.0, "went off {from_tower} px out, beyond the target's middle");
                assert!(pos.distance_xz(centre).to_f32() < 40.0, "went off {:.1} px from the target", pos.distance_xz(centre).to_f32());
                // On the shot's line, `when` of the way through its last tick.
                let on_line = spent.prev_pos + (spent.pos - spent.prev_pos).scale(when);
                assert!(on_line.distance(pos).to_f32() < 0.5, "the blast is where the flight met the target");
                checked += 1;
            }
        }
        if checked >= 3 {
            break;
        }
    }
    assert!(checked >= 3, "the tower should have hit the Goliath a few times ({checked})");
}

/// Shells falling into the sea go off on its surface, where they meet it: splashes, every one.
#[test]
fn shells_splash_where_they_meet_the_sea() {
    use ta_sim::{Command, Order, Target};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (out, land) = shore(&sim, sim.types.id("ARMTSHIP").unwrap(), sim.types.id("ARMFLASH").unwrap(), None).expect("no shoreline found");
    let sea = sim.world.sea_level;
    let gun = sim.spawn_unit(sim.types.id("ARMGUARD").unwrap(), 0, land, 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![gun], order: Order::Attack(Target::Ground(out + FxVec3::from_ints(120, 0, 0))), queue: false }]);
    let mut splashes = 0;
    for _ in 0..30 * 30 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::Explosion { pos, water, .. } = e
                && sim.world.depth_at(pos.x, pos.z) > Fx::ZERO
            {
                assert_eq!(pos.y, sea, "on the surface");
                assert!(water, "a splash");
                splashes += 1;
            }
        }
        if splashes >= 5 {
            break;
        }
    }
    assert!(splashes >= 5, "the Guardian should have shelled the sea ({splashes})");
}

#[test]
fn tank_fights_kbot() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let b = a + FxVec3::from_ints(180, 0, 0);
    let stumpy = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, a, 16384, true).unwrap();
    let ak = sim.spawn_unit(sim.types.id("CORAK").unwrap(), 1, b, -16384, true).unwrap();

    let mut shots = [0u32; 2];
    let mut explosions = 0;
    let mut first_shot_tick = None;
    let mut dead = None;
    for tick in 0..30 * 60 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::WeaponFired { unit, .. } => {
                    shots[(unit != stumpy) as usize] += 1;
                    first_shot_tick.get_or_insert(tick);
                }
                SimEvent::Explosion { .. } => explosions += 1,
                SimEvent::UnitDestroyed { id, .. } => {
                    dead.get_or_insert((id, tick));
                }
                _ => {}
            }
        }
        if dead.is_some() {
            break;
        }
    }
    let hp = |id| sim.unit(id).map(|u| u.health.to_f32());
    println!(
        "first shot at tick {first_shot_tick:?}; shots stumpy={} ak={}; explosions {explosions}; dead {dead:?}; hp stumpy {:?} ak {:?}; wrecks {}",
        shots[0],
        shots[1],
        hp(stumpy),
        hp(ak),
        sim.features.iter().filter(|f| sim.data.features[f.def].name.to_ascii_lowercase().contains("dead")).count()
    );
    assert!(first_shot_tick.is_some_and(|t| t < 90), "no shot within 3 s (aim scripts not completing?)");
    assert!(shots[0] > 0 && shots[1] > 0, "both units should fire");
    assert!(explosions > 0);
    assert!(dead.is_some(), "a minute of fighting should kill one of them");
}

#[test]
fn commander_dguns_on_command() {
    use ta_sim::{Command, Order, Target};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let com_type = sim.types.id("ARMCOM").unwrap();
    let com = sim.spawn_unit(com_type, 0, a, 16384, true).unwrap();
    let thud = sim.spawn_unit(sim.types.id("CORTHUD").unwrap(), 1, a + FxVec3::from_ints(90, 0, 0), -16384, true).unwrap();
    let dgun = sim.types.get(com_type).weapons.iter().flatten().copied().find(|&w| sim.weapon_types.get(w).command_fire).expect("Commander has a D-gun");

    let mut dgun_shots = 0;
    let count = |sim: &mut Sim, shots: &mut u32| {
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, weapon, .. } = e && unit == com && weapon == dgun {
                *shots += 1;
            }
        }
    };
    // Without an order the D-gun stays silent (the laser handles automatic fire).
    for _ in 0..90 {
        sim.step(&[]);
        count(&mut sim, &mut dgun_shots);
    }
    assert_eq!(dgun_shots, 0, "the D-gun must only fire on command");

    let energy_before = sim.players[0].economy.energy;
    sim.step(&[Command::Order { player: 0, units: vec![com], order: Order::ManualFire(Target::Unit(thud)), queue: false }]);
    let mut killed_at = None;
    for tick in 0..30 * 5 {
        sim.step(&[]);
        count(&mut sim, &mut dgun_shots);
        if sim.unit(thud).is_none_or(|u| u.dead) {
            killed_at = Some(tick);
            break;
        }
    }
    let orders_left = sim.unit(com).map_or(0, |u| u.orders.len());
    println!(
        "D-gun shots {dgun_shots}, target killed at tick {killed_at:?}, energy {:.0} -> {:.0}, orders left {orders_left}",
        energy_before.to_f32(),
        sim.players[0].economy.energy.to_f32()
    );
    assert_eq!(dgun_shots, 1, "one manual-fire order fires exactly one D-gun shot");
    assert!(killed_at.is_some(), "the D-gun should destroy a Thud outright");
    assert_eq!(orders_left, 0, "the manual-fire order completes after the shot");
}

#[test]
fn aircraft_bomb_land_and_dogfight() {
    use ta_sim::{Command, Order, Target};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // Flight, aiming and landing are tested with full intelligence; fog is covered separately.
    sim.los_enabled = false;
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let id = |sim: &Sim, n: &str| sim.types.id(n).unwrap();

    // Bomber run on a Thud 600 px away.
    let bomber = sim.spawn_unit(id(&sim, "ARMTHUND"), 0, a + FxVec3::from_ints(-600, 0, 0), 16384, true).unwrap();
    let thud = sim.spawn_unit(id(&sim, "CORTHUD"), 1, a + FxVec3::from_ints(100, 0, 0), -16384, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 1, units: vec![thud], fire: Some(0), movement: Some(0) },
        Command::Order { player: 0, units: vec![bomber], order: Order::Attack(Target::Unit(thud)), queue: false },
    ]);
    let (mut bombs, mut near_hits) = (0, 0);
    let thud_pos = sim.unit(thud).unwrap().pos;
    let mut killed = None;
    for tick in 0..30 * 40 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::WeaponFired { unit, .. } if unit == bomber => bombs += 1,
                SimEvent::Explosion { pos, .. } if pos.within_xz(thud_pos, Fx::from_int(64)) => near_hits += 1,
                _ => {}
            }
        }
        if sim.unit(thud).is_none_or(|u| u.dead) {
            killed = Some(tick);
            break;
        }
    }
    let b = sim.unit(bomber).unwrap();
    println!("bomber: {bombs} bombs, {near_hits} near the Thud, killed at {killed:?}, bomber alt {:.0}", (b.pos.y - sim.world.height_at(b.pos.x, b.pos.z)).to_f32());
    assert!(bombs > 0, "the bomber should drop bombs on an attack order");
    assert!(near_hits * 2 >= bombs, "most bombs should land near the target ({near_hits}/{bombs})");
    // (Aircraft stay up with an enemy about: a Thud the bombs left standing goes first.)
    if sim.unit(thud).is_some_and(|u| !u.dead) {
        sim.step(&[Command::SelfDestruct { player: 1, units: vec![thud] }]);
        for _ in 0..30 * 7 {
            sim.step(&[]);
        }
        assert!(sim.unit(thud).is_none_or(|u| u.dead), "the Thud blew itself up");
    }

    // An idle fighter lands.
    let fig = sim.spawn_unit(id(&sim, "ARMFIG"), 0, a + FxVec3::from_ints(-40, 0, 0), 0, true).unwrap();
    for _ in 0..30 * 8 {
        sim.step(&[]);
    }
    let f = sim.unit(fig).unwrap();
    let alt = (f.pos.y - sim.world.height_at(f.pos.x, f.pos.z)).to_f32();
    println!("idle fighter: landed {} altitude {alt:.1}, pos {:?}, speed {:?}, idle {}, goal {:?}, orders {:?}, pad {:?}", f.landed, f.pos, f.speed, f.idle_ticks, f.path_goal, f.orders, f.air_pad);
    // A full footprint can stand above its centre on uneven ground; its corners
    // must clear the terrain too, and lie level (within the rise a landing allows).
    let fp = sim.unit_type(f).footprint;
    let origin = ta_sim::world::World::footprint_origin(ta_sim::world::World::pos_to_anchor(f.pos, fp), fp);
    let surface = sim.world.footprint_top(origin, fp);
    assert!(f.landed && (f.pos.y - surface).abs() < Fx::from_int(2), "idle aircraft land clear of their terrain footprint");
    let lowest = (origin.1..=origin.1 + fp.1 as i32).flat_map(|z| (origin.0..=origin.0 + fp.0 as i32).map(move |x| (x, z))).map(|(x, z)| sim.world.vertex_height(x, z)).min().unwrap();
    assert!(surface - lowest <= Fx::from_int(sim.unit_type(f).landing().rise()), "idle aircraft land on level ground: {surface:?} over {lowest:?}");

    // Takes off again when ordered, and fights enemy fighters.
    let enemy = sim.spawn_unit(id(&sim, "CORVENG"), 1, a + FxVec3::from_ints(400, 0, 200), -16384, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![fig], order: Order::Attack(Target::Unit(enemy)), queue: false }]);
    let mut shots = 0;
    let mut dead = None;
    for tick in 0..30 * 60 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, .. } = e && unit == fig {
                shots += 1;
            }
        }
        if sim.unit(enemy).is_none_or(|u| u.dead) || sim.unit(fig).is_none_or(|u| u.dead) {
            dead = Some(tick);
            break;
        }
    }
    println!("dogfight: fighter shots {shots}, ended at {dead:?}; fighter alive {}", sim.unit(fig).is_some_and(|u| !u.dead));
    assert!(shots > 0, "the fighter should get shots off");
    assert!(dead.is_some(), "a minute of dogfighting should down one of them");
}

#[test]
fn nuke_launch_and_interception() {
    use ta_sim::{Command, Order, Target};
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let rich = |sim: &mut Sim| {
        for p in &mut sim.players {
            p.economy.metal_capacity = Fx::from_int(1_000_000);
            p.economy.energy_capacity = Fx::from_int(10_000_000);
            p.economy.metal = Fx::from_int(1_000_000);
            p.economy.energy = Fx::from_int(10_000_000);
        }
    };
    let (w, h) = sim.world.size_px();
    let silo_type = sim.types.id("ARMSILO").unwrap();
    let silo_at = sim.find_build_site(silo_type, FxVec3::new(w / 4, Fx::ZERO, h / 2), 40).unwrap();
    let silo = sim.spawn_unit(silo_type, 0, silo_at, 0, true).unwrap();
    // A Thud platoon on the far side of the map.
    let thud = sim.types.id("CORTHUD").unwrap();
    let centre = FxVec3::new(w * 3 / 4, Fx::ZERO, h / 2);
    let mut victims = Vec::new();
    for i in 0..9 {
        let want = centre + FxVec3::from_ints((i % 3) * 60 - 60, 0, (i / 3) * 60 - 60);
        if let Some(p) = sim.find_build_site(thud, want, 12) {
            victims.extend(sim.spawn_unit(thud, 1, p, 0, true));
        }
    }
    // Build the first missile.
    let mut built_at = None;
    for tick in 0..30 * 200 {
        rich(&mut sim);
        sim.step(&[]);
        if built_at.is_none() && sim.unit(silo).is_some_and(|u| u.stockpile > 0) {
            built_at = Some(tick);
            break;
        }
    }
    println!("first missile built after {built_at:?} ticks");
    assert!(built_at.is_some(), "the silo should stockpile a missile");

    let target = sim.unit(victims[4]).map(|u| u.pos).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![silo], order: Order::ManualFire(Target::Ground(target)), queue: false }]);
    let mut detonated = None;
    let nuke_w = sim.types.get(silo_type).weapons[0].unwrap();
    let nw = sim.weapon_types.get(nuke_w);
    println!("nuke weapon: velocity {:.2}/tick accel {:.3} turn {} ttl {} vlaunch {}", nw.velocity.to_f32(), nw.acceleration.to_f32(), nw.turn_rate, nw.flight_ticks, nw.vlaunch);
    for tick in 0..30 * 60 {
        rich(&mut sim);
        sim.step(&[]);
        if tick % 60 == 0 {
            let u = sim.unit(silo).unwrap();
            let p: Vec<String> = sim.projectiles.iter().map(|p| format!("({:.0},{:.0},{:.0}) age {}", p.pos.x.to_f32(), p.pos.y.to_f32(), p.pos.z.to_f32(), p.age)).collect();
            println!("tick {tick}: silo stock {} reload {} target {:?} orders {:?}; projectiles {p:?}", u.stockpile, u.weapons[0].reload, u.weapons[0].target.is_some(), u.orders);
        }
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, .. } = e && unit == silo {
                println!("tick {tick}: LAUNCH");
            }
            if let SimEvent::Explosion { weapon, pos, .. } = e {
                println!("tick {tick}: explosion of {} at ({:.0},{:.0},{:.0})", sim.weapon_types.get(weapon).name, pos.x.to_f32(), pos.y.to_f32(), pos.z.to_f32());
                if sim.weapon_types.get(weapon).def.stockpile && pos.within_xz(target, Fx::from_int(200)) {
                    detonated = Some(tick);
                }
            }
        }
        if detonated.is_some() {
            break;
        }
    }
    let survivors = victims.iter().filter(|&&v| sim.unit(v).is_some_and(|u| !u.dead)).count();
    println!("nuke detonated after {detonated:?} ticks; {survivors} of {} Thuds survived", victims.len());
    assert!(detonated.is_some(), "the nuke should reach its target");
    assert!(survivors <= 1, "a nuke flattens the platoon");

    // Core builds an anti-nuke next to the next target; the second nuke is intercepted.
    let fmd_type = sim.types.id("CORFMD").unwrap();
    let fmd_at = sim.find_build_site(fmd_type, centre + FxVec3::from_ints(0, 0, 300), 40).unwrap();
    let fmd = sim.spawn_unit(fmd_type, 1, fmd_at, 0, true).unwrap();
    for _ in 0..30 * 200 {
        rich(&mut sim);
        sim.step(&[]);
        let ready = sim.unit(silo).is_some_and(|u| u.stockpile > 0) && sim.unit(fmd).is_some_and(|u| u.stockpile > 0);
        if ready {
            break;
        }
    }
    assert!(sim.unit(fmd).is_some_and(|u| u.stockpile > 0), "the anti-nuke stockpiles an interceptor");
    sim.step(&[Command::Order { player: 0, units: vec![silo], order: Order::ManualFire(Target::Ground(centre)), queue: false }]);
    let (mut intercepted, mut nuked) = (false, false);
    for _ in 0..30 * 60 {
        rich(&mut sim);
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::Intercepted { .. } => intercepted = true,
                SimEvent::Explosion { weapon, pos, .. } if sim.weapon_types.get(weapon).def.stockpile && !sim.weapon_types.get(weapon).def.interceptor && pos.within_xz(centre, Fx::from_int(300)) => nuked = true,
                _ => {}
            }
        }
        if intercepted || nuked {
            break;
        }
    }
    println!("second launch: intercepted {intercepted}, detonated {nuked}; anti-nuke stock {:?}", sim.unit(fmd).map(|u| u.stockpile));
    assert!(intercepted && !nuked, "the anti-nuke should intercept the missile");
}

#[test]
fn shipped_static_and_mobile_anti_nukes_run_their_launch_scripts_and_intercept() {
    use ta_sim::{Projectile, Target};
    for name in ["ARMAMD", "CORFMD", "ARMSCAB", "CORMABM"] {
        let Some(mut sim) = sim_on("Painted Desert") else { eprintln!("TA install not found; skipping"); return };
        let Some(type_id) = sim.types.id(name) else { eprintln!("{name} not installed; skipping this unit"); continue };
        let (width, height) = sim.world.size_px();
        let at = sim.find_build_site(type_id, FxVec3::new(width / 4, Fx::ZERO, height / 2), 40).unwrap();
        let defender = sim.spawn_unit(type_id, 0, at, 0, true).unwrap();
        let silo_type = sim.types.id("CORSILO").unwrap();
        let silo_at = sim.find_build_site(silo_type, FxVec3::new(width * 3 / 4, Fx::ZERO, height / 2), 40).unwrap();
        let silo = sim.spawn_unit(silo_type, 1, silo_at, 0, true).unwrap();
        sim.grant_stockpile(defender, 1);
        let weapon = sim.types.get(silo_type).weapons[0].unwrap();
        // A long enough approach for the mobile launcher's deployment and climb;
        // a point-blank launch can legitimately outrun a defence's readiness delay.
        let pos = at + FxVec3::from_ints(3000, 100, 0);
        sim.projectiles.push(Projectile { id: 10000, weapon, owner: silo, owner_player: 1, pos, prev_pos: pos, vel: FxVec3::ZERO, target: Some(Target::Ground(at)), target_pos: at, age: 0, ttl: 12000, hits: Vec::new(), dead: false });
        let (mut launched, mut intercepted, mut nuked) = (None, false, false);
        let mut trace = Vec::new();
        for tick in 1..=30 * 60 {
            sim.step(&[]);
            if tick % 30 == 0 {
                trace.push(format!("{tick}: {:?}", sim.projectiles.iter().map(|p| (p.id, sim.weapon_types.get(p.weapon).name.clone(), p.pos.to_f32(), p.vel.to_f32(), p.age)).collect::<Vec<_>>()));
            }
            for event in sim.take_events() {
                match event {
                    SimEvent::WeaponFired { unit, .. } if unit == defender => { launched.get_or_insert(tick); }
                    SimEvent::Intercepted { .. } => intercepted = true,
                    SimEvent::Explosion { weapon: fired, .. } if fired == weapon => nuked = true,
                    _ => {}
                }
            }
            if intercepted || nuked { break; }
        }
        assert!(intercepted && !nuked, "{name} must intercept with its original weapon/model/script (launch={launched:?}): {trace:?}");
        if name == "ARMAMD" {
            assert!(launched.unwrap() >= 45, "the Protector's 1500 ms AimPrimary preparation cannot be bypassed");
        }
    }
}

#[test]
fn cloak_hides_and_veterans_gain_experience() {
    use ta_sim::{Command, Order, Seen, Target};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let com = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 16384, true).unwrap();
    let ak = sim.spawn_unit(sim.types.id("CORAK").unwrap(), 1, a + FxVec3::from_ints(150, 0, 0), -16384, true).unwrap();
    // Hold fire on both, then cloak the commander.
    sim.step(&[
        Command::Order { player: 0, units: vec![com], order: Order::SetStates { fire: 0, movement: 0 }, queue: false },
        Command::Order { player: 1, units: vec![ak], order: Order::SetStates { fire: 2, movement: 0 }, queue: false },
        Command::SetCloak { player: 0, units: vec![com], on: true },
    ]);
    let energy0 = sim.players[0].economy.energy;
    let mut shots_at_cloaked = 0;
    for _ in 0..60 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, .. } = e && unit == ak {
                shots_at_cloaked += 1;
            }
        }
    }
    let c = sim.unit(com).unwrap();
    println!("cloaked {} energy {:.0} -> {:.0}; AK shots {shots_at_cloaked}; seen {:?}", c.cloaked, energy0.to_f32(), sim.players[0].economy.energy.to_f32(), sim.seen_by(c, 1));
    assert!(c.cloaked, "the commander should be cloaked");
    assert_eq!(sim.seen_by(c, 1), Seen::Hidden, "a cloaked unit is invisible to the enemy");
    assert_eq!(shots_at_cloaked, 0, "enemies can't target what they can't see");
    // Walking into decloak range reveals it.
    let near = sim.unit(com).unwrap().pos + FxVec3::from_ints(30, 0, 0);
    sim.step(&[Command::Order { player: 1, units: vec![ak], order: Order::Move(near), queue: false }]);
    let mut revealed = false;
    for _ in 0..30 * 10 {
        sim.step(&[]);
        if sim.unit(com).is_some_and(|c| sim.seen_by(c, 1) == Seen::Visible) {
            revealed = true;
            break;
        }
    }
    assert!(revealed, "an enemy within MinCloakDistance sees through the cloak");

    // Veterancy: let the commander loose on the AK.
    sim.step(&[
        Command::SetCloak { player: 0, units: vec![com], on: false },
        Command::Order { player: 0, units: vec![com], order: Order::Attack(Target::Unit(ak)), queue: false },
    ]);
    for _ in 0..30 * 20 {
        sim.step(&[]);
        if sim.unit(ak).is_none() {
            break;
        }
    }
    let c = sim.unit(com).unwrap();
    println!("after the fight: kills {} xp {:.2} veterancy {:.2} rank {}", c.kills, c.xp.to_f32(), c.veterancy().to_f32(), c.rank());
    assert_eq!(c.kills, 1);
    assert!(c.xp > Fx::ZERO && c.veterancy() > Fx::ZERO);
}

#[test]
fn atlas_airlifts_a_tank() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (w, h) = sim.world.size_px();
    let start = FxVec3::new(w / 3, Fx::ZERO, h / 3);
    let stump_t = sim.types.id("ARMSTUMP").unwrap();
    let stump_at = sim.find_build_site(stump_t, start, 20).unwrap();
    let stump = sim.spawn_unit(stump_t, 0, stump_at, 0, true).unwrap();
    let atlas = sim.spawn_unit(sim.types.id("ARMATLAS").unwrap(), 0, stump_at + FxVec3::from_ints(-200, 0, -100), 0, true).unwrap();
    let dest = stump_at + FxVec3::from_ints(700, 0, 200);
    sim.step(&[
        Command::Order { player: 0, units: vec![atlas], order: Order::Load(stump), queue: false },
        Command::Order { player: 0, units: vec![atlas], order: Order::Unload(dest), queue: true },
    ]);
    let (mut loaded, mut unloaded) = (None, None);
    let mut max_alt = Fx::ZERO;
    for tick in 0..30 * 60 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::UnitLoaded { unit, .. } if unit == stump => loaded = Some(tick),
                SimEvent::UnitUnloaded { unit, .. } if unit == stump => unloaded = Some(tick),
                _ => {}
            }
        }
        if loaded.is_some() && unloaded.is_none() {
            let s = sim.unit(stump).unwrap();
            max_alt = max_alt.max(s.pos.y - sim.world.height_at(s.pos.x, s.pos.z));
        }
        if unloaded.is_some() {
            break;
        }
    }
    let s = sim.unit(stump).unwrap();
    let off = (s.pos - dest).length_xz().to_f32();
    println!("loaded at {loaded:?}, unloaded at {unloaded:?}; carried up to {:.0} px above ground; set down {off:.0} px from the target; carried_by {:?}", max_alt.to_f32(), s.carried_by);
    assert!(loaded.is_some(), "the Atlas should pick the tank up");
    assert!(max_alt > Fx::from_int(30), "the tank flies with the Atlas");
    assert!(unloaded.is_some() && s.carried_by.is_none(), "and gets set down");
    assert!(off < 80.0, "near where it was ordered");
    // The tank drives again afterwards.
    let p0 = s.pos;
    sim.step(&[Command::Order { player: 0, units: vec![stump], order: Order::Move(p0 + FxVec3::from_ints(150, 0, 0)), queue: false }]);
    for _ in 0..30 * 6 {
        sim.step(&[]);
    }
    assert!(!sim.unit(stump).unwrap().pos.within_xz(p0, Fx::from_int(60)), "the dropped tank should drive on");
}

/// Finds a shoreline: dry land for a tank at the water's edge, and water deep enough for a
/// ship straight out from it.
fn shore(sim: &Sim, ship: usize, tank: usize, skip: Option<FxVec3>) -> Option<(FxVec3, FxVec3)> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    let ht = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    let fits = |kind: usize, p| sim.world.passable(&sim.move_spec(kind), ta_sim::world::World::pos_to_anchor(p, sim.types.get(kind).footprint), None);
    for z in (300..h.to_int() - 300).step_by(96) {
        for x in (300..w.to_int() - 700).step_by(16) {
            // Land here, water just east of it.
            if !(ht(x, z) > sea && ht(x + 16, z) < sea) {
                continue;
            }
            let land = FxVec3::from_ints(x - 24, 0, z);
            // A second shoreline must lie further down the coast.
            if skip.is_some_and(|s| z < s.z.to_int() + 300) || ht(x - 24, z) <= sea || !fits(tank, land) {
                continue;
            }
            // Walk out to ship depth.
            if let Some(d) = (1..30).map(|i| x + 16 + i * 16).find(|&xx| ht(xx, z) < sea - Fx::from_int(18) && fits(ship, FxVec3::from_ints(xx + 32, 0, z))) {
                return Some((FxVec3::from_ints(d + 32, 0, z), land));
            }
        }
    }
    None
}

#[test]
fn transport_ship_ferries_a_tank() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let ship_t = sim.types.id("ARMTSHIP").unwrap();
    let tank_t = sim.types.id("ARMFLASH").unwrap();
    let Some((ship_at, tank_at)) = shore(&sim, ship_t, tank_t, None) else { panic!("no shoreline found") };
    let ship = sim.spawn_unit(ship_t, 0, ship_at, 0, true).unwrap();
    let tank = sim.spawn_unit(tank_t, 0, tank_at, 0, true).unwrap();
    let (_, drop_at) = shore(&sim, ship_t, tank_t, Some(ship_at)).expect("second shoreline");
    sim.step(&[
        Command::Order { player: 0, units: vec![ship], order: Order::Load(tank), queue: false },
        Command::Order { player: 0, units: vec![ship], order: Order::Unload(drop_at), queue: true },
    ]);
    let (mut loaded, mut unloaded) = (None, None);
    for tick in 0..30 * 180 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::UnitLoaded { unit, .. } if unit == tank => loaded = Some(tick),
                SimEvent::UnitUnloaded { unit, .. } if unit == tank => unloaded = Some(tick),
                SimEvent::OrderFailed { unit, reason } if unit == ship => println!("tick {tick}: ship order failed: {reason}"),
                _ => {}
            }
        }
        if unloaded.is_some() {
            break;
        }
    }
    let t = sim.unit(tank).unwrap();
    println!(
        "ship at {:?}, tank at {:?} -> drop {:?}; loaded {loaded:?}, unloaded {unloaded:?}; tank now ({:.0},{:.0}) carried {:?}, {:.0} px from the drop point",
        (ship_at.x.to_int(), ship_at.z.to_int()),
        (tank_at.x.to_int(), tank_at.z.to_int()),
        (drop_at.x.to_int(), drop_at.z.to_int()),
        t.pos.x.to_f32(),
        t.pos.z.to_f32(),
        t.carried_by,
        (t.pos - drop_at).length_xz().to_f32()
    );
    assert!(loaded.is_some(), "the ship's boom should take the tank aboard");
    assert!(unloaded.is_some() && t.carried_by.is_none(), "and set it ashore at the destination");
}

#[test]
fn hover_transport_ferries_tanks_over_water() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let hover_t = sim.types.id("ARMTHOVR").unwrap();
    let tank_t = sim.types.id("ARMFLASH").unwrap();
    let Some((_, tank_at)) = shore(&sim, hover_t, tank_t, None) else { panic!("no shoreline found") };
    let (_, drop_at) = shore(&sim, hover_t, tank_t, Some(tank_at)).expect("second shoreline");
    let hover = sim.spawn_unit(hover_t, 0, tank_at + FxVec3::from_ints(-40, 0, -90), 0, true).unwrap();
    let tanks: Vec<_> = (0..2).map(|i| sim.spawn_unit(tank_t, 0, tank_at + FxVec3::from_ints(-40 * i, 0, 0), 0, true).unwrap()).collect();
    sim.step(&[
        Command::Order { player: 0, units: vec![hover], order: Order::Load(tanks[0]), queue: false },
        Command::Order { player: 0, units: vec![hover], order: Order::Load(tanks[1]), queue: true },
        Command::Order { player: 0, units: vec![hover], order: Order::Unload(drop_at), queue: true },
    ]);
    let (mut loaded, mut unloaded) = (0, 0);
    for tick in 0..30 * 240 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::UnitLoaded { .. } => loaded += 1,
                SimEvent::UnitUnloaded { .. } => unloaded += 1,
                SimEvent::OrderFailed { unit, reason } if unit == hover => println!("tick {tick}: hover order failed: {reason}"),
                _ => {}
            }
        }
        if unloaded == 2 {
            break;
        }
    }
    let off: Vec<f32> = tanks.iter().map(|&t| (sim.unit(t).unwrap().pos - drop_at).length_xz().to_f32()).collect();
    println!("loaded {loaded}, unloaded {unloaded}; tanks {off:?} px from the drop point");
    assert_eq!(loaded, 2, "the hovercraft should take both tanks aboard");
    assert_eq!(unloaded, 2, "and set them down across the water");
    assert!(off.iter().all(|&d| d < 200.0), "near where it was ordered");
}

#[test]
fn self_destruct_counts_down_and_cancels() {
    use ta_sim::Command;
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let com = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    let stumpy = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, a + FxVec3::from_ints(200, 0, 0), 0, true).unwrap();
    // Start, then cancel.
    sim.step(&[Command::SelfDestruct { player: 0, units: vec![stumpy] }]);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert!(sim.unit(stumpy).unwrap().self_destruct_at.is_some(), "counting down");
    sim.step(&[Command::SelfDestruct { player: 0, units: vec![stumpy] }]);
    for _ in 0..200 {
        sim.step(&[]);
    }
    assert!(sim.unit(stumpy).is_some_and(|u| !u.dead && u.self_destruct_at.is_none()), "cancelled");
    // Another player cannot blow up our units.
    sim.step(&[Command::SelfDestruct { player: 1, units: vec![com] }]);
    assert!(sim.unit(com).unwrap().self_destruct_at.is_none());
    // Let it run: gone after five seconds, blowing up as its SelfDestructAs weapon.
    let sd = sim.unit_type(sim.unit(stumpy).unwrap()).def.self_destruct_as.clone();
    sim.step(&[Command::SelfDestruct { player: 0, units: vec![stumpy] }]);
    let (mut blast, mut died) = (false, None);
    for tick in 0..200 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::Explosion { weapon, .. } = e {
                blast |= sim.weapon_types.get(weapon).name == sd;
            }
        }
        if died.is_none() && sim.unit(stumpy).is_none_or(|u| u.dead) {
            died = Some(tick);
        }
    }
    let died = died.expect("the unit should blow up");
    println!("self-destructed after {died} ticks as {sd}");
    assert!((140..=160).contains(&died), "after the five-second countdown");
    assert!(blast, "it explodes as {sd}");
}

#[test]
fn factory_repeat_requeues_finished_units() {
    use ta_sim::Command;
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let lab_t = sim.types.id("ARMLAB").unwrap();
    let pw = sim.types.id("ARMPW").unwrap();
    let site = sim.find_build_site(lab_t, a, 20).expect("site");
    let lab = sim.spawn_unit(lab_t, 0, site, 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(20_000), Fx::from_int(20_000));
    sim.step(&[Command::Produce { player: 0, factory: lab, type_id: pw, count: 1 }, Command::SetRepeat { player: 0, factories: vec![lab], on: true }]);
    for _ in 0..30 * 120 {
        sim.step(&[]);
    }
    let built = sim.units().filter(|u| u.type_id == pw && u.is_complete()).count();
    println!("repeat built {built} Peewees");
    assert!(built >= 3, "repeat keeps producing");
    assert_eq!(sim.unit(lab).unwrap().factory_queue.len(), 1, "the one queued unit cycles");
}

/// Aircraft leave the factory like ground units: once clear of the yard they no longer hold
/// up the next unit (a transport or bomber busy on a long errand used to stall its plant).
#[test]
fn aircraft_leave_the_factory_yard() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (w, h) = sim.world.size_px();
    let plant_t = sim.types.id("ARMAP").unwrap();
    let atlas = sim.types.id("ARMATLAS").unwrap();
    let site = sim.find_build_site(plant_t, FxVec3::new(w / 3, Fx::ZERO, h / 3), 20).expect("site");
    let plant = sim.spawn_unit(plant_t, 0, site, 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(20_000), Fx::from_int(50_000));
    // The plant's aircraft patrol far off: their orders never run out.
    let far = FxVec3::new(w * 2 / 3, Fx::ZERO, h * 2 / 3);
    sim.step(&[
        Command::Produce { player: 0, factory: plant, type_id: atlas, count: 2 },
        Command::Order { player: 0, units: vec![plant], order: Order::Patrol(far), queue: false },
    ]);
    for _ in 0..30 * 200 {
        sim.step(&[]);
    }
    let built: Vec<&ta_sim::Unit> = sim.units().filter(|u| u.type_id == atlas && u.is_complete()).collect();
    println!("{} transports finished; out of the yard: {:?}", built.len(), built.iter().map(|u| u.factory.is_none()).collect::<Vec<_>>());
    assert_eq!(built.len(), 2, "the second transport is built while the first is out on patrol");
    assert!(built.iter().all(|u| u.factory.is_none() && !u.orders.is_empty()), "both have left the yard, patrolling");
}

/// Short of resources, construction still gets to the end: each tick applies only part of
/// what remains, and the last fraction of a percent used to round away to nothing, leaving
/// a building at 99.99% and its builder stuck on it for good.
#[test]
fn a_stalled_economy_still_finishes_buildings() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let [com, ck, win, lab, pw] = ["ARMCOM", "ARMCK", "ARMWIN", "ARMLAB", "ARMPW"].map(|n| sim.types.id(n).unwrap());
    sim.spawn_unit(com, 0, a, 0, true).unwrap();
    let b1 = sim.spawn_unit(ck, 0, a + FxVec3::from_ints(0, 0, 120), 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(0), Fx::from_int(50_000));
    // Two labs turning out kbots want far more metal than comes in: the build factor stays low.
    let mut orders = Vec::new();
    for dx in [-320, 320] {
        let site = sim.find_build_site(lab, a + FxVec3::from_ints(dx, 0, -200), 40).unwrap();
        let f = sim.spawn_unit(lab, 0, site, 0, true).unwrap();
        orders.push(Command::Produce { player: 0, factory: f, type_id: pw, count: 50 });
    }
    // And constructors on vehicle plants: demand that never pauses.
    let vp = sim.types.id("ARMVP").unwrap();
    for dx in [-360, 360] {
        let b = sim.spawn_unit(ck, 0, a + FxVec3::from_ints(dx, 0, 200), 0, true).unwrap();
        let site = sim.find_build_site(vp, a + FxVec3::from_ints(dx, 0, 330), 40).unwrap();
        orders.push(Command::Order { player: 0, units: vec![b], order: Order::Build { type_id: vp, pos: site }, queue: false });
    }
    let w_site = sim.find_build_site(win, a + FxVec3::from_ints(0, 0, 220), 30).unwrap();
    orders.push(Command::Order { player: 0, units: vec![b1], order: Order::Build { type_id: win, pos: w_site }, queue: false });
    sim.step(&orders);
    let mut finished = None;
    let mut factor = Fx::ONE;
    for tick in 0..30 * 400 {
        sim.step(&[]);
        factor = factor.min(sim.players[0].economy.build_factor);
        if sim.units().any(|u| u.type_id == win && u.is_complete()) {
            finished = Some(tick);
            break;
        }
    }
    for _ in 0..5 {
        sim.step(&[]);
    }
    let progress = sim.units().find(|u| u.type_id == win).map(|u| u.build_progress.to_f64());
    println!("wind generator finished at tick {finished:?} (progress {progress:?}, lowest build factor {:.2})", factor.to_f32());
    assert!(factor < Fx::HALF, "the economy should have been stalled");
    assert!(finished.is_some(), "the wind generator must be finished, not left at {progress:?}");
    assert!(sim.unit(b1).is_some_and(|b| b.build.is_none()), "and its builder freed");
}

#[test]
fn maneuvering_units_answer_fire_and_hold_position_stays() {
    use ta_sim::Command;
    let run = |move_state: u8| -> Option<(f32, f32)> {
        let mut sim = sim_on("Coast To Coast")?;
        let a = flat_spot(&sim, 220)?;
        let stumpy_t = sim.types.id("ARMSTUMP")?;
        let reach = |sim: &Sim, t: usize| sim.types.get(t).weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).range.to_f32()).fold(0.0, f32::max);
        let mine = reach(&sim, stumpy_t);
        // An attacker that outranges the tank.
        let (enemy_t, theirs) = ["CORTHUD", "CORWOLV", "CORMORT", "CORPUN"]
            .iter()
            .filter_map(|n| sim.types.id(n))
            .map(|t| (t, reach(&sim, t)))
            .find(|&(_, r)| r > mine + 80.0)?;
        let d = (theirs - 30.0).min(mine + 300.0);
        let stumpy = sim.spawn_unit(stumpy_t, 0, a, 16384, true)?;
        let start = sim.unit(stumpy)?.pos;
        let enemy = sim.spawn_unit(enemy_t, 1, a + FxVec3::new(Fx::from_f32(d), Fx::ZERO, Fx::ZERO), -16384, true)?;
        sim.step(&[]);
        assert_eq!(sim.seen_by(sim.unit(enemy)?, 0), ta_sim::Seen::Hidden, "the tank still cannot see the artillery it will answer");
        println!("{} (range {theirs:.0}) shells a Stumpy (range {mine:.0}) from {d:.0} px; move state {move_state}", sim.types.get(enemy_t).name);
        sim.step(&[
            Command::SetStates { player: 0, units: vec![stumpy], fire: Some(2), movement: Some(move_state) },
            // Shell the known position, without granting either side a hidden unit target.
            Command::Order { player: 1, units: vec![enemy], order: ta_sim::Order::Attack(ta_sim::Target::Ground(start)), queue: false },
        ]);
        let mut closest = f32::MAX;
        let mut moved = 0.0f32;
        for _ in 0..30 * 25 {
            sim.step(&[]);
            let Some(u) = sim.unit(stumpy) else { break };
            moved = moved.max((u.pos - start).length_xz().to_f32());
            if let Some(e) = sim.unit(enemy) {
                closest = closest.min((u.pos - e.pos).length_xz().to_f32());
            }
        }
        Some((moved, closest))
    };
    let Some((moved, closest)) = run(1) else {
        eprintln!("TA install or units not found; skipping");
        return;
    };
    println!("maneuver: moved {moved:.0} px, closest {closest:.0} px");
    assert!(moved > 80.0, "a maneuvering unit goes after an attacker it cannot reach");
    let (held, _) = run(0).unwrap();
    println!("hold position: moved {held:.0} px");
    assert!(held < 12.0, "hold position stays put");
}

#[test]
fn allies_share_vision_and_hold_fire_on_each_other() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let players: Vec<PlayerSetup> = (0..3)
        .map(|i| PlayerSetup { side: ["ARM", "ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 5);
    // Players 0 and 1 are allies; 2 is against both.
    sim.set_team(0, 0);
    sim.set_team(1, 0);
    sim.set_team(2, 1);
    assert!(sim.allied(0, 1) && sim.is_enemy(0, 2) && sim.is_enemy(1, 2) && !sim.is_enemy(1, 1));
    // (Dry land all the way to the enemy 200 px west: in the sea it would be under the water,
    // where only torpedoes reach.)
    let a = flat_spot(&sim, 440).expect("no flat land on the map") + FxVec3::from_ints(220, 0, 0);
    let stumpy = types.id("ARMSTUMP").unwrap();
    let mine = sim.spawn_unit(stumpy, 0, a, 16384, true).unwrap();
    let ally = sim.spawn_unit(stumpy, 1, a + FxVec3::from_ints(60, 0, 0), 16384, true).unwrap();
    // Allies side by side for a few seconds: nobody fires.
    let mut shots = 0;
    for _ in 0..30 * 4 {
        sim.step(&[]);
        shots += sim.take_events().iter().filter(|e| matches!(e, SimEvent::WeaponFired { .. })).count();
    }
    assert_eq!(shots, 0, "allies never target each other");
    // An enemy far from the ally but close to our tank: the ally sees it through us.
    let enemy = sim.spawn_unit(types.id("CORRAID").unwrap(), 2, a + FxVec3::from_ints(-200, 0, 0), 16384, true).unwrap();
    for _ in 0..8 {
        sim.step(&[]);
    }
    let e = sim.unit(enemy).unwrap();
    assert_eq!(sim.seen_by(e, 0), ta_sim::Seen::Visible);
    assert_eq!(sim.seen_by(e, 1), ta_sim::Seen::Visible, "allies share line of sight");
    assert_eq!(sim.seen_by(sim.unit(ally).unwrap(), 0), ta_sim::Seen::Visible);
    // And both allies turn on the common enemy.
    let mut fired_by = std::collections::BTreeSet::new();
    for _ in 0..30 * 20 {
        sim.step(&[]);
        for ev in sim.take_events() {
            if let SimEvent::WeaponFired { unit, .. } = ev {
                fired_by.insert(sim.unit(unit).map(|u| u.owner));
            }
        }
        if sim.unit(enemy).is_none_or(|u| u.dead) {
            break;
        }
    }
    println!("fired by players {fired_by:?}; mine alive {}, ally alive {}", sim.unit(mine).is_some(), sim.unit(ally).is_some());
    assert!(fired_by.contains(&Some(0)) && fired_by.contains(&Some(2)), "the enemy and our side exchange fire");
}

#[test]
fn necro_resurrects_a_wreck_for_its_side() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let Some(necro_t) = sim.types.id("CORNECRO") else {
        eprintln!("no Necro in this data; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let stumpy_t = sim.types.id("ARMSTUMP").unwrap();
    // An Arm tank dies to small arms fire (a light killing blow leaves a wreck).
    let victim = sim.spawn_unit(stumpy_t, 0, a + FxVec3::from_ints(120, 0, 0), 0, true).unwrap();
    let before = sim.features.iter().filter(|f| f.alive).count();
    let killer = sim.spawn_unit(sim.types.id("CORAK").unwrap(), 1, a + FxVec3::from_ints(260, 0, 0), -16384, true).unwrap();
    // The tank holds its fire, or it would win.
    sim.step(&[Command::SetStates { player: 0, units: vec![victim], fire: Some(0), movement: Some(0) }]);
    for _ in 0..30 * 90 {
        sim.step(&[]);
        if sim.unit(victim).is_none() {
            break;
        }
    }
    assert!(sim.unit(victim).is_none(), "the tank should die");
    let wreck = (0..sim.features.len()).rev().find(|&i| sim.features[i].alive && sim.wreck_type(i) == Some(stumpy_t));
    let Some(wreck) = wreck else {
        eprintln!("no wreck left (killing blow too heavy); skipping; features {before} -> {}", sim.features.iter().filter(|f| f.alive).count());
        return;
    };
    // Core's Necro raises it.
    sim.set_starting_resources(1, Fx::from_int(5000), Fx::from_int(20_000));
    let necro = sim.spawn_unit(necro_t, 1, a, 16384, true).unwrap();
    let _ = killer;
    sim.step(&[Command::Order { player: 1, units: vec![necro], order: Order::Resurrect(wreck), queue: false }]);
    let mut risen = None;
    for tick in 0..30 * 90 {
        sim.step(&[]);
        if let Some(u) = sim.units().find(|u| u.type_id == stumpy_t && u.owner == 1) {
            risen = Some((tick, u.health.to_f32()));
            break;
        }
    }
    let (tick, health) = risen.expect("the wreck should rise as a Core-owned Stumpy");
    println!("resurrected after {tick} ticks with health {health}");
    assert!(!sim.features[wreck].alive, "the wreck is used up");
    assert!(sim.unit(necro).is_some_and(|u| u.orders.is_empty()), "the order completes");
}

#[test]
fn missile_towers_shoot_aircraft_and_infantry_prefer_the_ground() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let t = |sim: &Sim, n: &str| sim.types.id(n).unwrap();
    // A Defender missile tower (no toairweapon flag, BadTargetCategory=NOTAIR) and a
    // Peewee (BadTargetCategory=VTOL) under a Core fighter, with a Core AK on the ground.
    let tower_t = t(&sim, "ARMRL");
    let site = sim.find_build_site(tower_t, a, 12).unwrap();
    let tower = sim.spawn_unit(tower_t, 0, site, 0, true).unwrap();
    let pw = sim.spawn_unit(t(&sim, "ARMPW"), 0, a + FxVec3::from_ints(-60, 0, 0), 16384, true).unwrap();
    let fighter = sim.spawn_unit(t(&sim, "CORVENG"), 1, a + FxVec3::from_ints(40, 0, 60), 0, true).unwrap();
    let ak = sim.spawn_unit(t(&sim, "CORAK"), 1, a + FxVec3::from_ints(-150, 0, 0), -16384, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 1, units: vec![fighter, ak], fire: Some(0), movement: Some(0) },
        Command::Order { player: 1, units: vec![fighter], order: Order::Patrol(a + FxVec3::from_ints(40, 0, -60)), queue: false },
    ]);
    let (mut tower_at_air, mut pw_targets) = (0, Vec::new());
    for _ in 0..30 * 12 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, .. } = e && unit == tower {
                tower_at_air += 1;
            }
        }
        if let Some(target) = sim.unit(pw).and_then(|u| u.weapons[0].target) {
            pw_targets.push(target);
        }
    }
    println!("tower shots {tower_at_air}; fighter alive {}; peewee targets {:?}", sim.unit(fighter).is_some(), pw_targets.first());
    assert!(tower_at_air > 0, "a missile tower must engage aircraft");
    assert!(pw_targets.first().is_some_and(|t| *t == ta_sim::Target::Unit(ak)), "the Peewee picks the ground target first");
}

#[test]
fn a_destroyed_nanoframe_ends_the_build_order() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let com = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    let solar = sim.types.id("ARMSOLAR").unwrap();
    let site = sim.find_build_site(solar, a + FxVec3::from_ints(0, 0, 90), 12).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![com], order: Order::Build { type_id: solar, pos: site }, queue: false }]);
    let mut frame = None;
    for _ in 0..30 * 10 {
        sim.step(&[]);
        frame = sim.units().find(|u| u.type_id == solar && !u.is_complete()).map(|u| u.id);
        if frame.is_some() {
            break;
        }
    }
    let frame = frame.expect("the Commander lays a nanoframe");
    // Blow the frame up (the Commander's own self-destruct order would do, on a unit it owns).
    sim.step(&[Command::SelfDestruct { player: 0, units: vec![frame] }]);
    for _ in 0..30 * 8 {
        sim.step(&[]);
    }
    let frames = sim.units().filter(|u| u.type_id == solar).count();
    println!("solars after the frame died: {frames}; commander orders {:?}", sim.unit(com).map(|u| u.orders.len()));
    assert_eq!(frames, 0, "no fresh nanoframe is laid");
    assert!(sim.unit(com).is_some_and(|u| u.orders.is_empty()), "the build order ended with its frame");
}

#[test]
fn mines_go_off_under_enemies_and_invaders_crawl_to_their_target() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let (Some(mine_t), Some(vader_t)) = (sim.types.id("ARMMINE1"), sim.types.id("ARMVADER")) else {
        eprintln!("no mines in this data; skipping");
        return;
    };
    let ak_t = sim.types.id("CORAK").unwrap();
    // A mine in the path of a Core AK; our own Peewee walking over it first sets nothing off.
    let mine = sim.spawn_unit(mine_t, 0, a + FxVec3::from_ints(100, 0, 0), 0, true).unwrap();
    let pw = sim.spawn_unit(sim.types.id("ARMPW").unwrap(), 0, a, 16384, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 0, units: vec![pw], fire: Some(0), movement: Some(0) },
        Command::Order { player: 0, units: vec![pw], order: Order::Move(a + FxVec3::from_ints(200, 0, 0)), queue: false },
    ]);
    for _ in 0..30 * 6 {
        sim.step(&[]);
    }
    assert!(sim.unit(mine).is_some(), "friendly feet don't set a mine off");
    let ak = sim.spawn_unit(ak_t, 1, a + FxVec3::from_ints(260, 0, 30), -16384, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 1, units: vec![ak], fire: Some(0), movement: Some(0) },
        Command::Order { player: 1, units: vec![ak], order: Order::Move(a + FxVec3::from_ints(-100, 0, 0)), queue: false },
    ]);
    let mut blew = None;
    for tick in 0..30 * 10 {
        sim.step(&[]);
        if sim.unit(mine).is_none() {
            blew = Some(tick);
            break;
        }
    }
    println!("mine went off after {blew:?} ticks; AK health {:?}", sim.unit(ak).map(|u| u.health.to_f32()));
    assert!(blew.is_some(), "an enemy walking over the mine sets it off");
    assert!(sim.unit(ak).is_none_or(|u| u.health < sim.unit_type(u).max_health), "and it hurts");

    // An Invader told to attack walks up and blows up next to the target.
    // Its route and detonation are the subject here, with the target already known.
    sim.los_enabled = false;
    // Use the measured dry strip: subtracting 100 from its z once spawned the
    // Invader outside the map, relying on illegal movement to enter the battlefield.
    let remaining: Vec<_> = sim.units().map(|u| u.id).collect();
    for id in remaining { sim.remove_unit(id); }
    let target = sim.spawn_unit(ak_t, 1, a + FxVec3::from_ints(200, 0, 0), 0, true).unwrap();
    let vader = sim.spawn_unit(vader_t, 0, a, 16384, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 1, units: vec![target], fire: Some(0), movement: Some(0) },
        Command::Order { player: 0, units: vec![vader], order: Order::Attack(ta_sim::Target::Unit(target)), queue: false },
    ]);
    let mut went = None;
    for tick in 0..30 * 20 {
        sim.step(&[]);
        if sim.unit(vader).is_none() {
            went = Some(tick);
            break;
        }
    }
    println!("invader detonated after {went:?} ticks; target health {:?}", sim.unit(target).map(|u| u.health.to_f32()));
    assert!(went.is_some(), "the Invader reaches its target and detonates");
}

#[test]
fn spider_paralyzes_and_the_stun_wears_off() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let Some(spider_t) = sim.types.id("ARMSPID") else {
        eprintln!("no Spider in this data; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let spider = sim.spawn_unit(spider_t, 0, a, 16384, true).unwrap();
    let victim = sim.spawn_unit(sim.types.id("CORAK").unwrap(), 1, a + FxVec3::from_ints(150, 0, 0), -16384, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 1, units: vec![victim], fire: Some(0), movement: Some(0) },
        Command::Order { player: 0, units: vec![spider], order: Order::Attack(ta_sim::Target::Unit(victim)), queue: false },
    ]);
    let mut stunned_at = None;
    for tick in 0..30 * 10 {
        sim.step(&[]);
        if sim.unit(victim).is_some_and(|u| u.paralyzed_until > sim.tick) {
            stunned_at = Some(tick);
            break;
        }
    }
    let stunned_at = stunned_at.expect("the Spider's beam paralyzes the AK");
    let v = sim.unit(victim).unwrap();
    let left = v.paralyzed_until - sim.tick;
    println!("stunned at tick {stunned_at} for {left} ticks; health {}", v.health.to_f32());
    assert_eq!(v.health, sim.unit_type(v).max_health, "a paralyzer does no damage");
    assert!((400..=460).contains(&left), "900 in TA's units is 15 seconds of stun");
    // Stunned: an order doesn't move it.
    let before = v.pos;
    sim.step(&[
        // The Spider stops shooting (or it would keep the AK stunned).
        Command::Order { player: 0, units: vec![spider], order: Order::Stop, queue: false },
        Command::SetStates { player: 0, units: vec![spider], fire: Some(0), movement: None },
        Command::Order { player: 1, units: vec![victim], order: Order::Move(a + FxVec3::from_ints(400, 0, 0)), queue: false },
    ]);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert!((sim.unit(victim).unwrap().pos - before).length_xz() < Fx::from_int(2), "a stunned unit stays put");
    // It wears off.
    for _ in 0..30 * 16 {
        sim.step(&[]);
    }
    assert!((sim.unit(victim).unwrap().pos - before).length_xz() > Fx::from_int(40), "once the stun wears off it walks away");
}

#[test]
fn damaged_aircraft_mend_on_a_repair_pad() {
    use ta_sim::{Command, Order};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let pad_t = sim.types.id("ARMASP").unwrap();
    let site = sim.find_build_site(pad_t, a, 16).unwrap();
    let pad = sim.spawn_unit(pad_t, 0, site, 0, true).unwrap();
    let fighter = sim.spawn_unit(sim.types.id("ARMFIG").unwrap(), 0, a + FxVec3::from_ints(300, 0, 0), 0, true).unwrap();
    let max = sim.unit_type(sim.unit(fighter).unwrap()).max_health;
    sim.step(&[]);
    let hurt = |sim: &Sim| sim.unit(fighter).map(|u| u.health);
    // An enemy AK below shoots it up a little.
    let ak = sim.spawn_unit(sim.types.id("CORAK").unwrap(), 1, a + FxVec3::from_ints(300, 0, 40), 0, true).unwrap();
    for _ in 0..30 * 6 {
        sim.step(&[]);
        if hurt(&sim).is_some_and(|h| h < max * 7 / 10) {
            break;
        }
    }
    // Both sides hold fire now (the fighter would keep strafing the AK and never land).
    sim.step(&[
        Command::SetStates { player: 1, units: vec![ak], fire: Some(0), movement: Some(0) },
        Command::SetStates { player: 0, units: vec![fighter], fire: Some(0), movement: None },
    ]);
    let before = hurt(&sim).expect("the fighter survives");
    assert!(before < max, "the fighter should be damaged ({} of {})", before.to_f32(), max.to_f32());
    // Off to the pad; it lands and mends.
    let pad_pos = sim.unit(pad).unwrap().pos;
    sim.step(&[Command::Order { player: 0, units: vec![fighter], order: Order::Move(pad_pos), queue: false }]);
    let mut mended = None;
    for tick in 0..30 * 90 {
        sim.step(&[]);
        if hurt(&sim).is_some_and(|h| h >= max) {
            mended = Some(tick);
            break;
        }
    }
    println!("fighter at {:.0}/{:.0} mended after {mended:?} ticks; landed {:?}", before.to_f32(), max.to_f32(), sim.unit(fighter).map(|u| u.landed));
    assert!(mended.is_some(), "the pad repairs the landed fighter");
}

#[test]
fn allies_share_resources_and_a_resigning_player_blows_up() {
    use ta_sim::Command;
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    // Two players: both on team 0 at first (allies), then player 1 resigns.
    sim.set_team(1, 0);
    let com0 = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    // Far apart: a Commander's death blast is enormous.
    let com1 = sim.spawn_unit(sim.types.id("CORCOM").unwrap(), 1, a + FxVec3::from_ints(1200, 0, 0), 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(900), Fx::from_int(900));
    sim.set_starting_resources(1, Fx::from_int(100), Fx::from_int(100));
    sim.step(&[]);
    let (m0, m1) = (sim.players[0].economy.metal, sim.players[1].economy.metal);
    sim.step(&[Command::Share { player: 0, to: 1, metal: Fx::from_int(300), energy: Fx::ZERO }]);
    let moved = sim.players[1].economy.metal - m1;
    println!("shared: player 0 {} -> {}, player 1 gained {}", m0.to_f32(), sim.players[0].economy.metal.to_f32(), moved.to_f32());
    assert!(moved >= Fx::from_int(299) && moved <= Fx::from_int(310), "the ally receives the metal (plus a tick of income)");
    // Nothing moves to an enemy.
    sim.set_team(1, 1);
    let before = sim.players[1].economy.metal;
    sim.step(&[Command::Share { player: 0, to: 1, metal: Fx::from_int(100), energy: Fx::ZERO }]);
    assert!(sim.players[1].economy.metal - before < Fx::from_int(5), "no sharing with enemies");
    // Resigning: defeated, units blow up within a few seconds.
    sim.step(&[Command::Resign { player: 1 }]);
    assert!(sim.players[1].defeated);
    for _ in 0..30 * 8 {
        sim.step(&[]);
    }
    assert!(sim.unit(com1).is_none(), "the resigning player's units blow up");
    assert!(sim.unit(com0).is_some(), "the other player is unaffected");
}

/// TA's +setsharemetal N: everything above N goes to allies with room, each tick; +sharemetal
/// off: what full storage would waste is wasted, not shared.
#[test]
fn players_share_what_they_choose() {
    use ta_sim::{Command, Sharing};
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    sim.set_team(1, 0);
    sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    sim.spawn_unit(sim.types.id("CORCOM").unwrap(), 1, a + FxVec3::from_ints(1200, 0, 0), 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(900), Fx::from_int(900));
    sim.set_starting_resources(1, Fx::from_int(100), Fx::from_int(100));
    sim.step(&[]);
    // Keep 300 metal: the rest goes to the ally.
    let keep = Sharing { keep_metal: Some(Fx::from_int(300)), ..Sharing::default() };
    sim.step(&[Command::SetSharing { player: 0, sharing: keep }]);
    sim.step(&[]);
    let (m0, m1) = (sim.players[0].economy.metal.to_f32(), sim.players[1].economy.metal.to_f32());
    println!("kept {m0}, ally has {m1}");
    assert!((m0 - 300.0).abs() < 2.0, "player 0 keeps 300 metal, not {m0}");
    assert!(m1 > 650.0, "the ally received the rest, has {m1}");
    // Energy was left to overflow only: the ally got none of it.
    assert!(sim.players[1].economy.energy.to_f32() < 200.0);
    // Sharing off: a full store wastes what it cannot hold.
    let off = Sharing { metal: false, energy: false, keep_metal: None, keep_energy: None };
    sim.set_starting_resources(0, Fx::from_int(100_000), Fx::from_int(100_000));
    sim.step(&[Command::SetSharing { player: 0, sharing: off }]);
    let before = sim.players[1].economy.metal;
    sim.step(&[]);
    assert!(sim.players[1].economy.metal - before < Fx::from_int(5), "nothing shared while sharing is off");
}

#[test]
fn allies_can_be_given_units() {
    use ta_sim::Command;
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    sim.set_team(1, 0);
    let com0 = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    sim.spawn_unit(sim.types.id("CORCOM").unwrap(), 1, a + FxVec3::from_ints(1200, 0, 0), 0, true).unwrap();
    let stumpy = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, a + FxVec3::from_ints(120, 0, 0), 0, true).unwrap();
    let lab = sim.spawn_unit(sim.types.id("ARMLAB").unwrap(), 0, a + FxVec3::from_ints(0, 0, 160), 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(5000), Fx::from_int(5000));
    sim.set_starting_resources(1, Fx::from_int(5000), Fx::from_int(5000));
    let peewee = sim.types.id("ARMPW").unwrap();
    sim.step(&[Command::Produce { player: 0, factory: lab, type_id: peewee, count: 1 }]);
    let mut yard = None;
    for _ in 0..30 * 10 {
        sim.step(&[]);
        yard = sim.unit(lab).and_then(|u| u.build.as_ref()).map(|b| b.target);
        if yard.is_some() {
            break;
        }
    }
    let yard = yard.expect("the lab starts a Peewee");
    assert!(sim.unit(yard).is_some_and(|u| u.type_id == peewee && u.owner == 0 && !u.is_complete() && u.factory == Some(lab)), "the gift includes an unfinished Peewee on the lab's pad");
    let counts = (sim.players[0].unit_count, sim.players[1].unit_count);
    sim.step(&[Command::GiveUnits { player: 0, units: vec![com0, stumpy, lab], to: 1 }]);
    let given: Vec<_> = sim.take_events().into_iter().filter_map(|e| if let SimEvent::UnitsGiven { from, to, count } = e { Some((from, to, count)) } else { None }).collect();
    assert_eq!(given, vec![(0, 1, 3)], "the gift reports the Stumpy, the lab and its unfinished Peewee");
    assert_eq!(sim.unit(com0).map(|u| u.owner), Some(0), "Commanders stay");
    assert_eq!(sim.unit(stumpy).map(|u| u.owner), Some(1));
    assert_eq!(sim.unit(lab).map(|u| u.owner), Some(1));
    assert_eq!(sim.unit(yard).map(|u| u.owner), Some(1), "the unit in the yard goes with its factory");
    assert_eq!(sim.unit(lab).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [peewee], "the inherited frame stays represented in its new owner's queue");
    assert_eq!((sim.players[0].unit_count, sim.players[1].unit_count), (counts.0 - 3, counts.1 + 3));
    // The lab finishes the Peewee for its new owner.
    for _ in 0..30 * 40 {
        sim.step(&[]);
        if sim.unit(yard).is_some_and(|u| u.is_complete()) {
            break;
        }
    }
    assert!(sim.unit(yard).is_some_and(|u| u.is_complete() && u.owner == 1), "the Peewee is finished for player 1");
    // Nothing goes to an enemy.
    sim.set_team(1, 1);
    let other = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, a + FxVec3::from_ints(-120, 0, 0), 0, true).unwrap();
    sim.step(&[Command::GiveUnits { player: 0, units: vec![other], to: 1 }]);
    assert_eq!(sim.unit(other).map(|u| u.owner), Some(0), "no gifts to enemies");
    // A player who leaves with no ally keeps its army; with one, the ally takes it.
    sim.step(&[Command::Abandon { player: 0 }]);
    assert_eq!(sim.unit(other).map(|u| u.owner), Some(0), "no ally: the units stay");
    sim.set_team(1, 0);
    sim.step(&[Command::Abandon { player: 0 }]);
    assert_eq!(sim.unit(other).map(|u| u.owner), Some(1), "the ally takes over");
    assert_eq!(sim.unit(com0).map(|u| u.owner), Some(0), "but not the Commander");
}

#[test]
fn allies_take_what_full_storage_would_waste() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let players: Vec<PlayerSetup> = (0..3)
        .map(|i| PlayerSetup { side: ["ARM", "ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 5);
    sim.set_team(0, 0);
    sim.set_team(1, 0);
    sim.set_team(2, 1);
    let a = flat_spot(&sim, 400).expect("no flat land on the map");
    for (p, name, dx) in [(0u8, "ARMCOM", 0), (1, "ARMCOM", 150), (2, "CORCOM", 300)] {
        sim.spawn_unit(types.id(name).unwrap(), p, a + FxVec3::from_ints(dx, 0, 0), 0, true).unwrap();
    }
    sim.step(&[]);
    // Player 0 full to the brim; the other two empty, with the same income and storage.
    let cap = sim.players[0].economy.energy_capacity;
    sim.players[0].economy.energy = cap;
    sim.players[1].economy.energy = Fx::ZERO;
    sim.players[2].economy.energy = Fx::ZERO;
    for _ in 0..30 * 10 {
        sim.step(&[]);
    }
    let energy = |p: usize| sim.players[p].economy.energy.to_f32();
    println!("after 10 s: full {:.0}/{:.0}, ally {:.0}, enemy {:.0}", energy(0), cap.to_f32(), energy(1), energy(2));
    assert!((energy(0) - cap.to_f32()).abs() < 1.0, "still full");
    assert!(energy(1) > energy(2) + 150.0, "the ally gets what would have been wasted");
}

/// Acid seas (TA's `waterdoesdamage`): a kbot wading in the acid loses the map's damage
/// each second; a hovercraft over it and a kbot on the shore do not.
#[test]
fn acid_seas_eat_what_is_in_them() {
    let Some(mut sim) = sim_on("Acid Pools") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    assert!(sim.water_damage > Fx::ZERO, "the map's water does damage");
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    let at = |deep: bool| {
        (64..h.to_int() - 64).step_by(16).flat_map(|z| (64..w.to_int() - 64).step_by(16).map(move |x| FxVec3::from_ints(x, 0, z))).find(|p| {
            let ground = sim.world.height_at(p.x, p.z);
            if deep { ground < sea - Fx::from_int(12) } else { ground > sea + Fx::from_int(20) }
        })
    };
    let (wet, dry) = (at(true).expect("acid on the map"), at(false).expect("dry land on the map"));
    let wading = sim.spawn_unit(sim.types.id("ARMPW").unwrap(), 0, wet, 0, true).unwrap();
    let hover = sim.spawn_unit(sim.types.id("ARMSH").unwrap(), 0, wet + FxVec3::from_ints(0, 0, 0), 0, true).unwrap();
    let ashore = sim.spawn_unit(sim.types.id("ARMPW").unwrap(), 0, dry, 0, true).unwrap();
    let health = |sim: &Sim, id| sim.unit(id).map_or(Fx::ZERO, |u| u.health);
    let before = [health(&sim, wading), health(&sim, hover), health(&sim, ashore)];
    for _ in 0..90 {
        sim.step(&[]);
    }
    let lost = |k: usize, id| (before[k] - health(&sim, id)).to_f32();
    println!("lost in 3 s: wading {:.0}, hovering {:.0}, ashore {:.0} (acid {:.0} a second)", lost(0, wading), lost(1, hover), lost(2, ashore), sim.water_damage.to_f32());
    assert!((lost(0, wading) - 3.0 * sim.water_damage.to_f32()).abs() < 0.5, "the acid eats the wading kbot");
    assert_eq!(lost(1, hover), 0.0, "hovercraft ride over it");
    assert_eq!(lost(2, ashore), 0.0, "dry land is safe");
}

/// Flames harm neither the unit that throws them nor its side, as in TA (docs/FORMATS.md): a
/// Pyro burning an enemy right beside it, a friend at its side, burns the enemy alone. Burnt by
/// its own flames, a Pyro lost nearly a fifth of its health there in ten seconds (a Sea
/// Serpent, without the Pyro's resistance, takes five times as much a flame).
#[test]
fn flames_spare_their_own_side() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let target = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a + FxVec3::from_ints(100, 0, 0), -16384, true).unwrap();
    let pyro = sim.spawn_unit(sim.types.id("CORPYRO").unwrap(), 1, a + FxVec3::from_ints(70, 0, 0), 16384, true).unwrap();
    let friend = sim.spawn_unit(sim.types.id("CORAK").unwrap(), 1, a + FxVec3::from_ints(70, 0, 28), 16384, true).unwrap();
    sim.step(&[
        ta_sim::Command::SetStates { player: 0, units: vec![target], fire: Some(0), movement: Some(0) },
        ta_sim::Command::SetStates { player: 1, units: vec![pyro, friend], fire: Some(0), movement: Some(0) },
        ta_sim::Command::Order { player: 1, units: vec![pyro], order: ta_sim::Order::Attack(ta_sim::Target::Unit(target)), queue: false },
    ]);
    let max = |sim: &Sim, id| sim.unit(id).map(|u| sim.unit_type(u).max_health).unwrap();
    let (pyro_max, friend_max, target_max) = (max(&sim, pyro), max(&sim, friend), max(&sim, target));
    let health = |sim: &Sim, id| sim.unit(id).map_or(Fx::ZERO, |u| u.health);
    let mut flames = 0;
    for _ in 0..30 * 10 {
        sim.step(&[]);
        flames += sim.take_events().iter().filter(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == pyro)).count();
    }
    println!("{flames} flames: pyro {}/{pyro_max}, friend {}/{friend_max}, enemy {}/{target_max}", health(&sim, pyro), health(&sim, friend), health(&sim, target));
    assert!(health(&sim, target) < target_max * 3 / 4, "the flames burn the enemy");
    assert_eq!(health(&sim, pyro), pyro_max, "the Pyro's own flames don't burn it");
    assert_eq!(health(&sim, friend), friend_max, "nor its side");
}

/// A blast no archive has (the Leveler's `ExplodeAs` MEDIUM_UNITEX, the moho metal makers'
/// BIG_BUILDINGEX) is no blast, as no name (walls) is: what TA keeps for none is its weapon 0,
/// NOWEAPON, of no damage (docs/FORMATS.md). A Raider's BIG_UNITEX hurts those beside it.
#[test]
fn blasts_no_archive_has_do_no_harm() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.cheats_allowed = true;
    // (Neither side is beaten as its units come and go.)
    for p in &mut sim.players {
        p.endless = true;
    }
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    let id = |sim: &Sim, n: &str| sim.types.id(n).unwrap();
    let maker = sim.find_build_site(id(&sim, "CORMMKR"), a + FxVec3::from_ints(160, 0, 0), 12).expect("room for a moho metal maker");
    let mut hurt = Vec::new();
    for (name, at) in [("CORLEVLR", a), ("CORMMKR", maker), ("CORRAID", a + FxVec3::from_ints(80, 0, 0))] {
        let dying = sim.spawn_unit(id(&sim, name), 1, at, 0, true).unwrap();
        let r = sim.unit_type(sim.unit(dying).unwrap()).radius;
        // A friend and an enemy close beside it.
        let beside: Vec<_> = [(1u8, -1i64), (0, 1)].iter().map(|&(p, side)| sim.spawn_unit(id(&sim, "CORAK"), p, at + FxVec3::new(r * side + Fx::from_int(12 * side), Fx::ZERO, Fx::ZERO), 0, true).unwrap()).collect();
        sim.step(&[ta_sim::Command::SetStates { player: 0, units: vec![beside[1]], fire: Some(0), movement: Some(0) }, ta_sim::Command::SetStates { player: 1, units: vec![beside[0], dying], fire: Some(0), movement: Some(0) }]);
        let before: Vec<Fx> = beside.iter().map(|&b| sim.unit(b).unwrap().health).collect();
        sim.step(&[ta_sim::Command::Cheat { player: 1, cheat: ta_sim::Cheat::Destroy(vec![dying]) }]);
        sim.step(&[]);
        assert!(sim.unit(dying).is_none_or(|u| u.dead), "{name} is destroyed");
        let lost: Vec<f32> = beside.iter().zip(&before).map(|(&b, &h)| (h - sim.unit(b).map_or(Fx::ZERO, |u| u.health)).to_f32()).collect();
        println!("{name} ({}) destroyed: those beside it lost {lost:?}", sim.types.get(id(&sim, name)).def.explode_as);
        hurt.push(lost.iter().any(|&l| l > 0.0));
        for b in beside {
            sim.remove_unit(b);
        }
        sim.step(&[]);
    }
    assert_eq!(hurt, [false, false, true], "only the Raider's blast, which an archive has, hurts");
}

/// Resigning in a team game: the army goes to an ally (the Commander still blows up).
#[test]
fn resigning_beside_an_ally_leaves_the_army_to_them() {
    use ta_sim::Command;
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 220).expect("no flat land on the map");
    sim.set_team(1, 0);
    let com0 = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    let com1 = sim.spawn_unit(sim.types.id("CORCOM").unwrap(), 1, a + FxVec3::from_ints(1200, 0, 0), 0, true).unwrap();
    let tank = sim.spawn_unit(sim.types.id("CORRAID").unwrap(), 1, a + FxVec3::from_ints(400, 0, 0), 0, true).unwrap();
    sim.step(&[]);
    sim.step(&[Command::Resign { player: 1 }]);
    assert!(sim.players[1].defeated);
    assert_eq!(sim.unit(tank).map(|u| u.owner), Some(0), "the ally takes the army");
    for _ in 0..30 * 8 {
        sim.step(&[]);
    }
    assert!(sim.unit(com1).is_none(), "the Commander blows up");
    assert!(sim.unit(tank).is_some_and(|u| !u.dead && u.owner == 0), "the army fights on for the ally");
    assert!(sim.unit(com0).is_some());
}
