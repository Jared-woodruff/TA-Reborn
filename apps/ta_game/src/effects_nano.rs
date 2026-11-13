//! The nanolathe: a stream of glowing particles from a builder's nano emitter to whatever it
//! is building, repairing, capturing or resurrecting (and back from what it reclaims).
//!
//! Presentation only; the emitter is found by piece name (TA models call it `nanospray`,
//! `beam`, `nano1`...), so no unit script is run for it.

use glam::Vec3;
use ta_render::BeamInstance;
use ta_render::sprites::SpriteInstance;
use ta_sim::{Order, Sim, Unit, UnitId};

use super::Effects;

/// One nano particle, flying from `from` to `to`.
pub(super) struct Nano {
    from: Vec3,
    to: Vec3,
    born: f32,
    life: f32,
    /// Sideways wobble.
    swirl: Vec3,
}

const NANO_MAX: usize = 4_000;
/// Particles per second from a builder with TA's typical 100 worker time.
const NANO_RATE: f32 = 24.0;

/// Emitter pieces by preference.
fn emitter_rank(name: &str) -> Option<u8> {
    let n = name.to_ascii_lowercase();
    if n == "nanospray" || n.starts_with("nanospray") {
        Some(0)
    } else if n == "nanogun" || n == "nano" || n == "nano1" || n == "beam" || n == "beam1" {
        Some(1)
    } else if n.starts_with("nano") || n.starts_with("beam") {
        Some(2)
    } else if n.starts_with("flare") {
        Some(3)
    } else {
        None
    }
}

impl Effects {
    /// Model piece of a unit type's nano emitter, looked up once per type.
    fn nano_piece(&mut self, sim: &Sim, type_id: usize) -> Option<usize> {
        if let Some(&p) = self.nano_pieces.get(&type_id) {
            return p;
        }
        let t = sim.types.get(type_id);
        let p = t.model.pieces.iter().enumerate().filter_map(|(i, p)| emitter_rank(&p.name).map(|r| (r, i))).min().map(|(_, i)| i);
        self.nano_pieces.insert(type_id, p);
        p
    }

    fn unit_centre(sim: &Sim, u: &Unit) -> Vec3 {
        Vec3::from_array(u.pos.to_f32()) + Vec3::Y * (sim.unit_type(u).model.height.to_f32() * 0.5)
    }

    /// What a builder is working on: (point, radius, towards the builder?).
    fn nano_target(sim: &Sim, u: &Unit) -> Option<(Vec3, f32, bool)> {
        // Scripts can leave IN_BUILD_STANCE set while a constructor travels to its
        // next job. A pose alone must never draw a beam across the map.
        if !u.in_build_stance || u.dead { return None; }
        let unit_target = |id: UnitId, reverse: bool| sim.unit(id)
            .filter(|t| !t.dead && (t.factory == Some(u.id) || sim.in_build_range(u, t.pos, sim.unit_type(t).footprint)))
            .map(|t| (Self::unit_centre(sim, t), sim.unit_type(t).radius.to_f32() * 0.8, reverse));
        let feature = |i: usize, reverse: bool| {
            sim.features.get(i).filter(|f| f.alive && {
                let def = &sim.data.features[f.def];
                sim.in_build_range(u, f.pos, (def.footprint_x, def.footprint_z))
            }).map(|f| {
                let def = &sim.data.features[f.def];
                (Vec3::from_array(f.pos.to_f32()) + Vec3::Y * 6.0, def.footprint_x.max(def.footprint_z) as f32 * 6.0, reverse)
            })
        };
        match u.orders.front() {
            Some(Order::Reclaim(f)) => feature(*f, true),
            Some(Order::Resurrect(f)) => feature(*f, false),
            Some(Order::ReclaimUnit(t)) => unit_target(*t, true),
            Some(Order::Capture(t)) => unit_target(*t, false),
            _ => u.build.filter(|b| b.started && b.target != u.id).and_then(|b| unit_target(b.target, false)),
        }
    }

    /// Spawns and moves nano particles; draws the ones in view.
    pub(super) fn update_nano(&mut self, sim: &Sim, now: f32, dt: f32) {
        let view = self.view;
        let in_view = |p: Vec3| view.is_none_or(|(lo, hi)| p.x >= lo.x - 64.0 && p.x <= hi.x + 64.0 && p.z >= lo.y - 64.0 && p.z <= hi.y + 64.0);
        let mut streams = Vec::new();
        for u in sim.units() {
            if !u.in_build_stance || u.dead || !in_view(Vec3::from_array(u.pos.to_f32())) {
                continue;
            }
            if self.viewer.is_some_and(|pl| sim.seen_by(u, pl) != ta_sim::Seen::Visible) {
                continue;
            }
            let Some(target) = Self::nano_target(sim, u) else { continue };
            let t = sim.unit_type(u);
            streams.push((u.id, u.type_id, target, t.worker_rate.to_f32() * 30.0));
        }
        self.nano_lights.clear();
        for (id, type_id, (to, radius, reverse), worker) in streams {
            self.nano_lights.push((to, radius));
            let Some(u) = sim.unit(id) else { continue };
            let t = sim.unit_type(u);
            // The piece the unit's script names (factories alternate between two), else one
            // named like an emitter.
            let scripted = u.nano_piece.and_then(|p| u.cob.as_ref()?.program.piece_map.get(p as usize).copied().flatten());
            let from = match scripted.or_else(|| self.nano_piece(sim, type_id)) {
                Some(p) => Vec3::from_array(ta_sim::unit::piece_world_position(u, t, u.cob.as_ref(), p).to_f32()),
                None => Vec3::from_array(u.pos.to_f32()) + Vec3::Y * t.model.height.to_f32(),
            };
            // More workers, thicker stream (a Commander's 300 sprays three times a
            // constructor's 100).
            let rate = NANO_RATE * (worker / 100.0).clamp(0.5, 4.0);
            let mut n = rate * dt + self.rand();
            while n >= 1.0 && self.ambient_nano_len() < NANO_MAX {
                n -= 1.0;
                let spot = to + Vec3::new((self.rand() - 0.5) * 2.0, self.rand() * 0.3, (self.rand() - 0.5) * 2.0) * radius;
                let (a, b) = if reverse { (spot, from) } else { (from, spot) };
                let life = (a.distance(b) / 260.0).clamp(0.2, 1.4);
                let swirl = (Vec3::new(self.rand(), self.rand(), self.rand()) - 0.5) * 18.0;
                self.nano.push(Nano { from: a, to: b, born: now, life, swirl });
            }
        }
        self.nano.retain(|p| now - p.born < p.life);
        // Out along a gentle arc, the wobble fading as the particle arrives.
        let path = |p: &Nano, k: f32| p.from.lerp(p.to, k) + p.swirl * (k * std::f32::consts::PI).sin() + Vec3::Y * (k * (1.0 - k) * 16.0);
        for p in &self.nano {
            let k = ((now - p.born) / p.life).clamp(0.0, 1.0);
            let pos = path(p, k);
            if !in_view(pos) {
                continue;
            }
            // A fine streak of green light, a tenth of its way long, with a bright head that
            // flares as it reaches the work: a stream of sparks. (Large soft glows stacked up
            // where a Commander works close by into a white cloud hiding its arm.)
            let fade = (k * 5.0).min(1.0) * (1.0 - k * k);
            let arrive = ((k - 0.8) / 0.2).clamp(0.0, 1.0);
            let spark = arrive * (1.0 - arrive) * 4.0;
            let streak = BeamInstance::new(pos.to_array(), path(p, (k - 0.1).max(0.0)).to_array(), 2.4, [0.6 * fade, 2.4 * fade, 0.75 * fade], 1.0);
            self.beams.push(streak.core(0.2).tapered(1.2, 1.0).cover(0.4));
            self.additive.push(SpriteInstance::new(pos.to_array(), self.glow).scale(0.045 + 0.04 * spark).tint([0.35, 1.0, 0.45]).brightness(1.6 * fade + 2.2 * spark));
        }
    }

    fn ambient_nano_len(&self) -> usize {
        self.nano.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ta_sim::{Fx, FxVec3};

    #[test]
    fn a_stale_build_stance_never_draws_a_distant_or_unstarted_assist() {
        let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
        for text in [
            "[UNITINFO] { UnitName=SUB; Builder=1; CanMove=1; MaxVelocity=2; BuildDistance=64; MinWaterDepth=20; Floater=1; }",
            "[UNITINFO] { UnitName=BUILDING; MaxDamage=1000; BuildTime=600; FootprintX=4; FootprintZ=4; MinWaterDepth=20; Floater=1; }",
        ] {
            data.units.push(ta_data::UnitDef::from_tdf(&ta_formats::tdf::parse(text).sections[0]).unwrap());
        }
        let data = Arc::new(data);
        let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 64, height: 64, sea_level: 100, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![0; 4096], features: vec![ta_formats::tnt::FEATURE_NONE; 4096], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let player = ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false };
        let mut sim = Sim::new(types, data, &map, &Default::default(), 0, &[player], 7);
        let sub = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 0, 100), 0, true).unwrap();
        let target = sim.spawn_unit(1, 0, FxVec3::from_ints(750, 0, 750), 0, false).unwrap();
        let mut builder = sim.unit(sub).unwrap().clone();
        builder.in_build_stance = true;
        builder.build = Some(ta_sim::unit::BuildJob { target, started: true });
        assert!(Effects::nano_target(&sim, &builder).is_none(), "no beam across the sea");
        let close = sim.unit(target).unwrap().pos + FxVec3::from_ints(-80, 0, 0);
        builder.pos = close;
        assert!(Effects::nano_target(&sim, &builder).is_some(), "beam starts within range");
        builder.build.as_mut().unwrap().started = false;
        assert!(Effects::nano_target(&sim, &builder).is_none(), "wait for the assist job to start");
        builder.build.as_mut().unwrap().started = true;
        sim.remove_unit(target);
        assert!(Effects::nano_target(&sim, &builder).is_none(), "no beam to a removed target");
    }
}
