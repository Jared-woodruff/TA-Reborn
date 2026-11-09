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
    pub interceptor: bool,
    pub targetable: bool,
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
            interceptor: b("interceptor"),
            targetable: b("targetable"),
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
        }
    }

    /// Damage dealt to a unit type (TA looks up the target's unit name).
    pub fn damage_against(&self, unit_name: &str) -> f64 {
        self.damage.get(unit_name).copied().unwrap_or(self.default_damage)
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

    #[test]
    fn underwater_launch_permission_defaults_to_water_capability_but_can_be_overridden() {
        for (keys, expected) in [("", false), ("waterweapon=1;", true), ("waterweapon=1; firesubmersed=0;", false), ("firesubmersed=1;", true)] {
            let doc = tdf::parse(&format!("[W] {{ {keys} }}"));
            assert_eq!(WeaponDef::from_tdf(&doc.sections[0]).fire_submersed, expected, "{keys}");
        }
    }
}
