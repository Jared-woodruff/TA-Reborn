//! Unit definitions (`units/*.fbi`).

use ta_formats::tdf::TdfSection;

/// The most weapons a unit carries: TA's three (`Weapon1` to `Weapon3`) and five more for mods
/// (`Weapon4` to `Weapon8`).
pub const MAX_WEAPONS: usize = 8;

/// The script functions the engine calls for each weapon slot (from the first), by what it asks
/// of them ([`WeaponFunction`]): TA's names for the first three slots (`AimPrimary`,
/// `AimSecondary`, `AimTertiary`), and `AimWeapon1` to `AimWeapon8` (Spring's) for every slot. A
/// script defines one or the other; TA's name goes first where it defines both.
pub const WEAPON_FUNCTIONS: [[[&str; 2]; MAX_WEAPONS]; 4] = [
    [
        ["AimPrimary", "AimWeapon1"], ["AimSecondary", "AimWeapon2"], ["AimTertiary", "AimWeapon3"], ["AimWeapon4", "AimWeapon4"],
        ["AimWeapon5", "AimWeapon5"], ["AimWeapon6", "AimWeapon6"], ["AimWeapon7", "AimWeapon7"], ["AimWeapon8", "AimWeapon8"],
    ],
    [
        ["AimFromPrimary", "AimFromWeapon1"], ["AimFromSecondary", "AimFromWeapon2"], ["AimFromTertiary", "AimFromWeapon3"], ["AimFromWeapon4", "AimFromWeapon4"],
        ["AimFromWeapon5", "AimFromWeapon5"], ["AimFromWeapon6", "AimFromWeapon6"], ["AimFromWeapon7", "AimFromWeapon7"], ["AimFromWeapon8", "AimFromWeapon8"],
    ],
    [
        ["QueryPrimary", "QueryWeapon1"], ["QuerySecondary", "QueryWeapon2"], ["QueryTertiary", "QueryWeapon3"], ["QueryWeapon4", "QueryWeapon4"],
        ["QueryWeapon5", "QueryWeapon5"], ["QueryWeapon6", "QueryWeapon6"], ["QueryWeapon7", "QueryWeapon7"], ["QueryWeapon8", "QueryWeapon8"],
    ],
    [
        ["FirePrimary", "FireWeapon1"], ["FireSecondary", "FireWeapon2"], ["FireTertiary", "FireWeapon3"], ["FireWeapon4", "FireWeapon4"],
        ["FireWeapon5", "FireWeapon5"], ["FireWeapon6", "FireWeapon6"], ["FireWeapon7", "FireWeapon7"], ["FireWeapon8", "FireWeapon8"],
    ],
];

/// What the engine asks of a weapon's script (an index into [`WEAPON_FUNCTIONS`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponFunction {
    /// `AimPrimary(heading, pitch)`: turns the turret, returns once aimed.
    Aim = 0,
    /// `AimFromPrimary(piece)`: the piece aimed from.
    AimFrom = 1,
    /// `QueryPrimary(piece)`: the piece the shot leaves from.
    Query = 2,
    /// `FirePrimary()`: recoil and flash as it fires.
    Fire = 3,
}

impl WeaponFunction {
    pub const ALL: [WeaponFunction; 4] = [WeaponFunction::Aim, WeaponFunction::AimFrom, WeaponFunction::Query, WeaponFunction::Fire];

    /// The names a script may give this function of weapon `slot` (from 0), TA's first.
    pub fn names(self, slot: usize) -> [&'static str; 2] {
        WEAPON_FUNCTIONS[self as usize][slot]
    }
}

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
    /// Its weapons by slot: `Weapon1` to `Weapon8` (TA's units carry three at most).
    pub weapons: [Option<String>; MAX_WEAPONS],
    /// The categories (`Category` tokens) of the units the unit never picks as targets on its own
    /// (TA: `BadTargetCategory`; an attack order still names them).
    pub bad_target_category: String,
    /// The same for each weapon: `wpri_`, `wsec_` and `wspe_badTargetCategory` (TA3D's `wthi_`
    /// for the third), or for any slot `BadTargetCategory<n>` (Spring's, `n` from 1); empty, the
    /// unit's [`UnitDef::bad_target_category`] goes for it.
    pub weapon_bad_target_category: [String; MAX_WEAPONS],
    /// The categories of the only units each weapon may take on, of its own accord or by order
    /// (`OnlyTargetCategory<n>`, Spring's, `n` from 1): none given, any (TA's own name none).
    /// A weapon that names some never fires on the ground, nor on a unit of no category named,
    /// whatever it is told: Supreme Commander's anti-air (`VTOL`).
    pub weapon_only_target_category: [Option<String>; MAX_WEAPONS],
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
    /// A targeting facility (`IsTargetingUpgrade`: Core Contingency's Targeting Facilities,
    /// ARMTARG and CORTARG, "Automatic Radar Targeting"): while one of a player's is at work, its
    /// units fire at will on what its side has on radar as on what it sees.
    pub is_targeting_upgrade: bool,
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
    /// Units this builder offers a player, in menu order: its GUI pages' and the download
    /// entries naming it (`GameData::build_menus`).
    pub build_options: Vec<String>,
    /// The unit it becomes when upgraded in place (`UpgradesTo`, Supreme Commander's
    /// `General.UpgradesTo`): given the order, it builds that unit on itself with its own build
    /// power (`WorkerTime`), paying the unit's cost over its build time as construction does, and
    /// is that unit when done, where it stood. Upper case; empty: none (TA's own units have none).
    pub upgrades_to: String,
    /// What it may install on itself (`[ENHANCEMENTS]` within its `[UNITINFO]`, in the file's
    /// order: [`crate::EnhancementDef`]): Supreme Commander's commander enhancements. None of TA's.
    pub enhancements: Vec<crate::EnhancementDef>,
    /// A factory's ways out (`[EXITS]` within its `[UNITINFO]`, in the file's order:
    /// [`ExitDef`]): Supreme Commander's roll-off points. None of TA's, whose units drive out of
    /// the front.
    pub exits: Vec<ExitDef>,
    /// The way it faces while a factory with ways out builds it, whichever way it will leave by
    /// (`BuildHeading`, degrees from the factory's front as a way's `Heading` counts them):
    /// Supreme Commander's `ForcedBuildSpin`, its long transports and gunships built along their
    /// air factories' platforms. None given, it faces its way out.
    pub build_heading: Option<f64>,
    /// What it gives the buildings of its player beside it (`[ADJACENCY]` within its `[UNITINFO]`,
    /// in the file's order: [`crate::AdjacencyDef`]), and its size class for what it takes from
    /// them (`AdjacencySize`; 0: none, it takes any size's bonus and from as many as touch it):
    /// Supreme Commander's adjacency. None of TA's.
    pub adjacency: Vec<crate::AdjacencyDef>,
    pub adjacency_size: u32,
    /// How the link between it and a building its adjacency helps is drawn on the battlefield
    /// (`AdjacencyBeamColor`, red, green and blue from 0 to 1, and `AdjacencyBeamThickness`,
    /// pixels across): a beam from it to that building, as Supreme Commander draws its
    /// structures' adjacency. None: the battlefield's own link alone. Presentation only.
    pub adjacency_beam: Option<([f32; 3], f32)>,
    /// Its own particle effects (`SfxEffects`: names of `effects/*.tdf`'s effects, in order), which
    /// its script plays with `emit-sfx 1024 + n` (the nth, counted from 0: [`crate::effect`]).
    /// Presentation only.
    pub sfx_effects: Vec<String>,
    /// What the computer players build with it besides: the 3.1 patch's `[CANBUILD]` list for it
    /// (`gamedata/sidedata.tdf`), its entries not on its menu. TA's computer players went by
    /// these lists, its players by the menus: mods list units of their AI there that players are
    /// never offered (TA: The Final Frontier's and Total Mayhem's AI Boost: `AAISOLAR` beside the
    /// `ARMSOLAR` a player builds, its own factories, the space mod's ground labs).
    pub ai_build_options: Vec<String>,
    /// TA Reborn's stealth fields (`RadarStealthField`, `SonarStealthField`, px): its side's units
    /// within them are hidden from enemy radar (and sonar) as `Stealth=1` hides one, while it is at
    /// work (Supreme Commander's stealth field generators). TA's jammers do the same and hide
    /// missiles too.
    pub radar_stealth_field: f64,
    pub sonar_stealth_field: f64,
    /// Remote viewing (`RemoteViewRadius`, px; `RemoteViewEnergy`, energy a second while it
    /// views): told to view a place, it sees round it as though from high over it, as long as the
    /// order stands and it pays (the Aeon's Eye of Rhianne).
    pub remote_view_radius: f64,
    pub remote_view_energy: f64,
    /// A regeneration aura (`RegenAuraRadius`, px; `RegenAuraRate`, hit points a second; or
    /// `RegenAuraShare`, a share of each one's most health a second, up to the rate where both are
    /// given): it mends its side's other finished units within it, once a second.
    pub regen_aura_radius: f64,
    pub regen_aura_rate: f64,
    pub regen_aura_share: f64,
    /// Its weapon toggle (`[TOGGLE]` within its `[UNITINFO]`: [`crate::ToggleDef`]). None of TA's.
    pub toggle: Option<crate::ToggleDef>,
    /// Its shield (`ShieldHealth` and the other shield keys: [`crate::ShieldDef`]), a personal one or
    /// a bubble. None of TA's.
    pub shield: Option<crate::ShieldDef>,
    /// An air transport's slots (`TransportSlots`: [`crate::TransportSlotDef`]), each a piece its
    /// passenger hangs from, with its class: it carries a unit in each. None of TA's, whose air
    /// transports carry one.
    pub transport_slots: Vec<crate::TransportSlotDef>,
    /// The class of slot it takes aboard a transport with slots (`TransportClass`; 1 where none is
    /// given): one of its class, else of the smallest larger class free.
    pub transport_class: u32,
    /// Its teleport (`CanTeleport`, `TeleportEnergy`, `TeleportTime`: [`crate::TeleportDef`]). None
    /// of TA's.
    pub teleport: Option<crate::TeleportDef>,
    /// Its drones (`[DRONES]` within its `[UNITINFO]`: [`crate::DroneDef`]). None of TA's.
    pub drones: Vec<crate::DroneDef>,
    /// A factory that moves (`MobileFactory=1`): it builds the units of its menu from its queue as a
    /// factory does, each on itself at its `BuildPiece` (a piece of its model; none: its middle)
    /// while it goes, and sets each down beside it once it is done (an aircraft takes off). None of
    /// TA's, whose factories stand.
    pub mobile_factory: bool,
    pub build_piece: String,
}

impl UnitDef {
    /// Whether it stands on land, as a land unit or building does (not flying, hovering, floating
    /// or needing water): what its model reaches below its foot is under the ground, which hides
    /// it in a battle (TA: Escalation's Guardian has a skirt 73 high under its base, to close the
    /// gap on a slope), and its pictures leave that out.
    pub fn stands_on_land(&self) -> bool {
        !self.can_fly && !self.can_hover && !self.floater && self.min_water_depth <= 0.0
    }

    /// Whether it is one of TA's underwater buildings: a building that needs water under it
    /// (`MinWaterDepth`) and neither floats (`Floater`) nor sits at a `WaterLine`, so it stands on
    /// the sea bed: the underwater metal extractors, storages and fusion plants. TA's buildings on
    /// the water (the floating towers, torpedo launchers, tidal generators, sonar stations and
    /// shipyards) give a `WaterLine`.
    pub fn stands_on_sea_bed(&self) -> bool {
        self.is_building() && self.min_water_depth > 0.0 && !self.floater && self.water_line <= 0.0
    }

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
            weapons: std::array::from_fn(|slot| weapon(&format!("Weapon{}", slot + 1))),
            bad_target_category: st("BadTargetCategory"),
            weapon_bad_target_category: std::array::from_fn(|slot| {
                let ta = match slot {
                    0 => st("wpri_badTargetCategory"),
                    1 => st("wsec_badTargetCategory"),
                    2 => Some(st("wspe_badTargetCategory")).filter(|v| !v.is_empty()).unwrap_or_else(|| st("wthi_badTargetCategory")),
                    _ => String::new(),
                };
                if ta.is_empty() { st(&format!("BadTargetCategory{}", slot + 1)) } else { ta }
            }),
            weapon_only_target_category: std::array::from_fn(|slot| Some(st(&format!("OnlyTargetCategory{}", slot + 1))).filter(|v| !v.is_empty())),
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
            is_targeting_upgrade: b("IsTargetingUpgrade", false),
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
            upgrades_to: st("UpgradesTo").to_ascii_uppercase(),
            enhancements: crate::EnhancementDef::of_unit(s),
            exits: ExitDef::of_unit(s),
            build_heading: s.get_f64("BuildHeading").filter(|v| v.is_finite()),
            adjacency: crate::AdjacencyDef::of_unit(s),
            adjacency_size: s.get_i64_or("AdjacencySize", 0).clamp(0, u32::MAX as i64) as u32,
            adjacency_beam: s.get("AdjacencyBeamColor").and_then(crate::weapon::rgb).map(|c| (c, s.get_f64("AdjacencyBeamThickness").filter(|t| t.is_finite() && *t > 0.0).map_or(ADJACENCY_BEAM, |t| t.min(64.0) as f32))),
            sfx_effects: crate::effect::unit_effects(s),
            build_options: Vec::new(),
            ai_build_options: Vec::new(),
            radar_stealth_field: f("RadarStealthField", 0.0).max(0.0),
            sonar_stealth_field: f("SonarStealthField", 0.0).max(0.0),
            remote_view_radius: f("RemoteViewRadius", 0.0).max(0.0),
            remote_view_energy: f("RemoteViewEnergy", 0.0).max(0.0),
            regen_aura_radius: f("RegenAuraRadius", 0.0).max(0.0),
            regen_aura_rate: f("RegenAuraRate", 0.0).max(0.0),
            regen_aura_share: f("RegenAuraShare", 0.0).max(0.0),
            toggle: crate::ToggleDef::of_unit(s),
            shield: crate::ShieldDef::of(s),
            transport_slots: crate::TransportSlotDef::of_unit(s),
            transport_class: crate::transport::transport_class(s),
            teleport: crate::TeleportDef::of(s),
            drones: crate::DroneDef::of(s),
            mobile_factory: b("MobileFactory", false),
            build_piece: st("BuildPiece"),
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

/// How thick an adjacency beam is drawn where its unit gives a colour and no thickness (pixels).
pub const ADJACENCY_BEAM: f32 = 2.5;

/// Most ways out a factory's file may give (the rest are left out, with a warning).
pub const MAX_EXITS: usize = 16;

/// One of a factory's ways out (a section of the `[EXITS]` within its `[UNITINFO]`, any name):
/// Supreme Commander's roll-off points. A unit the factory makes is built facing the way out
/// nearest where it is sent next (its first rally point; with none, the factory's front), rolls
/// off its pad along it as far as its end, and goes on from there. The pad keeps its height to
/// the top of the way's ramp, and the unit comes down it to the ground at its foot, rather than
/// dropping through the factory's floor.
///
/// ```text
/// [UNITINFO]
/// {
///     UnitName=UEB0101;
///     ...
///     [EXITS]
///     {
///         [EXIT1] { X=96; Z=0; Heading=90; Ramp=40 68; }
///         [EXIT2] { X=-96; Z=0; Heading=-90; Ramp=40 68; }
///     }
/// }
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExitDef {
    /// Where the way out leads (`X`, `Z`): pixels from the factory's middle as it stands facing
    /// south, TA's buildings' way (x east, z south), turned with it.
    pub x: f64,
    pub z: f64,
    /// The way a unit made for it faces as it is built (`Heading`, degrees as TA counts a unit's
    /// heading: 0 the factory's front, its +z; 90 towards its +x), turned with the factory.
    pub heading: f64,
    /// Its ramp (`Ramp`, two numbers: pixels along the way from the pad): the unit keeps the pad's
    /// height as far as the first, and comes down evenly to the ground at the second. None given,
    /// it is on the ground at once.
    pub ramp: (f64, f64),
}

impl ExitDef {
    pub fn from_tdf(s: &TdfSection) -> ExitDef {
        let f = |k: &str| s.get_f64_or(k, 0.0);
        let numbers: Vec<f64> = s.get("Ramp").unwrap_or("").split([' ', ',', '\t']).filter_map(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).collect();
        let ramp = match numbers.as_slice() {
            [] => (0.0, 0.0),
            [top] => (top.max(0.0), top.max(0.0)),
            [top, foot, ..] => (top.max(0.0), foot.max(top.max(0.0))),
        };
        ExitDef { x: f("X"), z: f("Z"), heading: f("Heading"), ramp }
    }

    /// The ways out of a unit's `[UNITINFO]` section: its `[EXITS]` section's own, in the file's
    /// order (the first of two equally near is taken).
    pub fn of_unit(info: &TdfSection) -> Vec<ExitDef> {
        let Some(list) = info.sections.iter().find(|s| s.name.eq_ignore_ascii_case("EXITS")) else { return Vec::new() };
        if list.sections.len() > MAX_EXITS {
            log::warn!("{}: more than {MAX_EXITS} ways out; the rest are left out", info.get("UnitName").unwrap_or("?"));
        }
        list.sections.iter().take(MAX_EXITS).map(ExitDef::from_tdf).collect()
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
        assert_eq!(u.weapons[..3], [Some("EMG".to_string()), None, None], "an empty slot is none");
        assert!(u.weapons[3..].iter().all(Option::is_none));
        // Mods' units carry up to eight, each with its own bad targets as Spring names them.
        let many = read("[UNITINFO]{UnitName=MANY;Weapon1=A;Weapon5=E;Weapon8=H;wpri_badTargetCategory=VTOL;BadTargetCategory1=SHIP;BadTargetCategory5=NOTAIR;}").unwrap();
        assert_eq!(many.weapons.iter().map(|w| w.as_deref().unwrap_or("-")).collect::<String>(), "A---E--H");
        assert_eq!((many.weapon_bad_target_category[0].as_str(), many.weapon_bad_target_category[4].as_str()), ("VTOL", "NOTAIR"), "TA's own key goes first");
        assert_eq!(WeaponFunction::Aim.names(0), ["AimPrimary", "AimWeapon1"]);
        assert_eq!(WeaponFunction::Fire.names(7), ["FireWeapon8", "FireWeapon8"]);
        assert_eq!(WeaponFunction::Query.names(2), ["QueryTertiary", "QueryWeapon3"]);
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
        assert!(u.upgrades_to.is_empty(), "TA's units upgrade into nothing");
        assert_eq!(read("[UNITINFO]{UnitName=UEB0101; UpgradesTo= ueb0201 ;}").unwrap().upgrades_to, "UEB0201", "the unit it upgrades into, in capitals");
        // TA's units restrict no weapon's targets.
        assert!(u.weapon_only_target_category.iter().all(Option::is_none));
        // TA's factories name no ways out, nor its units a way to face while built.
        assert!(u.exits.is_empty() && u.build_heading.is_none());
    }

    /// A weapon's only targets (`OnlyTargetCategory<n>`, by slot from 1).
    #[test]
    fn only_targets_are_read() {
        let u = read("[UNITINFO]{UnitName=TOWER; Weapon1=GUN; Weapon3=AA; OnlyTargetCategory3= VTOL ; OnlyTargetCategory1=; }").unwrap();
        assert_eq!(u.weapon_only_target_category[2].as_deref(), Some("VTOL"));
        assert!(u.weapon_only_target_category[0].is_none(), "an empty list is none");
        assert!(u.weapon_only_target_category[1].is_none(), "none given");
    }

    /// A factory's ways out (`[EXITS]`, a section each, in the file's order; a ramp of one number
    /// is a step, of none on the ground at once), and the way a unit faces while built by one
    /// (`BuildHeading`).
    #[test]
    fn ways_out_are_read() {
        let u = read(
            "[UNITINFO]{UnitName=PLANT;
             [EXITS]{ [EXIT1]{ X=96; Z=0; Heading=90; Ramp=40 68; } [WEST]{ X=-96; Z=8; Heading=-90; Ramp=30; } [FRONT]{ Z=80; } } }",
        )
        .unwrap();
        assert_eq!(
            u.exits,
            [
                ExitDef { x: 96.0, z: 0.0, heading: 90.0, ramp: (40.0, 68.0) },
                ExitDef { x: -96.0, z: 8.0, heading: -90.0, ramp: (30.0, 30.0) },
                ExitDef { x: 0.0, z: 80.0, heading: 0.0, ramp: (0.0, 0.0) },
            ]
        );
        assert_eq!(read("[UNITINFO]{UnitName=LIFTER; BuildHeading= 90 ;}").unwrap().build_heading, Some(90.0));
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

    /// Core Contingency's Targeting Facilities say `istargetingupgrade=1`, in lower case as TA's
    /// keys often are.
    #[test]
    fn a_targeting_facility_is_read_as_one() {
        assert!(read("[UNITINFO]{UnitName=ARMTARG; onoffable=1; ActivateWhenBuilt=1; istargetingupgrade=1;}").unwrap().is_targeting_upgrade);
        assert!(read("[UNITINFO]{UnitName=X; IsTargetingUpgrade=1;}").unwrap().is_targeting_upgrade);
        assert!(!read("[UNITINFO]{UnitName=ARMRAD; RadarDistance=2200;}").unwrap().is_targeting_upgrade);
    }

    /// TA's underwater buildings stand on the sea bed: they need water and give no `WaterLine`;
    /// its buildings on the water give one.
    #[test]
    fn underwater_buildings_stand_on_the_sea_bed() {
        assert!(read("[UNITINFO]{UnitName=ARMUWMEX; MinWaterDepth=19; ExtractsMetal=0.001;}").unwrap().stands_on_sea_bed());
        assert!(!read("[UNITINFO]{UnitName=ARMFRT; MinWaterDepth=2; WaterLine=0.3;}").unwrap().stands_on_sea_bed(), "a floating tower");
        assert!(!read("[UNITINFO]{UnitName=ARMSUB; BMcode=1; MaxVelocity=2; MinWaterDepth=20; WaterLine=20;}").unwrap().stands_on_sea_bed(), "a submarine");
        assert!(!read("[UNITINFO]{UnitName=ARMPT; BMcode=1; MaxVelocity=3; MinWaterDepth=6; Floater=1;}").unwrap().stands_on_sea_bed(), "a boat");
        assert!(!read("[UNITINFO]{UnitName=ARMLLT;}").unwrap().stands_on_sea_bed(), "a tower on land");
    }

    /// TA Reborn's abilities read from a unit's file: stealth fields, remote viewing, a regeneration
    /// aura and a weapon toggle; none of them on TA's own units.
    #[test]
    fn abilities_are_read() {
        let u = read(
            "[UNITINFO]{UnitName=GEN; RadarStealthField=768; SonarStealthField=960; RemoteViewRadius=800; RemoteViewEnergy=5000;\
             RegenAuraRadius=480; RegenAuraRate=75; RegenAuraShare=0.0055; Weapon1=GUN; [TOGGLE]{ Name=Long Gun; Weapon1=LONG; } }",
        )
        .unwrap();
        assert_eq!((u.radar_stealth_field, u.sonar_stealth_field), (768.0, 960.0));
        assert_eq!((u.remote_view_radius, u.remote_view_energy), (800.0, 5000.0));
        assert_eq!((u.regen_aura_radius, u.regen_aura_rate, u.regen_aura_share), (480.0, 75.0, 0.0055));
        assert_eq!(u.toggle.as_ref().map(|t| t.weapons[0].as_deref()), Some(Some("LONG")));
        let plain = read("[UNITINFO]{UnitName=ARMPW; RegenAuraRadius=-3;}").unwrap();
        assert_eq!((plain.radar_stealth_field, plain.remote_view_radius, plain.regen_aura_radius), (0.0, 0.0, 0.0));
        assert!(plain.toggle.is_none());
        // An air transport's slots, a passenger's class and a teleport; none on TA's.
        let lifter = read("[UNITINFO]{UnitName=LIFTER; CanFly=1; TransportSlots=A B Mid:2; TransportClass=2; CanTeleport=1; TeleportEnergy=100;}").unwrap();
        assert_eq!(lifter.transport_slots.len(), 3);
        assert_eq!((lifter.transport_class, lifter.teleport.map(|t| t.energy)), (2, Some(100.0)));
        assert!(plain.transport_slots.is_empty() && plain.teleport.is_none() && plain.transport_class == 1);
        // Drones and a factory that moves; none on TA's.
        let carrier = read("[UNITINFO]{UnitName=CARRIER; MobileFactory=1; BuildPiece=Deck; [DRONES]{ Unit=POD; Count=2; } }").unwrap();
        assert!(carrier.mobile_factory && carrier.build_piece == "Deck" && carrier.drones.len() == 1);
        assert!(!plain.mobile_factory && plain.build_piece.is_empty() && plain.drones.is_empty());
    }

    /// TA's units spell the initial cloak `init_cloaked` (Core Contingency's mines).
    #[test]
    fn mines_start_cloaked_as_their_files_say() {
        assert!(read("[UNITINFO]{UnitName=CORMINE1; CloakCost=5; init_cloaked=1;}").unwrap().init_cloaked);
        assert!(read("[UNITINFO]{UnitName=X; InitCloaked=1;}").unwrap().init_cloaked);
        assert!(!read("[UNITINFO]{UnitName=X; CloakCost=5;}").unwrap().init_cloaked);
    }
}
