//! Mutators: rules that change how a whole skirmish plays, for every player alike, chosen as it
//! is set up (as Supreme Commander: Forged Alliance's lobby offered): resources flowing faster or
//! slower, construction faster, units tougher or frailer, a ceasefire to build up in (no rush),
//! and whole kinds of unit left out of the build menus. Each is applied where it changes the
//! rules: income (the economy's tick), build power, damage, and what can be built
//! ([`Sim::allowed`], which missions use too and the computer players respect).

use crate::defs::UnitType;
use crate::sim::Sim;
use crate::{Fx, FxVec3, Locomotion, UnitId};

/// A skirmish's mutators (see the module): as TA plays by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Mutators {
    /// Income from every source, in quarters of TA's (4: as TA has it; 2 half, 8 double).
    pub resources: u8,
    /// Build power, as many times TA's (1: as TA has it).
    pub build_speed: u8,
    /// How tough units are, in quarters (4: as TA has it): damage is divided by it, so 8 takes
    /// twice the beating and 2 half.
    pub toughness: u8,
    /// A ceasefire over the battle's first minutes (no rush): weapons do enemies no harm, and
    /// nobody builds near an enemy's start (0: none).
    pub no_rush_minutes: u8,
    /// Kinds of unit left out of the build menus, with the factories that would make only them:
    /// aircraft; ships and submarines; missile silos, nuclear and EMP (and the defences against
    /// them); long-range plasma cannons; and the experimentals (mobile units of 10,000 metal or
    /// more but the Commanders: the Krogoth).
    pub no_air: bool,
    pub no_naval: bool,
    pub no_nukes: bool,
    pub no_artillery: bool,
    pub no_experimentals: bool,
}

impl Default for Mutators {
    fn default() -> Self {
        Mutators { resources: 4, build_speed: 1, toughness: 4, no_rush_minutes: 0, no_air: false, no_naval: false, no_nukes: false, no_artillery: false, no_experimentals: false }
    }
}

/// The ranges at and past which a fixed gun is long-range artillery (world units), and the
/// cost from which a mobile unit is an experimental (metal).
const ARTILLERY_RANGE: i64 = 3000;
const EXPERIMENTAL_METAL: i64 = 10_000;

/// How far round an enemy's start nobody builds in a ceasefire (world units), at most: two
/// fifths of the way between the two starts where they are closer, so each keeps the rest.
const HOME_GROUND: i64 = 1000;

impl Mutators {
    /// Whether any is set (a battle as TA plays it otherwise).
    pub fn any(&self) -> bool {
        *self != Mutators::default()
    }

    /// Whether any kind of unit is left out.
    pub fn restricts(&self) -> bool {
        self.no_air || self.no_naval || self.no_nukes || self.no_artillery || self.no_experimentals
    }

    /// Mutators from a list such as `resources=8,no_rush=5,no_air` (the tests' and captures'
    /// `TA_MUTATORS`): each field by name, with a number, or a switch alone (`=0` off); what it
    /// doesn't name is as TA has it.
    pub fn parse(spec: &str) -> Mutators {
        let mut m = Mutators::default();
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=').map_or((part, None), |(k, v)| (k.trim(), Some(v.trim())));
            let number = |was: u8| value.and_then(|v| v.parse().ok()).unwrap_or(was);
            let on = value.is_none_or(|v| v != "0" && v != "false");
            match key {
                "resources" => m.resources = number(m.resources),
                "build_speed" => m.build_speed = number(m.build_speed),
                "toughness" => m.toughness = number(m.toughness),
                "no_rush" | "no_rush_minutes" => m.no_rush_minutes = number(m.no_rush_minutes),
                "no_air" => m.no_air = on,
                "no_naval" => m.no_naval = on,
                "no_nukes" => m.no_nukes = on,
                "no_artillery" => m.no_artillery = on,
                "no_experimentals" => m.no_experimentals = on,
                other => log::warn!("unknown mutator {other:?}"),
            }
        }
        m
    }

    /// The ceasefire's length in ticks (0: none).
    pub fn ceasefire_ticks(&self) -> u64 {
        u64::from(self.no_rush_minutes) * 60 * crate::cob::TICKS_PER_SECOND as u64
    }
}

/// Income `v` (a tick's, from one unit) as the resources mutator `quarters` has it.
pub(crate) fn income(v: Fx, quarters: u8) -> Fx {
    match quarters {
        4 => v,
        q => v * i64::from(q) / 4,
    }
}

impl Sim {
    /// Sets the battle's mutators (as it is made): kinds of unit left out of what can be built,
    /// with what any mission already leaves out.
    pub fn set_mutators(&mut self, mutators: Mutators) {
        self.mutators = mutators;
        if !mutators.restricts() {
            return;
        }
        let types = self.types.clone();
        let out = |t: &UnitType| -> bool {
            let weapons = || t.weapons.iter().flatten().map(|&w| self.weapon_types.get(w));
            (mutators.no_air && t.locomotion == Locomotion::Air)
                || (mutators.no_naval && t.locomotion == Locomotion::Naval)
                || (mutators.no_nukes && weapons().any(|w| w.def.targetable || w.def.interceptor))
                || (mutators.no_artillery && t.locomotion == Locomotion::Static && weapons().any(|w| !w.def.targetable && !w.def.interceptor && w.range >= Fx::from_int(ARTILLERY_RANGE)))
                || (mutators.no_experimentals && t.locomotion != Locomotion::Static && !t.def.is_commander && t.cost_metal >= Fx::from_int(EXPERIMENTAL_METAL))
        };
        let mut allowed: Vec<bool> = types.types.iter().enumerate().map(|(i, t)| self.is_allowed(i) && !out(t)).collect();
        // A factory none of whose units may be built goes too: an aircraft plant without
        // aircraft. (What else its menu lists stays out of it: the Core's plants offer a
        // building.)
        let mobile = |o: usize| types.types.get(o).is_some_and(|p| p.locomotion != Locomotion::Static);
        for (i, t) in types.types.iter().enumerate() {
            let mut units = t.ai_build_options.iter().copied().filter(|&o| mobile(o)).peekable();
            if t.is_factory && units.peek().is_some() && units.all(|o| !allowed[o]) {
                allowed[i] = false;
            }
        }
        self.allowed = Some(allowed);
    }

    /// Ticks of the ceasefire left (none once it is over, or without one).
    pub fn ceasefire_left(&self) -> Option<u64> {
        let end = self.mutators.ceasefire_ticks();
        (self.tick < end).then(|| end - self.tick)
    }

    /// Whether the ceasefire keeps `source` from harming `victim` (an enemy's unit, while it
    /// lasts).
    pub(crate) fn ceasefire_spares(&self, source: UnitId, victim: UnitId) -> bool {
        self.unit(source).zip(self.unit(victim)).is_some_and(|(s, v)| self.ceasefire_spares_side(s.owner, v.owner))
    }

    /// Projectile ownership survives the firing unit, including during a ceasefire.
    pub(crate) fn ceasefire_spares_side(&self, source: u8, victim: u8) -> bool {
        self.ceasefire_left().is_some() && !self.allied(source, victim)
    }

    /// Whether the ceasefire keeps `player` from building at `pos`: near an enemy's start
    /// (see [`HOME_GROUND`]), while it lasts.
    pub fn ceasefire_bars(&self, player: u8, pos: FxVec3) -> bool {
        let Some(own) = self.players.get(player as usize).map(|p| p.start) else { return false };
        self.ceasefire_left().is_some()
            && self.players.iter().any(|p| {
                let reach = Fx::from_int(HOME_GROUND).min(p.start.distance_xz(own) * 2 / 5);
                !self.allied(player, p.index) && !p.defeated && pos.within_xz(p.start, reach)
            })
    }

    /// Build power `rate` (work a tick) as the build speed mutator has it.
    pub(crate) fn mutator_work_rate(&self, rate: u64) -> u64 {
        rate.saturating_mul(u64::from(self.mutators.build_speed.max(1)))
    }

    /// `damage` as the mutators have it: divided by how tough units are (an enemy's in a
    /// ceasefire does none at all: [`Sim::ceasefire_spares`]).
    pub(crate) fn mutator_damage(&self, damage: Fx) -> Fx {
        match self.mutators.toughness {
            4 | 0 => damage,
            q => damage * 4 / i64::from(q),
        }
    }
}
