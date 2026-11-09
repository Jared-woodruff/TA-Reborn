//! What a click with the order button does to a unit under the pointer, from what is
//! selected. The cursor shows it and the click carries it out, both from this one account, so
//! the two cannot disagree: constructors over a factory show the help they will give, and the
//! click gives it.

use ta_sim::{Locomotion, Sim, UnitId};

use crate::cursor::CursorKind;

/// What a click does to a unit under the pointer, with the selected units that take part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum UnitClick {
    /// Aircraft sent to a repair pad fly there and land on it.
    Land(Vec<UnitId>),
    /// The first transport with room picks it up.
    Load(UnitId),
    /// The armed units attack it.
    Attack(Vec<UnitId>),
    /// A unit of ours or an ally's is helped. `repair`: constructors mend it, or help finish
    /// building it. `guard`: units that follow it and stand by it (constructors help whatever
    /// it builds; a factory sends its units to guard it). `assists`: one of them is a
    /// constructor guarding a factory: help with its production, now and later.
    Help { repair: Vec<UnitId>, guard: Vec<UnitId>, assists: bool },
}

impl UnitClick {
    /// The cursor that says what the click does. Help with building or mending shows the one
    /// TA gives a constructor over a nanoframe, the wrench, and so does help for a factory's
    /// production; standing guard shows the shield.
    pub(super) fn cursor(&self) -> CursorKind {
        match self {
            UnitClick::Attack(_) => CursorKind::Attack,
            UnitClick::Load(_) => CursorKind::Load,
            UnitClick::Land(_) => CursorKind::Repair,
            UnitClick::Help { repair, assists, .. } if !repair.is_empty() || *assists => CursorKind::Repair,
            UnitClick::Help { guard, .. } if !guard.is_empty() => CursorKind::Defend,
            // (Nothing selected can take part: the click does nothing to it.)
            UnitClick::Help { .. } => CursorKind::Select,
        }
    }
}

/// Mobile constructors: they repair, help build and reclaim (a factory builds in place).
fn is_constructor(t: &ta_sim::UnitType) -> bool {
    t.is_builder && !t.is_factory
}

/// Guard and repair commands accept our own units and our allies' units alike.
pub(super) fn allied_target(sim: &Sim, me: u8, hovered: Option<UnitId>) -> Option<UnitId> {
    hovered.filter(|&id| sim.unit(id).is_some_and(|u| !u.dead && sim.allied(me, u.owner)))
}

/// What a click on `target` does to the selection, or `None` when it does nothing to the unit
/// itself (an enemy the selection cannot fire on, the selection's own unit): the click then goes
/// on to the ground beneath, to reclaim what lies there or to move.
pub(super) fn click_on(sim: &Sim, me: u8, selection: &[UnitId], target: UnitId) -> Option<UnitClick> {
    let tu = sim.unit(target).filter(|_| !selection.contains(&target))?;
    let tt = sim.unit_type(tu);
    let helps = sim.allied(tu.owner, me);
    let mine = |id: UnitId| sim.unit(id).map(|u| (u, sim.unit_type(u)));
    // Aircraft sent to a repair pad fly there and land on it.
    if helps && tt.def.is_airbase {
        let planes: Vec<UnitId> = selection.iter().copied().filter(|&id| mine(id).is_some_and(|(_, t)| t.locomotion == Locomotion::Air)).collect();
        if !planes.is_empty() {
            return Some(UnitClick::Land(planes));
        }
    }
    // Our units are picked up (by a transport with room for one).
    if tu.owner == me {
        let carrier = selection.iter().copied().find(|&id| mine(id).is_some_and(|(u, t)| t.can_carry(tt) && (u.cargo.len() as u32) < t.transport_capacity));
        if let Some(carrier) = carrier {
            return Some(UnitClick::Load(carrier));
        }
    }
    if !helps {
        let armed: Vec<UnitId> = selection.iter().copied().filter(|&id| mine(id).is_some_and(|(_, t)| super::fires_on_attack(sim, t))).collect();
        return (!armed.is_empty()).then_some(UnitClick::Attack(armed));
    }
    // Constructors repair a unit that needs work, and guard one that doesn't (a constructor's
    // guard is help for what it builds, a factory's units included); other mobile units guard
    // it, and factories send their units to it.
    let needs_work = !tu.is_complete() || tu.health < tt.max_health;
    let (mut repair, mut guard, mut assists) = (Vec::new(), Vec::new(), false);
    for &id in selection {
        let Some((_, t)) = mine(id) else { continue };
        if is_constructor(t) && needs_work {
            repair.push(id);
        } else if t.locomotion != Locomotion::Static || t.is_builder {
            guard.push(id);
            assists |= is_constructor(t) && tt.is_factory;
        }
    }
    Some(UnitClick::Help { repair, guard, assists })
}

/// With left-button orders, whether a click on this unit selects it rather than gives the
/// selection an order there: our own units are selected, except a nanoframe, and a factory at
/// work, that the selected constructors can help (the click helps them; with Shift it adds
/// the factory to the selection, as Shift always does). Spectators pick anything to look at. A
/// unit already selected is never helped by itself: it is picked.
pub(super) fn selects(sim: &Sim, me: u8, spectator: bool, selection: &[UnitId], id: UnitId, shift: bool) -> bool {
    let Some(u) = sim.unit(id) else { return false };
    if spectator {
        return true;
    }
    if u.owner != me {
        return false;
    }
    let helpers = || selection.iter().any(|&s| s != id && sim.unit(s).is_some_and(|b| is_constructor(sim.unit_type(b))));
    if !u.is_complete() {
        return !helpers();
    }
    !(sim.unit_type(u).is_factory && at_work(u) && !shift && !selection.contains(&id) && helpers())
}

/// A factory that is making something or has something queued: there is production to help.
pub(super) fn at_work(u: &ta_sim::Unit) -> bool {
    u.build.is_some() || !u.factory_queue.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ta_sim::{Command, Fx, FxVec3};

    #[test]
    fn guard_and_repair_accept_allies_but_not_enemies_or_dead_units() {
        let mut yard = Yard::new();
        let own = yard.put("CON", 0, 100, true);
        let ally = yard.put("LAB", 1, 200, false);
        let enemy = yard.put("LAB", 2, 300, true);
        assert_eq!(allied_target(&yard.sim, 0, Some(own)), Some(own));
        assert_eq!(allied_target(&yard.sim, 0, Some(ally)), Some(ally));
        assert_eq!(allied_target(&yard.sim, 0, Some(enemy)), None);
        yard.sim.remove_unit(ally);
        assert_eq!(allied_target(&yard.sim, 0, Some(ally)), None);
    }

    #[test]
    fn both_fixed_wing_and_unarmed_aircraft_can_use_an_allied_repair_pad() {
        let mut yard = Yard::new();
        let fighter = yard.put("PLANE", 0, 100, true);
        let transport = yard.put("AIRTRANSPORT", 0, 200, true);
        let pad = yard.put("PAD", 1, 300, true);
        assert_eq!(yard.click(&[fighter, transport], pad), Some(UnitClick::Land(vec![fighter, transport])));
    }

    /// Game data from TDF files (path, text), loaded as a mod's folder is.
    fn data_of(files: &[(String, String)]) -> ta_data::GameData {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("ta-clicks-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        std::fs::remove_dir_all(&dir).ok();
        data
    }

    fn unit(name: &str, keys: &str) -> (String, String) {
        (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nFootprintX=2;\nFootprintZ=2;\nBuildTime=100;\n{keys}\n}}\n"))
    }

    /// A lab, a constructor, a tank with a gun, a radar tower, a plane, a repair pad and a
    /// transport, for two players, allied or not.
    struct Yard {
        sim: Sim,
    }

    impl Yard {
        fn new() -> Yard {
            let data = data_of(&[
                unit("LAB", "Builder=1; WorkerTime=100; FootprintX=6; FootprintZ=6;"),
                unit("CON", "Builder=1; WorkerTime=100; CanMove=1; MaxVelocity=2;"),
                unit("TANK", "CanMove=1; MaxVelocity=2; Weapon1=GUN;"),
                unit("TOWER", "Weapon1=GUN;"),
                unit("PLANE", "CanFly=1; CanMove=1; MaxVelocity=6; Weapon1=GUN;"),
                unit("PAD", "IsAirBase=1;"),
                unit("TRANSPORT", "CanMove=1; MaxVelocity=2; TransportCapacity=2; TransportSize=3;"),
                unit("AIRTRANSPORT", "CanFly=1; CanMove=1; MaxVelocity=2; TransportCapacity=2; TransportSize=3;"),
                unit("SILO", "CanDgun=1; Weapon1=NUKE;"),
                unit("ANTI", "CanDgun=1; Weapon1=INTERCEPTOR;"),
                unit("BOMBER", "CanFly=1; CanMove=1; MaxVelocity=6; Weapon1=BOMB;"),
                unit("COMMANDER", "Builder=1; CanMove=1; MaxVelocity=2; CanDgun=1; CanReclamate=1; CanCapture=1; Weapon1=GUN; Weapon3=DGUN;"),
                unit("NANO", "Builder=1; WorkerTime=100; CanReclamate=1; CanResurrect=1;"),
                ("weapons/gun.tdf".into(), "[GUN]\n{\nname=Gun;\nrange=300;\nreloadtime=1;\n[DAMAGE]\n{\ndefault=10;\n}\n}\n".into()),
                ("weapons/special.tdf".into(), "[NUKE] { range=1000; stockpile=1; commandfire=1; } [INTERCEPTOR] { stockpile=1; interceptor=1; } [BOMB] { dropped=1; commandfire=1; } [DGUN] { rendertype=3; commandfire=1; noexplode=1; }".into()),
                // The lab builds tanks.
                ("download/menu.tdf".into(), "[MENUENTRY1]\n{\nUNITMENU=LAB;\nUNITNAME=TANK;\nMENU=1;\nBUTTON=0;\n}\n".into()),
            ]);
            let data = Arc::new(data);
            let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
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
            let players: Vec<ta_sim::PlayerSetup> =
                (0..3).map(|i| ta_sim::PlayerSetup { side: "ARM".into(), color: i, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
            let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
            // Players 0 and 1 are allies; 2 is the enemy.
            sim.set_team(1, 0);
            assert!(sim.allied(0, 1) && !sim.allied(0, 2));
            Yard { sim }
        }

        fn put(&mut self, kind: &str, owner: u8, x: i64, complete: bool) -> UnitId {
            let t = self.sim.types.id(kind).unwrap();
            self.sim.spawn_unit(t, owner, FxVec3::from_ints(x, 60, 300), 0, complete).unwrap()
        }

        fn click(&self, selection: &[UnitId], target: UnitId) -> Option<UnitClick> {
            click_on(&self.sim, 0, selection, target)
        }
    }

    #[test]
    fn contextual_commands_match_weapons_and_construction_capabilities() {
        use crate::game::{CommandMode::*, type_supports_mode};
        let mut y = Yard::new();
        let enemy = y.put("TANK", 2, 800, true);
        for (name, attack, dgun) in [("SILO", true, false), ("ANTI", false, false), ("BOMBER", true, false), ("COMMANDER", true, true), ("CON", false, false)] {
            let id = y.put(name, 0, 100, true);
            let ty = y.sim.unit_type(y.sim.unit(id).unwrap());
            assert_eq!(type_supports_mode(&y.sim, ty, Attack), attack, "{name} Attack");
            assert_eq!(type_supports_mode(&y.sim, ty, DGun), dgun, "{name} D-Gun");
            assert_eq!(matches!(y.click(&[id], enemy), Some(UnitClick::Attack(_))), attack, "{name} contextual attack");
        }
        let nano = y.sim.types.by_name("NANO").unwrap();
        for mode in [Guard, Repair, Reclaim, Resurrect] {
            assert!(type_supports_mode(&y.sim, nano, mode), "nano turret {mode:?}");
        }
        for mode in [Move, Patrol, Fight, Load, DGun, Attack, Capture] {
            assert!(!type_supports_mode(&y.sim, nano, mode), "nano turret {mode:?}");
        }
        let pad = y.sim.types.by_name("PAD").unwrap();
        for mode in [Move, Guard, Repair, Reclaim, Resurrect, Attack, DGun, Load] {
            assert!(!type_supports_mode(&y.sim, pad, mode), "repair pad {mode:?}");
        }
    }

    #[test]
    fn constructors_over_a_factory_assist_it_and_the_click_is_a_guard() {
        let mut y = Yard::new();
        let (con, lab) = (y.put("CON", 0, 100, true), y.put("LAB", 0, 200, true));
        let click = y.click(&[con], lab).unwrap();
        assert_eq!(click, UnitClick::Help { repair: vec![], guard: vec![con], assists: true });
        assert_eq!(click.cursor(), CursorKind::Repair, "the wrench, not the shield");
        // An ally's factory too: help is help.
        let ally_lab = y.put("LAB", 1, 300, true);
        assert_eq!(y.click(&[con], ally_lab).unwrap().cursor(), CursorKind::Repair);
        // Tanks only stand guard over it: no wrench.
        let tank = y.put("TANK", 0, 120, true);
        let click = y.click(&[tank], lab).unwrap();
        assert_eq!(click, UnitClick::Help { repair: vec![], guard: vec![tank], assists: false });
        assert_eq!(click.cursor(), CursorKind::Defend);
        // A constructor with the tank: the wrench, and both guard.
        assert_eq!(y.click(&[tank, con], lab).unwrap(), UnitClick::Help { repair: vec![], guard: vec![tank, con], assists: true });
        // A constructor over another constructor guards it (it follows), no wrench.
        let other = y.put("CON", 0, 150, true);
        assert_eq!(y.click(&[con], other).unwrap().cursor(), CursorKind::Defend);
    }

    #[test]
    fn a_damaged_or_unfinished_unit_is_repaired_not_guarded() {
        let mut y = Yard::new();
        let (con, tank) = (y.put("CON", 0, 100, true), y.put("TANK", 0, 140, true));
        let frame = y.put("TANK", 0, 200, false);
        assert_eq!(y.click(&[con], frame).unwrap(), UnitClick::Help { repair: vec![con], guard: vec![], assists: false });
        // (A factory unit that is not a constructor guards even a nanoframe.)
        assert_eq!(y.click(&[tank], frame).unwrap(), UnitClick::Help { repair: vec![], guard: vec![tank], assists: false });
        // A factory still being built is repaired (helped on), and so is a damaged one.
        let lab = y.put("LAB", 0, 300, false);
        assert!(matches!(y.click(&[con], lab).unwrap(), UnitClick::Help { repair, .. } if repair == [con]));
        // Nothing that can help it: the click does nothing, and the cursor does not say it does.
        let tower = y.put("TOWER", 0, 400, true);
        let damaged_friend = y.put("TANK", 1, 420, true);
        assert_eq!(y.click(&[tower], damaged_friend).map(|c| c.cursor()), Some(CursorKind::Select));
    }

    #[test]
    fn enemies_are_attacked_by_the_armed_and_left_to_the_ground_by_the_rest() {
        let mut y = Yard::new();
        let (con, tank) = (y.put("CON", 0, 100, true), y.put("TANK", 0, 140, true));
        let foe = y.put("TANK", 2, 300, true);
        assert_eq!(y.click(&[con, tank], foe), Some(UnitClick::Attack(vec![tank])));
        assert_eq!(y.click(&[tank], foe).map(|c| c.cursor()), Some(CursorKind::Attack));
        assert_eq!(y.click(&[con], foe), None, "an unarmed constructor goes to the ground beside it");
        assert_eq!(y.click(&[tank], tank), None, "a selected unit is not its own target");
        let tower = y.put("TOWER", 0, 500, true);
        assert_eq!(y.click(&[tower], foe), Some(UnitClick::Attack(vec![tower])));
    }

    #[test]
    fn aircraft_land_on_pads_and_transports_load_only_with_room() {
        let mut y = Yard::new();
        let (plane, tank, pad) = (y.put("PLANE", 0, 100, true), y.put("TANK", 0, 140, true), y.put("PAD", 0, 200, true));
        assert_eq!(y.click(&[plane, tank], pad), Some(UnitClick::Land(vec![plane])));
        assert_eq!(y.click(&[plane], pad).map(|c| c.cursor()), Some(CursorKind::Repair), "landing to be mended");
        assert_eq!(y.click(&[tank], pad).map(|c| c.cursor()), Some(CursorKind::Defend), "a tank guards the pad");
        let lift = y.put("TRANSPORT", 0, 300, true);
        assert_eq!(y.click(&[lift], tank), Some(UnitClick::Load(lift)));
        assert_eq!(y.click(&[lift], tank).map(|c| c.cursor()), Some(CursorKind::Load));
        // (Not the enemy's, and an ally's is helped, not carried.)
        let foe = y.put("TANK", 2, 340, true);
        assert_eq!(y.click(&[lift], foe), None);
        let ally = y.put("TANK", 1, 360, true);
        assert!(matches!(y.click(&[lift], ally), Some(UnitClick::Help { .. })));
    }

    #[test]
    fn with_left_orders_a_click_selects_unless_it_can_help() {
        let mut y = Yard::new();
        let (con, tank, lab) = (y.put("CON", 0, 100, true), y.put("TANK", 0, 140, true), y.put("LAB", 0, 200, true));
        let selects = |y: &Yard, selection: &[UnitId], id: UnitId| selects(&y.sim, 0, false, selection, id, false);
        // Our own units are picked, an idle factory too: to give it its orders.
        assert!(selects(&y, &[con], tank));
        assert!(selects(&y, &[con], lab));
        // At work, a factory is helped by the constructors, and picked by anything else.
        let t = y.sim.types.id("TANK").unwrap();
        y.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: t, count: 1 }]);
        assert!(at_work(y.sim.unit(lab).unwrap()));
        assert!(!selects(&y, &[con], lab), "the constructor helps");
        assert!(selects(&y, &[tank], lab), "a tank cannot help");
        assert!(selects(&y, &[], lab), "nothing selected: picked");
        assert!(selects(&y, &[con, lab], lab), "the selected factory itself is picked");
        assert!(super::selects(&y.sim, 0, false, &[con], lab, true), "Shift adds the factory to the selection, as ever");
        // A unit of ours not finished is helped by constructors, picked by the rest.
        let frame = y.put("TANK", 0, 260, false);
        assert!(!selects(&y, &[con], frame));
        assert!(selects(&y, &[tank], frame));
        // Not ours: never picked (and spectators pick everything).
        let foe = y.put("TANK", 2, 300, true);
        assert!(!selects(&y, &[con], foe));
        assert!(super::selects(&y.sim, 0, true, &[con], foe, false));
    }
}
