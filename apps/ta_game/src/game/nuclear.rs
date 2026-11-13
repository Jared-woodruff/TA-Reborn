//! Nuclear destination intelligence is separate from detecting an airborne radar contact.

use glam::{Vec2, Vec3};
use ta_sim::Sim;

pub(crate) fn destination_known(sim: &Sim, viewer: Option<u8>, owner: u8) -> bool {
    viewer.is_none_or(|player| sim.allied(player, owner))
}

/// Missiles a defence can shoot down (TA's `targetable`: the nukes, the EMP and tactical
/// missiles) and the interceptors that do: their own side follows them wherever they fly, and
/// sees and hears them go off or be shot down, in sight or not (what they hit there stays out of
/// sight).
pub(crate) fn strategic(w: &ta_sim::WeaponType) -> bool {
    w.def.targetable || w.def.interceptor
}

/// Whether `viewer` is shown where a missile is going (and when it gets there): its own side's,
/// unless it follows a unit the viewer cannot see now (showing where it goes would show where the
/// unit is: a mod's stockpiled missile that says `tracks`; TA's own are sent at the place the unit
/// stood at their launch, `ta_sim::WeaponType::strikes_a_place`, always shown). A unit gone, its
/// missile goes on to where it last was.
pub(crate) fn destination_shown(sim: &Sim, viewer: Option<u8>, p: &ta_sim::Projectile) -> bool {
    destination_known(sim, viewer, p.owner_player)
        && match (viewer, p.target) {
            (Some(player), Some(ta_sim::Target::Unit(id))) => sim.unit(id).is_none_or(|u| u.dead || sim.seen_by(u, player) != ta_sim::Seen::Hidden),
            _ => true,
        }
}

/// Seconds until a strategic missile comes down where it is aimed, as it flies now: the rest of
/// its climb out of its tube, then (one of TA's `cruise` missiles) level until its target is 60°
/// below it and down onto it, else straight at it; at its top speed once it has gathered it.
/// (Within a second or so of the simulation's own flight: a countdown, not a promise, for a
/// target that moves.)
pub(crate) fn time_to_impact(sim: &Sim, p: &ta_sim::Projectile) -> f32 {
    let w = sim.weapon_types.get(p.weapon);
    let (top, gather) = (w.velocity.to_f32().max(0.01), w.acceleration.to_f32());
    let mut at = Vec3::from_array(p.pos.to_f32());
    let mut speed = Vec3::from_array(p.vel.to_f32()).length();
    let mut ticks = 0.0;
    if w.vlaunch {
        for _ in p.age..w.launch_ticks {
            speed = (speed + gather).min(top);
            at.y += speed;
            ticks += 1.0;
        }
    }
    let to = Vec3::from_array(p.target_pos.to_f32()) - at;
    let (flat, drop) = (Vec2::new(to.x, to.z).length(), -to.y);
    // (Level to where the target is 60° below, then down: tan 60° is 1.732, sin 60° 0.866.)
    let path = if w.vlaunch && w.def.cruise && drop > 0.0 && drop < flat * 1.732 { flat - drop / 1.732 + drop / 0.866 } else { flat.hypot(drop) };
    // (Gathering speed to its top, then at it.)
    let (gathering, gathered) = if gather > 0.0 && speed < top { ((top - speed) / gather, (top + speed) * 0.5 * (top - speed) / gather) } else { (0.0, 0.0) };
    ticks += if path <= gathered { path / ((top + speed) * 0.5).max(0.01) } else { gathering + (path - gathered) / top };
    ticks / 30.0
}

/// Use launch-time ownership even if the silo was captured or destroyed in this tick.
pub(crate) fn launch_owner(sim: &Sim, unit: ta_sim::UnitId, weapon: usize) -> Option<u8> {
    sim.projectiles.iter().chain(sim.spent.iter().map(|(p, _)| p))
        .find(|p| p.owner == unit && p.weapon == weapon && p.age == 1).map(|p| p.owner_player)
}

pub(crate) fn interception_known(sim: &Sim, viewer: Option<u8>, defender: u8, attacker: u8, pos: ta_sim::FxVec3) -> bool {
    viewer.is_none_or(|player| sim.allied(defender, player) || sim.missile_seen_at(attacker, pos, player) != ta_sim::Seen::Hidden)
}

pub(crate) struct ImpactMarker {
    pub at: Vec3,
    pub radius: f32,
    pub age: f32,
    /// Until it comes down.
    pub seconds: f32,
}

/// Derive warnings from live missiles, so interception, expiration, replay seeking and
/// changes of viewer cannot leave stale targets or disclose an enemy's intended destination.
pub(crate) fn impact_markers(sim: &Sim, viewer: Option<u8>) -> impl Iterator<Item = ImpactMarker> + '_ {
    sim.projectiles.iter().filter_map(move |p| {
        let w = sim.weapon_types.get(p.weapon);
        (!p.dead && w.def.stockpile && !w.def.interceptor && destination_shown(sim, viewer, p))
            .then(|| ImpactMarker { at: Vec3::from_array(p.target_pos.to_f32()), radius: w.aoe_radius.to_f32(), age: p.age as f32 / 30.0, seconds: time_to_impact(sim, p) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ta_sim::{Fx, FxVec3, PlayerSetup, Projectile, UnitId, UnitTypes};

    fn missiles() -> Sim {
        let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
        data.weapons.push(ta_data::WeaponDef { stockpile: true, targetable: true, area_of_effect: 512.0, ..Default::default() });
        let types = Arc::new(UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 32, height: 32, sea_level: 0, heights: vec![60; 32 * 32], features: vec![ta_formats::tnt::FEATURE_NONE; 32 * 32], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let players: Vec<_> = (0..4).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false }).collect();
        let mut sim = Sim::new(types, Arc::new(data), &map, &ta_data::MapInfo::default(), 0, &players, 7);
        sim.set_team(1, 0);
        for owner in 0..3 {
            let at = FxVec3::from_ints(100 + owner as i64 * 100, 60, 300);
            sim.projectiles.push(Projectile { id: owner as u32, weapon: 0, owner: UnitId { slot: owner as u32, generation: 1 }, owner_player: owner, pos: at, prev_pos: at, vel: FxVec3::ZERO, target: None, target_pos: at, age: 2700, ttl: 12000, hits: Vec::new(), dead: false });
        }
        sim
    }

    #[test]
    fn destination_markers_follow_the_current_viewers_alliances() {
        let sim = missiles();
        let xs = |viewer| impact_markers(&sim, viewer).map(|m| m.at.x).collect::<Vec<_>>();
        assert_eq!(xs(Some(0)), [100.0, 200.0]);
        assert_eq!(xs(Some(1)), [100.0, 200.0]);
        assert_eq!(xs(Some(2)), [300.0]);
        assert_eq!(xs(None), [100.0, 200.0, 300.0]);
        assert!(!destination_known(&sim, Some(0), 2), "enemy launch notices must not link to their hidden target");
    }

    #[test]
    fn long_flights_keep_the_authored_warning_until_each_missile_retires() {
        let mut sim = missiles();
        let markers: Vec<_> = impact_markers(&sim, Some(0)).collect();
        assert_eq!(markers.len(), 2, "a flight beyond 45 seconds still has a warning");
        assert!(markers.iter().all(|m| m.radius == 256.0 && m.age == 90.0));
        sim.projectiles[0].dead = true;
        assert_eq!(impact_markers(&sim, Some(0)).count(), 1, "one intercepted missile cannot clear another's warning");
        sim.projectiles.remove(1);
        assert_eq!(impact_markers(&sim, Some(0)).count(), 0);
    }

    #[test]
    fn interception_notices_respect_participants_sight_radar_and_jamming() {
        use ta_sim::vision;
        let mut sim = missiles();
        sim.los_enabled = true;
        sim.vision.rebuild(&[]);
        let pos = FxVec3::from_ints(400, 300, 400);
        assert!(interception_known(&sim, None, 2, 3, pos));
        assert!(interception_known(&sim, Some(0), 1, 2, pos), "our ally's defence reports its interception");
        assert!(interception_known(&sim, Some(0), 2, 1, pos), "our ally's missile is lost");
        assert!(!interception_known(&sim, Some(0), 2, 3, pos), "unseen enemy fighting is not our intelligence");
        sim.vision.set_all(0, vision::RADAR);
        assert!(interception_known(&sim, Some(0), 2, 3, pos));
        sim.vision.set_all(3, vision::RADAR_JAM);
        assert!(!interception_known(&sim, Some(0), 2, 3, pos));
        sim.vision.set_all(0, vision::LOS);
        assert!(interception_known(&sim, Some(0), 2, 3, pos));
    }

    /// A battlefield 4,800 px square, flat at 60, with a nuke like TA's (and a unit for players
    /// to aim at).
    fn range() -> Sim {
        let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
        let nuke = "[NUKE]{ vlaunch=1; cruise=1; guidance=1; stockpile=1; targetable=1; commandfire=1; range=32000; weapontimer=5; flighttime=400; weaponvelocity=350; weaponacceleration=50; turnrate=32768; areaofeffect=512; [DAMAGE]{ default=5500; } }";
        data.weapons.push(ta_data::WeaponDef::from_tdf(&ta_formats::tdf::parse(nuke).sections[0]));
        let unit = "[UNITINFO]{ UnitName=POST; FootprintX=2; FootprintZ=2; MaxDamage=3000; }";
        data.units.push(ta_data::UnitDef::from_tdf(&ta_formats::tdf::parse(unit).sections[0]).unwrap());
        let types = Arc::new(UnitTypes::compile(&data, &|_| None));
        let side = 300;
        let map = ta_formats::tnt::TntMap { width: side, height: side, sea_level: 0, heights: vec![60; (side * side) as usize], features: vec![ta_formats::tnt::FEATURE_NONE; (side * side) as usize], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let players: Vec<_> = (0..2).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false }).collect();
        Sim::new(types, Arc::new(data), &map, &ta_data::MapInfo::default(), 0, &players, 7)
    }

    fn launch(sim: &mut Sim, from: FxVec3, target: ta_sim::Target, target_pos: FxVec3) {
        let ttl = sim.weapon_types.get(0).flight_ticks;
        sim.projectiles.push(Projectile { id: 1, weapon: 0, owner: UnitId { slot: 900, generation: 1 }, owner_player: 0, pos: from, prev_pos: from, vel: FxVec3::ZERO, target: Some(target), target_pos, age: 0, ttl, hits: Vec::new(), dead: false });
    }

    /// The countdown at the launch is within a second of the flight the simulation flies, near
    /// and far; and it counts down as the missile flies.
    #[test]
    fn the_time_to_impact_is_the_flights_own() {
        for far in [600, 1800, 4000] {
            let mut sim = range();
            let target = FxVec3::from_ints(400 + far, 60, 2400);
            launch(&mut sim, FxVec3::from_ints(400, 70, 2400), ta_sim::Target::Ground(target), target);
            let told = time_to_impact(&sim, &sim.projectiles[0]);
            let mut ticks = 0;
            let mut halfway = None;
            while !sim.projectiles.is_empty() && ticks < 30 * 120 {
                sim.step(&[]);
                ticks += 1;
                if ticks == (told * 15.0) as u32 && let Some(p) = sim.projectiles.first() {
                    halfway = Some(time_to_impact(&sim, p));
                }
            }
            let flown = ticks as f32 / 30.0;
            assert!((told - flown).abs() <= 1.0, "{far} px: told {told:.1} s, flown {flown:.1} s");
            let halfway = halfway.expect("still in flight halfway");
            assert!((halfway - (flown - told / 2.0)).abs() <= 1.0, "{far} px, halfway: told {halfway:.1} s of {flown:.1} s");
        }
    }

    /// A missile's destination shows to its own side (and a spectator) only; sent at a unit,
    /// only while that unit is in its sight or on its radar (the missile homes on it).
    #[test]
    fn destinations_show_to_the_missiles_side_and_not_for_a_unit_out_of_sight() {
        use ta_sim::vision;
        let mut sim = range();
        sim.los_enabled = true;
        sim.vision.rebuild(&[]);
        let post = sim.spawn_unit(0, 1, FxVec3::from_ints(3000, 60, 2400), 0, true).unwrap();
        let at = sim.unit(post).unwrap().pos;
        launch(&mut sim, FxVec3::from_ints(400, 70, 2400), ta_sim::Target::Unit(post), at);
        let p = sim.projectiles[0].clone();
        assert!(!destination_shown(&sim, Some(1), &p), "never to the enemy");
        assert!(destination_shown(&sim, None, &p), "a spectator sees all");
        assert!(!destination_shown(&sim, Some(0), &p), "the unit out of sight: not where it is");
        sim.vision.set_all(0, vision::RADAR);
        assert!(destination_shown(&sim, Some(0), &p), "on radar");
        sim.vision.set_all(0, vision::LOS);
        assert!(destination_shown(&sim, Some(0), &p), "in sight");
        sim.vision.set_all(0, 0);
        sim.remove_unit(post);
        assert!(destination_shown(&sim, Some(0), &p), "gone, the missile goes on to where it last was");
        let ground = Projectile { target: Some(ta_sim::Target::Ground(at)), ..p };
        assert!(destination_shown(&sim, Some(0), &ground), "a place is always its own side's to see");
    }

    #[test]
    fn launch_intelligence_uses_missile_ownership_even_after_the_silo_is_gone() {
        let mut sim = missiles();
        sim.projectiles[0].age = 1;
        let launcher = sim.projectiles[0].owner;
        assert!(sim.unit(launcher).is_none());
        assert_eq!(launch_owner(&sim, launcher, 0), Some(0));
        let missile = sim.projectiles.remove(0);
        sim.spent.push((missile, Fx::HALF));
        assert_eq!(launch_owner(&sim, launcher, 0), Some(0), "a same-tick interception or impact keeps launch ownership");
    }
}
