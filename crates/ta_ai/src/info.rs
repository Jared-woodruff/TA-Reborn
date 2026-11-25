//! What each unit type is good for, derived once per game from its definition (never from
//! its name), so the AI plays mods without changes.

use ta_sim::world::MoveSpec;
use ta_sim::{Fx, FxVec3, Locomotion, Sim, Trajectory, UnitType, WeaponType};

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
    /// Missile launcher (stockpiled, command-fired, interceptable): TA's strategic silos, a mod's
    /// tactical launchers too (`TypeInfo::silo_classes`: [`TypeInfo::strategic_silo`],
    /// [`TypeInfo::tactical_silo`]).
    Silo,
    /// Anti-missile defence against what strategic silos fire (an interceptor of the strategic
    /// class: `ta_sim::STRATEGIC_CLASS`).
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
    /// Damage per 10 seconds against ground and air targets, health, longest ground range and
    /// longest reach at aircraft.
    pub ground_dps: i64,
    pub air_dps: i64,
    pub hp: i64,
    pub range: i64,
    pub air_range: i64,
    /// The nearest its ground weapons fire (their `minrange`: the least of them, none where one
    /// fires as near as it likes): a tower with one guards none of the ground within it (Supreme
    /// Commander's artillery and missile launchers; TA's own have none).
    pub min_range: i64,
    /// Its shield (TA Reborn's, `ShieldHealth`): its hit points (counted in `hp` too, for a personal
    /// one), a bubble's radius across (px; none for a personal shield or none at all) and its
    /// upkeep (energy a second). None of TA's.
    pub shield: i64,
    pub dome: i64,
    pub shield_upkeep: i64,
    /// A combat unit on the ground (or hovering) that lobs its shells or missiles over the field
    /// from afar: a ground weapon, ballistic or guided, reaching [`ARTILLERY_RANGE`] or more (TA's
    /// Luger and Merl, the Supreme Commander factions' mobile artillery and missile launchers).
    /// Rated by its reach, it would make up whole armies: `pick_combat` holds it to a share.
    pub artillery: bool,
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
    /// Anti-missile coverage radius, the classes of missiles it takes there
    /// (`WeaponDef::interceptor_classes`), and whether it takes them from a stock (else at its
    /// reload, as many as come: a tactical defence).
    pub coverage: Option<Fx>,
    pub cover_classes: u32,
    pub cover_unstocked: bool,
    /// A silo's missiles' classes (`WeaponDef::targetable_classes`; 0 for any other unit), how
    /// far they reach, and what one costs (metal, energy: TA's nuke 2,000 and 180,000, Forged
    /// Alliance's 12,000 and 1,350,000).
    pub silo_classes: u32,
    pub missile_range: i64,
    pub missile_cost: (i64, i64),
    /// Energy per second its stockpiled missiles take while it builds them as fast as it can
    /// (a silo's, an anti-missile defence's: TA's nuke 1,000, Forged Alliance's 4,500).
    pub stock_energy: i64,
    pub worker: Fx,
    pub speed: Fx,
    /// Transports: how many units fit aboard (0: not a transport).
    pub transport: u32,
    /// An experimental: an armed mobile unit (not a Commander) costing [`EXPERIMENTAL_COST`] or
    /// more (TA's Krogoth, Forged Alliance's Monkeylord, and its Fatboy, a factory on tracks,
    /// which fights with the army); and a factory whose every combat unit is one (TA's Gantry).
    pub experimental: bool,
    pub experimental_factory: bool,
    /// A factory that moves (`MobileFactory`: Forged Alliance's carriers, Atlantis, Tempest, CZAR
    /// and Fatboy): it fights with the army, making units as it goes (`mobile.rs`).
    pub mobile_factory: bool,
    /// A building whose guns reach [`LONG_RANGE`] or further: a long-range gun (Big Bertha, the
    /// Intimidator, the Vulcan), fired at what we know of the enemy's base (`projects.rs`).
    pub long_range: bool,
    /// Energy per second its guns take firing as fast as they can (TA's Annihilator draws a
    /// thousand a shot): stockpiled missiles not counted.
    pub fire_energy: i64,
    /// The way to experimentals in the tree (`mark_experimental_tree`): a constructor that builds
    /// one (or a factory of them), a factory that turns out such a constructor, and a constructor
    /// that puts up such an (advanced) factory.
    pub builds_experimentals: bool,
    pub leads_to_experimentals: bool,
    pub opens_way_to_experimentals: bool,
    /// A constructor whose options hold a strategic silo (`strategic_silo`): Forged Alliance's
    /// tech 3 engineers and support commanders, TA's advanced constructors.
    pub builds_strategic: bool,
}

impl TypeInfo {
    /// A launcher of strategic missiles (the class anti-nuclear defences take: TA's every silo, its
    /// nukes and the EMP and tactical missiles it fires, Forged Alliance's strategic launchers).
    pub fn strategic_silo(&self) -> bool {
        self.role == Role::Silo && self.silo_classes & ta_sim::STRATEGIC_CLASS != 0
    }

    /// A launcher of missiles of other classes alone (Forged Alliance's tactical launchers, which
    /// its tactical missile defences take): strikes on what stands within its reach.
    pub fn tactical_silo(&self) -> bool {
        self.role == Role::Silo && self.silo_classes & ta_sim::STRATEGIC_CLASS == 0
    }

    /// Whether a tower of this type at `tower` guards `pos`: within its guns' reach (and a little
    /// more), not nearer it than they fire (its [`TypeInfo::min_range`]).
    pub fn guards(&self, tower: FxVec3, pos: FxVec3) -> bool {
        tower.within_xz(pos, Fx::from_int(self.range.max(200) + 96)) && (self.min_range <= 0 || !tower.within_xz(pos, Fx::from_int(self.min_range)))
    }

    /// A building that shoots down missiles of classes other than the strategic one, and none of
    /// that (Forged Alliance's tactical missile defences): cover against tactical launchers.
    pub fn tactical_cover(&self) -> bool {
        self.kind == Locomotion::Static && self.coverage.is_some() && self.cover_classes & !ta_sim::STRATEGIC_CLASS != 0 && self.cover_classes & ta_sim::STRATEGIC_CLASS == 0
    }
}

/// What a mobile unit costs (metal and energy / 60) to count as an experimental.
pub(crate) const EXPERIMENTAL_COST: i64 = 5000;
/// How far (px) a building's guns reach to count as a long-range gun.
pub(crate) const LONG_RANGE: i64 = 2000;
/// How far (px) a mobile unit's lobbed shells or missiles reach for it to count as artillery
/// (as the soak's tech audit counts it).
pub(crate) const ARTILLERY_RANGE: i64 = 600;

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
    for &o in &t.ai_build_options {
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
        } else if weapons.iter().any(|w| w.def.interceptor_classes() & ta_sim::STRATEGIC_CLASS != 0) {
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
            let air_range = weapons.iter().filter(|w| hits_air(w)).map(|w| w.range.to_int()).max().unwrap_or(0);
            let min_range = weapons.iter().filter(|w| hits_ground(w)).map(|w| w.min_range.to_int().max(0)).min().unwrap_or(0);
            let lobs = weapons.iter().any(|w| hits_ground(w) && matches!(w.trajectory, Trajectory::Ballistic | Trajectory::Guided) && w.range.to_int() >= ARTILLERY_RANGE);
            // (A personal shield takes the damage first: as much more to wear down.)
            let shield = t.abilities.shield.filter(|s| !s.bubble()).map_or(0, |s| s.health.to_int().max(0));
            let hp = t.max_health.to_int().saturating_add(shield).max(1);
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
            let options = |r: Role| t.ai_build_options.iter().any(|&o| roles[o] == r && sim.types.get(o).def.min_water_depth <= 0.0);
            let kind = if role == Role::Factory { factory_kind(sim, t) } else { t.locomotion };
            let spec = matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval).then(|| MoveSpec::for_type(t, &sim.data.move_classes));
            let coverage = weapons.iter().map(|w| w.interceptor_coverage()).max().filter(|&r| r > Fx::ZERO);
            let cover_classes = weapons.iter().fold(0, |c, w| c | w.def.interceptor_classes());
            let cover_unstocked = weapons.iter().any(|w| w.def.interceptor && !w.def.stockpile);
            let silo_classes = if role == Role::Silo {
                weapons.iter().filter(|w| w.def.stockpile && w.def.command_fire).fold(0, |c, w| c | w.def.targetable_classes())
            } else {
                0
            };
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
                air_range,
                min_range,
                shield: t.abilities.shield.map_or(0, |s| s.health.to_int().max(0)),
                dome: t.abilities.shield.filter(|s| s.bubble()).map_or(0, |s| s.radius.to_int().max(0)),
                shield_upkeep: t.abilities.shield.map_or(0, |s| (s.energy * 30).to_int().max(0)),
                artillery: role == Role::Combat && matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover) && lobs,
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
                cover_classes,
                cover_unstocked,
                silo_classes,
                missile_range: weapons.iter().filter(|w| w.def.stockpile && w.def.command_fire).map(|w| w.range.to_int()).max().unwrap_or(0),
                missile_cost: weapons.iter().find(|w| w.def.stockpile && w.def.command_fire).map_or((0, 0), |w| (w.metal_per_shot.to_int().max(0), w.energy_per_shot.to_int().max(0))),
                stock_energy: weapons.iter().filter(|w| w.def.stockpile).map(|w| w.energy_per_shot.to_int().max(0) * 30 / w.reload_ticks.max(1) as i64).sum(),
                worker: t.worker_rate * 30,
                speed: t.max_speed,
                transport: if t.locomotion != Locomotion::Static && !t.is_builder { t.transport_capacity } else { 0 },
                // (Not a constructor of buildings, however it is armed and whatever it costs: the
                // factions' support commanders are constructors.)
                experimental: experimental(t, ground_dps + air_dps, cost) && !builds_buildings(sim, t),
                builds_experimentals: false,
                leads_to_experimentals: false,
                opens_way_to_experimentals: false,
                builds_strategic: false,
                fire_energy: weapons.iter().filter(|w| !w.def.stockpile).map(|w| w.energy_per_shot.to_int().max(0) * w.burst.max(1) as i64 * 30 / w.reload_ticks.max(1) as i64).sum(),
                experimental_factory: false,
                mobile_factory: t.mobile_factory,
                long_range: role == Role::Defense && range >= LONG_RANGE,
            }
        })
        .collect::<Vec<TypeInfo>>()
        .into_iter()
        .enumerate()
        .map(|(k, mut i)| {
            // (A factory every combat unit of which is an experimental.)
            if i.role == Role::Factory {
                let t = sim.types.get(k);
                let combat = |o: &&usize| matches!(roles[**o], Role::Combat | Role::AntiAirUnit);
                i.experimental_factory = t.ai_build_options.iter().any(|o| combat(&o)) && t.ai_build_options.iter().filter(combat).all(|&o| {
                    let u = sim.types.get(o);
                    u.weapons.iter().flatten().next().is_some() && experimental(u, 1, (u.cost_metal + u.cost_energy / 60).to_int())
                });
            }
            i
        })
        .collect()
}

/// Marks the way to experimentals in the tree (once a game: the tree never changes): the
/// constructors that build them (or a factory of them), the factories that turn those out, and
/// the constructors that put those (advanced) factories up.
pub(crate) fn mark_experimental_tree(sim: &Sim, info: &mut [TypeInfo]) {
    let n = info.len();
    let opts = |t: usize| &sim.types.get(t).ai_build_options;
    let constructor = |i: &TypeInfo| i.role == Role::Builder && (i.builds_mex || i.builds_energy) && !i.experimental;
    let builds: Vec<bool> = (0..n).map(|c| constructor(&info[c]) && opts(c).iter().any(|&x| info[x].experimental || info[x].experimental_factory)).collect();
    let leads: Vec<bool> = (0..n).map(|f| info[f].role == Role::Factory && opts(f).iter().any(|&c| builds[c])).collect();
    let advanced = |f: usize| info[f].role == Role::Factory && opts(f).iter().any(|&o| info[o].role == Role::Builder && info[o].tech >= 2 && (info[o].builds_energy || info[o].builds_mex));
    let opens: Vec<bool> = (0..n).map(|c| constructor(&info[c]) && opts(c).iter().any(|&f| advanced(f) && leads[f])).collect();
    for (t, i) in info.iter_mut().enumerate() {
        i.builds_experimentals = builds[t];
        i.leads_to_experimentals = leads[t];
        i.opens_way_to_experimentals = opens[t];
    }
}

/// Constructors of the first tier by their tags (or of none) that put up every building a
/// constructor of the third tier puts up count as of that tier (the Supreme Commander factions'
/// support commanders carry no tier, and build all a tech 3 engineer does); and which
/// constructors put up a strategic silo. Once a game, before [`mark_experimental_tree`] (an
/// advanced factory is one that turns out an advanced constructor). TA's constructors go no
/// higher than the second tier: none of them changes.
pub(crate) fn mark_constructors(sim: &Sim, info: &mut [TypeInfo]) {
    let opts = |t: usize| &sim.types.get(t).ai_build_options;
    let constructor = |i: &TypeInfo| i.role == Role::Builder && (i.builds_mex || i.builds_energy) && !i.commander && !i.experimental;
    let top: Vec<usize> = (0..info.len()).filter(|&c| constructor(&info[c]) && info[c].tech >= 3 && !opts(c).is_empty()).collect();
    let promoted: Vec<usize> = (0..info.len())
        .filter(|&c| constructor(&info[c]) && info[c].tech == 1 && top.iter().any(|&t| opts(t).iter().all(|o| opts(c).contains(o))))
        .collect();
    for c in promoted {
        info[c].tech = 3;
    }
    let strategic: Vec<bool> = info.iter().map(TypeInfo::strategic_silo).collect();
    for (c, i) in info.iter_mut().enumerate() {
        i.builds_strategic = i.role == Role::Builder && opts(c).iter().any(|&o| strategic[o]);
    }
}

/// Whether a unit type is an experimental (see [`TypeInfo::experimental`]), from its damage (any
/// of it) and cost.
fn experimental(t: &UnitType, dps: i64, cost: i64) -> bool {
    t.locomotion != Locomotion::Static && !t.def.is_commander && dps > 0 && cost >= EXPERIMENTAL_COST
}

/// A builder that puts up buildings (a constructor: the Supreme Commander factions' support
/// commanders among them), not one that only turns out units (Forged Alliance's Fatboy, a factory
/// on tracks, which is an experimental all the same).
fn builds_buildings(sim: &Sim, t: &UnitType) -> bool {
    t.is_builder && t.ai_build_options.iter().any(|&o| sim.types.get(o).locomotion == Locomotion::Static)
}
