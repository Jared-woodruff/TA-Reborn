//! What each unit type is good for, derived once per game from its definition (never from
//! its name), so the AI plays mods without changes.

use ta_sim::world::MoveSpec;
use ta_sim::{Fx, Locomotion, Sim, Trajectory, UnitType, WeaponType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Extractor,
    Generator,
    MetalMaker,
    Factory,
    /// Static weapon against ground targets.
    Defense,
    /// Static weapon that can only engage aircraft.
    AntiAir,
    /// Strategic missile launcher (stockpiled, command-fired, interceptable).
    Silo,
    /// Anti-missile defence.
    AntiNuke,
    Storage,
    Builder,
    Combat,
    /// Mobile unit whose weapons only engage aircraft.
    AntiAirUnit,
    Other,
}

pub(crate) const ROLES: usize = Role::Other as usize + 1;

#[derive(Clone, Debug)]
pub(crate) struct TypeInfo {
    pub role: Role,
    /// 1 to 3 from LEVELn or LEVn category tags (higher mod tiers use the top AI tier).
    pub tech: u8,
    /// Factories: what they mostly build; everything else: how it moves.
    pub kind: Locomotion,
    /// Metal plus energy / 60, at least 1.
    pub cost: i64,
    /// What the unit counts for in a fight, as a cost: its cost, except for a Commander,
    /// whose price (some 30,000, as it is never built) says nothing of its guns: there from
    /// its firepower, toughness and reach, as other units' value per cost goes (about an
    /// advanced tank or two).
    pub worth: i64,
    /// Buildings: can stand on dry land.
    pub land: bool,
    pub commander: bool,
    /// Builders: can put up extractors, generators and factories (real constructors, not
    /// mine layers or assist-only units).
    pub builds_mex: bool,
    pub builds_energy: bool,
    pub builds_factory: bool,
    /// Damage per 10 seconds against ground and air targets, health, longest ground range.
    pub ground_dps: i64,
    pub air_dps: i64,
    pub hp: i64,
    pub range: i64,
    /// Combat value per unit of cost (higher is better); 0 when unarmed.
    pub value: i64,
    /// What a fixed budget of these is worth in a fight: firepower times toughness grows
    /// with the square of the number of units (Lanchester), so per cost^1.5 here; with a
    /// little extra for reach and less for units too slow to keep up with an attack.
    pub efficiency: i64,
    pub bomber: bool,
    /// Aircraft that fight like ground units (hover at range).
    pub gunship: bool,
    /// Planes whose weapons prefer air targets (TA's `BadTargetCategory=NOTAIR`).
    pub fighter: bool,
    pub spec: Option<MoveSpec>,
    /// Net energy per second, not counting wind or tides.
    pub energy: Fx,
    /// Metal per second made from energy, and the energy per second that costs.
    pub makes_metal: Fx,
    pub maker_upkeep: Fx,
    /// Anti-missile coverage radius.
    pub coverage: Option<Fx>,
    pub worker: Fx,
    pub speed: Fx,
    /// Transports: how many units fit aboard (0: not a transport).
    pub transport: u32,
}

/// Tech level from whole category tokens. TA uses LEVELn; Escalation uses LEVn.
/// Its fourth and fifth tiers remain advanced units in the AI's three-tier planning.
pub(crate) fn tech_level(t: &UnitType) -> u8 {
    t.def.category.split_whitespace().filter_map(|token| {
        let token = token.to_ascii_uppercase();
        token.strip_prefix("LEVEL").or_else(|| token.strip_prefix("LEV"))?.parse::<u8>().ok()
            .filter(|level| (1..=5).contains(level)).map(|level| level.min(3))
    }).max().unwrap_or(1)
}

/// A weapon that damages ground targets automatically (not a stockpiled missile, an
/// interceptor, a paralyser or a command-only gun like the D-gun).
pub(crate) fn hits_ground(w: &WeaponType) -> bool {
    w.hits_ground
        && !w.def.stockpile
        && !w.def.interceptor
        && !w.paralyzer
        && w.default_damage > Fx::ZERO
        && (!w.command_fire || w.trajectory == Trajectory::Dropped)
}

fn hits_air(w: &WeaponType) -> bool {
    w.hits_air && !w.def.stockpile && !w.def.interceptor && !w.paralyzer && w.default_damage > Fx::ZERO
}

/// Units whose bad targets name NOTAIR (`BadTargetCategory`, as the simulation reads it: TA's
/// anti-air hovercraft too): they take on aircraft alone of their own accord (missile towers,
/// anti-air kbots, trucks and hovercraft, flak, fighters).
fn prefers_air(sim: &Sim, t: &UnitType) -> bool {
    t.bad_target_mask.intersects(&sim.types.categories("NOTAIR"))
}

/// Damage per 10 seconds.
fn dps10(w: &WeaponType) -> i64 {
    w.default_damage.to_int().max(0) * w.burst.max(1) as i64 * 300 / w.reload_ticks.max(1) as i64
}

/// What a factory mostly produces: ground (kbots/vehicles), hover, naval or air units.
pub(crate) fn factory_kind(sim: &Sim, t: &UnitType) -> Locomotion {
    let mut counts = [0usize; 5];
    let index = |l: Locomotion| match l {
        Locomotion::Static => 0,
        Locomotion::Ground => 1,
        Locomotion::Hover => 2,
        Locomotion::Naval => 3,
        Locomotion::Air => 4,
    };
    for &o in &t.build_options {
        counts[index(sim.types.get(o).locomotion)] += 1;
    }
    let best = (0..5).max_by_key(|&i| (counts[i], std::cmp::Reverse(i))).unwrap_or(1);
    [Locomotion::Static, Locomotion::Ground, Locomotion::Hover, Locomotion::Naval, Locomotion::Air][best]
}

/// The role from the type's own properties.
fn basic_role(sim: &Sim, t: &UnitType) -> Role {
    let weapons: Vec<&WeaponType> = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).collect();
    if t.locomotion == Locomotion::Static {
        if sim.types.makes_units(t) {
            // Some conversions give factories their own resource income. Production
            // remains their primary job; treating them as generators prevents armies.
            Role::Factory
        } else if t.extracts_metal > Fx::ZERO {
            Role::Extractor
        } else if t.makes_metal > Fx::ZERO {
            Role::MetalMaker
        } else if t.energy_make - t.energy_use > Fx::ZERO && t.radar.is_zero() || t.wind_generator > Fx::ZERO || t.tidal_generator > Fx::ZERO {
            // Net producers only: radar towers make and use the same amount.
            Role::Generator
        } else if t.is_factory {
            if sim.types.makes_units(t) { Role::Factory } else { Role::Other }
        } else if weapons.iter().any(|w| w.def.stockpile && w.def.command_fire && w.def.targetable) {
            Role::Silo
        } else if weapons.iter().any(|w| w.def.interceptor) {
            Role::AntiNuke
        } else if weapons.iter().any(|w| hits_air(w)) && (prefers_air(sim, t) || !weapons.iter().any(|w| hits_ground(w))) {
            Role::AntiAir
        } else if weapons.iter().any(|w| hits_ground(w)) {
            Role::Defense
        } else if t.energy_storage > Fx::ZERO || t.metal_storage > Fx::ZERO {
            Role::Storage
        } else {
            Role::Other
        }
    } else if t.is_builder {
        Role::Builder
    } else if t.def.is_commander || t.def.kamikaze {
        Role::Other
    } else if weapons.iter().any(|w| hits_air(w)) && (prefers_air(sim, t) || !weapons.iter().any(|w| hits_ground(w))) {
        Role::AntiAirUnit
    } else if weapons.iter().any(|w| hits_ground(w)) {
        Role::Combat
    } else {
        Role::Other
    }
}

pub(crate) fn classify(sim: &Sim) -> Vec<TypeInfo> {
    let roles: Vec<Role> = sim.types.types.iter().map(|t| basic_role(sim, t)).collect();
    sim.types
        .types
        .iter()
        .map(|t| {
            let role = roles[t.id];
            let weapons: Vec<&WeaponType> = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).collect();
            let ground_dps: i64 = weapons.iter().filter(|w| hits_ground(w)).map(|w| dps10(w)).sum();
            let air_dps: i64 = weapons.iter().filter(|w| hits_air(w)).map(|w| dps10(w)).sum();
            let range = weapons.iter().filter(|w| hits_ground(w)).map(|w| w.range.to_int()).max().unwrap_or(0);
            let hp = t.max_health.to_int().max(1);
            let cost = (t.cost_metal + t.cost_energy / 60).to_int().max(1);
            // Combat value per cost: firepower times toughness, a little extra for reach.
            let value = if ground_dps > 0 { ground_dps * hp / cost * (300 + range.min(1200)) / 300 } else { 0 };
            let (brisk, speed) = (Fx::from_ratio(12, 10).0 as i128, t.max_speed.0 as i128);
            let efficiency = if ground_dps > 0 {
                // (In i128: mods can have outlandish numbers.)
                let e = ground_dps as i128 * hp as i128 * 100 / (cost as i128 * cost.isqrt().max(1) as i128) * (300 + range.min(1200)) as i128 / 300 * speed.clamp(1, brisk) / brisk;
                e.min(i64::MAX as i128) as i64
            } else {
                0
            };
            let bomber = t.locomotion == Locomotion::Air && weapons.iter().any(|w| w.trajectory == Trajectory::Dropped && hits_ground(w));
            let options = |r: Role| t.build_options.iter().any(|&o| roles[o] == r && sim.types.get(o).def.min_water_depth <= 0.0);
            let kind = if role == Role::Factory { factory_kind(sim, t) } else { t.locomotion };
            let spec = matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval).then(|| MoveSpec::for_type(t, &sim.data.move_classes));
            let coverage = weapons.iter().map(|w| w.interceptor_coverage()).max().filter(|&r| r > Fx::ZERO);
            let worth = if t.def.is_commander && ground_dps > 0 {
                // (Ordinary units are worth about 3,000 of `value` per point of cost.)
                (ground_dps as i128 * hp as i128 * (300 + range.min(1200)) as i128 / 300 / 3000).max(100).min(cost as i128) as i64
            } else {
                cost
            };
            TypeInfo {
                role,
                tech: tech_level(t),
                kind,
                cost,
                worth,
                land: t.def.min_water_depth <= 0.0,
                commander: t.def.is_commander,
                builds_mex: t.is_builder && options(Role::Extractor),
                builds_energy: t.is_builder && options(Role::Generator),
                builds_factory: t.is_builder && options(Role::Factory),
                ground_dps,
                air_dps,
                hp,
                range,
                value,
                efficiency,
                bomber,
                gunship: t.locomotion == Locomotion::Air && t.hover_attack && ground_dps > 0,
                fighter: t.is_plane() && !bomber && prefers_air(sim, t),
                spec,
                energy: (t.energy_make - t.energy_use) * 30,
                makes_metal: if role == Role::MetalMaker { t.makes_metal * 30 } else { Fx::ZERO },
                maker_upkeep: if role == Role::MetalMaker { t.energy_use * 30 } else { Fx::ZERO },
                coverage,
                worker: t.worker_rate * 30,
                speed: t.max_speed,
                transport: if t.locomotion != Locomotion::Static && !t.is_builder { t.transport_capacity } else { 0 },
            }
        })
        .collect()
}
