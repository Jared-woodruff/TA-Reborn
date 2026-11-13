//! What a unit makes and uses each second, as the economy counts it: the figures the interface
//! shows over a building (the hover card, TA's unit bar, the placement ghost). Read-only: a
//! mirror of what `Sim::economy_end` credits and charges a unit each tick, taken a second at a
//! time, so that the figures of all the units add up to the income and the spending the top
//! bar shows.

use crate::cob::TICKS_PER_SECOND;
use crate::fixed::{Fx, FxVec3};
use crate::sim::Sim;
use crate::unit::Unit;
use crate::world::World;

/// Metal and energy made and used a second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitFlows {
    pub metal_make: Fx,
    pub metal_use: Fx,
    pub energy_make: Fx,
    pub energy_use: Fx,
}

impl std::ops::AddAssign for UnitFlows {
    fn add_assign(&mut self, other: UnitFlows) {
        self.metal_make += other.metal_make;
        self.metal_use += other.metal_use;
        self.energy_make += other.energy_make;
        self.energy_use += other.energy_use;
    }
}

impl UnitFlows {
    /// The smallest rate worth showing, a second's twentieth of a unit.
    const SHOWN: Fx = Fx(65536 / 20);

    /// Whether any of the four is worth showing.
    pub fn any(&self) -> bool {
        [self.metal_make, self.metal_use, self.energy_make, self.energy_use].iter().any(|&v| v >= Self::SHOWN)
    }
}

impl Sim {
    /// What a unit of `type_id` finished and switched on at `pos` would make and use a second:
    /// its type's production and upkeep, the wind's and the tide's share, the metal under an
    /// extractor there.
    pub fn type_flows(&self, type_id: usize, pos: FxVec3) -> UnitFlows {
        let t = self.types.get(type_id);
        let origin = World::footprint_origin(World::pos_to_anchor(pos, t.footprint), t.footprint);
        self.flows_of(type_id, true, true, Some(origin))
    }

    /// What `u` makes and uses a second now: a finished unit's production and upkeep (while it is
    /// switched on and the upkeep was paid: an extractor or metal maker short of energy makes and
    /// uses nothing), what its cloak costs, and what its build job or missile stockpile draws at full pace.
    /// A building's are its own and those of the modules it has taken on (a mod's upgrade: TA:
    /// Escalation's fusion reactor's +250 energy), which the economy counts as it would standing
    /// (`Sim::economy_end`); a module's own are none, counted in its building's, and a transport's
    /// passengers make and use nothing.
    pub fn unit_flows(&self, u: &Unit) -> UnitFlows {
        if u.carried_by.is_some() {
            return UnitFlows::default();
        }
        let mut f = self.own_flows(u);
        if self.unit_type(u).transport_capacity == 0 {
            for module in u.cargo.iter().filter_map(|&m| self.unit(m)).filter(|m| m.carried_by == Some(u.id)) {
                f += self.own_flows(module);
            }
        }
        f
    }

    /// What `u` itself makes and uses a second, as [`Sim::unit_flows`] has it: a module on its
    /// building as though it stood (it has no cloak to pay for, nor builds).
    fn own_flows(&self, u: &Unit) -> UnitFlows {
        let t = self.unit_type(u);
        let mut f = UnitFlows::default();
        if u.dead || self.tick < u.paralyzed_until {
            return f;
        }
        if u.is_complete() {
            // (As the economy takes it: upkeep is paid while switched on, which TA does when a
            // unit is built or the player does; what is neither idles for free.)
            let active = u.activated || !t.def.on_offable && t.energy_use.is_zero();
            f = self.flows_of(u.type_id, active, u.upkeep_paid, u.occupied_origin);
        }
        if u.cloak_on && t.def.cloak_cost > 0.0 && u.carried_by.is_none() {
            let cost = if u.moving && t.def.cloak_cost_moving > 0.0 { t.def.cloak_cost_moving } else { t.def.cloak_cost };
            f.energy_use += Fx::from_f64(cost);
        }
        // Building: a second of its work, as a share of what it builds, at the builder's pace (as
        // `Sim::apply_build_power` has it: the engineer's perk and the build-speed mutator too;
        // nothing for mending one being taken apart).
        let working = u.is_complete() && u.carried_by.is_none() && self.tick >= u.paralyzed_until;
        if working && let Some(target) = u.build.and_then(|b| self.unit(b.target)).filter(|b| !b.dead && b.carried_by.is_none() && !b.is_complete() && !self.being_taken_apart(b)) {
            let rate = self.mutator_work_rate(self.perk_work_rate(u.id, t.work_rate));
            let (tt, per_second) = (self.unit_type(target), rate.saturating_mul(TICKS_PER_SECOND as u64));
            f.metal_use += tt.cost_metal.share(per_second, tt.build_work.max(1));
            f.energy_use += tt.cost_energy.share(per_second, tt.build_work.max(1));
        }
        if working && u.in_build_stance && u.build.is_some() && let Some(crate::unit::Order::Resurrect(feature)) = u.orders.front()
            && let Some(type_id) = self.wreck_type(*feature).filter(|&t| self.is_allowed(t))
        {
            let rate = self.mutator_work_rate(self.perk_work_rate(u.id, t.work_rate));
            let rt = self.types.get(type_id);
            if self.features[*feature].resurrect_work < rt.build_work {
                f.energy_use += rt.cost_energy.share(rate.saturating_mul(TICKS_PER_SECOND as u64), rt.build_work);
            }
        }
        // Missile construction has no nanoframe/build job, but pays continuously just as
        // one does. Include it in the unit card's draw, stopping when full or switched off.
        if u.is_complete() && u.carried_by.is_none() && self.tick >= u.paralyzed_until
            && u.stockpile < crate::combat::MAX_STOCKPILE && (u.activated || !t.def.on_offable)
            && let Some(w) = t.weapons.iter().flatten().map(|&w| self.weapon_types.get(w)).find(|w| w.def.stockpile)
        {
            f.metal_use += w.metal_per_shot.share(TICKS_PER_SECOND as u64, w.reload_ticks.max(1) as u64);
            f.energy_use += w.energy_per_shot.share(TICKS_PER_SECOND as u64, w.reload_ticks.max(1) as u64);
        }
        f
    }

    fn flows_of(&self, type_id: usize, active: bool, paid: bool, origin: Option<(i32, i32)>) -> UnitFlows {
        let t = self.types.get(type_id);
        let per_tick = |v: Fx| v / TICKS_PER_SECOND;
        let mut energy = t.energy_make;
        let mut metal = t.metal_make;
        if t.wind_generator > Fx::ZERO {
            energy += per_tick(self.wind.min(t.wind_generator));
        }
        if t.tidal_generator > Fx::ZERO {
            energy += per_tick(self.tidal_strength() * t.tidal_generator);
        }
        let mut upkeep = Fx::ZERO;
        if active {
            // Negative EnergyUse is production while active (solar collectors).
            if t.energy_use < Fx::ZERO {
                energy -= t.energy_use;
            } else if paid {
                upkeep = t.energy_use;
            }
        }
        if active && paid {
            if t.extracts_metal > Fx::ZERO && let Some(origin) = origin {
                metal += per_tick(t.extracts_metal * Fx::from_int(self.world.metal_under(origin, t.footprint)));
            }
            if t.makes_metal > Fx::ZERO {
                metal += t.makes_metal;
            }
        }
        // (The resources mutator's flow.)
        let resources = self.mutators.resources;
        UnitFlows {
            metal_make: crate::mutators::income(metal, resources) * TICKS_PER_SECOND,
            metal_use: Fx::ZERO,
            energy_make: crate::mutators::income(energy, resources) * TICKS_PER_SECOND,
            energy_use: upkeep * TICKS_PER_SECOND,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit::PlayerSetup;
    use std::sync::Arc;

    fn data_of(files: &[(String, String)]) -> ta_data::GameData {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("ta-sim-flows-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        std::fs::remove_dir_all(&dir).ok();
        data
    }

    fn unit(name: &str, keys: &str) -> (String, String) {
        (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nFootprintX=2;\nFootprintZ=2;\nBuildTime=100;\n{keys}\n}}\n"))
    }

    fn sim(files: &[(String, String)]) -> Sim {
        let data = Arc::new(data_of(files));
        let types = Arc::new(crate::defs::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap {
            width: 64,
            height: 64,
            sea_level: 0,
            tiles: Vec::new(),
            tile_map: Vec::new(),
            heights: vec![60; 64 * 64],
            features: vec![ta_formats::tnt::FEATURE_NONE; 64 * 64],
            feature_names: Vec::new(),
            minimap: Default::default(),
        };
        let players = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
        Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7)
    }

    /// The figures of every unit add up to what the economy counts: the top bar's income and
    /// spending (a second's, after a tick).
    #[test]
    fn the_figures_of_the_units_add_up_to_the_economy() {
        let mut sim = sim(&[
            unit("SOLAR", "EnergyMake=20;"),
            unit("SOLAR2", "EnergyUse=-15; OnOffable=1; ActivateWhenBuilt=1;"),
            unit("WIND", "WindGenerator=30;"),
            unit("MEX", "ExtractsMetal=0.001; EnergyUse=3; ActivateWhenBuilt=1;"),
            unit("MAKER", "MakesMetal=1; EnergyUse=60; OnOffable=1; ActivateWhenBuilt=1;"),
            unit("RADAR", "EnergyUse=25; OnOffable=1; ActivateWhenBuilt=1; RadarDistance=1500;"),
            unit("DEPOT", "MetalStorage=100; EnergyStorage=200;"),
        ]);
        // Ground rich in metal under the extractor.
        let at = FxVec3::from_ints(300, 60, 300);
        let t = sim.types.id("MEX").unwrap();
        let origin = World::footprint_origin(World::pos_to_anchor(at, sim.types.get(t).footprint), sim.types.get(t).footprint);
        for dz in 0..2 {
            for dx in 0..2 {
                let i = ((origin.1 + dz) as usize) * sim.world.width as usize + (origin.0 + dx) as usize;
                sim.world.metal[i] = 255;
            }
        }
        let mut ids = Vec::new();
        for (k, name) in ["SOLAR", "SOLAR2", "WIND", "MEX", "MAKER", "RADAR", "DEPOT"].into_iter().enumerate() {
            let type_id = sim.types.id(name).unwrap();
            let pos = if name == "MEX" { at } else { FxVec3::from_ints(100 + k as i64 * 80, 60, 700) };
            ids.push((name, sim.spawn_unit(type_id, 0, pos, 0, true).unwrap()));
        }
        sim.step(&[]);
        sim.step(&[]);
        let figure = |name: &str| sim.unit_flows(sim.unit(ids.iter().find(|(n, _)| *n == name).unwrap().1).unwrap());
        let (solar, solar2, maker, radar, mex, depot) = (figure("SOLAR"), figure("SOLAR2"), figure("MAKER"), figure("RADAR"), figure("MEX"), figure("DEPOT"));
        // (A tick's share is held to a 65536th, so a second's is a little off a round number.)
        let near = |a: Fx, b: i64| assert!((a - Fx::from_int(b)).0.abs() < 40, "{a:?} against {b}");
        near(solar.energy_make, 20);
        near(solar2.energy_make, 15);
        near(maker.metal_make, 1);
        near(maker.energy_use, 60);
        near(radar.energy_use, 25);
        assert!(mex.metal_make > Fx::ZERO, "metal under the extractor: {mex:?}");
        near(mex.energy_use, 3);
        assert!(!depot.any(), "storage makes and uses nothing");
        let wind = figure("WIND");
        assert!((wind.energy_make - sim.wind.min(Fx::from_int(30))).0.abs() < 40, "the wind's share, as strong as it blows now: {wind:?} in a wind of {:?}", sim.wind);
        let (mut metal_make, mut energy_make, mut energy_use) = (Fx::ZERO, Fx::ZERO, Fx::ZERO);
        for (_, id) in &ids {
            let f = sim.unit_flows(sim.unit(*id).unwrap());
            (metal_make, energy_make, energy_use) = (metal_make + f.metal_make, energy_make + f.energy_make, energy_use + f.energy_use);
        }
        let economy = &sim.players[0].economy;
        assert_eq!(metal_make, economy.metal_income, "metal made");
        assert_eq!(energy_make, economy.energy_income, "energy made");
        assert_eq!(energy_use, economy.energy_expense, "energy used");
        // Switched off, a unit stops using (and making) what it does only while on.
        let radar_id = ids.iter().find(|(n, _)| *n == "RADAR").unwrap().1;
        sim.step(&[crate::sim::Command::SetActive { player: 0, units: vec![radar_id], on: false }]);
        assert!(!sim.unit_flows(sim.unit(radar_id).unwrap()).any(), "off: no upkeep");
        // And a unit of a type put somewhere (a ghost) is judged as if finished there.
        let ghost = sim.type_flows(t, at);
        assert_eq!(ghost.metal_make, sim.extraction_at(t, at), "the placement ghost says what it would make there");
        near(ghost.energy_use, 3);
        near(sim.type_flows(sim.types.id("SOLAR").unwrap(), at).energy_make, 20);
    }

    /// A building's figures take in the module it has taken on (a mod's upgrade), which the economy
    /// counts as standing; a transport's passenger makes nothing, in the figures or the economy;
    /// and every unit's figures still add up to the economy's.
    #[test]
    fn a_building_shows_what_its_module_makes() {
        let mut sim = sim(&[
            unit("REACTOR", "EnergyMake=1000;"),
            unit("MODULE", "EnergyMake=250; EnergyStorage=200; FootprintX=0; FootprintZ=0;"),
            unit("TRANSPORT", "TransportCapacity=1; CanMove=1; MaxVelocity=2; Acceleration=0.1;"),
            unit("PASSENGER", "EnergyMake=25; CanMove=1; MaxVelocity=1; Acceleration=0.1;"),
        ]);
        let id = |sim: &Sim, name: &str| sim.types.id(name).unwrap();
        let reactor = sim.spawn_unit(id(&sim, "REACTOR"), 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let module = sim.spawn_unit(id(&sim, "MODULE"), 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let transport = sim.spawn_unit(id(&sim, "TRANSPORT"), 0, FxVec3::from_ints(600, 60, 300), 0, true).unwrap();
        let passenger = sim.spawn_unit(id(&sim, "PASSENGER"), 0, FxVec3::from_ints(600, 60, 300), 0, true).unwrap();
        sim.attach(reactor, module, None);
        sim.attach(transport, passenger, None);
        sim.step(&[]);
        sim.step(&[]);
        let figures = |id| sim.unit_flows(sim.unit(id).unwrap());
        let near = |a: Fx, b: i64| assert!((a - Fx::from_int(b)).0.abs() < 40, "{a:?} against {b}");
        near(figures(reactor).energy_make, 1250);
        assert!(!figures(module).any(), "the module's are its building's");
        assert!(!figures(transport).any() && !figures(passenger).any(), "a passenger makes nothing");
        let mut energy_make = Fx::ZERO;
        for id in [reactor, module, transport, passenger] {
            energy_make += figures(id).energy_make;
        }
        assert_eq!(energy_make, sim.players[0].economy.energy_income, "the figures add up to the economy's");
    }

    /// A builder's draw is the cost of what it builds, a second's share of it at its pace.
    #[test]
    fn a_builders_draw_is_a_seconds_share_of_the_cost() {
        let mut sim = sim(&[
            unit("BUILDER", "Builder=1; WorkerTime=10; CanMove=1; MaxVelocity=2; BuildDistance=200;"),
            unit("HUT", "BuildCostMetal=100; BuildCostEnergy=400; BuildTime=100;"),
        ]);
        let (b, h) = (sim.types.id("BUILDER").unwrap(), sim.types.id("HUT").unwrap());
        let builder = sim.spawn_unit(b, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let frame = sim.spawn_unit(h, 0, FxVec3::from_ints(340, 60, 300), 0, false).unwrap();
        sim.step(&[crate::sim::Command::Order { player: 0, units: vec![builder], order: crate::unit::Order::Repair(frame), queue: false }]);
        for _ in 0..90 {
            sim.step(&[]);
        }
        let u = sim.unit(builder).unwrap();
        assert_eq!(u.build.map(|j| j.target), Some(frame), "it is building");
        let f = sim.unit_flows(u);
        // WorkerTime 10 a second of a build time of 100: a tenth of the cost each second.
        assert_eq!((f.metal_use, f.energy_use), (Fx::from_int(10), Fx::from_int(40)));
        assert_eq!((f.metal_make, f.energy_make), (Fx::ZERO, Fx::ZERO));
        // At double build speed (the mutator), twice the work and twice the draw, as the
        // simulation spends it.
        sim.mutators.build_speed = 2;
        let f = sim.unit_flows(sim.unit(builder).unwrap());
        assert_eq!((f.metal_use, f.energy_use), (Fx::from_int(20), Fx::from_int(80)));
    }

    /// A metal maker short of energy makes and uses nothing, as the economy has it: the figures
    /// do not go on showing its full output while the player's power is out.
    #[test]
    fn a_maker_without_the_energy_for_it_makes_nothing() {
        let mut sim = sim(&[unit("MAKER", "MakesMetal=1; EnergyUse=60; OnOffable=1; ActivateWhenBuilt=1;")]);
        let maker = sim.spawn_unit(sim.types.id("MAKER").unwrap(), 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        sim.step(&[]);
        let f = sim.unit_flows(sim.unit(maker).unwrap());
        assert!(f.metal_make > Fx::ZERO && f.energy_use > Fx::ZERO, "with energy in store, it works: {f:?}");
        // The store empty, and nothing coming in: no upkeep paid, no metal made.
        sim.players[0].economy.energy = Fx::ZERO;
        sim.step(&[]);
        let f = sim.unit_flows(sim.unit(maker).unwrap());
        assert_eq!((f.metal_make, f.energy_use), (Fx::ZERO, Fx::ZERO), "out of energy: {f:?}");
        assert_eq!(sim.players[0].economy.metal_income, Fx::ZERO, "and the economy says the same");
        // (A ghost of one placed is judged as switched on with the energy for it.)
        assert!(sim.type_flows(sim.types.id("MAKER").unwrap(), FxVec3::from_ints(500, 60, 300)).metal_make > Fx::ZERO);
    }
}
