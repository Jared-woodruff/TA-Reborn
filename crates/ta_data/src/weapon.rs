//! Weapon definitions (`weapons/*.tdf`; each top-level section is one weapon).

use rustc_hash::FxHashMap;
use ta_formats::tdf::TdfSection;

/// How a projectile is drawn (TA's `rendertype`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderType {
    /// 3DO model (missiles, bombs, torpedoes).
    #[default]
    Model,
    /// Laser beam / line.
    Laser,
    /// Dot/particle.
    Point,
    /// Lightning bolt.
    Lightning,
    /// The Commander's D-gun energy ball.
    Disintegrator,
    /// Dropped bombs (drawn with a model).
    Bomb,
    /// Plasma/cannon shell sprite ("ball").
    Shell,
    /// Flame thrower particles.
    Flame,
    /// Unknown values are kept for mods.
    Other(i32),
}

impl RenderType {
    fn from_i32(v: i32) -> RenderType {
        match v {
            // Mapping verified against the shipped weapons (e.g. 0 = ARMFHLT_LASER,
            // 3 = ARM_DISINTEGRATOR, 6 = ARMBOMB, 7 = LIGHTNING).
            0 => RenderType::Laser,
            1 => RenderType::Model,
            2 => RenderType::Point,
            3 => RenderType::Disintegrator,
            4 => RenderType::Shell,
            5 => RenderType::Flame,
            6 => RenderType::Bomb,
            7 => RenderType::Lightning,
            other => RenderType::Other(other),
        }
    }
}

/// The class of missiles TA's own interceptors and targetable missiles name (`interceptor=1`,
/// `targetable=1`): the strategic ones, its nukes and the EMP and tactical missiles its silos
/// fire, and what its anti-nukes shoot down. A mod may name others (bit 2 the Supreme Commander
/// factions' tactical missiles, for their tactical missile defences).
pub const STRATEGIC_CLASS: u32 = 1;

#[derive(Clone, Debug, Default)]
pub struct WeaponDef {
    /// Upper-case section name, referenced by `Weapon1=` in FBIs.
    pub name: String,
    pub display_name: String,
    pub id: i32,
    pub render_type: RenderType,
    pub raw_render_type: i32,
    pub range: f64,
    /// Seconds between shots (bursts count as one shot).
    pub reload_time: f64,
    /// Pixels per second.
    pub velocity: f64,
    pub start_velocity: f64,
    pub acceleration: f64,
    pub turn_rate: f64,
    /// Diameter of the damage area in pixels.
    pub area_of_effect: f64,
    pub edge_effectiveness: f64,
    pub default_damage: f64,
    /// Per-target-unit damage overrides (upper-case unit names).
    pub damage: FxHashMap<String, f64>,
    pub burst: u32,
    pub burst_rate: f64,
    pub spray_angle: f64,
    pub accuracy: f64,
    pub tolerance: f64,
    pub pitch_tolerance: f64,
    pub ballistic: bool,
    pub line_of_sight: bool,
    pub dropped: bool,
    pub self_prop: bool,
    pub vlaunch: bool,
    pub tracks: bool,
    pub guidance: bool,
    pub turret: bool,
    pub beam_weapon: bool,
    /// Beam duration in seconds (lasers).
    pub duration: f64,
    pub fire_starter: f64,
    pub water_weapon: bool,
    /// Underwater launch permission; defaults to water_weapon when omitted.
    pub fire_submersed: bool,
    pub to_air_weapon: bool,
    pub no_auto_range: bool,
    pub no_explode: bool,
    pub paralyzer: bool,
    pub command_fire: bool,
    pub stockpile: bool,
    /// An anti-missile weapon (TA's `interceptor` not 0): it shoots down the missiles whose
    /// classes it shares ([`WeaponDef::intercepts`]).
    pub interceptor: bool,
    /// A missile anti-missile weapons may shoot down (TA's `targetable` not 0).
    pub targetable: bool,
    /// TA's `interceptor` and `targetable` as written, read as sets of classes as the Spring
    /// engine reads them: an interceptor takes a missile whose `targetable` shares a bit with its
    /// `interceptor` (TA's own say 1 and 1: its anti-nukes take its nukes, its EMP and tactical
    /// missiles). 0 when not given; a definition built with the flags alone is class 1
    /// ([`WeaponDef::interceptor_classes`]).
    pub interceptor_mask: u32,
    pub targetable_mask: u32,
    pub coverage: f64,
    pub energy_per_shot: f64,
    pub metal_per_shot: f64,
    /// Palette colour indices for lasers.
    pub color: u8,
    pub color2: u8,
    pub model: String,
    pub start_smoke: bool,
    pub end_smoke: bool,
    pub smoke_trail: bool,
    pub smoke_delay: f64,
    pub weapon_timer: f64,
    pub flight_time: f64,
    pub twophase: bool,
    pub cruise: bool,
    pub min_barrel_angle: f64,
    pub unitsonly: bool,
    pub explosion_gaf: String,
    pub explosion_art: String,
    pub water_explosion_gaf: String,
    pub water_explosion_art: String,
    pub lava_explosion_gaf: String,
    pub lava_explosion_art: String,
    pub sound_start: String,
    pub sound_hit: String,
    pub sound_water: String,
    pub sound_trigger: bool,
    pub shake_magnitude: f64,
    pub shake_duration: f64,
    /// The nearest it fires (`minrange`, pixels across the ground from its unit to the target):
    /// nothing nearer is taken on, and an attack order keeps its unit between this and its range
    /// (Supreme Commander's `MinRadius`). None (0): as near as it likes, as all of TA's own.
    pub min_range: f64,
    /// TA Reborn's stun (`paralyzetime`, seconds): the enemies its shot hits are left unable to
    /// move, fire or build for that long, beside its damage (Supreme Commander's EMP and Chrono
    /// Dampener). 0: none. ([`WeaponDef::paralyzer`] is TA's, whose damage is the stun alone.)
    pub paralyze_time: f64,
    /// How far round where its shot goes off the stun reaches (`paralyzeradius`, px; 0: as far as
    /// its blast, half its `areaofeffect`), the unit it hits always.
    pub paralyze_radius: f64,
    /// The stun goes off round the unit firing it as it fires (`paralyzeonfire=1`), not where its
    /// shot goes off: the Chrono Dampener's pulse.
    pub paralyze_on_fire: bool,
    /// The categories of the units its stun spares (`paralyzeexempt`, `Category` tokens: any one
    /// of them; empty, none).
    pub paralyze_exempt: String,
}

impl WeaponDef {
    pub fn from_tdf(s: &TdfSection) -> WeaponDef {
        let f = |k: &str, d: f64| s.get_f64_or(k, d);
        let b = |k: &str| s.get_bool_or(k, false);
        let st = |k: &str| s.get(k).unwrap_or("").trim().to_string();
        let mut damage = FxHashMap::default();
        let mut default_damage = 0.0;
        if let Some(d) = s.section("DAMAGE") {
            for (k, v) in &d.entries {
                let v: f64 = v.trim().parse().unwrap_or(0.0);
                if k.eq_ignore_ascii_case("default") {
                    default_damage = v;
                } else {
                    damage.insert(k.trim().to_ascii_uppercase(), v);
                }
            }
        }
        let raw_render_type = s.get_i32_or("rendertype", 1);
        // (Any number as written; a negative one, every class.)
        let mask = |k: &str| match s.get_i64_or(k, 0) {
            v if v > 0 => v.min(u32::MAX as i64) as u32,
            0 => 0,
            _ => u32::MAX,
        };
        WeaponDef {
            name: s.name.trim().to_ascii_uppercase(),
            display_name: st("name"),
            id: s.get_i32_or("ID", -1),
            render_type: RenderType::from_i32(raw_render_type),
            raw_render_type,
            range: f("range", 0.0),
            reload_time: f("reloadtime", 1.0),
            velocity: f("weaponvelocity", 0.0),
            start_velocity: f("startvelocity", 0.0),
            acceleration: f("weaponacceleration", 0.0),
            turn_rate: f("turnrate", 0.0),
            area_of_effect: f("areaofeffect", 8.0),
            edge_effectiveness: f("edgeeffectiveness", 0.0),
            default_damage,
            damage,
            burst: s.get_i64_or("burst", 1).clamp(1, u32::MAX as i64) as u32,
            burst_rate: f("burstrate", 0.1),
            spray_angle: f("sprayangle", 0.0),
            accuracy: f("accuracy", 0.0),
            tolerance: f("tolerance", 0.0),
            pitch_tolerance: f("pitchtolerance", 0.0),
            ballistic: b("ballistic"),
            line_of_sight: b("lineofsight"),
            dropped: b("dropped"),
            self_prop: b("selfprop"),
            vlaunch: b("vlaunch"),
            tracks: b("tracks"),
            guidance: b("guidance"),
            turret: b("turret"),
            beam_weapon: b("beamweapon"),
            duration: f("duration", 0.0),
            fire_starter: f("firestarter", 0.0),
            water_weapon: b("waterweapon"),
            fire_submersed: s.get_bool_or("firesubmersed", b("waterweapon")),
            to_air_weapon: b("toairweapon"),
            no_auto_range: b("noautorange"),
            no_explode: b("noexplode"),
            paralyzer: b("paralyzer"),
            command_fire: b("commandfire"),
            stockpile: b("stockpile"),
            interceptor: mask("interceptor") != 0,
            targetable: mask("targetable") != 0,
            interceptor_mask: mask("interceptor"),
            targetable_mask: mask("targetable"),
            coverage: f("coverage", 0.0),
            energy_per_shot: f("energypershot", 0.0),
            metal_per_shot: f("metalpershot", 0.0),
            color: s.get_i32_or("color", 0).clamp(0, 255) as u8,
            color2: s.get_i32_or("color2", 0).clamp(0, 255) as u8,
            model: st("model"),
            start_smoke: b("startsmoke"),
            end_smoke: b("endsmoke"),
            smoke_trail: b("smoketrail"),
            smoke_delay: f("smokedelay", 0.1),
            weapon_timer: f("weapontimer", 0.0),
            flight_time: f("flighttime", 0.0),
            twophase: b("twophase"),
            cruise: b("cruise"),
            min_barrel_angle: f("minbarrelangle", 0.0),
            unitsonly: b("unitsonly"),
            explosion_gaf: st("explosiongaf"),
            explosion_art: st("explosionart"),
            water_explosion_gaf: st("waterexplosiongaf"),
            water_explosion_art: st("waterexplosionart"),
            lava_explosion_gaf: st("lavaexplosiongaf"),
            lava_explosion_art: st("lavaexplosionart"),
            sound_start: st("soundstart"),
            sound_hit: st("soundhit"),
            sound_water: st("soundwater"),
            sound_trigger: b("soundtrigger"),
            shake_magnitude: f("shakemagnitude", 0.0),
            shake_duration: f("shakeduration", 0.0),
            min_range: f("minrange", 0.0).max(0.0),
            paralyze_time: f("paralyzetime", 0.0).max(0.0),
            paralyze_radius: f("paralyzeradius", 0.0).max(0.0),
            paralyze_on_fire: b("paralyzeonfire"),
            paralyze_exempt: st("paralyzeexempt").to_ascii_uppercase(),
        }
    }

    /// Whether its hits stun (TA Reborn's `paralyzetime`; not TA's `paralyzer`, whose damage is
    /// its stun).
    pub fn stuns(&self) -> bool {
        self.paralyze_time > 0.0 && !self.paralyzer
    }

    /// Damage dealt to a unit type (TA looks up the target's unit name).
    pub fn damage_against(&self, unit_name: &str) -> f64 {
        self.damage.get(unit_name).copied().unwrap_or(self.default_damage)
    }

    /// The classes of missiles it shoots down (0: none, not an interceptor).
    pub fn interceptor_classes(&self) -> u32 {
        classes(self.interceptor, self.interceptor_mask)
    }

    /// The classes of interceptors that may shoot it down (0: none, not targetable).
    pub fn targetable_classes(&self) -> u32 {
        classes(self.targetable, self.targetable_mask)
    }

    /// Whether this interceptor may shoot down `quarry`'s missiles: they share a class.
    pub fn intercepts(&self, quarry: &WeaponDef) -> bool {
        self.interceptor_classes() & quarry.targetable_classes() != 0
    }
}

/// The classes of a flag and its mask: none without the flag, the mask's with it (class 1 where a
/// definition built by hand gives the flag alone).
fn classes(flag: bool, mask: u32) -> u32 {
    match (flag, mask) {
        (false, _) => 0,
        (true, 0) => 1,
        (true, m) => m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    #[test]
    fn a_weapon_is_read_with_its_damage_table() {
        let doc = tdf::parse("[ArmLaser]{ name=Light Laser; rendertype=0; range=300; burst=-2; color=300; color2=-5; [DAMAGE]{ default = 75; ARMPW=10; corak=12.5; } }");
        let w = WeaponDef::from_tdf(&doc.sections[0]);
        assert_eq!((w.name.as_str(), w.display_name.as_str()), ("ARMLASER", "Light Laser"));
        assert_eq!((w.render_type, w.range, w.burst, w.color, w.color2), (RenderType::Laser, 300.0, 1, 255, 0), "at least one shot a burst; colours a palette's");
        assert_eq!((w.reload_time, w.area_of_effect), (1.0, 8.0), "TA's defaults");
        assert_eq!(w.damage_against("ARMPW"), 10.0);
        assert_eq!(w.damage_against("CORAK"), 12.5, "the table's names in capitals");
        assert_eq!(w.damage_against("ARMCOM"), 75.0, "the rest take the default");
        // TA's numbers: 1 a model, 7 lightning; others drawn as Reborn's fallback.
        for (n, kind) in [(1, RenderType::Model), (4, RenderType::Shell), (7, RenderType::Lightning), (9, RenderType::Other(9))] {
            assert_eq!(WeaponDef::from_tdf(&tdf::parse(&format!("[W]{{rendertype={n};}}")).sections[0]).render_type, kind);
        }
        // Without a damage table: none.
        assert_eq!(WeaponDef::from_tdf(&tdf::parse("[W]{range=1;}").sections[0]).damage_against("ARMPW"), 0.0);
    }

    /// TA's `interceptor` and `targetable` as classes: TA's own 1 and 1 match; a mod's anti-nuke
    /// (1) takes no tactical missile (2), its tactical defence (2) no nuke, one of both (3) either.
    #[test]
    fn interceptors_take_the_missiles_of_their_classes() {
        let read = |keys: &str| WeaponDef::from_tdf(&tdf::parse(&format!("[W] {{ {keys} }}")).sections[0]);
        let (nuke, anti) = (read("targetable=1;"), read("interceptor=1;"));
        assert!(anti.intercepts(&nuke) && anti.interceptor && nuke.targetable && !nuke.interceptor);
        let (tactical, defence, both) = (read("targetable=2;"), read("interceptor=2;"), read("interceptor=3;"));
        assert!(!anti.intercepts(&tactical) && !defence.intercepts(&nuke) && defence.intercepts(&tactical));
        assert!(both.intercepts(&nuke) && both.intercepts(&tactical));
        assert_eq!((tactical.targetable_mask, defence.interceptor_mask, read("").interceptor_mask), (2, 2, 0));
        assert!(!read("").intercepts(&nuke) && !anti.intercepts(&read("")), "neither is one");
        assert_eq!(read("interceptor=-1;").interceptor_classes(), u32::MAX, "a negative one, every class");
        // Built by hand with the flags alone: class 1, as TA's.
        let by_hand = WeaponDef { interceptor: true, ..Default::default() };
        assert!(by_hand.intercepts(&WeaponDef { targetable: true, ..Default::default() }));
    }

    /// TA Reborn's own keys: a minimum range and a stun beside the damage, its reach, when it goes
    /// off and whom it spares; none of them on TA's own weapons.
    #[test]
    fn minimum_ranges_and_stuns_are_read() {
        let read = |keys: &str| WeaponDef::from_tdf(&tdf::parse(&format!("[W] {{ {keys} }}")).sections[0]);
        let w = read("range=900; minrange=480; paralyzetime=3.5; paralyzeradius=704; paralyzeonfire=1; paralyzeexempt= Commander experimental ;");
        assert_eq!((w.min_range, w.paralyze_time, w.paralyze_radius), (480.0, 3.5, 704.0));
        assert!(w.paralyze_on_fire && w.stuns());
        assert_eq!(w.paralyze_exempt, "COMMANDER EXPERIMENTAL", "tokens in capitals");
        let plain = read("range=900;");
        assert_eq!((plain.min_range, plain.paralyze_time, plain.paralyze_radius), (0.0, 0.0, 0.0));
        assert!(!plain.stuns() && !plain.paralyze_on_fire && plain.paralyze_exempt.is_empty());
        assert_eq!(read("minrange=-5; paralyzetime=-1;").min_range, 0.0, "no negative reach");
        assert!(!read("paralyzer=1; paralyzetime=4;").stuns(), "TA's paralyzer stuns by its damage alone");
    }

    #[test]
    fn underwater_launch_permission_defaults_to_water_capability_but_can_be_overridden() {
        for (keys, expected) in [("", false), ("waterweapon=1;", true), ("waterweapon=1; firesubmersed=0;", false), ("firesubmersed=1;", true)] {
            let doc = tdf::parse(&format!("[W] {{ {keys} }}"));
            assert_eq!(WeaponDef::from_tdf(&doc.sections[0]).fire_submersed, expected, "{keys}");
        }
    }
}
