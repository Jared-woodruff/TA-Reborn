//! Simulation-ready unit types, compiled once from [`ta_data::GameData`] into fixed-point
//! values in per-tick units, together with each type's piece hierarchy and COB program.

use std::sync::Arc;

use rustc_hash::FxHashMap;
use ta_data::{GameData, UnitDef};
use ta_formats::cob::CobScript;
use ta_formats::model3do::Model3do;

use crate::cob::{CobProgram, TICKS_PER_SECOND};
use crate::fixed::{Fx, FxVec3};

/// A model piece: parent link and rest offset in world orientation (3DO z flipped).
#[derive(Clone, Debug)]
pub struct PieceDef {
    pub name: String,
    pub parent: Option<usize>,
    pub offset: FxVec3,
    /// Highest local vertex, for the surface of an authored landing pad.
    pub top: Fx,
}

#[derive(Clone, Debug, Default)]
pub struct ModelDef {
    pub pieces: Vec<PieceDef>,
    /// Highest point of the rest pose above the model origin.
    pub height: Fx,
    /// How far the rest pose reaches from the model origin across the ground (its barrels and
    /// all): the room a unit takes in a formation.
    pub reach: Fx,
}

impl ModelDef {
    pub fn from_3do(m: &Model3do) -> ModelDef {
        let pieces: Vec<PieceDef> = m
            .pieces
            .iter()
            .map(|p| PieceDef {
                name: p.name.clone(),
                parent: p.parent,
                offset: FxVec3::new(Fx(p.offset[0] as i64), Fx(p.offset[1] as i64), Fx(-(p.offset[2] as i64))),
                top: Fx(p.vertices.iter().map(|v| v[1] as i64).max().unwrap_or(0)),
            })
            .collect();
        let mut rest: Vec<(i64, i64, i64)> = Vec::with_capacity(pieces.len());
        let mut height = 0i64;
        let mut reach = 0i128;
        for (i, p) in pieces.iter().enumerate() {
            let base = p.parent.map_or((0, 0, 0), |q| rest[q]);
            let at = (base.0 + p.offset.x.0, base.1 + p.offset.y.0, base.2 + p.offset.z.0);
            rest.push(at);
            for v in &m.pieces[i].vertices {
                height = height.max(at.1 + v[1] as i64);
                let (x, z) = ((at.0 + v[0] as i64) as i128, (at.2 - v[2] as i64) as i128);
                reach = reach.max(x * x + z * z);
            }
        }
        ModelDef { pieces, height: Fx(height), reach: Fx(crate::fixed::isqrt_u128(reach as u128) as i64) }
    }

    pub fn piece_index(&self, name: &str) -> Option<usize> {
        self.pieces.iter().position(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// How a unit type moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locomotion {
    /// Buildings and other units that never move.
    Static,
    /// Walkers and tracked/wheeled vehicles (may go underwater up to their max depth).
    Ground,
    /// Hovercraft: land and water surface.
    Hover,
    /// Ships and submarines: water only.
    Naval,
    Air,
}

/// Which of TA's engine calls a unit's script answers (looked up once per type).
#[derive(Clone, Copy, Debug, Default)]
pub struct Callins {
    /// Extractors (the arm spins with the extraction rate) and wind generators.
    pub set_speed: bool,
    /// Wind generators: they turn to face the wind.
    pub set_direction: bool,
    /// Told the longest weapon reload once, after `Create` (turrets time their rest by it).
    pub set_max_reload_time: bool,
    /// Aircraft: `MoveRate0`..`MoveRate3` as their speed changes (wings, jets, rolls).
    pub move_rate: [bool; 4],
    /// Rocked by the recoil of their own shots (tanks, floating turrets).
    pub rock_unit: bool,
    /// Told what they move over (land, shallows, deep water): amphibians swim, hovercraft
    /// and seaplanes throw wakes.
    pub set_sfx_occupy: bool,
    pub target_cleared: bool,
    /// Builders: the piece the nano spray leaves from (factories alternate between two).
    pub query_nano_piece: bool,
}

impl Callins {
    fn of(script: Option<&CobProgram>) -> Callins {
        let has = |name: &str| script.is_some_and(|s| s.function(name).is_some());
        Callins {
            set_speed: has("SetSpeed"),
            set_direction: has("SetDirection"),
            set_max_reload_time: has("SetMaxReloadTime"),
            move_rate: std::array::from_fn(|i| has(&format!("MoveRate{i}"))),
            rock_unit: has("RockUnit"),
            set_sfx_occupy: has("SetSFXoccupy"),
            target_cleared: has("TargetCleared"),
            query_nano_piece: has("QueryNanoPiece"),
        }
    }
}

/// A unit's target tags (`BadTargetCategory`, its weapons' and `NoChaseCategory`) as TA's files
/// give them, but for one slip: the Battle Tactics anti-air hovercraft, the Arm's Swatter (ARMAH)
/// and the Core's Slinger (CORAH), carry the ground hovercraft's, VTOL, so that by TA's rule
/// they would never take on an aircraft of their own accord and would turn their anti-air
/// missiles on tanks. They are read as every other anti-air unit's, NOTAIR. (A mod that has put
/// them right keeps its own.)
fn target_tags<'a>(u: &UnitDef, tags: &'a str) -> &'a str {
    let anti_air_hovercraft = u.name.eq_ignore_ascii_case("ARMAH") || u.name.eq_ignore_ascii_case("CORAH");
    if anti_air_hovercraft && tags.trim().eq_ignore_ascii_case("VTOL") { "NOTAIR" } else { tags }
}

/// A set of `Category`-style tokens (`Category`, `BadTargetCategory`): a bit for each distinct
/// token of the game data's `Category` lines, numbered as they first appear, up to
/// [`CategorySet::CAPACITY`] of them (stock TA uses 54; mods add their own). Fixed size and
/// `Copy`, so testing a target against a unit's list costs a few instructions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CategorySet([u64; CategorySet::WORDS]);

impl CategorySet {
    const WORDS: usize = 4;
    /// Distinct tokens a game's data can use.
    pub const CAPACITY: usize = CategorySet::WORDS * 64;
    pub const EMPTY: CategorySet = CategorySet([0; CategorySet::WORDS]);

    /// Adds token number `bit`; false (and nothing added) past the capacity.
    pub fn insert(&mut self, bit: usize) -> bool {
        let Some(word) = self.0.get_mut(bit / 64) else { return false };
        *word |= 1 << (bit % 64);
        true
    }

    pub fn contains(&self, bit: usize) -> bool {
        self.0.get(bit / 64).is_some_and(|w| w & 1 << (bit % 64) != 0)
    }

    /// Whether the two sets share a token.
    #[inline]
    pub fn intersects(&self, other: &CategorySet) -> bool {
        self.0.iter().zip(&other.0).fold(0, |m, (a, b)| m | a & b) != 0
    }

    pub fn is_empty(&self) -> bool {
        *self == CategorySet::EMPTY
    }
}

#[derive(Clone, Debug)]
pub struct UnitType {
    pub id: usize,
    pub name: String,
    pub def: UnitDef,
    pub model: Arc<ModelDef>,
    pub script: Option<Arc<CobProgram>>,
    pub callins: Callins,
    /// Model piece index → COB piece index (for animation lookups).
    pub model_to_cob: Vec<Option<usize>>,
    pub locomotion: Locomotion,
    pub move_class: Option<usize>,
    pub footprint: (u32, u32),
    /// Collision radius in pixels (half the footprint diagonal-ish).
    pub radius: Fx,
    pub max_health: Fx,
    /// Pixels per tick.
    pub max_speed: Fx,
    /// Pixels per tick².
    pub acceleration: Fx,
    pub brake: Fx,
    /// Angle units per tick.
    pub turn_rate: i32,
    pub cruise_alt: Fx,
    /// Naval model depth below its surface movement anchor (WaterLine).
    pub waterline: Fx,
    pub build_time: Fx,
    /// The work it takes to build one: its build time in worker-time ticks (`BuildTime` × 30,
    /// raw 16.16), which builders do [`UnitType::work_rate`] of a tick. Whole numbers, so
    /// progress is exact for any build time.
    pub build_work: u64,
    pub cost_metal: Fx,
    pub cost_energy: Fx,
    /// Build points added per tick.
    pub worker_rate: Fx,
    /// Work done on a build (repair, reclaim, resurrection, capture) a tick: `WorkerTime` (raw
    /// 16.16), as [`UnitType::build_work`] counts it.
    pub work_rate: u64,
    pub build_distance: Fx,
    pub sight: Fx,
    pub radar: Fx,
    pub weapons: [Option<usize>; 3],
    /// It builds missiles to launch (a `stockpile` weapon: nuclear silos, anti-missile
    /// defences).
    pub stockpile: bool,
    // Economy, all per tick.
    pub energy_make: Fx,
    pub metal_make: Fx,
    pub energy_use: Fx,
    pub extracts_metal: Fx,
    pub makes_metal: Fx,
    pub energy_storage: Fx,
    pub metal_storage: Fx,
    pub wind_generator: Fx,
    pub tidal_generator: Fx,
    pub build_options: Vec<usize>,
    /// Per footprint cell, whether the building blocks movement with its yard closed.
    pub blocking_cells: Vec<bool>,
    /// Blocking cells while the script has opened its yard (c/i cells swap).
    pub open_yard_cells: Vec<bool>,
    pub is_builder: bool,
    pub is_factory: bool,
    /// Must be built over a geothermal vent (G cells in the yard map).
    pub needs_geo: bool,
    /// The unit's `Category` tokens and its `BadTargetCategory` tokens, as sets over every
    /// token in the game data.
    pub category_mask: CategorySet,
    pub bad_target_mask: CategorySet,
    /// What each weapon never picks as a target on its own (TA's bad targets: an attack order
    /// still names them): its `wpri_`, `wsec_` or `wspe_badTargetCategory`, else the unit's
    /// `BadTargetCategory`. TA's anti-air units list NOTAIR (fighters never strafe, AA K-bots
    /// never shoot tanks), its other armed units VTOL, the cruisers' depth charges NOTSUB. (Empty
    /// for a weapon whose bad targets are every unit a player can field that it could hit:
    /// `lift_bad_targets_of_everything`.)
    pub weapon_bad_masks: [CategorySet; 3],
    /// What it never goes after (`NoChaseCategory`: TA's ground units list VTOL).
    pub no_chase_mask: CategorySet,
    /// Gunships: aircraft that hover at range while attacking.
    pub hover_attack: bool,
    /// Transports: how many units fit (air transports carry one) and the largest
    /// footprint they take.
    pub transport_capacity: u32,
    pub transport_size: u32,
    /// A big walker (see [`big_walker`]): it stands level whatever the ground under it, as
    /// TA's `Upright=1` units do ([`UnitType::level`]).
    pub big_walker: bool,
}

/// Walkers at least this tall (px: the highest point of their model at rest) are big walkers.
/// TA's own walkers that the ground tilts (those without `Upright=1`) are 32 px tall at most
/// (the Eraser; the Sumo 30, the Fido 28), TA's Krogoth 68 (upright); the mods' giants that the
/// ground tilted are 63 to 136 px tall (TA: Enhanced's SpiderBot 136, XBot Fury 101, Spiderbot
/// Pod 92, Behemoth Rhino 75, Mech Drake 72, Cavor Aristocrate 63; TA: Escalation's Juggernaut and
/// Total Mayhem's Wolf 70).
pub const BIG_WALKER_HEIGHT: i64 = 60;

/// The farthest round a landing aircraft the ground is looked at (px; TA's reach 46 at most).
pub const MAX_LANDING_REACH: i64 = 96;

/// Whether a ground unit is a big walker: one that walks (a kbot or a Commander by its
/// `TEDClass`, a kbot by its `Category`, or a spider, whose movement class climbs any slope:
/// TA's spiders are `TEDClass=TANK` on `SPID3`) and stands at least [`BIG_WALKER_HEIGHT`] tall.
/// Tilted with the slope under its footprint, as TA tilts what is not `Upright=1`, such a unit
/// rocked with every bump: a footprint is a cell or a few (TA: Enhanced's units name none, and
/// take a cell), the SpiderBot's legs reach 121 px out to each side, and its top swung tens of
/// pixels as bumps under its belly passed (up to 45 degrees on Cinder Basin). Big walkers stand
/// level, as TA's Krogoth and the mods' upright giants do; the client keeps their feet on the
/// ground (`apps/ta_game/src/scene/footing.rs`).
pub fn big_walker(u: &UnitDef, model: &ModelDef, class: Option<&ta_data::MoveClass>, locomotion: Locomotion) -> bool {
    let walks = matches!(u.tedclass.as_str(), "KBOT" | "COMMANDER") || u.has_category("KBOT") || class.is_some_and(|c| c.max_slope >= 255.0);
    locomotion == Locomotion::Ground && walks && model.height >= Fx::from_int(BIG_WALKER_HEIGHT)
}

impl UnitType {
    pub(crate) fn collision_radius(&self) -> Fx {
        self.radius.max(self.model.height.max(Fx::from_int(8)) / 2).max(Fx::from_int(8)) * 3 / 4
    }

    /// Whether it stands level whatever the ground under it: TA's `Upright=1` units (most
    /// kbots) and big walkers ([`big_walker`]). The rest tilt with the slope under their
    /// footprint (`Sim::settle`).
    pub fn level(&self) -> bool {
        self.def.upright || self.big_walker
    }

    /// Whether this transport can carry a unit of type `other`.
    pub fn can_carry(&self, other: &UnitType) -> bool {
        self.transport_capacity > 0
            && matches!(other.locomotion, Locomotion::Ground | Locomotion::Hover)
            && (self.transport_size == 0 || other.footprint.0.max(other.footprint.1) <= self.transport_size.max(1).saturating_mul(2))
    }

    /// Fixed-wing combat aircraft: they attack in passes and can't hover over a target.
    pub fn is_plane(&self) -> bool {
        self.locomotion == Locomotion::Air && !self.hover_attack && self.weapons.iter().any(|w| w.is_some())
    }

    /// Where it may set down, as an aircraft (`World::air_landing_height`): its footprint,
    /// its model's reach (at most [`MAX_LANDING_REACH`]: a mod's giant asks no more ground than
    /// that), its `MaxSlope`, and for a seaplane (`Amphibious`) its `MaxWaterDepth`.
    pub fn landing(&self) -> crate::world::Landing {
        crate::world::Landing {
            footprint: self.footprint,
            reach: self.model.reach.to_int().clamp(0, MAX_LANDING_REACH),
            max_slope: self.def.max_slope.clamp(0.0, 255.0) as i64,
            water: self.def.amphibious.then(|| self.def.max_water_depth.clamp(0.0, 255.0) as i64),
        }
    }
}

fn per_tick(v: f64) -> Fx {
    Fx::from_f64(v / TICKS_PER_SECOND as f64)
}

/// All unit types, indexed like `GameData::units`.
pub struct UnitTypes {
    pub types: Vec<UnitType>,
    pub(crate) airbase_reach: Fx,
    pub(crate) collision_reach: Fx,
    pub(crate) target_reach: Fx,
    by_name: FxHashMap<String, usize>,
    /// Upper-case `Category` token → its number in a [`CategorySet`].
    categories: FxHashMap<String, usize>,
}

impl UnitTypes {
    /// Compiles every unit type. `load` reads a VFS path (models and scripts).
    pub fn compile(data: &GameData, load: &dyn Fn(&str) -> Option<Vec<u8>>) -> UnitTypes {
        // Category tokens get numbers in the order they first appear.
        let mut categories: FxHashMap<String, usize> = FxHashMap::default();
        for u in &data.units {
            for tok in u.category.split_whitespace() {
                let next = categories.len();
                categories.entry(tok.to_ascii_uppercase()).or_insert(next);
            }
        }
        if categories.len() > CategorySet::CAPACITY {
            log::warn!(
                "the units name {} category tokens; only the first {} count for targeting (BadTargetCategory)",
                categories.len(),
                CategorySet::CAPACITY
            );
        }
        let mut model_cache: FxHashMap<String, Arc<ModelDef>> = FxHashMap::default();
        let mut types = Vec::with_capacity(data.units.len());
        for (id, u) in data.units.iter().enumerate() {
            let object = u.object.to_ascii_lowercase();
            let model = model_cache
                .entry(object.clone())
                .or_insert_with(|| {
                    let parsed = load(&format!("objects3d/{object}.3do")).and_then(|b| Model3do::parse(&b).ok());
                    if parsed.is_none() {
                        log::warn!("{}: model {object}.3do missing or invalid", u.name);
                    }
                    Arc::new(parsed.as_ref().map(ModelDef::from_3do).unwrap_or_default())
                })
                .clone();
            let piece_names: Vec<String> = model.pieces.iter().map(|p| p.name.clone()).collect();
            // (Its own script, else its model's: `UnitDef::script_paths`.)
            let script = u
                .script_paths()
                .iter()
                .find_map(|path| load(path))
                .and_then(|b| CobScript::parse(&b).ok())
                .map(|s| Arc::new(CobProgram::new(s, &piece_names)));
            let model_to_cob = (0..model.pieces.len())
                .map(|mi| script.as_ref().and_then(|s| s.piece_map.iter().position(|&m| m == Some(mi))))
                .collect();

            let locomotion = if u.can_fly {
                Locomotion::Air
            } else if !u.can_move || u.max_velocity <= 0.0 {
                Locomotion::Static
            } else if u.can_hover {
                Locomotion::Hover
            } else if u.min_water_depth > 0.0 || u.floater && u.max_water_depth <= 0.0 {
                Locomotion::Naval
            } else {
                Locomotion::Ground
            };
            let fp = (u.footprint_x.clamp(1, ta_data::MAX_FOOTPRINT_SIDE), u.footprint_z.clamp(1, ta_data::MAX_FOOTPRINT_SIDE));
            let radius = Fx::from_f64((fp.0.max(fp.1) as f64) * 8.0);
            let blocking_cells = yard_blocking(&u.yard_map, fp, false);
            let open_yard_cells = yard_blocking(&u.yard_map, fp, true);
            let is_builder = u.builder && u.worker_time > 0.0;
            let build_time = Fx::from_f64(u.build_time);
            let weapons: [Option<usize>; 3] = std::array::from_fn(|i| u.weapons[i].as_deref().and_then(|w| data.weapon_id(w)));
            let move_class = data.move_classes.iter().position(|m| m.name == u.movement_class);
            let big_walker = big_walker(u, &model, move_class.map(|c| &data.move_classes[c]), locomotion);
            let build_options: Vec<_> = u.build_options.iter().filter_map(|b| data.unit_id(b)).collect();
            let is_factory = is_builder && locomotion == Locomotion::Static && !build_options.is_empty();
            types.push(UnitType {
                id,
                name: u.name.clone(),
                model,
                callins: Callins::of(script.as_deref()),
                script,
                model_to_cob,
                locomotion,
                move_class,
                footprint: fp,
                radius,
                max_health: Fx::from_f64(u.max_damage),
                max_speed: Fx::from_f64(u.max_velocity),
                acceleration: Fx::from_f64(u.acceleration.max(0.01)),
                brake: Fx::from_f64(u.brake_rate.max(0.02)),
                turn_rate: (u.turn_rate.max(1.0) as i32).min(32768),
                cruise_alt: Fx::from_f64(u.cruise_alt),
                waterline: if locomotion == Locomotion::Naval { Fx::from_f64(u.water_line.max(0.0)) } else { Fx::ZERO },
                build_time,
                build_work: (build_time.0.max(1) as u64).saturating_mul(TICKS_PER_SECOND as u64),
                cost_metal: Fx::from_f64(u.build_cost_metal),
                cost_energy: Fx::from_f64(u.build_cost_energy),
                worker_rate: per_tick(u.worker_time),
                work_rate: Fx::from_f64(u.worker_time).0.max(0) as u64,
                build_distance: Fx::from_f64(u.build_distance.max(32.0)),
                sight: Fx::from_f64(u.sight_distance),
                radar: Fx::from_f64(u.radar_distance),
                weapons,
                stockpile: weapons.iter().flatten().any(|&w| data.weapons[w].stockpile),
                energy_make: per_tick(u.energy_make),
                metal_make: per_tick(u.metal_make),
                energy_use: per_tick(u.energy_use),
                extracts_metal: Fx::from_f64(u.extracts_metal),
                makes_metal: per_tick(u.makes_metal),
                energy_storage: Fx::from_f64(u.energy_storage),
                metal_storage: Fx::from_f64(u.metal_storage),
                wind_generator: Fx::from_f64(u.wind_generator),
                tidal_generator: Fx::from_f64(u.tidal_generator),
                build_options,
                blocking_cells,
                open_yard_cells,
                is_builder,
                // Factories set CanMove=1 for rally orders. A stationary builder
                // without products is an assist turret, not a production queue.
                is_factory,
                needs_geo: u.yard_map.contains(['G', 'g']),
                category_mask: token_set(&categories, &u.category),
                bad_target_mask: token_set(&categories, target_tags(u, &u.bad_target_category)),
                weapon_bad_masks: std::array::from_fn(|k| match u.weapon_bad_target_category[k].trim() {
                    "" => token_set(&categories, target_tags(u, &u.bad_target_category)),
                    list => token_set(&categories, target_tags(u, list)),
                }),
                no_chase_mask: token_set(&categories, target_tags(u, &u.no_chase_category)),
                hover_attack: u.hover_attack,
                transport_capacity: if u.transport_capacity == 0 { 0 } else if locomotion == Locomotion::Air { 1 } else { u.transport_capacity },
                transport_size: u.transport_size,
                big_walker,
                def: u.clone(),
            });
        }
        lift_bad_targets_of_everything(&mut types, data);
        let by_name = types.iter().map(|t| (t.name.clone(), t.id)).collect();
        let airbase_reach = types.iter().filter(|t| t.def.is_airbase).map(|t| t.radius + Fx::from_int(24)).max().unwrap_or(Fx::ZERO);
        let collision_reach = types.iter().map(UnitType::collision_radius).max().unwrap_or(Fx::ZERO);
        let target_reach = types.iter().map(|t| t.radius).max().unwrap_or(Fx::ZERO);
        UnitTypes { types, by_name, categories, airbase_reach, collision_reach, target_reach }
    }

    pub fn get(&self, id: usize) -> &UnitType {
        &self.types[id]
    }

    /// The set of the `Category` tokens in `list` (a unit's `Category` or `BadTargetCategory`
    /// line; tokens no unit's `Category` names are left out).
    pub fn categories(&self, list: &str) -> CategorySet {
        token_set(&self.categories, list)
    }

    pub fn by_name(&self, name: &str) -> Option<&UnitType> {
        self.by_name.get(&name.to_ascii_uppercase()).map(|&i| &self.types[i])
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.by_name.get(&name.to_ascii_uppercase()).copied()
    }
}

/// The set of the `Category`-style tokens in `list` (unknown tokens, and those past the
/// set's capacity, are ignored).
fn token_set(categories: &FxHashMap<String, usize>, list: &str) -> CategorySet {
    let mut set = CategorySet::EMPTY;
    for t in list.split_whitespace() {
        if let Some(&bit) = categories.get(&t.to_ascii_uppercase()) {
            set.insert(bit);
        }
    }
    set
}

/// TA yard maps list one character per footprint cell: y stays free, c opens with
/// the yard, i closes with it, and g is a solid geothermal cell. Missing/unknown
/// cells are solid, as ordinary o cells are.
fn yard_blocking(yard: &str, fp: (u32, u32), open: bool) -> Vec<bool> {
    let n = (fp.0 * fp.1) as usize;
    let cells: Vec<char> = yard.chars().filter(|c| !c.is_whitespace()).take(n).collect();
    (0..n)
        .map(|i| match cells.get(i).map(|c| c.to_ascii_lowercase()) {
            Some('y') => false,
            Some('c') => !open,
            Some('i') => open,
            _ => true,
        })
        .collect()
}

/// How a projectile moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trajectory {
    /// Straight line at constant speed (lasers, plasma bolts, D-gun).
    Straight,
    /// Affected by gravity (cannons, artillery).
    Ballistic,
    /// Steers towards its target (missiles, torpedoes).
    Guided,
    /// Released from aircraft and falls (bombs).
    Dropped,
}

/// A weapon compiled into fixed-point, per-tick units.
#[derive(Clone, Debug)]
pub struct WeaponType {
    pub id: usize,
    pub name: String,
    pub range: Fx,
    pub reload_ticks: u32,
    /// Pixels per tick.
    pub velocity: Fx,
    pub start_velocity: Fx,
    /// Pixels per tick².
    pub acceleration: Fx,
    /// Angle units per tick.
    pub turn_rate: i32,
    pub aoe_radius: Fx,
    pub edge_effectiveness: Fx,
    pub default_damage: Fx,
    /// Damage overrides by unit type id.
    pub damage: FxHashMap<usize, Fx>,
    pub burst: u32,
    pub burst_ticks: u32,
    pub trajectory: Trajectory,
    pub tolerance: i32,
    /// Random aim error (angle units).
    pub spread: i32,
    pub energy_per_shot: Fx,
    pub metal_per_shot: Fx,
    pub flight_ticks: u32,
    /// Vertically launched missiles climb this long before guidance takes over.
    pub launch_ticks: u32,
    pub to_air: bool,
    /// What automatic targeting may engage: `toairweapon` weapons only aircraft; other
    /// direct-fire and guided weapons both; shells and bombs only the ground.
    pub hits_air: bool,
    pub hits_ground: bool,
    pub water: bool,
    /// Torpedoes/depth charges, as distinct from water-capable beams or shells.
    pub torpedo: bool,
    pub command_fire: bool,
    /// Passes through targets damaging each (the Commander's D-gun).
    pub no_explode: bool,
    pub vlaunch: bool,
    pub paralyzer: bool,
    pub line_of_sight: bool,
    pub def: ta_data::WeaponDef,
}

impl WeaponType {
    /// Protected radius of an anti-missile weapon, distinct from its missile's travel range.
    /// Older mods without `coverage` keep the existing fallback, capped at 2000 pixels.
    pub fn interceptor_coverage(&self) -> Fx {
        if !self.def.interceptor {
            Fx::ZERO
        } else if self.def.coverage > 0.0 {
            Fx::from_f64(self.def.coverage)
        } else {
            self.range.min(Fx::from_int(2000))
        }
    }

    pub fn damage_to(&self, unit_type: usize) -> Fx {
        self.damage.get(&unit_type).copied().unwrap_or(self.default_damage)
    }

    /// Whether its missiles strike a place, not a unit: sent at a unit, one flies to the ground
    /// the unit stands on as it is launched and never follows it, as TA's nukes did. TA's
    /// strategic missiles, stockpiled and guided but no interceptor (both nukes, the EMP and
    /// tactical missiles), none of which says `tracks` (its anti-nukes do); a mod's that says
    /// `tracks` follows its target. (Other guided missiles follow theirs, `tracks` or not.)
    pub fn strikes_a_place(&self) -> bool {
        self.trajectory == Trajectory::Guided && self.def.stockpile && !self.def.interceptor && !self.def.tracks
    }
}

pub struct WeaponTypes {
    pub types: Vec<WeaponType>,
    /// Name (exactly as written) → the first weapon of that name.
    by_name: FxHashMap<String, usize>,
}

/// How a weapon flies, and whether it is a torpedo (which runs in the water only).
fn flight(w: &ta_data::WeaponDef) -> (Trajectory, bool) {
    let torpedo = w.water_weapon && !w.vlaunch && !w.ballistic && !w.beam_weapon
        && matches!(w.render_type, ta_data::RenderType::Model | ta_data::RenderType::Bomb);
    // (A vertically launched missile turns towards its target once it has climbed,
    // whatever its `guidance` says: TA's own all declare it, but Total Mayhem's nine
    // leave it out, its missile trucks', ships' and hovercraft's, its EMP and tactical
    // missiles and both nukes, which climbed straight up for ever or, leaving the tube
    // at rest and gathering speed only when guided, hung over their silos.)
    let trajectory = if w.dropped {
        Trajectory::Dropped
    } else if w.tracks || (w.guidance || w.vlaunch) && w.turn_rate > 0.0 {
        Trajectory::Guided
    } else if w.ballistic {
        Trajectory::Ballistic
    } else {
        Trajectory::Straight
    };
    (trajectory, torpedo)
}

/// What a weapon can hit: aircraft in flight, and what is on the ground or the water
/// ([`WeaponType::hits_air`], [`WeaponType::hits_ground`]).
fn reach(w: &ta_data::WeaponDef) -> (bool, bool) {
    let (trajectory, torpedo) = flight(w);
    (
        !w.interceptor && !torpedo && (w.to_air_weapon || matches!(trajectory, Trajectory::Straight | Trajectory::Guided)),
        !w.interceptor && !w.to_air_weapon,
    )
}

/// A weapon's bad targets that cover every unit of the game data it could hit (in flight or
/// landed, for an aircraft) would leave it never firing of its own accord, which no unit of TA's
/// is made to do: TA's own data has no such weapon. Mods with tags of their own have them: Total
/// Mayhem's flak towers list `ALL`, which every one of its units carries, Escalation's anti-air
/// missiles a tag all of its aircraft carry (`NLND`, not a land unit), TA: Enhanced's Nightflower
/// `NOTSUB` on rockets that hit only aircraft. Until bad targets were held to (SIMULATION_VERSION
/// 72) those weapons took aircraft on, as their makers meant them to. Such a weapon's bad targets
/// are lifted: every target it can take on is one of them alike, so it takes on what it can.
/// The unit types a player can field: each side's Commander, what it builds, what those build,
/// and so on. Every type, when no side names a Commander the data has.
fn fieldable(types: &[UnitType], data: &GameData) -> Vec<bool> {
    let mut queue: Vec<usize> = data.sides.iter().filter_map(|s| data.unit_id(&s.commander)).collect();
    if queue.is_empty() {
        return vec![true; types.len()];
    }
    let mut seen = vec![false; types.len()];
    while let Some(i) = queue.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        queue.extend(types[i].build_options.iter().copied().filter(|&o| !seen[o]));
    }
    seen
}

fn lift_bad_targets_of_everything(types: &mut [UnitType], data: &GameData) {
    // (Only units a player can field count: Escalation's joke unit and mission-only units,
    // which nobody builds, lack the `ALL` its weapons list, and kept the Raptor's Starburst
    // Missiles, the Victory's rockets and others from ever firing of their own accord.)
    let fieldable = fieldable(types, data);
    let lifted: Vec<(usize, usize)> = types
        .iter()
        .enumerate()
        .flat_map(|(i, t)| (0..3).map(move |k| (i, k, t)))
        .filter(|&(_, k, t)| !t.weapon_bad_masks[k].is_empty())
        .filter_map(|(i, k, t)| {
            let (air, ground) = reach(&data.weapons[t.weapons[k]?]);
            // (An anti-missile hits no unit: its bad targets never come into it.)
            if !air && !ground {
                return None;
            }
            let left = types.iter().any(|u| {
                let hit = if u.locomotion == Locomotion::Air { air || ground } else { ground };
                fieldable[u.id] && hit && !u.category_mask.intersects(&t.weapon_bad_masks[k])
            });
            (!left).then_some((i, k))
        })
        .collect();
    if !lifted.is_empty() {
        let names: Vec<String> = lifted.iter().map(|&(i, k)| format!("{} {}", types[i].name, k + 1)).collect();
        log::info!("{} weapons' bad targets are every unit they could hit: lifted ({})", lifted.len(), names.join(", "));
    }
    for (i, k) in lifted {
        types[i].weapon_bad_masks[k] = CategorySet::EMPTY;
    }
}

impl WeaponTypes {
    pub fn compile(data: &GameData) -> WeaponTypes {
        let tps = TICKS_PER_SECOND as f64;
        let types = data
            .weapons
            .iter()
            .enumerate()
            .map(|(id, w)| {
                let velocity = Fx::from_f64((w.velocity / tps).max(0.5));
                let (trajectory, torpedo) = flight(w);
                let (hits_air, hits_ground) = reach(w);
                let range = Fx::from_f64(w.range.max(1.0));
                // Missiles self-destruct after `flighttime`; on vertically launched missiles
                // `weapontimer` is the climb before guidance starts (gathering speed from their
                // start speed), elsewhere it doubles as the lifetime. Other projectiles fly until they have covered their range.
                let flight_ticks = if w.flight_time > 0.0 {
                    (w.flight_time * tps) as u32
                } else if w.weapon_timer > 0.0 && !w.vlaunch {
                    (w.weapon_timer * tps) as u32
                } else {
                    (((w.range / (w.velocity / tps).max(0.5)) * 1.6) as u32).saturating_add(10)
                };
                let launch_ticks = if w.vlaunch { (w.weapon_timer.max(0.4) * tps) as u32 } else { 0 };
                WeaponType {
                    id,
                    name: w.name.clone(),
                    range,
                    reload_ticks: ((w.reload_time * tps).round() as u32).max(1),
                    velocity,
                    start_velocity: Fx::from_f64(w.start_velocity / tps),
                    acceleration: Fx::from_f64(w.acceleration / (tps * tps)),
                    turn_rate: (w.turn_rate / tps) as i32,
                    aoe_radius: Fx::from_f64((w.area_of_effect * 0.5).max(4.0)),
                    edge_effectiveness: Fx::from_f64(w.edge_effectiveness.clamp(0.0, 0.99)),
                    default_damage: Fx::from_f64(w.default_damage),
                    damage: w.damage.iter().filter_map(|(k, &v)| Some((data.unit_id(k)?, Fx::from_f64(v)))).collect(),
                    burst: w.burst.max(1),
                    burst_ticks: ((w.burst_rate * tps).round() as u32).max(1),
                    trajectory,
                    tolerance: (w.tolerance as i32).max(0),
                    spread: ((w.accuracy + w.spray_angle) as i32).max(0),
                    energy_per_shot: Fx::from_f64(w.energy_per_shot),
                    metal_per_shot: Fx::from_f64(w.metal_per_shot),
                    flight_ticks: flight_ticks.clamp(5, 30 * 600),
                    launch_ticks,
                    to_air: w.to_air_weapon,
                    hits_air,
                    hits_ground,
                    water: w.water_weapon,
                    torpedo,
                    command_fire: w.command_fire,
                    no_explode: w.no_explode,
                    vlaunch: w.vlaunch,
                    paralyzer: w.paralyzer,
                    line_of_sight: w.line_of_sight,
                    def: w.clone(),
                }
            })
            .collect::<Vec<WeaponType>>();
        let mut by_name = FxHashMap::default();
        for w in &types {
            by_name.entry(w.name.clone()).or_insert(w.id);
        }
        WeaponTypes { types, by_name }
    }

    pub fn get(&self, id: usize) -> &WeaponType {
        &self.types[id]
    }

    /// The first weapon named exactly `name` (a unit's `ExplodeAs`, say).
    pub fn by_name(&self, name: &str) -> Option<&WeaponType> {
        self.by_name.get(name).map(|&i| &self.types[i])
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn extreme_definition_dimensions_and_transport_sizes_compile_safely() {
        let doc = ta_formats::tdf::parse("[UNITINFO]{UnitName=HOSTILE;FootprintX=4294967295;FootprintZ=2;CanMove=1;MaxVelocity=1;TransportCapacity=1;TransportSize=4294967295;} [weapon]{Range=999999999999999;}");
        let mut data = GameData::load(&ta_vfs::Vfs::new());
        let mut unit = UnitDef::from_tdf(&doc.sections[0]).unwrap();
        // Directly constructed public definitions also pass through the compile guard.
        unit.footprint_x = u32::MAX;
        data.units.push(unit);
        data.weapons.push(ta_data::WeaponDef::from_tdf(&doc.sections[1]));
        let units = UnitTypes::compile(&data, &|_| None);
        let unit = &units.types[0];
        assert_eq!(unit.footprint, (ta_data::MAX_FOOTPRINT_SIDE, 2));
        assert_eq!(unit.blocking_cells.len(), ta_data::MAX_FOOTPRINT_SIDE as usize * 2);
        assert!(unit.can_carry(unit));
        assert_eq!(WeaponTypes::compile(&data).types[0].flight_ticks, 30 * 600);
    }
}

#[cfg(test)]
mod script_lookup_tests {
    use super::*;

    /// The smallest script `CobScript::parse` reads: one function named `function`, one piece.
    fn script(function: &str) -> Vec<u8> {
        let word = |b: &mut Vec<u8>, at: usize, n: u32| b[at..at + 4].copy_from_slice(&n.to_le_bytes());
        let (offsets, names, pieces, code_at) = (44, 48, 52, 56);
        let mut b = vec![0; code_at + 4];
        word(&mut b, 0, 4);
        word(&mut b, 4, 1);
        word(&mut b, 8, 1);
        word(&mut b, 12, 1);
        word(&mut b, 24, offsets);
        word(&mut b, 28, names);
        word(&mut b, 32, pieces);
        word(&mut b, 36, code_at as u32);
        word(&mut b, code_at, ta_formats::cob::op::RETURN);
        let at = b.len() as u32;
        word(&mut b, names as usize, at);
        b.extend_from_slice(function.as_bytes());
        b.push(0);
        let at = b.len() as u32;
        word(&mut b, pieces as usize, at);
        b.extend_from_slice(b"base\0");
        b
    }

    /// TA runs a unit's own script (`scripts/<UnitName>.cob`) whatever model it borrows, as TA:
    /// Enhanced's 35 units with another's model and scripts under their own names only show;
    /// a unit with no script of its own runs its model's (Escalation's xARMMLS).
    #[test]
    fn a_unit_runs_its_own_script_before_its_models() {
        let doc = ta_formats::tdf::parse(
            "[UNITINFO]{UnitName=Lender;Objectname=LENDER;} [UNITINFO]{UnitName=Borrower;Objectname=Lender;} \
             [UNITINFO]{UnitName=Copier;Objectname=lender;} [UNITINFO]{UnitName=Stray;Objectname=nobody;}",
        );
        let mut data = GameData::load(&ta_vfs::Vfs::new());
        data.units.extend(doc.sections.iter().map(|s| UnitDef::from_tdf(s).unwrap()));
        let files = |path: &str| match path {
            "scripts/lender.cob" => Some(script("LenderScript")),
            "scripts/borrower.cob" => Some(script("BorrowerScript")),
            _ => None,
        };
        let types = UnitTypes::compile(&data, &files);
        let runs = |name: &str| {
            let t = types.types.iter().find(|t| t.name == name).unwrap();
            t.script.as_ref().map(|s| s.script.functions[0].name.clone())
        };
        assert_eq!(runs("LENDER").as_deref(), Some("LenderScript"));
        assert_eq!(runs("BORROWER").as_deref(), Some("BorrowerScript"), "its own script, not its model's");
        assert_eq!(runs("COPIER").as_deref(), Some("LenderScript"), "no script of its own: its model's");
        assert_eq!(runs("STRAY"), None);
        let stray = &data.units[3];
        assert_eq!(stray.script_paths(), ["scripts/stray.cob", "scripts/nobody.cob"]);
        assert_eq!(stray.script_path(|_| false), "scripts/stray.cob", "a missing script is its own");
        assert_eq!(data.units[0].script_paths(), ["scripts/lender.cob"], "one name for both: one path");
    }
}
