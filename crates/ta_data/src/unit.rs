//! Unit definitions (`units/*.fbi`).

use ta_formats::tdf::TdfSection;

/// A unit type, parsed from the `[UNITINFO]` section of an FBI file. Units follow TA's
/// conventions: distances in map pixels, speeds in pixels per tick (30 ticks/s), turn rates
/// in TA angle units (65536 = 360°) per tick, costs in whole resource units.
#[derive(Clone, Debug, Default)]
pub struct UnitDef {
    /// Upper-case identifier (`ARMCOM`); the key used everywhere else.
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub side: String,
    /// Model base name (`objects3d/<object>.3do`), `Objectname`. The script goes by the unit's
    /// own name first: [`UnitDef::script_paths`].
    pub object: String,
    pub footprint_x: u32,
    pub footprint_z: u32,
    /// It takes no ground on the map's grid: its file gives it no footprint (`FootprintX=0` or
    /// `FootprintZ=0`), or it is a mobile unit that cannot get going ([`UnitDef::cannot_get_going`]),
    /// which TA kept off the grid as it kept every mobile unit. The mods' upgrade modules and
    /// upgrades are such units (Total Mayhem's `ARMGEOUP` and its underwater moho mines' 1x1
    /// `ARMUMOHOUP`, TA: Escalation's `ALL_L2`): made where their building or factory stands,
    /// they hold nothing up. (Its footprint is still at least one cell, for its size.)
    pub takes_no_ground: bool,
    /// Its file gives it a speed and an acceleration of none (`BMcode=1`, `MaxVelocity=1`,
    /// `Acceleration=0`): to TA a mobile unit, that never gets going. The mods' upgrade modules
    /// are such units (Total Mayhem's `ARMGEOUP`, TA: Escalation's `ARMHLT_UPGRADE`); none of TA's
    /// own is, and every one of them gives its acceleration. It stands where it was made, and takes
    /// no ground ([`UnitDef::takes_no_ground`]). (A file that gives no acceleration at all is
    /// moved as it always was, slowly.)
    pub cannot_get_going: bool,
    pub build_cost_energy: f64,
    pub build_cost_metal: f64,
    /// Build time in worker-time units; a builder with `worker_time` W finishes in
    /// `build_time / W` seconds.
    pub build_time: f64,
    pub max_damage: f64,
    pub max_water_depth: f64,
    pub min_water_depth: f64,
    pub max_slope: f64,
    pub water_line: f64,
    pub energy_use: f64,
    pub energy_make: f64,
    pub metal_make: f64,
    /// Metal makers: energy converted to metal per second when active.
    pub makes_metal: f64,
    pub extracts_metal: f64,
    pub energy_storage: f64,
    pub metal_storage: f64,
    pub wind_generator: f64,
    pub tidal_generator: f64,
    pub worker_time: f64,
    pub builder: bool,
    pub build_distance: f64,
    pub sight_distance: f64,
    pub radar_distance: f64,
    pub sonar_distance: f64,
    pub radar_jam: f64,
    pub sonar_jam: f64,
    pub stealth: bool,
    pub cloak_cost: f64,
    pub cloak_cost_moving: f64,
    pub min_cloak_distance: f64,
    /// Starts cloaked (mines).
    pub init_cloaked: bool,
    pub can_move: bool,
    pub can_fly: bool,
    pub can_hover: bool,
    /// Aircraft that hover while attacking (gunships) instead of making passes.
    pub hover_attack: bool,
    pub floater: bool,
    /// Aircraft that set down on the water as well as on land: TA's seaplanes say `Amphibious=1`
    /// (with `MaxWaterDepth=255`; the fighters and torpedo bombers that use the carriers give
    /// the depth alone).
    pub amphibious: bool,
    pub upright: bool,
    pub movement_class: String,
    pub max_velocity: f64,
    pub acceleration: f64,
    pub brake_rate: f64,
    pub turn_rate: f64,
    pub cruise_alt: f64,
    pub maneuver_leash: f64,
    pub weapons: [Option<String>; 3],
    /// The categories (`Category` tokens) of the units the unit never picks as targets on its own
    /// (TA: `BadTargetCategory`; an attack order still names them).
    pub bad_target_category: String,
    /// The same for each weapon: `wpri_`, `wsec_` and `wspe_badTargetCategory` (TA3D's `wthi_`
    /// for the third); empty, the unit's [`UnitDef::bad_target_category`] goes for it.
    pub weapon_bad_target_category: [String; 3],
    /// The categories of the units it never goes after (TA: `NoChaseCategory`).
    pub no_chase_category: String,
    pub category: String,
    pub tedclass: String,
    pub explode_as: String,
    pub self_destruct_as: String,
    pub corpse: String,
    pub sound_category: String,
    pub on_offable: bool,
    pub activate_when_built: bool,
    pub can_attack: bool,
    pub can_guard: bool,
    pub can_patrol: bool,
    pub can_stop: bool,
    pub can_reclaim: bool,
    pub can_capture: bool,
    /// Can raise unit wrecks back into working units (Core Contingency's Necro).
    pub can_resurrect: bool,
    pub can_dgun: bool,
    pub can_load: bool,
    /// Maximum passengers: explicit `TransportMaxUnits`, else `TransportCapacity`.
    pub transport_capacity: u32,
    pub transport_size: u32,
    pub is_airbase: bool,
    pub is_commander: bool,
    pub heal_time: f64,
    pub damage_modifier: f64,
    pub immune_to_paralyzer: bool,
    pub hide_damage: bool,
    pub no_auto_fire: bool,
    pub shoot_me: bool,
    pub kamikaze: bool,
    pub kamikaze_distance: f64,
    pub self_destruct_countdown: f64,
    pub standing_fire_order: u32,
    pub standing_move_order: u32,
    pub default_mission: String,
    /// `yardmap` string: per-footprint-cell occupancy for buildings ('o' solid, 'c'
    /// yard/passable when open, 'y' always passable, ...).
    pub yard_map: String,
    pub build_angle: f64,
    pub is_feature: bool,
    pub bank_scale: f64,
    pub pitch_scale: f64,
    pub steering_mode: u32,
    pub unit_number: u32,
    /// Units this builder can build, in menu order (filled from CANBUILD/GUI/download data).
    pub build_options: Vec<String>,
}

impl UnitDef {
    /// Where the unit's script may be, in the order they are tried. TA runs a unit's own
    /// script, `scripts/<UnitName>.cob`, whatever model it borrows: TA: Enhanced gives 62
    /// units another unit's model, 35 of them with scripts under their own names only. A unit
    /// with no script of its own runs its model's, `scripts/<Objectname>.cob`, as it always did
    /// here (Escalation's xARMMLS, Total Mayhem's stockpile units). TA's own units go by one
    /// name for both.
    pub fn script_paths(&self) -> Vec<String> {
        let own = ta_vfs::normalize(&format!("scripts/{}.cob", self.name.trim()));
        let model = ta_vfs::normalize(&format!("scripts/{}.cob", self.object.trim()));
        if model == own { vec![own] } else { vec![own, model] }
    }

    /// The unit's script among the files there are: the first of
    /// [`script_paths`](Self::script_paths) that `exists`, else its own (the one reported
    /// missing).
    pub fn script_path(&self, exists: impl Fn(&str) -> bool) -> String {
        let mut paths = self.script_paths();
        let found = paths.iter().position(|p| exists(p)).unwrap_or(0);
        paths.swap_remove(found)
    }

    pub fn from_tdf(s: &TdfSection) -> Option<UnitDef> {
        let name = s.get("UnitName")?.trim().to_ascii_uppercase();
        if name.is_empty() {
            return None;
        }
        let f = |k: &str, d: f64| s.get_f64_or(k, d);
        let b = |k: &str, d: bool| s.get_bool_or(k, d);
        // (Footprints are at least a cell; other counts at least none.)
        let count = |k: &str, d: i64| s.get_i64_or(k, d).clamp(d.min(1), u32::MAX as i64) as u32;
        let st = |k: &str| s.get(k).unwrap_or("").trim().to_string();
        let weapon = |k: &str| s.get(k).map(|w| w.trim().to_ascii_uppercase()).filter(|w| !w.is_empty());
        let bmcode = s.get_i32_or("BMcode", 0);
        let can_fly = b("CanFly", false);
        let cannot_get_going = !can_fly && f("MaxVelocity", 0.0) > 0.0 && s.get("Acceleration").is_some() && f("Acceleration", 0.0) <= 0.0;
        Some(UnitDef {
            display_name: st("Name"),
            description: st("Description"),
            side: st("Side").to_ascii_uppercase(),
            object: s.get("Objectname").map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).unwrap_or_else(|| name.to_ascii_lowercase()),
            footprint_x: crate::footprint_side(s, "FootprintX"),
            footprint_z: crate::footprint_side(s, "FootprintZ"),
            takes_no_ground: s.get_i64_or("FootprintX", 1) <= 0 || s.get_i64_or("FootprintZ", 1) <= 0 || cannot_get_going,
            cannot_get_going,
            build_cost_energy: f("BuildCostEnergy", 0.0),
            build_cost_metal: f("BuildCostMetal", 0.0),
            build_time: f("BuildTime", 1.0).max(1.0),
            max_damage: f("MaxDamage", 1.0).max(1.0),
            max_water_depth: f("MaxWaterDepth", 0.0),
            min_water_depth: f("MinWaterDepth", -10000.0),
            max_slope: f("MaxSlope", 255.0),
            water_line: f("WaterLine", 0.0),
            energy_use: f("EnergyUse", 0.0),
            energy_make: f("EnergyMake", 0.0),
            metal_make: f("MetalMake", 0.0),
            makes_metal: f("MakesMetal", 0.0),
            extracts_metal: f("ExtractsMetal", 0.0),
            energy_storage: f("EnergyStorage", 0.0),
            metal_storage: f("MetalStorage", 0.0),
            wind_generator: f("WindGenerator", 0.0),
            tidal_generator: f("TidalGenerator", 0.0),
            worker_time: f("WorkerTime", 0.0),
            builder: b("Builder", false),
            build_distance: f("BuildDistance", 64.0),
            sight_distance: f("SightDistance", 0.0),
            radar_distance: f("RadarDistance", 0.0),
            sonar_distance: f("SonarDistance", 0.0),
            radar_jam: f("RadarDistanceJam", 0.0),
            sonar_jam: f("SonarDistanceJam", 0.0),
            stealth: b("Stealth", false),
            cloak_cost: f("CloakCost", 0.0),
            cloak_cost_moving: f("CloakCostMoving", 0.0),
            min_cloak_distance: f("MinCloakDistance", 0.0),
            // (TA's units spell it `init_cloaked`: Core Contingency's mines.)
            init_cloaked: b("init_cloaked", b("InitCloaked", false)),
            can_move: b("CanMove", bmcode != 0),
            can_fly,
            can_hover: b("CanHover", false),
            hover_attack: b("HoverAttack", false),
            floater: b("Floater", false),
            amphibious: b("Amphibious", false),
            upright: b("Upright", false),
            movement_class: st("MovementClass").to_ascii_uppercase(),
            max_velocity: f("MaxVelocity", 0.0),
            acceleration: f("Acceleration", 0.0),
            brake_rate: f("BrakeRate", 0.0),
            turn_rate: f("TurnRate", 0.0),
            cruise_alt: f("CruiseAlt", if can_fly { 100.0 } else { 0.0 }),
            maneuver_leash: f("ManeuverLeashLength", 640.0),
            weapons: [weapon("Weapon1"), weapon("Weapon2"), weapon("Weapon3")],
            bad_target_category: st("BadTargetCategory"),
            weapon_bad_target_category: [
                st("wpri_badTargetCategory"),
                st("wsec_badTargetCategory"),
                Some(st("wspe_badTargetCategory")).filter(|v| !v.is_empty()).unwrap_or_else(|| st("wthi_badTargetCategory")),
            ],
            no_chase_category: st("NoChaseCategory"),
            category: st("Category"),
            tedclass: st("TEDClass").to_ascii_uppercase(),
            explode_as: st("ExplodeAs").to_ascii_uppercase(),
            self_destruct_as: st("SelfDestructAs").to_ascii_uppercase(),
            corpse: st("Corpse"),
            sound_category: st("SoundCategory").to_ascii_uppercase(),
            on_offable: b("OnOffable", false),
            activate_when_built: b("ActivateWhenBuilt", false),
            can_attack: b("CanAttack", false),
            can_guard: b("CanGuard", false),
            can_patrol: b("CanPatrol", false),
            can_stop: b("CanStop", false),
            can_reclaim: b("CanReclamate", false),
            can_capture: b("CanCapture", false),
            can_resurrect: b("CanResurrect", false),
            can_dgun: b("CanDgun", false),
            can_load: b("CanLoad", false),
            transport_capacity: match count("TransportMaxUnits", 0) { 0 => count("TransportCapacity", 0), n => n },
            transport_size: count("TransportSize", 0),
            is_airbase: b("IsAirBase", false),
            is_commander: b("Commander", false),
            heal_time: f("HealTime", 0.0),
            damage_modifier: f("DamageModifier", 1.0),
            immune_to_paralyzer: b("ImmuneToParalyzer", false),
            hide_damage: b("HideDamage", false),
            no_auto_fire: b("NoAutoFire", false),
            shoot_me: b("ShootMe", false),
            kamikaze: b("Kamikaze", false),
            kamikaze_distance: f("KamikazeDistance", 0.0),
            self_destruct_countdown: f("SelfDestructCountdown", 5.0),
            standing_fire_order: s.get_i32_or("StandingFireOrder", 2).clamp(0, 2) as u32,
            standing_move_order: s.get_i32_or("StandingMoveOrder", 1).clamp(0, 2) as u32,
            default_mission: st("DefaultMissionType"),
            yard_map: st("YardMap"),
            build_angle: f("BuildAngle", 0.0),
            is_feature: b("IsFeature", false),
            bank_scale: f("BankScale", 1.0),
            pitch_scale: f("PitchScale", 1.0),
            steering_mode: s.get_i64_or("SteeringMode", 1).clamp(0, u32::MAX as i64) as u32,
            unit_number: count("UnitNumber", 0),
            build_options: Vec::new(),
            name,
        })
    }

    /// Buildings are units that cannot move. Factories set `CanMove=1` (to accept rally
    /// orders), so speed is the reliable signal.
    pub fn is_building(&self) -> bool {
        self.max_velocity <= 0.0 && !self.can_fly
    }

    pub fn has_category(&self, cat: &str) -> bool {
        self.category.split_whitespace().any(|c| c.eq_ignore_ascii_case(cat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    fn read(text: &str) -> Option<UnitDef> {
        UnitDef::from_tdf(&tdf::parse(text).sections[0])
    }

    #[test]
    fn a_unit_is_read_as_ta_reads_its_definition() {
        let u = read("[UNITINFO]{ unitname = armpw ; name=Peewee; side=arm; objectname=; weapon1= emg ; Weapon2=; BMcode=1; MaxVelocity=1.5; TransportCapacity=-4; StandingFireOrder=9; Category=KBOT  WEAPON ;footprintx=0;}").unwrap();
        assert_eq!((u.name.as_str(), u.display_name.as_str(), u.side.as_str()), ("ARMPW", "Peewee", "ARM"), "names in capitals, trimmed");
        assert_eq!(u.object, "armpw", "no model named: the unit's own name");
        assert_eq!(u.weapons, [Some("EMG".to_string()), None, None], "an empty slot is none");
        assert!(u.can_move, "its BMcode moves it");
        assert_eq!((u.max_velocity, u.transport_capacity, u.standing_fire_order, u.footprint_x, u.footprint_z), (1.5, 0, 2, 1, 1));
        assert!(u.has_category("weapon") && u.has_category("KBOT") && !u.has_category("SHIP"));
        assert!(!u.is_building() && u.cruise_alt == 0.0);
        // A flyer cruises at TA's height by default, and is no building whatever its speed.
        let f = read("[UNITINFO]{UnitName=FLY;CanFly=1;}").unwrap();
        assert_eq!(f.cruise_alt, 100.0);
        assert!(!f.is_building() && !f.can_move);
        // A building: nothing moves it.
        assert!(read("[UNITINFO]{UnitName=WALL;}").unwrap().is_building());
        // No name, no unit.
        assert!(read("[UNITINFO]{UnitName= ;}").is_none());
        assert!(read("[UNITINFO]{Side=ARM;}").is_none());
        // Numbers as TA's tools wrote them, read as its atoi and atof did: the number at the
        // start, none without one.
        let x = read("[UNITINFO]{UnitName=X; MaxDamage=0; BuildTime=.5; MaxVelocity=2.5 fast; Builder=yes; WorkerTime=-;}").unwrap();
        assert_eq!((x.max_damage, x.build_time, x.max_velocity, x.worker_time), (1.0, 1.0, 2.5, 0.0), "at least a point of damage and a tick of building");
        assert!(!x.builder, "yes is no number: 0, as TA read it");
    }

    /// The mods' upgrade modules: a footprint of none, or a speed with an acceleration of none (a
    /// mobile unit to TA, that never gets going), takes no ground; a speed with no acceleration
    /// given moves as before.
    #[test]
    fn a_module_takes_no_ground() {
        let module = read("[UNITINFO]{UnitName=ARMGEOUP; FootprintX=0; FootprintZ=0; BMcode=1; MaxVelocity=1; Acceleration=0;}").unwrap();
        assert!(module.takes_no_ground && module.cannot_get_going);
        let underwater = read("[UNITINFO]{UnitName=ARMUMOHOUP; FootprintX=1; FootprintZ=1; MaxVelocity=1; Acceleration=0;}").unwrap();
        assert!(underwater.takes_no_ground && underwater.cannot_get_going, "a 1x1 one that never gets going");
        let upgrade = read("[UNITINFO]{UnitName=ALL_L2; FootprintX=0; FootprintZ=0; canmove=0; MaxVelocity=1; Acceleration=0;}").unwrap();
        assert!(upgrade.takes_no_ground);
        let tank = read("[UNITINFO]{UnitName=TANK; FootprintX=2; FootprintZ=2; MaxVelocity=1; Acceleration=0.1;}").unwrap();
        assert!(!tank.takes_no_ground && !tank.cannot_get_going);
        let unsaid = read("[UNITINFO]{UnitName=TANK; FootprintX=2; FootprintZ=2; MaxVelocity=1;}").unwrap();
        assert!(!unsaid.takes_no_ground && !unsaid.cannot_get_going, "no acceleration given: moved as it always was");
        let plane = read("[UNITINFO]{UnitName=PLANE; CanFly=1; MaxVelocity=8; Acceleration=0;}").unwrap();
        assert!(!plane.cannot_get_going);
    }

    /// TA's units spell the initial cloak `init_cloaked` (Core Contingency's mines).
    #[test]
    fn mines_start_cloaked_as_their_files_say() {
        assert!(read("[UNITINFO]{UnitName=CORMINE1; CloakCost=5; init_cloaked=1;}").unwrap().init_cloaked);
        assert!(read("[UNITINFO]{UnitName=X; InitCloaked=1;}").unwrap().init_cloaked);
        assert!(!read("[UNITINFO]{UnitName=X; CloakCost=5;}").unwrap().init_cloaked);
    }
}
