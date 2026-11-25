//! A unit's file (`units/<id>.fbi`) and its weapons' (`weapons/supcom_<faction>.tdf`), made from
//! its blueprint at TA's scale (docs/SUPCOM_UNITS.md): 32 pixels an ogrid, 30 ticks a second, its
//! health, costs, build time, speeds, sight and weapons Forged Alliance's own.
//!
//! - **Build time:** Forged Alliance builds a unit in `BuildTime / BuildRate` seconds, as TA does
//!   in `BuildTime / WorkerTime`; both are taken ten times ([`WORK`]), so its builders' work
//!   stands beside TA's (a commander's 100 against TA's 300) and every unit takes as long to
//!   build as in Forged Alliance.
//! - **Weapons:** up to eight slots ([`Weapons`], [`MAX_WEAPONS`]: TA's three and the five more TA
//!   Reborn gives mods): a commander's main gun first and its Overcharge second, made TA's D-gun
//!   (`commandfire`); otherwise the most damaging first, an anti-air one kept where there is one.
//!   Weapons an enhancement brings, those the unit's script switches off as it is made, and
//!   dummies are left out; the death weapon is what the unit explodes as. Each looks as TA's
//!   weapon of its kind does (a laser, a shell, a missile, a torpedo, a bomb) and hits as Forged
//!   Alliance's: its damage a second Forged Alliance's by its own firing rules ([`fa_dps`],
//!   [`firing`]). A missile its launcher builds and stores (`CountedProjectile`) is TA's
//!   stockpile, built at its projectile's price over its build time; missile defences take what
//!   Forged Alliance's do (TA's `interceptor` and `targetable` as classes: [`STRATEGIC`],
//!   [`TACTICAL`]).

use std::fmt::Write as _;

use super::UNIT_SCALE;
use crate::unit::{Faction, Motion, Rack, Tech, UnitBlueprint, WeaponBlueprint};

/// Forged Alliance's build rates and build times, taken ten times.
pub const WORK: f64 = 10.0;

/// Ticks a second.
const TICKS: f64 = 30.0;

/// The class of strategic missiles (TA's `targetable` and `interceptor` bit 1: TA's own nukes and
/// anti-nukes say 1), what anti-nukes take (`TargetRestrictOnlyAllow = 'STRATEGIC MISSILE'`).
pub const STRATEGIC: u32 = 1;
/// The class of tactical missiles (bit 2), what tactical missile defences and ships' point
/// defences take (`'TACTICAL MISSILE'`, `'TACTICAL,MISSILE'`).
pub const TACTICAL: u32 = 2;

/// TA's side name for a faction.
pub fn side(faction: &Faction) -> &'static str {
    match faction {
        Faction::Uef => "UEF",
        Faction::Cybran => "CYBRAN",
        Faction::Aeon => "AEON",
        Faction::Seraphim => "SERAPHIM",
        Faction::Other(_) => "SUPCOM",
    }
}

/// Text made safe for a TDF value: none of TA's marks (`;` ends a value, `{`/`}` and `[`/`]`
/// open and close sections, `//` and `/*` start comments), on one line.
pub fn tdf_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        out.push(match c {
            ';' => ',',
            '{' | '[' => '(',
            '}' | ']' => ')',
            '\n' | '\r' | '\t' => ' ',
            c => c,
        });
    }
    while out.contains("//") || out.contains("/*") {
        out = out.replace("//", "/").replace("/*", "/");
    }
    out.trim().to_string()
}

/// The most weapons a unit carries in TA Reborn (`Weapon1` to `Weapon8`: ta_data's `MAX_WEAPONS`).
pub const MAX_WEAPONS: usize = 8;

/// The weapons in its slots, and the one it dies with.
pub struct Weapons<'a> {
    pub slots: Vec<&'a WeaponBlueprint>,
    pub death: Option<&'a WeaponBlueprint>,
    /// The slot of the commander's Overcharge (TA's D-gun), if any.
    pub dgun: Option<usize>,
    /// Its `Kamikaze` weapon (the Fire Beetle's): what it blows itself up as, near its enemy.
    pub kamikaze: Option<&'a WeaponBlueprint>,
}

/// What a weapon does, by its category and its projectile's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Laser,
    Shell,
    Artillery,
    Missile,
    Torpedo,
    Bomb,
    AntiAir,
    Overcharge,
    Nuke,
    AntiMissile,
}

/// Whether a target restriction (`TargetRestrictOnlyAllow`, categories joined by spaces, choices
/// by commas) names `category`.
fn restricts_to(w: &WeaponBlueprint, category: &str) -> bool {
    w.only_allow.split([',', ' ']).any(|c| c.trim().eq_ignore_ascii_case(category))
}

pub fn kind(w: &WeaponBlueprint) -> Kind {
    let proj = w.projectile.as_deref().unwrap_or("").to_ascii_lowercase();
    let cat = w.category.to_ascii_lowercase();
    if w.damage_type.eq_ignore_ascii_case("overcharge") || w.label.eq_ignore_ascii_case("overcharge") {
        return Kind::Overcharge;
    }
    if w.nuke.is_some() {
        return Kind::Nuke;
    }
    if restricts_to(w, "MISSILE") || (cat == "defense" && !restricts_to(w, "TORPEDO")) || proj.contains("antimissile") || proj.contains("antinuke") || proj.contains("intercept") {
        return Kind::AntiMissile;
    }
    if cat == "bomb" || proj.contains("bomb") {
        return Kind::Bomb;
    }
    if cat == "anti navy" || proj.contains("torpedo") || proj.contains("depthcharge") {
        return Kind::Torpedo;
    }
    if cat == "anti air" || air_only(w) {
        return Kind::AntiAir;
    }
    if cat == "missile" || proj.contains("missile") || proj.contains("rocket") || w.shot.as_ref().is_some_and(|s| s.is("MISSILE")) {
        return Kind::Missile;
    }
    if cat == "artillery" || w.ballistic_arc.eq_ignore_ascii_case("RULEUBA_HighArc") {
        return Kind::Artillery;
    }
    if beam(w) || proj.contains("laser") || proj.contains("beam") || proj.contains("disintegrator") || proj.contains("sonic") {
        return Kind::Laser;
    }
    Kind::Shell
}

/// Whether it shoots at aircraft alone (`FireTargetLayerCapsTable`: 'Air' from every layer).
fn air_only(w: &WeaponBlueprint) -> bool {
    !w.target_layers.is_empty() && w.target_layers.iter().all(|(_, to)| to.split('|').all(|l| l.trim().eq_ignore_ascii_case("air")))
}

/// Whether it is anti-air that takes on aircraft alone in Forged Alliance, of its own accord or by
/// order: 'Air' from every layer ([`air_only`]: the anti-air towers, mobile and ships' anti-air),
/// or none but `AIR` units (`TargetRestrictOnlyAllow`: the aircraft's guns and missiles, an
/// interceptor's among them, whose layers name the land as well). Not a missile defence, whose
/// 'Air' is the missiles it shoots down (TA Reborn's interceptors take nothing else anyway).
pub fn aircraft_alone(w: &WeaponBlueprint) -> bool {
    kind(w) == Kind::AntiAir && (air_only(w) || restricts_to(w, "AIR"))
}

/// Whether it may shoot at aircraft (its `FireTargetLayerCapsTable` names 'Air' from some layer;
/// none given, it may).
pub fn shoots_air(w: &WeaponBlueprint) -> bool {
    w.target_layers.is_empty() || w.target_layers.iter().any(|(_, to)| to.split('|').any(|l| l.trim().eq_ignore_ascii_case("air")))
}

/// Whether it is a beam (Forged Alliance's beam weapons all give `BeamLifetime`, 0 for one kept on
/// while it has a target).
fn beam(w: &WeaponBlueprint) -> bool {
    w.beam_lifetime > 0.0 || w.continuous_beam || w.table.get_ci("BeamLifetime").is_some()
}

/// The class of missile defence that takes its shots (0: none): a strategic missile's
/// [`STRATEGIC`], a tactical missile's [`TACTICAL`] (its projectile's categories).
pub fn targetable_class(w: &WeaponBlueprint) -> u32 {
    match &w.shot {
        Some(s) if s.strategic_missile() => STRATEGIC,
        Some(s) if s.tactical_missile() => TACTICAL,
        Some(_) => 0,
        // (A nuke whose projectile the game lacks: a strategic missile still.)
        None if w.nuke.is_some() => STRATEGIC,
        None => 0,
    }
}

/// The class of missiles a missile defence takes: anti-nukes the strategic (their restriction
/// names `STRATEGIC`), the rest (tactical missile defences, ships' point defences) the tactical.
pub fn interceptor_class(w: &WeaponBlueprint) -> u32 {
    if kind(w) != Kind::AntiMissile {
        0
    } else if restricts_to(w, "STRATEGIC") {
        STRATEGIC
    } else {
        TACTICAL
    }
}

/// How a weapon fires, by Forged Alliance's rules (its `lua/sim/defaultweapons.lua`): each time it
/// fires one rack fires, in turn, or all of them (`RackFireTogether`); a rack fires its muzzles
/// one after another `MuzzleSalvoSize` times, or all of them at once (`MuzzleSalvoDelay` 0); it
/// fires `RateOfFire` times a second, no faster than its muzzles' charges and delays allow, waits
/// `RackSalvoReloadTime` and charges `RackSalvoChargeTime` once every rack has fired, and fires
/// no faster than its energy (`EnergyRequired` at `EnergyDrainPerSecond`) comes. A shot deals its
/// `Damage` each of its damage over time's pulses, as many times as its projectile splits into
/// pieces (all the way down), or as its beam hits (every `BeamCollisionDelay` and a tick, for its
/// `BeamLifetime`; a beam kept on while it has a target hits so for as long).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cycle {
    /// What one shot deals (a beam's every hit).
    pub per_shot: f64,
    /// Shots each time it fires, on average.
    pub shots: f64,
    /// Seconds from one firing to the next, on average.
    pub period: f64,
    /// Seconds between the shots of one firing (0: all at once).
    pub spacing: f64,
    /// A beam kept on: `per_shot` every `period`.
    pub continuous: bool,
}

/// One tick of Forged Alliance's simulation (seconds): a beam hits every `BeamCollisionDelay`
/// and one of these.
const FA_TICK: f64 = 0.1;

pub fn cycle(w: &WeaponBlueprint) -> Cycle {
    let is_beam = beam(w);
    let continuous = is_beam && (w.continuous_beam || w.beam_lifetime <= 0.0);
    let hit_every = w.beam_collision_delay.max(0.0) + FA_TICK;
    let hits = if is_beam && !continuous { (w.beam_lifetime / hit_every).max(1.0) } else { 1.0 };
    let per_shot = w.damage.max(0.0) * w.pulses() * w.hits_per_shot() * hits;
    let all_at_once = w.salvo_delay <= 0.0;
    let rack_shots = |r: &Rack| if all_at_once { r.muzzles.len().max(1) } else { w.salvo_size.max(1) as usize };
    let racks: Vec<usize> = if w.racks.is_empty() { vec![if all_at_once { 1 } else { w.salvo_size.max(1) as usize }] } else { w.racks.iter().map(rack_shots).collect() };
    let total: usize = racks.iter().sum();
    if continuous {
        return Cycle { per_shot, shots: total as f64, period: hit_every, spacing: 0.0, continuous };
    }
    let fires = if w.racks_together { 1 } else { racks.len() };
    let shots = total as f64 / fires as f64;
    let rate = if w.rate_of_fire > 0.0 { w.rate_of_fire } else { 1.0 };
    // (A firing takes its muzzles' charges and delays, one after another.)
    let firing = shots * (w.muzzle_charge.max(0.0) + w.salvo_delay.max(0.0));
    let mut period = (1.0 / rate).max(firing) + (w.rack_reload.max(0.0) + w.rack_charge.max(0.0)) / fires as f64;
    if w.energy_required > 0.0 && w.energy_drain > 0.0 {
        period = period.max(w.energy_required / w.energy_drain);
    }
    let spacing = if all_at_once { w.muzzle_charge.max(0.0) } else { w.salvo_delay + w.muzzle_charge.max(0.0) };
    Cycle { per_shot, shots, period, spacing, continuous }
}

/// Forged Alliance's damage a second for the weapon, by its own rules ([`Cycle`]).
pub fn fa_dps(w: &WeaponBlueprint) -> f64 {
    let c = cycle(w);
    c.per_shot * c.shots / c.period.max(1e-6)
}

/// A weapon's firing in TA's keys: a burst of `burst` shots `burst_rate` seconds apart, each
/// dealing `damage`, every `reload` seconds (a whole number of TA's ticks). The burst is the
/// shots it fires each time (at least one); the reload is what gives Forged Alliance's damage a
/// second, the damage taken a little up or down for the reload's ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Firing {
    pub damage: f64,
    pub burst: u32,
    pub burst_rate: f64,
    pub reload: f64,
}

/// A weapon's firing in TA's keys, its damage a second Forged Alliance's ([`fa_dps`]).
pub fn firing(w: &WeaponBlueprint) -> Firing {
    let c = cycle(w);
    let dps = c.per_shot * c.shots / c.period.max(1e-6);
    let burst = (c.shots.round() as u32).max(1);
    if dps <= 0.0 || c.per_shot <= 0.0 {
        return Firing { damage: c.per_shot, burst, burst_rate: 0.1, reload: ticks_of(c.period) };
    }
    let reload = ticks_of(c.per_shot * burst as f64 / dps);
    let damage = dps * reload / burst as f64;
    // (A burst done within its reload: its shots no further apart than the reload allows.)
    let spacing = if c.spacing > 0.0 { c.spacing } else { 1.0 / TICKS };
    let burst_rate = if burst > 1 { spacing.min(reload / burst as f64).max(1.0 / TICKS) } else { 0.1 };
    Firing { damage, burst, burst_rate, reload }
}

/// `seconds` as a whole number of ticks (one at least), in seconds.
fn ticks_of(seconds: f64) -> f64 {
    (seconds * TICKS).round().max(1.0) / TICKS
}

/// A number as a TDF value: whole where it is within half a percent of a whole number, else to two
/// places.
fn number(v: f64) -> String {
    let whole = v.round();
    if (v - whole).abs() <= v.abs() * 0.005 || v.abs() >= 1000.0 { format!("{}", whole as i64) } else { format!("{v:.2}") }
}

/// Damage a second, for choosing which weapons take the slots.
fn power(w: &WeaponBlueprint) -> f64 {
    fa_dps(w) + w.nuke.as_ref().map_or(0.0, |n| n.inner_damage)
}

/// Whether a weapon of `u`'s takes a slot: not its death or its kamikaze blast, a dummy, an
/// anti-torpedo defence or a weapon of a point a shot (the Galactic Colossus's tractor claws); nor,
/// unless `enhanced` (one its enhancements switch on, `factions::enhance`), one an enhancement
/// switches on or its script switches off as it is made (a toggled second gun; the Overcharge, off
/// until ordered, is TA's D-gun).
pub fn usable(u: &UnitBlueprint, w: &WeaponBlueprint, enhanced: bool) -> bool {
    let k = kind(w);
    !w.category.eq_ignore_ascii_case("death")
        && !w.category.eq_ignore_ascii_case("kamikaze")
        && (enhanced || w.table.get_ci("EnabledByEnhancement").is_none())
        && (enhanced || k == Kind::Overcharge || !u.birth.weapon_off(&w.label))
        && !w.label.to_ascii_lowercase().contains("dummy")
        && !w.label.to_ascii_lowercase().contains("antitorpedo")
        && !w.projectile.as_deref().unwrap_or("").to_ascii_lowercase().contains("antitorpedo")
        && !restricts_to(w, "TORPEDO")
        && (w.damage > 1.0 || w.nuke.is_some() || k == Kind::AntiMissile || k == Kind::Overcharge || super::abilities::stuns(w))
}

/// The weapons that take its slots (eight at most), the death weapon and its kamikaze blast.
pub fn weapons(u: &UnitBlueprint) -> Weapons<'_> {
    let usable = |w: &&WeaponBlueprint| usable(u, w, false);
    let death = u.weapons.iter().find(|w| w.category.eq_ignore_ascii_case("death") || w.label.eq_ignore_ascii_case("DeathWeapon"));
    let kamikaze = u.weapons.iter().find(|w| w.category.eq_ignore_ascii_case("kamikaze") && w.damage > 0.0);
    let mut armed: Vec<&WeaponBlueprint> = u.weapons.iter().filter(usable).collect();
    let mut slots: Vec<&WeaponBlueprint> = Vec::new();
    let mut dgun = None;
    if u.is("COMMAND") || u.is("SUBCOMMANDER") {
        // The main gun, then the Overcharge.
        if let Some(i) = armed.iter().position(|w| kind(w) != Kind::Overcharge) {
            slots.push(armed.remove(i));
        }
        if let Some(i) = armed.iter().position(|w| kind(w) == Kind::Overcharge) {
            dgun = Some(slots.len());
            slots.push(armed.remove(i));
        }
    }
    // The rest by their damage (the first among equals first), an anti-air one kept.
    armed.sort_by(|a, b| power(b).total_cmp(&power(a)));
    let anti_air = armed.iter().position(|w| kind(w) == Kind::AntiAir);
    while slots.len() < MAX_WEAPONS && !armed.is_empty() {
        if slots.len() == MAX_WEAPONS - 1
            && anti_air.is_some()
            && !slots.iter().any(|w| kind(w) == Kind::AntiAir)
            && let Some(i) = armed.iter().position(|w| kind(w) == Kind::AntiAir)
        {
            slots.push(armed.remove(i));
            continue;
        }
        slots.push(armed.remove(0));
    }
    Weapons { slots, death, dgun, kamikaze }
}

/// A weapon's section name: `<id>_<label>`, letters and digits; a label the unit's weapons give
/// more than once numbered after its first (an Aeon carrier's two `AntiAirMissiles01`).
pub fn weapon_name(u: &UnitBlueprint, w: &WeaponBlueprint) -> String {
    let clean = |w: &WeaponBlueprint| -> String { w.label.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase() };
    let label = clean(w);
    let label = if label.is_empty() { "WEAPON".to_string() } else { label };
    let before = u.weapons.iter().take_while(|other| !std::ptr::eq(*other, w)).filter(|other| clean(other) == clean(w)).count();
    let name = format!("{}_{label}", u.id).to_ascii_uppercase();
    if before == 0 { name } else { format!("{name}_{}", before + 1) }
}

/// TA's explosion for a blast `diameter` pixels across.
fn explosion(diameter: f64) -> &'static str {
    match diameter {
        d if d < 24.0 => "explode5",
        d if d < 64.0 => "explode3",
        d if d < 160.0 => "explode2",
        _ => "explode4",
    }
}

/// What an air transport carries, by its mesh's attach bones (`…Attachpoint…`: Forged Alliance
/// sets its passengers there, small units on the plain or `_sml`/`_Small` ones, larger on `_Med`
/// and `_Lrg` ones; a side's own named `Left_`, `Right_`):
/// as many units as it has small slots (or larger, where it has none), as large as its largest
/// slots take (TA's `TransportSize`: footprints up to twice it). None without attach bones.
pub fn transport(mesh: &crate::scm::Mesh) -> Option<(u32, u32)> {
    let (mut small, mut medium, mut large) = (0u32, 0u32, 0u32);
    for b in &mesh.bones {
        let name = b.name.to_ascii_lowercase();
        if !name.contains("attachpoint") {
            continue;
        }
        if name.contains("lrg") {
            large += 1;
        } else if name.contains("med") {
            medium += 1;
        } else {
            small += 1;
        }
    }
    let units = if small > 0 { small } else { medium + large };
    (units > 0).then_some((units, if large > 0 { 3 } else if medium > 0 { 2 } else { 1 }))
}

/// A nuke's range, pixels: the whole of any map.
const NUKE_RANGE: i64 = 32000;

/// The speed a strategic missile flies at, at least: TA's nukes' (`NUCLEAR_MISSILE`'s 350 pixels a
/// second). Forged Alliance's leave their silo at a crawl (5 ogrids a second) and speed up on their
/// way, which TA's flight model, a missile at its weapon's velocity, cannot do: at the crawl, one
/// took a minute to cross a 20 km map.
const NUKE_SPEED: i64 = 350;

/// How long a missile launched upright climbs before it steers (seconds, TA's `weapontimer`), its
/// strategic and tactical missiles' (`NUCLEAR_MISSILE`, `CRBLMSSL`, `CORTRON_WEAPON`): out of the
/// tube high over everything, then level across the map (`cruise`) and down onto its target. A
/// nuke's, leaving at `LAUNCH_START` and gathering `LAUNCH_ACCELERATION`, takes it some 1,200 px
/// up (it tops out some 1,300 px over the ground as it turns over).
const LAUNCH_CLIMB: i64 = 5;

/// A tactical missile's climb (seconds): from `LAUNCH_START` some 590 px up, topping out some
/// 670 px over the ground as it turns over, about as high as TA's own tactical missile
/// (`CORTRON_WEAPON`, five seconds from rest: some 770 from its taller launcher).
const TACTICAL_CLIMB: i64 = 3;

/// How fast those missiles turn (TA's angle units a second): TA's strategic missiles' 32,768, half
/// a turn.
const CRUISE_TURN: i64 = 32768;

/// How fast those missiles gather speed out of the tube (px a second squared): TA's strategic
/// missiles' 50.
const LAUNCH_ACCELERATION: i64 = 50;

/// The least a homing anti-air shot turns (angle units a second): TA's missile towers' (`ARMRL`,
/// `CORRL`: 50,000; the Arm's missile truck 30,000).
const AA_TURN: i64 = 30000;

/// The least speed a tactical missile defence's shot flies at, from its muzzle (px a second): the
/// Seraphim's (90 ogrids a second; the UEF's Phalanx shoots at 100). The Cybran zapper is a beam
/// and the Aeon's flares draw the missiles onto them (5 ogrids a second), neither of which TA
/// Reborn's interceptors do: each flies at the missile it takes, and at the zapper's or the flares'
/// own speed one never caught a tactical missile coming down at 12 ogrids a second.
const COUNTERMEASURE_SPEED: i64 = 2880;

/// How long a missile flies before it falls, seconds (TA's `flighttime`, its projectile's life):
/// long enough to cover `range` pixels at its `speed` half again (it turns as it tracks), with
/// its climb to speed from `start` at `acceleration` and a second to launch.
pub fn flight_time(range: i64, speed: i64, start: i64, acceleration: i64) -> i64 {
    let speed = speed.max(1) as f64;
    let climb = (speed - start as f64).max(0.0) / acceleration.max(1) as f64;
    (range as f64 / speed * 1.5 + climb + 1.0).ceil().max(3.0) as i64
}

/// Whether its missiles leave their tube upright (`vlaunch`): a launcher's stored missiles (the
/// nukes, the tactical missiles) and the anti-nukes', as TA's strategic missiles and anti-nukes.
pub fn launched_upright(w: &WeaponBlueprint) -> bool {
    stockpiled(w) && matches!(kind(w), Kind::Missile | Kind::Nuke | Kind::AntiMissile)
}

/// The least speed a missile leaves its tube upright at (px a second): it arms on its third
/// tick, by when it has risen 12 px over the ground at this speed, so it never goes off in its
/// silo (the launch is from the silo's foot where its muzzle rests under the ground,
/// `script::muzzles`).
const LAUNCH_START: i64 = 120;

/// Whether the weapon's missiles are stored by its launcher (`CountedProjectile`), at a price:
/// TA's stockpile.
pub fn stockpiled(w: &WeaponBlueprint) -> bool {
    w.counted && (w.shot.as_ref().is_some_and(|s| s.priced()) || w.nuke.is_some())
}

/// A stored missile's price and the seconds its launcher takes to build one: its projectile's
/// `Economy` at the launcher's `Economy.BuildRate` (a nuke, 324,000 at 1,080: 300 s).
pub fn missile_price(u: &UnitBlueprint, w: &WeaponBlueprint) -> (f64, f64, f64) {
    let Some(s) = w.shot.as_ref().filter(|s| s.priced()) else { return (0.0, 0.0, 1.0 / w.rate_of_fire.max(0.01)) };
    let seconds = if u.build_rate > 0.0 { s.build_time / u.build_rate } else { s.build_time / 10.0 };
    (s.cost_mass, s.cost_energy, seconds.max(1.0))
}

/// A weapon's section: its range, reload, speed, blast and damage Forged Alliance's at TA's
/// scale; how it flies and looks as TA's weapons of its kind.
pub fn weapon_section(u: &UnitBlueprint, w: &WeaponBlueprint) -> String {
    weapon_section_named(u, w, &weapon_name(u, w))
}

/// A weapon's section under `name` (an enhancement's changed gun: `factions::enhance`).
pub fn weapon_section_named(u: &UnitBlueprint, w: &WeaponBlueprint, name: &str) -> String {
    let k = kind(w);
    let px = |ogrids: f64| (ogrids * UNIT_SCALE).round() as i64;
    let mut s = String::new();
    let mut put = |key: &str, value: String| {
        let _ = writeln!(s, "\t{key}={value};");
    };
    put("name", tdf_text(if w.display_name.is_empty() { &w.label } else { &w.display_name }));
    let range = if k == Kind::Nuke { NUKE_RANGE } else { px(w.max_radius.max(1.0)) };
    put("range", range.to_string());
    let stock = stockpiled(w);
    let fire = firing(w);
    if stock {
        // A stored missile: built over its build time, paid for as it is built.
        let (mass, energy, seconds) = missile_price(u, w);
        put("reloadtime", format!("{:.3}", ticks_of(seconds)));
        put("stockpile", "1".into());
        put("metalpershot", (mass.round() as i64).to_string());
        put("energypershot", (energy.round() as i64).to_string());
    } else {
        put("reloadtime", format!("{:.4}", fire.reload));
        if fire.burst > 1 {
            put("burst", fire.burst.to_string());
            put("burstrate", format!("{:.4}", fire.burst_rate));
        }
        if w.energy_required > 0.0 {
            put("energypershot", (w.energy_required.round() as i64).to_string());
        }
    }
    // The projectile's own flight, where its blueprint gives it: its speed leaving the muzzle, its
    // top speed and acceleration (ogrids a second).
    let shot = w.shot.as_ref();
    let muzzle = (w.muzzle_velocity.max(5.0) * UNIT_SCALE).round() as i64;
    let top = shot.map(|s| s.max_speed).filter(|&v| v > 0.0).map_or(muzzle, |v| (v * UNIT_SCALE).round() as i64).max(muzzle);
    let speed = match k {
        Kind::Nuke => muzzle.max(NUKE_SPEED),
        Kind::AntiMissile if !launched_upright(w) => top.max(COUNTERMEASURE_SPEED),
        Kind::Missile | Kind::AntiAir | Kind::AntiMissile => top,
        _ => muzzle,
    };
    put("weaponvelocity", speed.to_string());
    let flak = k == Kind::AntiAir && w.damage_radius > 0.0;
    let blast = match &w.nuke {
        Some(n) => n.outer_radius * 2.0 * UNIT_SCALE,
        None => (w.damage_radius * 2.0 * UNIT_SCALE).max(8.0),
    };
    put("areaofeffect", (blast.round() as i64).to_string());
    // (Forged Alliance's blasts deal their damage evenly out to their edge: flak, which bursts by
    // its target, hurts all that its burst reaches, as TA's flak does.)
    put("edgeeffectiveness", if w.nuke.is_some() { "0.25" } else if flak { "0.9" } else { "0.5" }.into());
    // Firing tolerance, degrees to TA's angle units.
    put("tolerance", ((w.firing_tolerance.max(1.0) * 65536.0 / 360.0).round() as i64).max(500).to_string());
    if w.turreted {
        put("turret", "1".into());
    }
    let homing_aa = k == Kind::AntiAir && !beam(w);
    let (render, model, start, hit) = match k {
        Kind::Laser => (0, "", "lasrfir1", "lasrhit1"),
        Kind::Shell => (4, "", "cannon1", "xplomed2"),
        Kind::Artillery => (4, "", "cannhvy1", "xplomed2"),
        Kind::Missile => (1, "missile", "rocklit3", "xplomed2"),
        Kind::Torpedo => (1, "torpedo", "torpedo1", "xplodep1"),
        Kind::Bomb => (6, "bomb", "bombrel", "xplomed2"),
        Kind::AntiAir if shot.is_some_and(|s| s.is("MISSILE")) => (1, "missile", "rockhvy2", "xplomed2"),
        Kind::AntiAir if beam(w) => (0, "", "lasrfir1", "lasrhit1"),
        Kind::AntiAir => (4, "", if flak { "canlite2" } else { "cannon1" }, if flak { "xplomed2" } else { "xplosml2" }),
        Kind::Overcharge => (3, "dgun", "disigun1", "xplomas2"),
        Kind::Nuke => (1, "crblmssl", "misicbm1", "xplomas2"),
        Kind::AntiMissile => (1, "amdrocket", "Rockhvy1", "xplomed4"),
    };
    put("rendertype", render.to_string());
    if !model.is_empty() {
        put("model", model.into());
    }
    match k {
        Kind::Laser => {
            put("lineofsight", "1".into());
            put("beamweapon", "1".into());
            let shown = if w.beam_lifetime > 0.0 { w.beam_lifetime } else { fire.reload };
            put("duration", format!("{:.2}", shown.min(fire.reload).clamp(0.05, 1.0)));
            put("color", "180".into());
            put("color2", "181".into());
        }
        Kind::Shell => {
            let arc = !w.ballistic_arc.eq_ignore_ascii_case("RULEUBA_None");
            put("ballistic", if arc { "1" } else { "0" }.into());
            put("lineofsight", if arc { "0" } else { "1" }.into());
        }
        Kind::Artillery => put("ballistic", "1".into()),
        Kind::Missile | Kind::AntiMissile | Kind::Nuke => {
            let strategic = k == Kind::Nuke;
            // A stored missile of a launcher (a nuke, a tactical missile), launched upright: TA's
            // strategic missiles' flight, high over everything and down onto the place it is sent
            // at (no `tracks`: TA's nukes strike a place).
            let launcher = stock && k == Kind::Missile || strategic;
            let upright = launched_upright(w);
            let (start, acceleration) = match shot {
                // (Out of the tube gathering speed as TA's strategic missiles do: `LAUNCH_CLIMB`,
                // `TACTICAL_CLIMB`.)
                _ if launcher => (LAUNCH_START, LAUNCH_ACCELERATION),
                // (A tactical defence's shot leaves its muzzle at full speed, as the Phalanx's
                // rounds and the zapper's beam do: `COUNTERMEASURE_SPEED`.)
                _ if k == Kind::AntiMissile && !upright => (speed, speed),
                Some(s) if s.max_speed > 0.0 => ((s.initial_speed * UNIT_SCALE).round() as i64, ((s.acceleration * UNIT_SCALE).round() as i64).max(60)),
                _ => ((speed / 3).max(60), (speed / 2).max(60)),
            };
            let start = if upright { start.max(LAUNCH_START) } else { start };
            put("lineofsight", "1".into());
            put("selfprop", "1".into());
            put("smoketrail", "1".into());
            put("startvelocity", start.to_string());
            put("weaponacceleration", acceleration.to_string());
            put("guidance", "1".into());
            // (An anti-nuke's missile leaves its tube upright, as TA's do; a tactical defence's
            // shot leaves its turret at what it chases.)
            if upright {
                put("vlaunch", "1".into());
            }
            // (A submarine fires its missiles from under the water, as Forged Alliance's do.)
            if u.motion == Motion::Sub {
                put("firesubmersed", "1".into());
            }
            if launcher {
                put("cruise", "1".into());
                put("weapontimer", if strategic { LAUNCH_CLIMB } else { TACTICAL_CLIMB }.to_string());
                put("turnrate", CRUISE_TURN.to_string());
                // (A tactical missile homes on what it is sent at, as Forged Alliance's do; a nuke
                // strikes the place, as TA's do: `WeaponType::strikes_a_place`.)
                if !strategic && shot.is_some_and(|s| s.tracks) {
                    put("tracks", "1".into());
                }
            } else {
                put("tracks", "1".into());
                // (A missile defence turns as TA's anti-nukes do, half a turn a second at least:
                // the missiles it chases come down steeply from high over it.)
                let least = if k == Kind::AntiMissile { CRUISE_TURN } else { 16000 };
                let turn = shot.map(|s| (s.turn_rate * 65536.0 / 360.0).round() as i64).filter(|&t| t > 0).unwrap_or(least);
                put("turnrate", turn.max(least).to_string());
            }
            let climb = match (launcher, strategic) {
                (false, _) => 0,
                (true, true) => LAUNCH_CLIMB,
                (true, false) => TACTICAL_CLIMB,
            };
            // (A missile defence's shot lives long enough to climb to what it chases, as TA's
            // anti-nukes' a minute: ten seconds at least.)
            let least = if k == Kind::AntiMissile { 10 } else { 0 };
            put("flighttime", (flight_time(range, speed, start, acceleration) + climb).max(least).to_string());
            if (stock || w.manual_fire) && k != Kind::AntiMissile {
                put("commandfire", "1".into());
            }
            if strategic || stock || k == Kind::AntiMissile {
                put("noautorange", "1".into());
            }
            match targetable_class(w) {
                0 => {}
                class => put("targetable", class.to_string()),
            }
            if k == Kind::AntiMissile {
                put("interceptor", interceptor_class(w).to_string());
                put("coverage", px(w.max_radius.max(1.0)).to_string());
            }
        }
        Kind::Torpedo => {
            put("waterweapon", "1".into());
            put("lineofsight", "1".into());
            put("selfprop", "1".into());
            put("tracks", "1".into());
            put("turnrate", "12000".into());
        }
        Kind::Bomb => put("dropped", "1".into()),
        Kind::AntiAir => {
            put("toairweapon", "1".into());
            put("lineofsight", "1".into());
            if homing_aa {
                // Homing on its aircraft as TA's missile towers' do (the engine leads nothing: an
                // unguided shot at a plane's place now misses it), from its projectile's own
                // speeds and turn where it gives them; flak bursts where it meets it.
                let start = shot.map(|s| (s.initial_speed * UNIT_SCALE).round() as i64).filter(|&v| v > 0).unwrap_or(speed).min(speed);
                let acceleration = shot.map(|s| (s.acceleration * UNIT_SCALE).round() as i64).filter(|&a| a > 0).unwrap_or(speed);
                let turn = shot.map(|s| (s.turn_rate * 65536.0 / 360.0).round() as i64).unwrap_or(0).max(AA_TURN);
                put("selfprop", "1".into());
                put("tracks", "1".into());
                put("guidance", "1".into());
                put("startvelocity", start.to_string());
                put("weaponacceleration", acceleration.to_string());
                put("turnrate", turn.to_string());
                put("flighttime", flight_time(range, speed, start, acceleration).to_string());
                if model == "missile" {
                    put("smoketrail", "1".into());
                }
            } else {
                put("beamweapon", "1".into());
                put("duration", format!("{:.2}", w.beam_lifetime.clamp(0.05, 1.0)));
                put("color", "180".into());
                put("color2", "181".into());
            }
        }
        Kind::Overcharge => {
            put("commandfire", "1".into());
            put("lineofsight", "1".into());
            put("noexplode", "1".into());
        }
    }
    put("soundstart", start.into());
    put("soundhit", hit.into());
    put("explosiongaf", "fx".into());
    put("explosionart", explosion(blast).into());
    put("waterexplosiongaf", "fx".into());
    put("waterexplosionart", "h2oboom1".into());
    // TA Reborn's abilities: its minimum range, its stun (`factions::abilities`).
    s.push_str(&super::abilities::weapon_keys(u, w, name));
    let damage = match &w.nuke {
        Some(n) => n.inner_damage,
        None if stock => cycle(w).per_shot,
        None => fire.damage,
    };
    format!("[{}]\n\t{{\n{s}\t[DAMAGE]\n\t\t{{\n\t\tdefault={};\n\t\t}}\n\t}}\n", name, number(damage.max(1.0)))
}

/// A unit's death: its death weapon's section (a blast where it falls), or none.
pub fn death_section(u: &UnitBlueprint, w: &WeaponBlueprint) -> Option<String> {
    blast_section(u, w, "DEATH")
}

/// A kamikaze unit's blast (the Fire Beetle's `Suicide`): what it blows itself up as.
pub fn kamikaze_section(u: &UnitBlueprint, w: &WeaponBlueprint) -> Option<String> {
    blast_section(u, w, "BLAST")
}

/// A blast where the unit is, as a weapon section `<id>_<what>`.
fn blast_section(u: &UnitBlueprint, w: &WeaponBlueprint, what: &str) -> Option<String> {
    let (damage, radius) = match &w.nuke {
        Some(n) => (n.inner_damage, n.outer_radius),
        None => (w.damage, w.damage_radius),
    };
    if damage <= 0.0 {
        return None;
    }
    let blast = (radius * 2.0 * UNIT_SCALE).max(16.0);
    let name = tdf_text(if w.display_name.is_empty() { what } else { &w.display_name });
    Some(format!(
        "[{}_{what}]\n\t{{\n\tname={name};\n\trendertype=4;\n\tballistic=1;\n\tturret=1;\n\trange=480;\n\treloadtime=3.45;\n\tweaponvelocity=250;\n\tareaofeffect={};\n\tedgeeffectiveness=0.5;\n\tsoundhit=xplolrg3;\n\texplosiongaf=fx;\n\texplosionart={};\n\t[DAMAGE]\n\t\t{{\n\t\tdefault={};\n\t\t}}\n\t}}\n",
        u.id.to_ascii_uppercase(),
        blast.round() as i64,
        explosion(blast),
        damage.round() as i64
    ))
}

/// How it builds: none, standing in place (a factory), moving (an engineer, a commander), or
/// standing and lending its build power to what is built round it (an engineering station, TA's
/// assist turret).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builds {
    No,
    Factory,
    Mobile,
    Assist,
}

pub fn builds(u: &UnitBlueprint) -> Builds {
    if u.build_rate <= 0.0 {
        return Builds::No;
    }
    // (An engineering station builds nothing of its own: its drones lend their power round it.)
    if u.motion == Motion::None && u.is("ENGINEERSTATION") {
        return Builds::Assist;
    }
    if build_lists(u).is_empty() {
        return Builds::No;
    }
    // (A building that is no factory builds only its own upgrade in Forged Alliance, on itself:
    // not yet in TA Reborn.)
    match u.motion {
        Motion::None if u.is("FACTORY") => Builds::Factory,
        Motion::None => Builds::No,
        _ => Builds::Mobile,
    }
}

/// What a unit's build lists name that it builds from the start: a commander's own tier (its
/// engineering enhancements, which bring the others, are not yet in TA Reborn), not upgrades of
/// itself.
pub fn build_lists(u: &UnitBlueprint) -> Vec<&str> {
    u.buildable
        .iter()
        .map(String::as_str)
        .filter(|e| {
            let upper = e.to_ascii_uppercase();
            !upper.contains("TIER2COMMANDER") && !upper.contains("TIER3COMMANDER") && !upper.ends_with("UPGRADE") && e.split_whitespace().count() > 1
        })
        .collect()
}

/// Whether it moves on the water or under it (a ship, a submarine).
fn naval(u: &UnitBlueprint) -> bool {
    matches!(u.motion, Motion::Water | Motion::Sub)
}

/// The whole square root of `n`, rounded to the nearest.
fn root(n: i64) -> i64 {
    let s = (n.max(0) as u64).isqrt() as i64;
    if (s + 1) * (s + 1) - n < n - s * s { s + 1 } else { s }
}

/// TA's footprint cells (16 pixels) of the unit's footprint (or its size where it gives none).
pub fn footprint(u: &UnitBlueprint) -> (i64, i64) {
    let [x, z] = u.footprint.unwrap_or([u.size[0].max(0.5), u.size[2].max(0.5)]);
    let cells = |ogrids: f64| ((ogrids * UNIT_SCALE / 16.0).round() as i64).clamp(1, 64);
    // (An extractor covers a metal spot as TA's does, three cells a side: what it gives is the
    // metal under it.)
    if u.is("MASSEXTRACTION") {
        return (cells(x).max(3), cells(z).max(3));
    }
    // (A ship takes a square of its hull's footprint's area, as TA's ships stand on squares less
    // than their length: a frigate 2 by 6 ogrids 7 by 7 cells, a battleship 18.)
    if naval(u) {
        let side = root(cells(x) * cells(z)).max(1);
        return (side, side);
    }
    // (A unit that moves takes a square, as TA's do: it turns where it stands.)
    if u.motion != Motion::None {
        let side = cells(x).max(cells(z));
        return (side, side);
    }
    (cells(x), cells(z))
}

/// A structure's footprint (ogrids) widened to what of its model stands on the ground (`ground`:
/// its extent there, `model::ground_extent`), where that is wider than its blueprint's: none where
/// it is not. Forged Alliance's footprints are what its units may not cross, smaller than many of
/// its buildings (a point defence's half an ogrid against its turret's whole one), the ground round
/// them levelled for them and kept clear (their skirts). TA Reborn levels the ground drawn under a
/// footprint, and a model reaching past its footprint ran into the hillside beside it (the
/// placement's grid smaller than the building, as well, and its neighbours built into it). A wall
/// keeps its own, to stand in line with the next; a building on the water or the sea bed alone
/// meets no hillside.
pub fn fitted_footprint(u: &UnitBlueprint, ground: Option<[f64; 2]>) -> Option<[f64; 2]> {
    let ground = ground?;
    if u.motion != Motion::None || !u.layers.land || u.is("WALL") {
        return None;
    }
    let [x, z] = u.footprint.unwrap_or([u.size[0].max(0.5), u.size[2].max(0.5)]);
    let fitted = [x.max(ground[0]), z.max(ground[1])];
    (fitted != [x, z]).then_some(fitted)
}

/// A factory's yard: a corridor of cells open while it works, from the middle to its front edge
/// (TA's buildings face +z, the yard map's last rows), the rest solid.
pub fn yard_map(fx: i64, fz: i64) -> String {
    let half = (fx / 4).max(1);
    let (from, to) = (fx / 2 - half, fx / 2 + half);
    let mut s = String::new();
    for z in 0..fz {
        for x in 0..fx {
            s.push(if x >= from && x < to && z >= fz / 2 - 1 { 'c' } else { 'o' });
        }
    }
    s
}

/// What its upkeep (`MaintenanceConsumptionPerSecondEnergy`) pays for that TA Reborn plays: an
/// extractor's or a fabricator's work, or an intelligence unit's radar, sonar or omni (radar and
/// sonar stations, sonar platforms). Shields, stealth fields, a unit's own stealth and cloak (TA's
/// stealth costs nothing), jamming and remote viewing are not in TA Reborn yet: their upkeep is
/// left out with them (the Fatboy's shield, the Monkeylord's stealth, a frigate's jamming: a
/// counter-intelligence unit's upkeep is for those, its radar free).
fn upkeep_pays(u: &UnitBlueprint) -> bool {
    let watches = (u.intel.radar > 0.0 && !u.birth.intel_off("Radar")) || (u.intel.sonar > 0.0 && !u.birth.intel_off("Sonar"));
    u.is("MASSEXTRACTION") || u.production_mass > 0.0 || watches && u.is("INTELLIGENCE") && (u.is("RADAR") || u.is("SONAR") || u.is("OMNI")) && !u.is("COUNTERINTELLIGENCE")
        // (A jammer's, a stealth field generator's: `factions::abilities`.)
        || super::abilities::upkeep_pays(u)
}

/// Whether its file has it switched on once built (`ActivateWhenBuilt`: TA's engine runs its
/// script's `Activate` then), as [`unit_file`] writes it: an extractor, a fabricator, what pays an
/// upkeep for its radar or sonar, an air staging platform. (Its `Deactivate` runs as its player
/// switches it off, where it has a switch.)
pub fn activated_when_built(u: &UnitBlueprint) -> bool {
    let upkeep = if upkeep_pays(u) { u.upkeep_energy } else { 0.0 };
    u.is("MASSEXTRACTION") || upkeep > 0.0 || u.is("AIRSTAGINGPLATFORM") && builds(u) == Builds::No && u.motion == Motion::None
}

/// An aircraft's movement (`Air`): its top speed (px a tick), its acceleration and braking (px a
/// tick squared) and its turn (TA's angle units a tick), as Forged Alliance flies it, at TA's scale.
///
/// - **Speed:** `MaxAirspeed`.
/// - **Acceleration:** to full speed in a second and a half times its `KMoveDamping` over its
///   `KMove` (Forged Alliance's thrust against its damping): an interceptor in 1.5 s (0.36 px a
///   tick², TA's Freedom Fighter's 0.35), a bomber in 3 (TA's Thunder's 3.8), a gunship in 3.75,
///   a transport in 3.2. Braking ten times that, as TA's aircraft brake many times faster than
///   they gather speed.
/// - **Turn:** a winged one's `TurnSpeed` (radians a second: an interceptor's 1.5, 86° a second,
///   TA's Freedom Fighter's 84°; a bomber's 0.7, 40°); a hovering one (gunships, transports, the
///   CZAR) its `Physics.TurnRate` where it gives one, else its `KTurn` as twice that.
pub fn flight(u: &UnitBlueprint) -> (f64, f64, f64, f64) {
    let air = |key: &str| u.table.path(&["Air"]).and_then(|a| a.get_ci(key));
    let num = |key: &str| air(key).and_then(crate::lua::Value::num);
    let speed = u.max_airspeed.unwrap_or(u.max_speed).max(0.1) * UNIT_SCALE / TICKS;
    let thrust = num("KMove").filter(|&k| k > 0.0).unwrap_or(1.0);
    let damping = num("KMoveDamping").filter(|&k| k > 0.0).unwrap_or(1.0);
    let acceleration = speed * thrust / (damping * 1.5 * TICKS);
    let brake = acceleration * 10.0;
    let degrees = |radians: f64| radians * 180.0 / std::f64::consts::PI;
    let turn = if winged(u) && let Some(t) = num("TurnSpeed").filter(|&t| t > 0.0) {
        degrees(t)
    } else if u.turn_rate > 0.0 {
        u.turn_rate
    } else if let Some(k) = num("KTurn").filter(|&k| k > 0.0) {
        degrees(k) * 2.0
    } else {
        60.0
    };
    (speed, acceleration, brake, turn * 65536.0 / 360.0 / TICKS)
}

/// Whether Forged Alliance flies it with wings (`Air.Winged`: its fighters, bombers and scouts),
/// making passes, rather than hovering (gunships, transports, the CZAR).
fn winged(u: &UnitBlueprint) -> bool {
    u.table.path(&["Air"]).and_then(|a| a.get_ci("Winged")).and_then(crate::lua::Value::bool).unwrap_or(false)
}

/// What a unit's file says that its own blueprint does not tell alone: what it upgrades into in
/// place (the unit its `General.UpgradesTo` names, of its faction and made), its enhancements'
/// section (`factions::enhance::section`), and its adjacency: the buffs it gives the structures
/// beside it (its `Adjacency` table's, as the game's file has them: `crate::adjacency`) and the
/// categories buffs name besides the size classes (`ARTILLERY`), which it carries in its
/// `Category` where it has them.
#[derive(Clone, Debug, Default)]
pub struct Extra<'a> {
    pub upgrades_to: Option<&'a str>,
    pub enhancements: String,
    /// The weapons its enhancements switch on, in the slots after its own
    /// (`factions::enhance::extra_weapons`): what of theirs holds for a slot whatever fills it.
    pub weapons: Vec<&'a WeaponBlueprint>,
    /// A factory's ways out ([`exits`]: they need its mesh).
    pub exits: Vec<Exit>,
    pub adjacency: &'a [crate::adjacency::Buff],
    pub adjacency_categories: Option<&'a std::collections::BTreeSet<String>>,
    /// The name it is shown by ([`shown_name`]); empty: its own, else its description.
    pub name: String,
    /// Its weapon toggle (`factions::abilities::toggle`: its slots after its own and those its
    /// enhancements fill).
    pub toggle: Option<super::abilities::Toggle<'a>>,
}

/// The name a unit is shown by: its own, else its description (the commanders' and most
/// buildings' and engineers'), else its id; where another unit of its faction of another tier would
/// be shown by the same (Forged Alliance tells its tiers apart by their pictures' frames, TA's menus
/// and cards by name), its tier before it, as Forged Alliance's players say it: "T2 Engineer",
/// "T3 Land Factory" (a name's article giving way to it: "T2 Kennel", not "T2 The Kennel").
/// Units of one tier sharing a name keep it (an upgrade's step tells those apart in the game).
pub fn shown_name(u: &UnitBlueprint, faction: &[&UnitBlueprint]) -> String {
    let own = |u: &UnitBlueprint| -> String {
        if !u.name.is_empty() { u.name.clone() } else if !u.description.is_empty() { u.description.clone() } else { u.id.to_ascii_uppercase() }
    };
    let name = own(u);
    let shared = faction.iter().any(|o| o.id != u.id && o.tech != u.tech && own(o).eq_ignore_ascii_case(&name));
    let bare = name.strip_prefix("The ").filter(|rest| !rest.trim().is_empty()).unwrap_or(&name).to_string();
    match u.tech {
        Some(Tech::T1) if shared => format!("T1 {bare}"),
        Some(Tech::T2) if shared => format!("T2 {bare}"),
        Some(Tech::T3) if shared => format!("T3 {bare}"),
        _ => name,
    }
}

/// One of a factory's ways out, for its unit file's `[EXITS]` (docs/MODDING.md; `ta_data`'s
/// `ExitDef`): where it leads (px from the factory's middle, the factory facing south), the
/// heading a unit made for it takes (degrees, as TA counts headings), and its ramp (px along the
/// way from the pad: the pad's height kept to the first, the ground reached at the second).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit {
    pub x: i64,
    pub z: i64,
    pub heading: i64,
    pub ramp: (i64, i64),
}

/// How far past a factory's footprint (px) a way out leads at least: TA's plants send their units
/// as far past their front (`Sim::factory_exit`), and a unit is let go once it is out of the
/// footprint (the factory's walls are its way out until then).
pub const CLEAR: f64 = 48.0;

/// How near (px) a way's floor must stand to the pad's height to count as level with it, and to
/// the ground to count as down.
const LEVEL: f64 = 1.5;

/// A factory's ways out ([`Exit`]), from its roll-off points (`Physics.RollOffPoints`, ogrids from
/// its middle; Forged Alliance sends a unit to the one nearest its rally point, and turns its build
/// bone for it by its `UnitSpin` as it is built, `FactoryUnit:CalculateRollOffPoint` and
/// `CreateBuildRotator`):
///
/// - **Where:** the point, as far out along the way from the pad (its `BuildAttachBone`) as clears
///   the factory's footprint by [`CLEAR`] at least: in TA Reborn a unit is let go once out of the
///   footprint, which a fitted one (`fitted_footprint`) may reach past Forged Alliance's points.
/// - **Heading:** the build bone's own at rest (the Cybran quantum gateway's rests turned half
///   round) and the point's `UnitSpin` (by the right hand about y: 90 towards +x, as TA's headings).
/// - **Ramp:** as far along the way as the floor of its model (under the pad's height: its build
///   platform; `model::floor_under`, the floats `left_out`) stands level with the pad, and where it
///   has come down to the ground or the model ends: the unit keeps the platform's height that far
///   and comes down its ramp, rather than dropping through it.
///
/// Integer roots and plain arithmetic: the same on every computer.
pub fn exits(u: &UnitBlueprint, mesh: &crate::scm::Mesh, left_out: &[usize]) -> Vec<Exit> {
    if u.roll_off.is_empty() || builds(u) != Builds::Factory {
        return Vec::new();
    }
    let px = u.display.uniform_scale * UNIT_SCALE;
    if px <= 0.0 {
        return Vec::new();
    }
    let rest = mesh.rest_poses();
    let bone = u.display.build_attach_bone.as_deref().and_then(|b| mesh.bone(b));
    let pad = bone.map_or([0.0; 3], |b| rest[b].0.map(|v| v as f64 * px));
    let rest_heading = bone.map_or(0, |b| super::script::rest_aim(mesh, b).heading as i64);
    let (fx, fz) = footprint(u);
    let half = [fx as f64 * 8.0 + CLEAR, fz as f64 * 8.0 + CLEAR];
    u.roll_off
        .iter()
        .map(|p| {
            let mut end = [p.x * UNIT_SCALE, p.z * UNIT_SCALE];
            let dir = [end[0] - pad[0], end[1] - pad[2]];
            // (Out along the way to the edge of the footprint cleared, where the point is short of it.)
            if end[0].abs() < half[0] && end[1].abs() < half[1] {
                let from = [pad[0], pad[2]];
                let reach = (0..2).filter(|&a| dir[a] != 0.0).map(|a| (half[a] * dir[a].signum() - from[a]) / dir[a]).fold(f64::INFINITY, f64::min);
                if reach.is_finite() && reach > 1.0 {
                    end = [from[0] + dir[0] * reach, from[1] + dir[1] * reach];
                }
            }
            let end = [end[0].round(), end[1].round()];
            let way = [end[0] - pad[0], end[1] - pad[2]];
            let length = ((way[0] * way[0] + way[1] * way[1]).round() as i64).max(0) as u64;
            let length = length.isqrt() as i64;
            // The floor under the way, a pixel at a time, under the pad's height.
            let ramp = if pad[1] <= LEVEL || length == 0 {
                (0, 0)
            } else {
                let points: Vec<[f64; 2]> = (0..=length).map(|s| [(pad[0] + way[0] * s as f64 / length as f64) / px, (pad[2] + way[1] * s as f64 / length as f64) / px]).collect();
                let floor: Vec<Option<f64>> = super::model::floor_under(mesh, left_out, &points, (pad[1] + LEVEL) / px).into_iter().map(|h| h.map(|h| h * px)).collect();
                let level = floor.iter().take_while(|h| h.is_some_and(|h| h >= pad[1] - LEVEL)).count() as i64;
                let top = (level - 1).max(0);
                let foot = (top..=length).find(|&s| floor[s as usize].is_none_or(|h| h <= LEVEL)).unwrap_or(length);
                (top, foot.max(top))
            };
            let spin = (p.spin * 65536.0 / 360.0).round() as i64;
            let heading = (rest_heading + spin).rem_euclid(65536);
            // (Degrees, whole, from -180 to 180.)
            let degrees = (heading * 360 + 32768).div_euclid(65536);
            let degrees = if degrees > 180 { degrees - 360 } else { degrees };
            Exit { x: end[0] as i64, z: end[1] as i64, heading: degrees, ramp }
        })
        .collect()
}

/// A factory's `[EXITS]` section (within its `[UNITINFO]`), or nothing.
pub fn exits_section(exits: &[Exit]) -> String {
    if exits.is_empty() {
        return String::new();
    }
    let mut s = String::from("\t[EXITS]\n\t\t{\n");
    for (k, e) in exits.iter().enumerate() {
        let _ = write!(s, "\t\t[EXIT{}]\n\t\t\t{{\n\t\t\tX={};\n\t\t\tZ={};\n\t\t\tHeading={};\n\t\t\tRamp={} {};\n\t\t\t}}\n", k + 1, e.x, e.z, e.heading, e.ramp.0, e.ramp.1);
    }
    s.push_str("\t\t}\n");
    s
}

/// An engineering station's build power and reach (ogrids): its own `BuildRate`, or its drones'
/// together where they have more (the tech 3 Kennel's two), and its `MaxBuildDistance`, or how far
/// round it its drones look for work (`AI.GuardScanRadius`), or five ogrids. TA Reborn's station
/// builds with its own beams: its drones are not made (the Kennel's), their work its own.
pub fn station_work(u: &UnitBlueprint) -> (f64, f64) {
    let pods: f64 = u.pods.iter().map(|p| p.build_rate.max(0.0)).sum();
    let own_reach = u.table.path(&["Economy"]).and_then(|e| e.get_ci("MaxBuildDistance")).and_then(|v| v.num());
    let pod_reach = u.pods.iter().map(|p| p.reach).fold(0.0f64, f64::max);
    (u.build_rate.max(pods), own_reach.or((pod_reach > 0.0).then_some(pod_reach)).unwrap_or(5.0))
}

/// The `[ADJACENCY]` section of a unit's file: each buff it gives the structures beside it, by
/// name, what it changes in them (`Affects`), by how much (`Add`), and which take it (`Size`, their
/// size class; `Category`, what else they must have).
pub fn adjacency_section(buffs: &[crate::adjacency::Buff]) -> String {
    if buffs.is_empty() {
        return String::new();
    }
    let mut s = String::from("\t[ADJACENCY]\n\t\t{\n");
    for b in buffs {
        let _ = writeln!(s, "\t\t[{}]\n\t\t\t{{\n\t\t\tAffects={};\n\t\t\tAdd={};", tdf_text(&b.name), b.affect.key(), b.add);
        if let Some(size) = b.size {
            let _ = writeln!(s, "\t\t\tSize={size};");
        }
        if !b.categories.is_empty() {
            let _ = writeln!(s, "\t\t\tCategory={};", b.categories.join(" "));
        }
        s.push_str("\t\t\t}\n");
    }
    s.push_str("\t\t}\n");
    s
}

/// The unit's file.
pub fn unit_file(u: &UnitBlueprint, weapons: &Weapons, carries: Option<(u32, u32)>, extra: &Extra) -> String {
    let id = u.id.to_ascii_uppercase();
    let (fx, fz) = footprint(u);
    let side = side(&u.faction);
    let builds = builds(u);
    let walks = u.display.walk.is_some();
    let mut f = String::new();
    let mut put = |k: &str, v: String| {
        let _ = writeln!(f, "\t{k}={v};");
    };
    put("UnitName", id.clone());
    put("Version", "1".into());
    put("Side", side.into());
    put("Objectname", u.id.to_ascii_lowercase());
    // (The commanders have no name of their own: their description. Its tier with it where its
    // faction has another of that name: `shown_name`.)
    let named = if !extra.name.is_empty() { &extra.name } else if !u.name.is_empty() { &u.name } else if !u.description.is_empty() { &u.description } else { &id };
    put("Name", tdf_text(named));
    put("Description", tdf_text(&u.description));
    put("FootprintX", fx.to_string());
    put("FootprintZ", fz.to_string());
    put("MaxDamage", (u.health.round().max(1.0) as i64).to_string());
    put("BuildCostMetal", (u.cost_mass.round() as i64).to_string());
    put("BuildCostEnergy", (u.cost_energy.round() as i64).to_string());
    put("BuildTime", ((u.build_time * WORK).round().max(1.0) as i64).to_string());
    put("SightDistance", ((u.intel.vision.max(u.intel.water_vision) * UNIT_SCALE).round() as i64).to_string());
    // Intel, as it is once made (the Cybran commander's sonar and stealth come with enhancements).
    if u.intel.radar > 0.0 && !u.birth.intel_off("Radar") {
        put("RadarDistance", ((u.intel.radar * UNIT_SCALE).round() as i64).to_string());
    }
    if u.intel.sonar > 0.0 && !u.birth.intel_off("Sonar") {
        put("SonarDistance", ((u.intel.sonar * UNIT_SCALE).round() as i64).to_string());
    }
    if u.intel.radar_stealth && !u.birth.intel_off("RadarStealth") {
        put("Stealth", "1".into());
    }
    // Jamming, stealth fields and remote viewing (`factions::abilities`).
    for (k, v) in super::abilities::unit_keys(u) {
        put(k, v);
    }
    // (Self-repair: TA Reborn mends 100 hit points every `HealTime` seconds.)
    if u.regen > 0.0 {
        put("HealTime", format!("{:.2}", 100.0 / u.regen));
    }
    // Economy.
    let upkeep = if upkeep_pays(u) { u.upkeep_energy } else { 0.0 };
    if u.production_energy > 0.0 {
        put("EnergyMake", format!("{}", u.production_energy));
    }
    if upkeep > 0.0 {
        put("EnergyUse", format!("{upkeep}"));
    }
    if u.storage_energy > 0.0 {
        put("EnergyStorage", (u.storage_energy.round() as i64).to_string());
    }
    if u.storage_mass > 0.0 {
        put("MetalStorage", (u.storage_mass.round() as i64).to_string());
    }
    if u.is("MASSEXTRACTION") {
        // TA's extractor gives about 2 metal a second on its spots: Forged Alliance's T1 gives 2,
        // its T2 6, its T3 18.
        let rate = match u.tech {
            Some(Tech::T2) => 0.003,
            Some(Tech::T3) => 0.009,
            _ => 0.001,
        };
        put("ExtractsMetal", format!("{rate}"));
        // (As TA's: working once built, its upkeep paid while it works.)
        put("ActivateWhenBuilt", "1".into());
        put("onoffable", "1".into());
    } else if u.production_mass > 0.0 {
        if upkeep > 0.0 {
            // A fabricator: energy made mass while it works.
            put("MakesMetal", format!("{}", u.production_mass));
            put("OnOffable", "1".into());
            put("ActivateWhenBuilt", "1".into());
        } else {
            put("MetalMake", format!("{}", u.production_mass));
        }
    } else if upkeep > 0.0 {
        // Kept working at an energy cost (radar, sonar): as TA's, on once built and switchable,
        // the upkeep paid while on.
        put("ActivateWhenBuilt", "1".into());
        put("OnOffable", "1".into());
    }
    // A hydrocarbon plant stands on a vent (the maps' hydrocarbon points), as TA's geothermal
    // plants do: every cell of its yard one that may.
    if u.is("HYDROCARBON") && u.motion == Motion::None {
        put("YardMap", "G".repeat((fx * fz).max(1) as usize));
    }
    // Building.
    if builds != Builds::No {
        // (An engineering station by its drones' work where they have more, and their reach.)
        let (rate, reach) = if builds == Builds::Assist {
            station_work(u)
        } else {
            (u.build_rate, u.table.path(&["Economy"]).and_then(|e| e.get_ci("MaxBuildDistance")).and_then(|v| v.num()).unwrap_or(5.0))
        };
        put("Builder", "1".into());
        put("WorkerTime", ((rate * WORK).round().max(1.0) as i64).to_string());
        put("BuildDistance", ((reach * UNIT_SCALE).round() as i64).max(64).to_string());
        put("CanReclamate", "1".into());
        // (A factory with ways out is solid, as Forged Alliance's are: its units leave by them,
        // through its walls as TA Reborn has a factory's units leave, and no other walks over
        // its platform; TA's open their yards to the front.)
        if builds == Builds::Factory {
            put("YardMap", if extra.exits.is_empty() { yard_map(fx, fz) } else { "o".repeat((fx * fz).max(1) as usize) });
        }
        if builds == Builds::Assist {
            // An engineering station works round it by itself whenever it has no orders, as
            // Forged Alliance's do: it helps build what is going up within its reach, mends the
            // damaged and reclaims wreckage (`AssistPatrol`, docs/MODDING.md). Told to help a
            // unit where Forged Alliance lets it (`CommandCaps`' guard: the tech 3 Kennel, the
            // Hives).
            put("DefaultMissionType", "AssistPatrol".into());
            let guards = u.table.path(&["General", "CommandCaps"]).and_then(|c| c.get_ci("RULEUCC_Guard")).and_then(crate::lua::Value::bool).unwrap_or(false);
            if guards {
                put("canguard", "1".into());
            }
        }
    }
    // Built facing a way of its own on a factory with ways out, whichever it takes (its
    // `ForcedBuildSpin`, the turn its air factory's build bone is given for it: those bones all rest
    // facing their factory's front, so it is a heading from there, docs/MODDING.md).
    if let Some(spin) = u.display.forced_build_spin {
        put("BuildHeading", (spin.round() as i64).to_string());
    }
    // Upgrading in place (`UpgradesTo`): on its own build power, which a building that builds
    // nothing else is given for it (an extractor's, a radar's: Forged Alliance's `BuildRate`).
    if let Some(to) = extra.upgrades_to {
        put("UpgradesTo", to.to_ascii_uppercase());
        if builds == Builds::No {
            put("WorkerTime", ((u.build_rate * WORK).round().max(1.0) as i64).to_string());
        }
    }
    // An air staging platform repairs the aircraft landed on it, as TA's repair pads do (their
    // 200 of work a second; no builder: the Cybran cruiser that is one stays a warship).
    if u.is("AIRSTAGINGPLATFORM") && builds == Builds::No {
        put("WorkerTime", "200".into());
        put("IsAirBase", "1".into());
        if u.motion == Motion::None {
            put("ActivateWhenBuilt", "1".into());
            put("onoffable", "1".into());
        }
    }
    let commander = u.is("COMMAND");
    if commander {
        put("Commander", "1".into());
        put("ShowPlayerName", "1".into());
        put("cancapture", "1".into());
    }
    // Weapons.
    for (slot, w) in weapons.slots.iter().enumerate() {
        put(&format!("Weapon{}", slot + 1), weapon_name(u, w));
    }
    // A weapon that shoots no aircraft in Forged Alliance takes them last in TA Reborn (its bad
    // targets, TA's rule: an anti-air weapon, a bomb, a shell or a torpedo cannot hit them at all).
    let mut chases_air = false;
    for (slot, w) in weapons.slots.iter().enumerate() {
        let k = kind(w);
        let hits_air_in_ta = matches!(k, Kind::Laser | Kind::Missile | Kind::Overcharge) || k == Kind::Shell && w.ballistic_arc.eq_ignore_ascii_case("RULEUBA_None");
        if shoots_air(w) || k == Kind::AntiAir {
            chases_air = true;
        } else if hits_air_in_ta {
            let key = match slot {
                0 => "wpri_badTargetCategory".to_string(),
                1 => "wsec_badTargetCategory".to_string(),
                2 => "wspe_badTargetCategory".to_string(),
                n => format!("BadTargetCategory{}", n + 1),
            };
            put(&key, "VTOL".into());
        }
    }
    if !weapons.slots.is_empty() && !chases_air {
        put("NoChaseCategory", "VTOL".into());
    }
    // Anti-air that takes on aircraft alone in Forged Alliance ([`aircraft_alone`]) takes none but
    // aircraft on, by order too (`OnlyTargetCategory<n>`: an anti-air tower or an interceptor told
    // to attack a tank or the ground does not; `toairweapon` alone keeps TA's rule, its own choice
    // of targets aircraft, an order firing on anything). Its own slots and those its enhancements
    // fill after them (the Cybran support commander's anti-air missiles).
    for (slot, w) in weapons.slots.iter().chain(&extra.weapons).take(MAX_WEAPONS).enumerate() {
        if aircraft_alone(w) {
            put(&format!("OnlyTargetCategory{}", slot + 1), "VTOL".into());
        }
    }
    // (Its weapon toggle's slots take theirs: `factions::abilities`.)
    let toggle = extra.toggle.as_ref().map(|t| super::abilities::toggle_section(u, t));
    for (k, v) in toggle.iter().flat_map(|(_, keys)| keys) {
        put(k, v.clone());
    }
    if weapons.dgun.is_some() {
        put("canDgun", "1".into());
    }
    if weapons.slots.iter().any(|w| stockpiled(w)) {
        put("onoffable", "0".into());
    }
    let blast = |what: &str| format!("{id}_{what}");
    match (weapons.kamikaze.and_then(|w| kamikaze_section(u, w)), weapons.death.and_then(|w| death_section(u, w))) {
        (Some(_), death) => {
            // A crawling bomb, as TA's are made: it blows itself up as its blast within half its
            // blast's reach of an enemy, so the blast takes it in (TA's crawling bombs at 40 and
            // 80 px, their blasts 278 across; at the blast's full reach the enemy stood just
            // outside it).
            let reach = weapons.kamikaze.map_or(1.0, |w| w.damage_radius.max(1.0));
            put("Kamikaze", "1".into());
            put("KamikazeDistance", (((reach * UNIT_SCALE / 2.0).round() as i64).max(16)).to_string());
            put("ExplodeAs", if death.is_some() { blast("DEATH") } else { blast("BLAST") });
            put("SelfDestructAs", blast("BLAST"));
        }
        (None, Some(_)) => {
            put("ExplodeAs", blast("DEATH"));
            put("SelfDestructAs", blast("DEATH"));
        }
        (None, None) => {
            let big = fx * fz >= 16;
            put("ExplodeAs", if big { "BIG_UNITEX" } else { "SMALL_UNITEX" }.into());
            put("SelfDestructAs", if big { "BIG_UNIT" } else { "SMALL_UNIT" }.into());
        }
    }
    if super::faction::leaves_wreck(u) {
        put("Corpse", format!("{}_dead", u.id.to_ascii_lowercase()));
    }
    // Kinds, for targeting, the computer players and the menus.
    let level = match u.tech {
        Some(Tech::T2) => "LEVEL2",
        Some(Tech::T3) | Some(Tech::Experimental) => "LEVEL3",
        _ => "LEVEL1",
    };
    let air = u.motion == Motion::Air;
    let mut cats: Vec<String> = vec![side.into(), level.into(), if air { "VTOL" } else { "NOTAIR" }.into()];
    if u.motion != Motion::Sub {
        cats.push("NOTSUB".into());
    }
    if !weapons.slots.is_empty() {
        cats.push("WEAPON".into());
    }
    if commander {
        cats.push("COMMANDER".into());
    }
    if weapons.kamikaze.is_some() {
        cats.push("KAMIKAZE".into());
    }
    if u.motion == Motion::None && weapons.slots.is_empty() {
        cats.push("NOWEAPON".into());
    }
    // (Of a structure, the categories the adjacency buffs name that it has, `ARTILLERY`: a
    // neighbour's buff for them reaches it.)
    if u.motion == Motion::None
        && let Some(words) = extra.adjacency_categories
    {
        for c in &u.categories {
            let c = c.to_ascii_uppercase();
            if words.contains(&c) && !cats.contains(&c) {
                cats.push(c);
            }
        }
    }
    // (What stuns spare: `factions::abilities::categories`.)
    cats.extend(super::abilities::categories(u).into_iter().map(String::from));
    put("Category", cats.join(" "));
    // Its size class among structures, for adjacency: which of a neighbour's buffs it takes, and
    // from how many neighbours at most (as many of the smallest structures as fit round it).
    if u.motion == Motion::None
        && let Some(size) = u.size_class()
    {
        put("AdjacencySize", size.to_string());
    }
    let ted = match (u.motion, builds) {
        _ if commander => "COMMANDER",
        (Motion::None, Builds::Factory) => "PLANT",
        (Motion::None, _) if u.is("MASSEXTRACTION") || u.is("MASSFABRICATION") => "METAL",
        (Motion::None, _) if u.is("ENERGYPRODUCTION") || u.is("ENERGYSTORAGE") => "ENERGY",
        (Motion::None, _) if !weapons.slots.is_empty() => "FORT",
        (Motion::None, _) => "SPECIAL",
        (Motion::Air, _) => "VTOL",
        (Motion::Water, _) => "SHIP",
        (Motion::Sub, _) => "WATER",
        (_, Builds::Mobile) => "CNSTR",
        _ if walks => "KBOT",
        _ => "TANK",
    };
    put("TEDClass", ted.into());
    put("SoundCategory", sound_category(u, ted).into());
    put("ThreeD", "1".into());
    put("ShootMe", "1".into());
    if !weapons.slots.is_empty() || weapons.kamikaze.is_some() {
        put("canattack", "1".into());
        put("firestandorders", "1".into());
        put("StandingFireOrder", "2".into());
    }
    // Movement.
    if u.motion != Motion::None {
        if air {
            let (speed, acceleration, brake, turn) = flight(u);
            put("MaxVelocity", format!("{speed:.3}"));
            put("Acceleration", format!("{acceleration:.4}"));
            put("BrakeRate", format!("{brake:.4}"));
            put("TurnRate", (turn.round().max(1.0) as i64).to_string());
        } else {
            put("MaxVelocity", format!("{:.3}", u.max_speed * UNIT_SCALE / TICKS));
            put("Acceleration", format!("{:.4}", (u.acceleration * UNIT_SCALE / 900.0).max(0.01)));
            put("BrakeRate", format!("{:.4}", (u.brake.max(u.acceleration) * UNIT_SCALE / 900.0).max(0.01)));
            // (Forged Alliance's own turn, however slow: the Atlantis's 12° a second.)
            let degrees = if u.turn_rate > 0.0 { u.turn_rate } else { 90.0 };
            put("TurnRate", ((degrees * 65536.0 / 360.0 / TICKS).round().max(1.0) as i64).to_string());
        }
        put("BMcode", "1".into());
        for k in ["canmove", "canpatrol", "canguard", "canstop", "mobilestandorders"] {
            put(k, "1".into());
        }
        put("StandingMoveOrder", "1".into());
        put("maneuverleashlength", "640".into());
        put("DefaultMissionType", "Standby".into());
        if walks {
            put("Upright", "1".into());
        }
        if let Some(class) = movement_class(u, fx) {
            put("MovementClass", class.into());
        }
        match u.motion {
            Motion::Hover => put("canhover", "1".into()),
            Motion::Water => {
                put("Floater", "1".into());
                put("MinWaterDepth", "6".into());
            }
            Motion::Sub => {
                put("MinWaterDepth", "20".into());
                put("WaterLine", "20".into());
                put("Upright", "1".into());
            }
            // (No class of TA's lets a unit down past 100: an amphibian goes by its own limits,
            // to the deepest sea bed.)
            Motion::Amphibious => {
                put("MaxWaterDepth", "255".into());
                put("MaxSlope", if walks { "32" } else { "15" }.into());
            }
            Motion::Air => {
                put("canfly", "1".into());
                put("cruisealt", ((u.elevation.unwrap_or(18.0) * 8.0).clamp(60.0, 250.0).round() as i64).to_string());
                if u.is("TRANSPORTATION")
                    && let Some((units, size)) = carries
                {
                    put("canload", "1".into());
                    put("TransportCapacity", units.to_string());
                    put("TransportMaxUnits", units.to_string());
                    put("TransportSize", size.to_string());
                }
                // Gunships hover over what they attack; Forged Alliance's other hovering craft
                // with guns (the CZAR) as well. Its fighters and bombers make passes.
                if u.is("GROUNDATTACK") || !winged(u) && !weapons.slots.is_empty() {
                    put("HoverAttack", "1".into());
                }
            }
            _ => {}
        }
    } else {
        put("BMcode", "0".into());
        // (As steep as TA's own buildings stand on: Forged Alliance levels the ground under its
        // buildings, TA Reborn the ground drawn under a footprint, a slope taken up round it.)
        put("MaxSlope", "10".into());
        // On the water (floating), under it (on the sea bed, or below the surface: extractors and
        // power plants on deposits there too), or on land.
        let under = u.layers.seabed || u.layers.sub;
        put("MaxWaterDepth", if u.layers.water || under { "255" } else { "0" }.into());
        if !u.layers.land {
            if u.layers.water {
                put("MinWaterDepth", "5".into());
                put("Floater", "1".into());
            } else if under {
                put("MinWaterDepth", "15".into());
            }
        }
    }
    let adjacency = if u.motion == Motion::None { adjacency_section(extra.adjacency) } else { String::new() };
    let toggle = toggle.map(|(section, _)| section).unwrap_or_default();
    format!("[UNITINFO]\n\t{{\n{f}{}{}{adjacency}{toggle}\t}}\n", extra.enhancements, exits_section(&extra.exits))
}

/// TA's sound category for the unit's kind (TA's own sounds, `gamedata/sound.tdf`'s: Forged
/// Alliance's are not read); the Cybran's and the Seraphim's the Core's.
fn sound_category(u: &UnitBlueprint, ted: &str) -> &'static str {
    let core = matches!(u.faction, Faction::Cybran | Faction::Seraphim);
    let pick = |arm: &'static str, cor: &'static str| if core { cor } else { arm };
    match ted {
        "COMMANDER" => pick("ARM_COM", "CORE_COM"),
        "PLANT" if u.is("AIR") => "AIRPLANT",
        "PLANT" if u.is("NAVAL") => "SHIPYARD",
        "PLANT" => "TANKPLANT",
        "VTOL" => pick("ARM_VTOL", "CORE_VTOL"),
        "SHIP" => pick("ARM_SHIP", "CORE_SHIP"),
        "WATER" => pick("ARM_SUB", "CORE_SUB"),
        "KBOT" => pick("ARM_KBOT", "COR_KBOT"),
        "TANK" => pick("ARM_TANK", "CORE_TANK"),
        "CNSTR" => pick("ARM_CVEHICLE", "CORE_CVEHICLE"),
        "METAL" => pick("ARM_MEX", "COR_MEX"),
        "ENERGY" => pick("ARM_SOLAR", "CORE_SOLAR"),
        "FORT" => "LLT",
        "SPECIAL" => pick("ARM_RADAR", "CORE_RADAR"),
        _ => pick("ARM_ESTOR", "CORE_ESTOR"),
    }
}

/// TA's movement class for the unit, by how it moves and its footprint (none for an amphibian:
/// it goes by its own depth and slope, deeper than TA's amphibious classes' 100).
pub fn movement_class(u: &UnitBlueprint, cells: i64) -> Option<&'static str> {
    Some(match u.motion {
        Motion::None | Motion::Air | Motion::Amphibious => return None,
        Motion::Water => {
            if cells <= 4 {
                "BOATS4"
            } else {
                "BOATS6"
            }
        }
        Motion::Sub => "BOATD3",
        Motion::Hover => {
            if cells <= 3 {
                "TANKHOVER3"
            } else {
                "TANKHOVER4"
            }
        }
        Motion::Land => match (u.display.walk.is_some(), cells) {
            (true, ..=2) => "KBOTSS2",
            (false, ..=2) => "TANKSH2",
            _ => "TANKSH3",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectile::ProjectileBlueprint;
    use crate::unit::{self, Rack};

    /// Where a faction has units of different tiers by one name, each is shown with its tier (and
    /// its wreck by the same name, never its id); a name of its own, or one alone of its name, as
    /// it is.
    #[test]
    fn tiers_sharing_a_name_are_told_apart() {
        let bp = |id: &str, tech: &str, general: &str| unit::parse(id, &format!(
            "UnitBlueprint {{ Categories = {{ 'UEF', 'MOBILE', 'LAND', '{tech}', 'ENGINEER' }}, Description = 'Engineer',              Defense = {{ Health = 100, MaxHealth = 100 }}, Economy = {{ BuildCostEnergy = 100, BuildCostMass = 50, BuildTime = 100 }},              General = {{ FactionName = 'UEF'{general} }}, Physics = {{ MotionType = 'RULEUMT_Land', MaxSpeed = 2, TurnRate = 90 }},              Footprint = {{ SizeX = 1, SizeZ = 1 }} }}")).unwrap();
        let t1 = bp("UEL0105", "TECH1", "");
        let t2 = bp("UEL0208", "TECH2", "");
        let named = bp("UEL0309", "TECH3", ", UnitName = 'Builder Bob'");
        let faction = [&t1, &t2, &named];
        assert_eq!(shown_name(&t1, &faction), "T1 Engineer");
        assert_eq!(shown_name(&t2, &faction), "T2 Engineer");
        assert_eq!(shown_name(&named, &faction), "Builder Bob");
        assert_eq!(shown_name(&t1, &[&t1]), "Engineer", "alone of its name: as it is");
        // (A name's article gives way to its tier.)
        let kennel = |id: &str, tech: &str| bp(id, tech, ", UnitName = 'The Kennel'");
        let (k2, k3) = (kennel("XEB0104", "TECH2"), kennel("XEB0204", "TECH3"));
        assert_eq!((shown_name(&k2, &[&k2, &k3]), shown_name(&k3, &[&k2, &k3])), ("T2 Kennel".to_string(), "T3 Kennel".to_string()));
        assert_eq!(shown_name(&k2, &[&k2]), "The Kennel", "alone of its name: as it is");
        let wrecks = super::super::faction::wrecks(&faction);
        assert!(wrecks.contains("description=T2 Engineer Wreckage;") && !wrecks.contains("UEL0"), "{wrecks}");
    }

    fn striker() -> UnitBlueprint {
        unit::parse(
            "UEL0201",
            "UnitBlueprint { Categories = { 'UEF', 'MOBILE', 'LAND', 'TECH1', 'DIRECTFIRE', 'BUILTBYTIER1FACTORY' }, \
             Defense = { Health = 300, MaxHealth = 300 }, \
             Economy = { BuildCostEnergy = 266, BuildCostMass = 56, BuildTime = 280 }, \
             General = { FactionName = 'UEF', UnitName = 'MA12 Striker', TechLevel = 'RULEUTL_Basic' }, \
             Intel = { VisionRadius = 20 }, \
             Physics = { MotionType = 'RULEUMT_Land', MaxSpeed = 3.4, MaxAcceleration = 3.4, MaxBrake = 3.4, TurnRate = 90 }, \
             Footprint = { SizeX = 1, SizeZ = 1 }, \
             Weapon = { { Label = 'MainGun', WeaponCategory = 'Direct Fire', DisplayName = 'Gauss Cannon', Damage = 24, MaxRadius = 18, RateOfFire = 1, \
               MuzzleVelocity = 25, BallisticArc = 'RULEUBA_LowArc', ProjectileId = '/projectiles/TDFGauss01/TDFGauss01_proj.bp', \
               Turreted = true, TurretBoneYaw = 'Turret', TurretBonePitch = 'Turret_Barrel', TurretBoneMuzzle = 'Turret_Muzzle', \
               RackBones = { { RackBone = 'Turret_Barrel', MuzzleBones = { 'Turret_Muzzle' } } }, \
               FireTargetLayerCapsTable = { Land = 'Land|Water|Seabed' } } } }",
        )
        .unwrap()
    }

    /// A value of `key` in a TDF section's text.
    fn value(section: &str, key: &str) -> Vec<String> {
        section.lines().filter_map(|l| l.trim().strip_prefix(&format!("{key}="))).map(|v| v.trim_end_matches(';').to_string()).collect()
    }

    /// The Striker's file: FA's health, costs, build time (ten times), speed at TA's scale, its
    /// gun's section; its weapon a shell, ballistic, its range 32 pixels an ogrid.
    #[test]
    fn a_tanks_file_and_its_gun() {
        let u = striker();
        let w = weapons(&u);
        assert_eq!(w.slots.len(), 1);
        let file = unit_file(&u, &w, None, &Extra::default());
        for line in ["UnitName=UEL0201;", "Side=UEF;", "MaxDamage=300;", "BuildCostMetal=56;", "BuildCostEnergy=266;", "BuildTime=2800;", "FootprintX=2;", "SightDistance=640;", "Weapon1=UEL0201_MAINGUN;", "MovementClass=TANKSH2;", "TEDClass=TANK;", "TurnRate=546;", "Corpse=uel0201_dead;"] {
            assert!(file.contains(line), "{line} in {file}");
        }
        assert!(file.contains("MaxVelocity=3.627;"), "{file}");
        // (A ballistic gun that shoots no aircraft cannot hit them in TA Reborn: no bad target,
        // but it chases none.)
        assert!(!file.contains("badTargetCategory") && file.contains("NoChaseCategory=VTOL;"), "{file}");
        let gun = weapon_section(&u, w.slots[0]);
        for line in ["[UEL0201_MAINGUN]", "range=576;", "reloadtime=1.0000;", "weaponvelocity=800;", "rendertype=4;", "ballistic=1;", "default=24;"] {
            assert!(gun.contains(line), "{line} in {gun}");
        }
        // Read back as TA Reborn reads it.
        let doc = ta_formats::tdf::parse(&gun);
        assert_eq!(doc.sections.len(), 1);
    }

    /// A commander: its main gun first, its Overcharge second as TA's D-gun; an enhancement's
    /// missile, a missile its script switches off as it is made and its death weapon not in its
    /// slots.
    #[test]
    fn a_commanders_overcharge_is_its_dgun() {
        let gun = |label: &str, category: &str, damage: f64| WeaponBlueprint { label: label.into(), category: category.into(), damage, rate_of_fire: 1.0, max_radius: 20.0, ..Default::default() };
        let mut u = UnitBlueprint { id: "UEL0001".into(), faction: Faction::Uef, categories: vec!["COMMAND".into()], ..Default::default() };
        let mut tac = gun("TacMissile", "Missile", 6000.0);
        tac.table = crate::lua::parse("t = { EnabledByEnhancement = 'TacticalMissile' }").unwrap()[0].1.table().cloned().unwrap();
        let mut oc = gun("OverCharge", "Direct Fire", 12000.0);
        oc.damage_type = "Overcharge".into();
        u.weapons = vec![gun("RightZephyr", "Direct Fire", 100.0), oc, tac, gun("Missile", "Missile", 6000.0), gun("DeathWeapon", "Death", 0.0)];
        // (Its script switches the Overcharge and the second missile off as it is made: the
        // first is the D-gun, the second left out.)
        u.birth.weapons_off = vec!["OverCharge".into(), "Missile".into()];
        let w = weapons(&u);
        assert_eq!(w.slots.iter().map(|w| w.label.as_str()).collect::<Vec<_>>(), ["RightZephyr", "OverCharge"]);
        assert_eq!(w.dgun, Some(1));
        let file = unit_file(&u, &w, None, &Extra::default());
        assert!(file.contains("canDgun=1;") && file.contains("Commander=1;") && file.contains("TEDClass=COMMANDER;"), "{file}");
        assert!(weapon_section(&u, w.slots[1]).contains("commandfire=1;"));
        let _ = Rack::default();
    }

    /// An air transport's slots, by its attach bones however they are named: small ones (plain,
    /// `_sml`, `_Small`, on either side) counted, sized by its largest; larger ones counted where
    /// there are no small; none at all, none.
    #[test]
    fn a_transport_carries_what_its_attach_bones_hold() {
        let still = [1.0, 0.0, 0.0, 0.0];
        let mesh = |bones: &[&str]| {
            let mut all = vec![("Hull", -1, [0.0f32; 3], still)];
            all.extend(bones.iter().map(|b| (*b, 0, [0.0f32; 3], still)));
            crate::scm::parse(&crate::scm::tests::write(&all, &[([0.0; 3], [0.0; 2], 0)], &[[0, 0, 0]])).unwrap()
        };
        assert_eq!(transport(&mesh(&["Left_Attachpoint_sml_01", "Right_Attachpoint_sml_01", "Attachpoint_Small_03", "Attachpoint_Med_01", "Attachpoint_Lrg_01"])), Some((3, 3)));
        assert_eq!(transport(&mesh(&["Attachpoint01", "Attachpoint02"])), Some((2, 1)));
        assert_eq!(transport(&mesh(&["Attachpoint_Med_01"])), Some((1, 2)), "a gunship's one slot");
        assert_eq!(transport(&mesh(&["Turret", "Muzzle"])), None);
    }

    fn projectile(categories: &[&str], mass: f64, energy: f64, build_time: f64) -> ProjectileBlueprint {
        ProjectileBlueprint { categories: categories.iter().map(|c| c.to_string()).collect(), cost_mass: mass, cost_energy: energy, build_time, ..Default::default() }
    }

    fn nuke_weapon() -> WeaponBlueprint {
        WeaponBlueprint {
            label: "NukeMissiles".into(),
            category: "Missile".into(),
            damage: 1.0,
            rate_of_fire: 1.0,
            max_radius: 20000.0,
            muzzle_velocity: 0.0,
            counted: true,
            manual_fire: true,
            nuke: Some(crate::unit::Nuke { inner_damage: 70000.0, inner_radius: 30.0, outer_damage: 500.0, outer_radius: 40.0 }),
            shot: Some(projectile(&["UEF", "PROJECTILE", "STRATEGIC", "MISSILE"], 12000.0, 1350000.0, 324000.0)),
            ..Default::default()
        }
    }

    fn silo() -> UnitBlueprint {
        UnitBlueprint { id: "UEB2305".into(), faction: Faction::Uef, build_rate: 1080.0, categories: vec!["STRUCTURE".into()], ..Default::default() }
    }

    /// A missile lives long enough to reach as far as its range, a nuke across the map.
    #[test]
    fn missiles_fly_as_far_as_their_range() {
        // 1,920 pixels at 640 a second from 213: 3 s, half again, 0.7 s to speed, a second.
        assert_eq!(flight_time(1920, 640, 213, 320), 7);
        assert!(flight_time(NUKE_RANGE, 1600, 533, 800) as f64 * 1600.0 >= NUKE_RANGE as f64);
        assert_eq!(flight_time(10, 640, 213, 320), 3, "never less than three seconds");
        let mut missile = WeaponBlueprint { label: "Tac".into(), category: "Missile".into(), damage: 100.0, rate_of_fire: 0.1, max_radius: 120.0, muzzle_velocity: 10.0, ..Default::default() };
        missile.projectile = Some("/projectiles/TIFMissileCruise01/TIFMissileCruise01_proj.bp".into());
        let section = weapon_section(&striker(), &missile);
        let life: f64 = value(&section, "flighttime")[0].parse().unwrap();
        assert!(life * 320.0 >= 120.0 * 32.0, "{section}");
        // A strategic missile reaches across the map at TA's nukes' speed at least, its range given
        // once.
        let section = weapon_section(&silo(), &nuke_weapon());
        assert_eq!(value(&section, "range"), [NUKE_RANGE.to_string()], "{section}");
        assert_eq!(value(&section, "weaponvelocity"), [NUKE_SPEED.to_string()], "{section}");
        let life: i64 = value(&section, "flighttime")[0].parse().unwrap();
        assert!(life * NUKE_SPEED >= NUKE_RANGE && life < 200, "{life} s: {section}");
    }

    /// A nuke is a stored missile built at its projectile's price over its build time at its
    /// silo's build rate (12,000 mass and 1,350,000 energy, 324,000 at 1,080: 300 s), launched
    /// upright to cruise high as TA's nukes do (a climb of five seconds, half a turn a second) at
    /// the place it is sent, a strategic missile for anti-nukes.
    #[test]
    fn a_nuke_is_built_at_its_price_and_flies_as_tas() {
        let section = weapon_section(&silo(), &nuke_weapon());
        for (key, want) in [("stockpile", "1"), ("metalpershot", "12000"), ("energypershot", "1350000"), ("reloadtime", "300.000"), ("targetable", "1"), ("commandfire", "1"), ("vlaunch", "1"), ("cruise", "1"), ("weapontimer", "5"), ("turnrate", "32768"), ("default", "70000")] {
            assert_eq!(value(&section, key), [want], "{key} in {section}");
        }
        assert!(value(&section, "tracks").is_empty(), "a nuke strikes a place: {section}");
        assert!(value(&section, "interceptor").is_empty(), "{section}");
        assert!(unit_file(&silo(), &Weapons { slots: vec![&nuke_weapon()], death: None, dgun: None, kamikaze: None }, None, &Extra::default()).contains("onoffable=0;"));
    }

    /// A tactical missile launcher's missiles are stored too (180 mass and 3,600 energy, 2,400 at
    /// 80: 30 s), ordered to fire, tactical missiles for missile defences.
    #[test]
    fn a_tactical_launcher_stores_its_missiles() {
        let launcher = UnitBlueprint { id: "UEB2108".into(), faction: Faction::Uef, build_rate: 80.0, categories: vec!["STRUCTURE".into()], ..Default::default() };
        let w = WeaponBlueprint {
            label: "CruiseMissile".into(),
            category: "Missile".into(),
            damage: 6000.0,
            rate_of_fire: 3.0,
            max_radius: 256.0,
            min_radius: 15.0,
            counted: true,
            manual_fire: true,
            shot: Some(projectile(&["UEF", "PROJECTILE", "TACTICAL", "MISSILE"], 180.0, 3600.0, 2400.0)),
            ..Default::default()
        };
        let section = weapon_section(&launcher, &w);
        for (key, want) in [("stockpile", "1"), ("metalpershot", "180"), ("energypershot", "3600"), ("reloadtime", "30.000"), ("targetable", "2"), ("commandfire", "1"), ("cruise", "1"), ("weapontimer", "3"), ("default", "6000")] {
            assert_eq!(value(&section, key), [want], "{key} in {section}");
        }
        assert!(value(&section, "minbarrelangle").is_empty(), "its least range is not TA's");
    }

    /// Missile defences take what Forged Alliance's do: an anti-nuke the strategic missiles (a
    /// stored missile of its own, at its price), a tactical missile defence and a ship's point
    /// defence the tactical ones, each at its own rate.
    #[test]
    fn missile_defences_take_their_own_class() {
        let station = UnitBlueprint { id: "UEB4302".into(), faction: Faction::Uef, build_rate: 1080.0, ..Default::default() };
        let anti_nuke = WeaponBlueprint {
            label: "AntiNuke".into(),
            category: "Defense".into(),
            damage: 30.0,
            rate_of_fire: 0.6,
            max_radius: 90.0,
            counted: true,
            only_allow: "STRATEGIC MISSILE".into(),
            shot: Some(projectile(&["UEF", "PROJECTILE", "ANTIMISSILE"], 3600.0, 360000.0, 259200.0)),
            ..Default::default()
        };
        let section = weapon_section(&station, &anti_nuke);
        for (key, want) in [("interceptor", "1"), ("stockpile", "1"), ("metalpershot", "3600"), ("energypershot", "360000"), ("reloadtime", "240.000"), ("coverage", "2880")] {
            assert_eq!(value(&section, key), [want], "{key} in {section}");
        }
        let tmd = WeaponBlueprint { label: "Turret01".into(), category: "Defense".into(), damage: 1.0, rate_of_fire: 0.5, max_radius: 30.0, only_allow: "TACTICAL MISSILE".into(), ..Default::default() };
        let section = weapon_section(&station, &tmd);
        assert_eq!((value(&section, "interceptor"), value(&section, "stockpile"), value(&section, "reloadtime")), (vec!["2".to_string()], vec![], vec!["2.0000".to_string()]), "{section}");
        // (A beam, as the Cybran zapper: its shot leaves at full speed, quick enough to meet a
        // missile; the anti-nuke's leaves its tube upright, at its own speed.)
        for key in ["weaponvelocity", "startvelocity"] {
            assert_eq!(value(&section, key), [COUNTERMEASURE_SPEED.to_string()], "{key} in {section}");
        }
        assert!(value(&section, "vlaunch").is_empty(), "{section}");
        let flare = WeaponBlueprint { muzzle_velocity: 5.0, ..tmd.clone() };
        assert_eq!(value(&weapon_section(&station, &flare), "weaponvelocity"), [COUNTERMEASURE_SPEED.to_string()]);
        let rounds = WeaponBlueprint { muzzle_velocity: 100.0, ..tmd.clone() };
        assert_eq!(value(&weapon_section(&station, &rounds), "weaponvelocity"), ["3200"], "the Phalanx's own, quicker");
        assert_eq!(value(&weapon_section(&station, &anti_nuke), "vlaunch"), ["1"]);
        let phalanx = WeaponBlueprint { only_allow: "TACTICAL,MISSILE".into(), ..tmd };
        assert_eq!(interceptor_class(&phalanx), TACTICAL);
        assert_eq!((targetable_class(&nuke_weapon()), interceptor_class(&nuke_weapon())), (STRATEGIC, 0));
    }

    /// Forged Alliance's firing rules: every muzzle of a rack at once where the muzzles fire
    /// together, every rack at once where the racks do, a damage over time's pulses, a beam's
    /// hits, a shell's pieces, the reload and charge after the racks; TA's keys give the same
    /// damage a second.
    #[test]
    fn damage_a_second_follows_forged_alliances_firing_rules() {
        let rack = |muzzles: usize| Rack { rack: "R".into(), muzzles: (0..muzzles).map(|m| format!("M{m}")).collect() };
        let base = WeaponBlueprint { label: "Gun".into(), category: "Direct Fire".into(), damage: 10.0, rate_of_fire: 1.0, max_radius: 20.0, ..Default::default() };
        // Two muzzles fired at once (no delay), once a second.
        let both = WeaponBlueprint { racks: vec![rack(2)], salvo_size: 1, salvo_delay: 0.0, ..base.clone() };
        assert!((fa_dps(&both) - 20.0).abs() < 1e-9);
        // Three racks together, one muzzle each: three shots each time.
        let together = WeaponBlueprint { racks: vec![rack(1), rack(1), rack(1)], racks_together: true, ..base.clone() };
        assert!((fa_dps(&together) - 30.0).abs() < 1e-9);
        // The same three in turn: one a second.
        let in_turn = WeaponBlueprint { racks_together: false, ..together.clone() };
        assert!((fa_dps(&in_turn) - 10.0).abs() < 1e-9);
        // Five pulses of damage over time.
        let dot = WeaponBlueprint { dot_time: 2.0, dot_pulses: 5, racks: vec![rack(1)], ..base.clone() };
        assert!((fa_dps(&dot) - 50.0).abs() < 1e-9);
        // A beam living a second, hitting every 0.1 s: ten hits a shot.
        let mut pulse = WeaponBlueprint { beam_lifetime: 1.0, beam_collision_delay: 0.0, racks: vec![rack(1)], rate_of_fire: 0.25, ..base.clone() };
        pulse.table = crate::lua::parse("t = { BeamLifetime = 1 }").unwrap()[0].1.table().cloned().unwrap();
        assert!((fa_dps(&pulse) - 25.0).abs() < 1e-9);
        // A beam kept on: every 0.3 s, whatever its rate of fire.
        let mut kept = WeaponBlueprint { beam_lifetime: 0.0, beam_collision_delay: 0.2, racks: vec![rack(1)], rate_of_fire: 0.2, ..base.clone() };
        kept.table = crate::lua::parse("t = { BeamLifetime = 0 }").unwrap()[0].1.table().cloned().unwrap();
        assert!((fa_dps(&kept) - 10.0 / 0.3).abs() < 1e-9);
        // A shell splitting into 36 pieces.
        let salvation = WeaponBlueprint { split: Some(crate::unit::Split { pieces: 36, share: 1.0 }), racks: vec![rack(1)], ..base.clone() };
        assert!((fa_dps(&salvation) - 360.0).abs() < 1e-9);
        // Fifteen muzzles a third of a second apart, charged two seconds, reloaded three: a shot
        // every 0.66 s over 9.95 s.
        let ravager = WeaponBlueprint { racks: vec![rack(1)], salvo_size: 15, salvo_delay: 0.33, rack_charge: 2.0, rack_reload: 3.0, ..base.clone() };
        assert!((fa_dps(&ravager) - 150.0 / 9.95).abs() < 1e-9, "{}", fa_dps(&ravager));
        // TA's keys give the same damage a second, within its reload's whole ticks.
        for w in [&both, &together, &in_turn, &dot, &pulse, &kept, &salvation, &ravager] {
            let f = firing(w);
            let ticks = (f.reload * 30.0).round();
            let ta = f.damage * f.burst as f64 / (ticks / 30.0);
            assert!((ta / fa_dps(w) - 1.0).abs() < 1e-6, "{f:?}: {ta} against {}", fa_dps(w));
            let section = weapon_section(&striker(), w);
            let written: f64 = value(&section, "default")[0].parse().unwrap();
            let burst: f64 = value(&section, "burst").first().map_or(1.0, |b| b.parse().unwrap());
            let reload: f64 = value(&section, "reloadtime")[0].parse().unwrap();
            let ta = written * burst / ((reload * 30.0).round() / 30.0);
            assert!((ta / fa_dps(w) - 1.0).abs() < 0.01, "{section}: {ta} against {}", fa_dps(w));
        }
        assert_eq!(firing(&together).burst, 3);
    }

    /// Anti-air shots home on their aircraft (the engine leads nothing): missiles at their
    /// projectile's speeds and turn, flak bursting over a real area.
    #[test]
    fn anti_air_homes_on_aircraft() {
        let mut sam = WeaponBlueprint { label: "MissileRack01".into(), category: "Anti Air".into(), damage: 200.0, rate_of_fire: 0.28, max_radius: 60.0, muzzle_velocity: 30.0, salvo_size: 6, salvo_delay: 0.2, ..Default::default() };
        sam.shot = Some(ProjectileBlueprint { categories: vec!["ANTIAIR".into(), "MISSILE".into()], initial_speed: 30.0, max_speed: 50.0, acceleration: 6.0, turn_rate: 180.0, tracks: true, ..Default::default() });
        let section = weapon_section(&striker(), &sam);
        for (key, want) in [("tracks", "1"), ("guidance", "1"), ("toairweapon", "1"), ("startvelocity", "960"), ("weaponvelocity", "1600"), ("weaponacceleration", "192"), ("turnrate", "32768"), ("burst", "6"), ("model", "missile")] {
            assert_eq!(value(&section, key), [want], "{key} in {section}");
        }
        let flak = WeaponBlueprint { label: "AAGun".into(), category: "Anti Air".into(), damage: 102.0, damage_radius: 4.0, rate_of_fire: 1.5, max_radius: 44.0, muzzle_velocity: 20.0, ..Default::default() };
        let section = weapon_section(&striker(), &flak);
        for (key, want) in [("tracks", "1"), ("toairweapon", "1"), ("areaofeffect", "256"), ("edgeeffectiveness", "0.9"), ("turnrate", "30000")] {
            assert_eq!(value(&section, key), [want], "{key} in {section}");
        }
    }

    /// The fields TA Reborn reads: builders reclaim (`CanReclamate`), a radar is on once built
    /// and switchable (its upkeep paid while on), an air transport carries what its attach bones
    /// hold, and structures stand on the water, under it or on land as their layers allow.
    #[test]
    fn builders_reclaim_sensors_switch_transports_carry_and_structures_stand_where_they_may() {
        use crate::unit::Layers;
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        let engineer = UnitBlueprint { id: "UEL0105".into(), faction: Faction::Uef, motion: Motion::Land, build_rate: 5.0, buildable: vec!["BUILTBYTIER1ENGINEER UEF".into()], categories: vec!["ENGINEER".into()], ..Default::default() };
        assert!(unit_file(&engineer, &none, None, &Extra::default()).contains("CanReclamate=1;"));
        let radar = UnitBlueprint { id: "UEB3101".into(), faction: Faction::Uef, categories: vec!["STRUCTURE".into(), "INTELLIGENCE".into(), "RADAR".into()], upkeep_energy: 20.0, intel: crate::unit::Intel { radar: 115.0, ..Default::default() }, layers: Layers { land: true, ..Default::default() }, ..Default::default() };
        let file = unit_file(&radar, &none, None, &Extra::default());
        assert!(file.contains("EnergyUse=20;") && file.contains("ActivateWhenBuilt=1;") && file.contains("OnOffable=1;") && file.contains("MaxWaterDepth=0;"), "{file}");
        let transport = UnitBlueprint { id: "UEA0107".into(), faction: Faction::Uef, motion: Motion::Air, categories: vec!["AIR".into(), "TRANSPORTATION".into()], ..Default::default() };
        let file = unit_file(&transport, &none, Some((6, 3)), &Extra::default());
        assert!(file.contains("TransportCapacity=6;") && file.contains("TransportMaxUnits=6;") && file.contains("TransportSize=3;") && file.contains("canload=1;"), "{file}");
        assert!(!unit_file(&transport, &none, None, &Extra::default()).contains("canload"), "no attach bones, no transport");
        // An extractor on land or the sea bed; a torpedo trap on the sea bed alone; a sonar afloat.
        let structure = |id: &str, layers: Layers| UnitBlueprint { id: id.into(), faction: Faction::Uef, categories: vec!["STRUCTURE".into()], layers, ..Default::default() };
        let file = unit_file(&structure("UEB1103", Layers { land: true, seabed: true, ..Default::default() }), &none, None, &Extra::default());
        assert!(file.contains("MaxWaterDepth=255;") && !file.contains("MinWaterDepth"), "{file}");
        let file = unit_file(&structure("XRB2308", Layers { sub: true, ..Default::default() }), &none, None, &Extra::default());
        assert!(file.contains("MaxWaterDepth=255;") && file.contains("MinWaterDepth=15;") && !file.contains("Floater"), "{file}");
        let file = unit_file(&structure("UEB3102", Layers { water: true, ..Default::default() }), &none, None, &Extra::default());
        assert!(file.contains("MinWaterDepth=5;") && file.contains("Floater=1;"), "{file}");
    }

    /// A structure's adjacency: the buffs it gives as its section, its size class, and the
    /// categories buffs name that it has in its `Category` (an artillery piece's); a unit that
    /// moves gives and takes nothing. An engineering station works round it by itself, by its own
    /// work or its drones' (the more) and its drones' reach where it gives none of its own.
    #[test]
    fn adjacency_and_stations_are_written_into_the_unit_file() {
        use crate::adjacency::{Affect, Buff};
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        let buffs = [
            Buff { name: "GenBuildSize4".into(), size: Some(4), categories: Vec::new(), affect: Affect::EnergyActive, add: -0.05 },
            Buff { name: "GenReloadSize4".into(), size: Some(4), categories: vec!["ARTILLERY".into()], affect: Affect::RateOfFire, add: -0.03 },
        ];
        let words: std::collections::BTreeSet<String> = ["ARTILLERY".to_string()].into();
        let extra = Extra { adjacency: &buffs, adjacency_categories: Some(&words), ..Extra::default() };
        let generator = UnitBlueprint { id: "UEB1101".into(), faction: Faction::Uef, categories: vec!["STRUCTURE".into(), "SIZE4".into()], production_energy: 20.0, ..Default::default() };
        let file = ta_formats::tdf::parse(&unit_file(&generator, &none, None, &extra)).sections.remove(0);
        assert_eq!(file.get("AdjacencySize"), Some("4"));
        let section = file.sections.iter().find(|s| s.name == "ADJACENCY").unwrap();
        assert_eq!(section.sections.iter().map(|s| (s.name.as_str(), s.get("Affects"), s.get("Add"), s.get("Size"), s.get("Category"))).collect::<Vec<_>>(), [
            ("GenBuildSize4", Some("BuildEnergy"), Some("-0.05"), Some("4"), None),
            ("GenReloadSize4", Some("ReloadTime"), Some("-0.03"), Some("4"), Some("ARTILLERY")),
        ]);
        let artillery = UnitBlueprint { id: "UEB2303".into(), faction: Faction::Uef, categories: vec!["STRUCTURE".into(), "ARTILLERY".into(), "SIZE4".into()], ..Default::default() };
        assert_eq!(ta_formats::tdf::parse(&unit_file(&artillery, &none, None, &extra)).sections[0].get("Category"), Some("UEF LEVEL1 NOTAIR NOTSUB NOWEAPON ARTILLERY STRUCTURE"), "(a structure: what stuns spare)");
        let tank = UnitBlueprint { id: "UEL0201".into(), faction: Faction::Uef, motion: Motion::Land, categories: vec!["ARTILLERY".into(), "SIZE4".into()], ..Default::default() };
        let file = unit_file(&tank, &none, None, &extra);
        assert!(!file.contains("ADJACENCY") && !file.contains("AdjacencySize") && !file.contains(" ARTILLERY"), "{file}");
        // Stations: the Hive by its own work and reach; the Kennel by its drones'.
        let station = |id: &str, rate: f64, economy: &str, pods: Vec<crate::unit::Pod>| {
            let mut u = crate::unit::parse(id, &format!("UnitBlueprint {{ Categories = {{ 'STRUCTURE', 'ENGINEERSTATION' }}, General = {{ FactionName = 'Cybran' }}, Economy = {{ {economy} }} }}")).unwrap();
            u.build_rate = rate;
            u.pods = pods;
            u
        };
        // (Made-up figures.)
        let drone = |rate: f64| crate::unit::Pod { id: "XEA3204".into(), build_rate: rate, reach: 12.0 };
        assert_eq!(station_work(&station("XRB0104", 14.0, "MaxBuildDistance = 13", Vec::new())), (14.0, 13.0));
        assert_eq!(station_work(&station("XEB0104", 18.0, "", vec![drone(12.0)])), (18.0, 12.0), "its own work, the more; its drone's reach");
        assert_eq!(station_work(&station("XEB0204", 18.0, "", vec![drone(12.0), drone(12.0)])), (24.0, 12.0), "two drones' work");
        assert_eq!(station_work(&station("XEB0304", 18.0, "", Vec::new())), (18.0, 5.0), "no drones and no reach of its own: five ogrids");
        let hive = unit_file(&station("XRB0104", 14.0, "MaxBuildDistance = 13", Vec::new()), &none, None, &Extra::default());
        assert!(hive.contains("DefaultMissionType=AssistPatrol;") && hive.contains("WorkerTime=140;") && hive.contains("BuildDistance=416;") && hive.contains("CanReclamate=1;"), "{hive}");
    }

    /// Upkeep only for what TA Reborn plays: a shield's (or the Fatboy's) and a stealth unit's is
    /// left out, an extractor's and a radar's kept; an extractor stores nothing (TA's none).
    #[test]
    fn upkeep_is_paid_only_for_what_is_there() {
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        let shield = UnitBlueprint { id: "UEB4202".into(), faction: Faction::Uef, categories: vec!["STRUCTURE".into(), "SHIELD".into()], upkeep_energy: 200.0, ..Default::default() };
        let file = unit_file(&shield, &none, None, &Extra::default());
        assert!(!file.contains("EnergyUse") && !file.contains("OnOffable"), "{file}");
        let stealthy = UnitBlueprint { id: "URA0303".into(), faction: Faction::Cybran, motion: Motion::Air, upkeep_energy: 25.0, intel: crate::unit::Intel { radar_stealth: true, ..Default::default() }, ..Default::default() };
        let file = unit_file(&stealthy, &none, None, &Extra::default());
        assert!(file.contains("Stealth=1;") && !file.contains("EnergyUse"), "{file}");
        let mex = UnitBlueprint { id: "UEB1103".into(), faction: Faction::Uef, categories: vec!["MASSEXTRACTION".into()], upkeep_energy: 2.0, production_mass: 2.0, ..Default::default() };
        let file = unit_file(&mex, &none, None, &Extra::default());
        assert!(file.contains("EnergyUse=2;") && file.contains("ExtractsMetal=0.001;") && !file.contains("MetalStorage"), "{file}");
    }

    /// What the scripts take a unit's `Activate` to come with (`activated_when_built`) is what its
    /// file says: an extractor, a radar paying its upkeep, a fabricator and a staging platform are
    /// switched on once built; a power plant, a shield (its upkeep left out) and a factory are not.
    #[test]
    fn switched_on_once_built_as_the_file_says() {
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        let building = |id: &str, cats: &[&str], upkeep: f64, mass: f64, energy: f64| UnitBlueprint {
            id: id.into(),
            faction: Faction::Uef,
            categories: cats.iter().map(|c| c.to_string()).collect(),
            upkeep_energy: upkeep,
            production_mass: mass,
            production_energy: energy,
            intel: crate::unit::Intel { radar: if cats.contains(&"RADAR") { 115.0 } else { 0.0 }, ..Default::default() },
            ..Default::default()
        };
        let mut factory = building("UEB0101", &["STRUCTURE", "FACTORY"], 0.0, 0.0, 0.0);
        factory.build_rate = 20.0;
        factory.buildable = vec!["BUILTBYTIER1FACTORY UEF MOBILE".into()];
        let mut pad = building("UEB5202", &["STRUCTURE", "AIRSTAGINGPLATFORM"], 0.0, 0.0, 0.0);
        pad.motion = Motion::None;
        for (u, on) in [
            (building("UEB1103", &["STRUCTURE", "MASSEXTRACTION"], 2.0, 2.0, 0.0), true),
            (building("UEB3101", &["STRUCTURE", "RADAR", "INTELLIGENCE"], 20.0, 0.0, 0.0), true),
            (building("UEB1104", &["STRUCTURE", "MASSFABRICATION"], 100.0, 1.0, 0.0), true),
            (pad, true),
            (building("UEB1101", &["STRUCTURE", "ENERGYPRODUCTION"], 0.0, 0.0, 20.0), false),
            (building("UEB4202", &["STRUCTURE", "SHIELD"], 200.0, 0.0, 0.0), false),
            (factory, false),
        ] {
            let file = unit_file(&u, &none, None, &Extra::default());
            assert_eq!((activated_when_built(&u), file.contains("ActivateWhenBuilt=1;")), (on, on), "{}: {file}", u.id);
        }
    }

    /// What a unit's script switches off as it is made is not given it: the Cybran commander's
    /// sonar and stealth. Its regeneration is TA's healing.
    #[test]
    fn abilities_off_at_birth_are_left_out() {
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        let mut acu = UnitBlueprint { id: "URL0001".into(), faction: Faction::Cybran, motion: Motion::Amphibious, regen: 15.0, intel: crate::unit::Intel { sonar: 60.0, radar_stealth: true, ..Default::default() }, ..Default::default() };
        acu.birth.intel_off = vec!["RadarStealth".into(), "Sonar".into()];
        let file = unit_file(&acu, &none, None, &Extra::default());
        assert!(!file.contains("SonarDistance") && !file.contains("Stealth=1;") && file.contains("HealTime=6.67;"), "{file}");
        // An amphibian goes by its own limits, to the deepest sea bed.
        assert!(file.contains("MaxWaterDepth=255;") && !file.contains("MovementClass"), "{file}");
    }

    /// Aircraft gather speed and turn as Forged Alliance's do, at TA's scale: an interceptor to
    /// full speed in a second and a half, turning 86° a second as TA's Freedom Fighter does; only
    /// gunships (and hovering craft with guns) hover to attack.
    #[test]
    fn aircraft_fly_as_forged_alliances() {
        let plane = |id: &str, cats: &[&str], air: &str| {
            let mut u = UnitBlueprint { id: id.into(), faction: Faction::Uef, motion: Motion::Air, max_airspeed: Some(15.0), categories: cats.iter().map(|c| c.to_string()).collect(), ..Default::default() };
            u.table = crate::lua::parse(&format!("t = {{ Air = {{ {air} }} }}")).unwrap()[0].1.table().cloned().unwrap();
            u
        };
        let fighter = plane("UEA0102", &["ANTIAIR"], "KMove = 1, KMoveDamping = 1, TurnSpeed = 1.5, Winged = true");
        let (speed, acceleration, brake, turn) = flight(&fighter);
        assert!((speed - 16.0).abs() < 1e-9 && (speed / acceleration - 45.0).abs() < 1e-6 && brake > acceleration, "{speed} {acceleration}");
        assert_eq!(turn.round(), 522.0, "86° a second");
        let gun = WeaponBlueprint { label: "Gun".into(), category: "Anti Air".into(), damage: 50.0, rate_of_fire: 1.0, turreted: true, ..Default::default() };
        let armed = Weapons { slots: vec![&gun], death: None, dgun: None, kamikaze: None };
        assert!(!unit_file(&fighter, &armed, None, &Extra::default()).contains("HoverAttack"), "a fighter makes passes");
        let gunship = plane("UEA0203", &["GROUNDATTACK"], "KMove = 0.8, KMoveDamping = 2, KTurn = 0.8");
        assert!(unit_file(&gunship, &armed, None, &Extra::default()).contains("HoverAttack=1;"));
    }

    /// A building's footprint covers what of its model stands on the ground where its blueprint's
    /// is smaller (a point defence's 0.6 ogrids, a cell, round a turret of almost an ogrid: 2 by 2
    /// cells), one way or both; a wall's, a unit's and a building's at sea alone stay as they are;
    /// and a building stands on slopes as steep as TA's own do.
    #[test]
    fn a_buildings_footprint_covers_its_model_on_the_ground() {
        let land = crate::unit::Layers { land: true, ..Default::default() };
        let pd = UnitBlueprint { id: "UEB2101".into(), faction: Faction::Uef, categories: vec!["STRUCTURE".into()], footprint: Some([0.6, 0.6]), layers: land, ..Default::default() };
        assert_eq!(footprint(&pd), (1, 1));
        let fitted = fitted_footprint(&pd, Some([0.98, 0.98]));
        assert_eq!(fitted, Some([0.98, 0.98]));
        assert_eq!(footprint(&UnitBlueprint { footprint: fitted, ..pd.clone() }), (2, 2));
        assert_eq!(fitted_footprint(&pd, Some([0.4, 1.3])), Some([0.6, 1.3]), "wider one way");
        assert_eq!(fitted_footprint(&pd, Some([0.5, 0.6])), None, "no wider");
        assert_eq!(fitted_footprint(&pd, None), None, "nothing on the ground");
        let wall = UnitBlueprint { categories: vec!["STRUCTURE".into(), "WALL".into()], ..pd.clone() };
        let tank = UnitBlueprint { motion: Motion::Land, ..pd.clone() };
        let sonar = UnitBlueprint { layers: crate::unit::Layers { water: true, ..Default::default() }, ..pd.clone() };
        for other in [&wall, &tank, &sonar] {
            assert_eq!(fitted_footprint(other, Some([2.0, 2.0])), None, "{:?}", other.categories);
        }
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        assert!(unit_file(&pd, &none, None, &Extra::default()).contains("MaxSlope=10;"));
    }

    /// A ship stands on a square of its hull's area (a frigate's 2 by 6 ogrids, 4 by 12 cells: 7);
    /// a hydrocarbon plant on a vent; an engineering station lends its power; an air staging
    /// platform repairs aircraft; a crawling bomb blows itself up near its enemy.
    #[test]
    fn ships_plants_stations_pads_and_bombs() {
        let none = Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        let frigate = UnitBlueprint { id: "UES0103".into(), faction: Faction::Uef, motion: Motion::Water, footprint: Some([2.0, 6.0]), ..Default::default() };
        assert_eq!(footprint(&frigate), (7, 7));
        let plant = UnitBlueprint { id: "UEB1102".into(), faction: Faction::Uef, categories: vec!["HYDROCARBON".into()], footprint: Some([1.0, 1.0]), ..Default::default() };
        assert!(unit_file(&plant, &none, None, &Extra::default()).contains("YardMap=GGGG;"));
        let station = UnitBlueprint { id: "XRB0104".into(), faction: Faction::Cybran, build_rate: 15.0, categories: vec!["ENGINEERSTATION".into()], buildable: vec!["xrb0204".into()], ..Default::default() };
        assert_eq!(builds(&station), Builds::Assist);
        let file = unit_file(&station, &none, None, &Extra::default());
        assert!(file.contains("Builder=1;") && file.contains("WorkerTime=150;") && !file.contains("YardMap"), "{file}");
        let pad = UnitBlueprint { id: "UEB5202".into(), faction: Faction::Uef, categories: vec!["AIRSTAGINGPLATFORM".into()], ..Default::default() };
        assert!(unit_file(&pad, &none, None, &Extra::default()).contains("IsAirBase=1;"));
        let suicide = WeaponBlueprint { label: "Suicide".into(), category: "Kamikaze".into(), damage: 4500.0, damage_radius: 3.0, max_radius: 3.0, ..Default::default() };
        let beetle = UnitBlueprint { id: "XRL0302".into(), faction: Faction::Cybran, motion: Motion::Land, weapons: vec![suicide], ..Default::default() };
        let w = weapons(&beetle);
        assert!(w.slots.is_empty() && w.kamikaze.is_some());
        let file = unit_file(&beetle, &w, None, &Extra::default());
        for line in ["Kamikaze=1;", "KamikazeDistance=48;", "SelfDestructAs=XRL0302_BLAST;", "canattack=1;"] {
            assert!(file.contains(line), "{line} in {file}");
        }
        assert!(kamikaze_section(&beetle, w.kamikaze.unwrap()).unwrap().contains("default=4500;"));
    }

    /// A gun that shoots no aircraft in Forged Alliance takes them last (its slot's bad targets),
    /// and its unit chases none.
    #[test]
    fn ground_guns_take_aircraft_last() {
        let gun = |label: &str, layers: &str| WeaponBlueprint { label: label.into(), category: "Direct Fire".into(), damage: 50.0, rate_of_fire: 1.0, ballistic_arc: "RULEUBA_None".into(), target_layers: vec![("Land".into(), layers.into())], ..Default::default() };
        let (a, b) = (gun("A", "Land|Water"), gun("B", "Land|Air"));
        let u = UnitBlueprint { id: "UEL0202".into(), faction: Faction::Uef, motion: Motion::Land, ..Default::default() };
        let file = unit_file(&u, &Weapons { slots: vec![&a, &b], death: None, dgun: None, kamikaze: None }, None, &Extra::default());
        assert!(file.contains("wpri_badTargetCategory=VTOL;") && !file.contains("wsec_badTargetCategory") && !file.contains("NoChaseCategory"), "{file}");
        let five: Vec<WeaponBlueprint> = (0..5).map(|k| gun(&format!("G{k}"), "Land")).collect();
        let file = unit_file(&u, &Weapons { slots: five.iter().collect(), death: None, dgun: None, kamikaze: None }, None, &Extra::default());
        assert!(file.contains("BadTargetCategory5=VTOL;") && file.contains("wspe_badTargetCategory=VTOL;") && file.contains("NoChaseCategory=VTOL;"), "{file}");
    }

    /// Anti-air that takes on aircraft alone in Forged Alliance (`FireTargetLayerCapsTable` 'Air' from
    /// every layer, or `TargetRestrictOnlyAllow = 'AIR'` as the interceptors' guns say beside the land
    /// in their layers) names them its only targets, by order too, in its own slot or one an
    /// enhancement fills; a gun that also shoots the ground names none, nor does a missile defence
    /// (its 'Air' the missiles it shoots down).
    #[test]
    fn anti_air_takes_no_other_target() {
        let weapon = |label: &str, layers: &[(&str, &str)]| WeaponBlueprint {
            label: label.into(),
            category: "Anti Air".into(),
            damage: 50.0,
            rate_of_fire: 1.0,
            max_radius: 40.0,
            target_layers: layers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            ..Default::default()
        };
        let (gun, aa) = (weapon("Gun", &[("Land", "Land|Air")]), weapon("AA", &[("Land", "Air"), ("Water", "Air")]));
        let defence = WeaponBlueprint { category: "Defense".into(), only_allow: "TACTICAL MISSILE".into(), ..weapon("AntiMissile", &[("Land", "Air")]) };
        let interceptor = WeaponBlueprint { only_allow: "AIR".into(), ..weapon("Railgun", &[("Air", "Air|Land"), ("Land", "Air|Land")]) };
        assert_eq!((kind(&aa), kind(&defence), kind(&interceptor)), (Kind::AntiAir, Kind::AntiMissile, Kind::AntiAir));
        assert!(aircraft_alone(&aa) && aircraft_alone(&interceptor) && !aircraft_alone(&gun) && !aircraft_alone(&defence));
        let tower = UnitBlueprint { id: "UEB2104".into(), faction: Faction::Uef, ..Default::default() };
        let file = unit_file(&tower, &Weapons { slots: vec![&gun, &aa, &defence, &interceptor], death: None, dgun: None, kamikaze: None }, None, &Extra::default());
        let only: Vec<&str> = file.lines().filter(|l| l.contains("OnlyTargetCategory")).map(str::trim).collect();
        assert_eq!(only, ["OnlyTargetCategory2=VTOL;", "OnlyTargetCategory4=VTOL;"], "{file}");
        let fbi = ta_formats::tdf::parse(&file);
        assert_eq!(fbi.section("UNITINFO").and_then(|u| u.get("OnlyTargetCategory2")), Some("VTOL"));
        // A support commander's gun, and the anti-air missiles an enhancement brings in the slot after it.
        let commander = UnitBlueprint { id: "URL0301".into(), faction: Faction::Cybran, ..Default::default() };
        let file = unit_file(&commander, &Weapons { slots: vec![&gun], death: None, dgun: None, kamikaze: None }, None, &Extra { weapons: vec![&aa], ..Extra::default() });
        let only: Vec<&str> = file.lines().filter(|l| l.contains("OnlyTargetCategory")).map(str::trim).collect();
        assert_eq!(only, ["OnlyTargetCategory2=VTOL;"], "{file}");
    }

    /// A factory's ways out, from its roll-off points and its model: a platform 16 px up (its build
    /// bone's height) over the middle, a ramp down to the ground on its east side. The east way (its
    /// point 96 px out, short of the footprint cleared: 64 and 48) is made to lead 112 px out, its
    /// unit facing east (`UnitSpin` 90), keeping the platform's height to where the ramp starts down
    /// and on the ground where it ends; the south one (160 px out, past the footprint) is kept as it
    /// is, its floor ending with the platform. Written as the unit file's `[EXITS]`, read back as the
    /// game reads it; the factory solid.
    #[test]
    fn a_factorys_ways_out_follow_its_roll_off_points_and_its_platform() {
        let still = [1.0f32, 0.0, 0.0, 0.0];
        let corners = [
            ([-10.0f32, 5.0, -10.0], 0u8),
            ([10.0, 5.0, -10.0], 0),
            ([10.0, 5.0, 10.0], 0),
            ([-10.0, 5.0, 10.0], 0),
            ([20.0, 0.0, -10.0], 0),
            ([20.0, 0.0, 10.0], 0),
        ];
        let vertices: Vec<([f32; 3], [f32; 2], u8)> = corners.iter().map(|&(p, b)| (p, [0.0, 0.0], b)).collect();
        let file = crate::scm::tests::write(&[("Factory", -1, [0.0; 3], still), ("Attachpoint", 0, [0.0, 5.0, 0.0], still)], &vertices, &[[0, 1, 2], [0, 2, 3], [1, 4, 5], [1, 5, 2]]);
        let mesh = crate::scm::parse(&file).unwrap();
        let u = crate::unit::parse(
            "UEB0101",
            "UnitBlueprint { Categories = { 'PRODUCTSC1', 'UEF', 'STRUCTURE', 'FACTORY', 'LAND', 'TECH1', 'BUILTBYTIER1ENGINEER' }, \
             General = { FactionName = 'UEF' }, Economy = { BuildRate = 20, BuildableCategory = { 'BUILTBYTIER1FACTORY UEF MOBILE' } }, \
             Footprint = { SizeX = 4, SizeZ = 4 }, Display = { UniformScale = 0.1, BuildAttachBone = 'Attachpoint' }, \
             Physics = { MotionType = 'RULEUMT_None', RollOffPoints = { { UnitSpin = 90, X = 3, Y = 0, Z = 0 }, { UnitSpin = 0, X = 0, Y = 0, Z = 5 } } } }",
        )
        .unwrap();
        assert_eq!(builds(&u), Builds::Factory);
        let ways = exits(&u, &mesh, &[]);
        assert_eq!(ways.len(), 2);
        let east = ways[0];
        assert_eq!((east.x, east.z, east.heading), (112, 0, 90), "{east:?}");
        // (The platform 32 px out at 16 px; the ramp down to the ground at 64 px.)
        assert!((32..=36).contains(&east.ramp.0) && (60..=64).contains(&east.ramp.1), "{east:?}");
        let south = ways[1];
        assert_eq!((south.x, south.z, south.heading), (0, 160, 0), "{south:?}");
        assert!((31..=33).contains(&south.ramp.0) && south.ramp.1 <= 34, "the floor ends with the platform: {south:?}");
        let file = unit_file(&u, &weapons(&u), None, &Extra { exits: ways.clone(), ..Extra::default() });
        let info = ta_formats::tdf::parse(&file).section("UNITINFO").cloned().unwrap();
        let section = info.sections.iter().find(|s| s.name == "EXITS").expect("[EXITS]");
        assert_eq!(section.sections.len(), 2);
        assert_eq!((section.sections[0].get("X"), section.sections[0].get("Heading")), (Some("112"), Some("90")));
        assert_eq!(section.sections[0].get("Ramp"), Some(format!("{} {}", east.ramp.0, east.ramp.1).as_str()));
        assert!(info.get("YardMap").is_some_and(|y| y.chars().all(|c| c == 'o')), "solid: {file}");
        // A factory without roll-off points has none, and its yard open to its front, as TA's.
        let plain = UnitBlueprint { roll_off: Vec::new(), ..u.clone() };
        assert!(exits(&plain, &mesh, &[]).is_empty());
        assert!(unit_file(&plain, &weapons(&plain), None, &Extra::default()).contains('c'));
        // A transport whose factory's build bone is turned a quarter for it, whatever way it takes,
        // faces so as it is built (`BuildHeading`); a tank names nothing.
        let transport = UnitBlueprint { id: "UEA0104".into(), display: crate::unit::Display { forced_build_spin: Some(90.0), ..Default::default() }, ..Default::default() };
        let file = unit_file(&transport, &weapons(&transport), None, &Extra::default());
        assert_eq!(ta_formats::tdf::parse(&file).section("UNITINFO").and_then(|u| u.get("BuildHeading")), Some("90"), "{file}");
        assert!(!unit_file(&plain, &weapons(&plain), None, &Extra::default()).contains("BuildHeading"));
    }

    #[test]
    fn a_factorys_yard_opens_a_corridor_to_its_front() {
        // Six by six: the back two rows solid, the middle two columns open from there to the front.
        assert_eq!(yard_map(6, 6), format!("{}{}", "o".repeat(12), "ooccoo".repeat(4)));
        // Two by two: all of it the yard.
        assert_eq!(yard_map(2, 2), "cccc");
    }

    #[test]
    fn text_is_made_safe_for_a_tdf() {
        assert_eq!(tdf_text("Gauss; Cannon {x} [y] // z\n"), "Gauss, Cannon (x) (y) / z");
        assert_eq!(tdf_text("a///b /* c"), "a/b / c");
    }
}
