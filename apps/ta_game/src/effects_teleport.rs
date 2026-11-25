//! Teleporting, seen (`Order::Teleport`: `ta_sim`'s `sim/teleport.rs`): a unit charging to teleport
//! gathers a blue glow and crackles with sparks drawn up round it, the more as its charge comes
//! (`Unit::teleport`); gone, it leaves a flash, a ring and a scatter of sparks where it stood, and
//! comes in with the same where it lands (`SimEvent::Teleported`). Each end shown where the viewer
//! sees it.

use glam::Vec3;
use ta_render::{ParticleBurst as Burst, ParticleKind as Kind};
use ta_sim::{FxVec3, Sim};

use super::Effects;

/// The blue of a teleport's light.
const BLUE: [f32; 3] = [0.5, 0.78, 1.0];
/// How far past the view a charging unit still sparks (world units).
const VIEW_MARGIN: f32 = 200.0;

impl Effects {
    /// This frame's charging teleports: sparks drawn up round each unit charging (the more, the
    /// further along its charge), and a glow over it.
    pub(super) fn update_teleports(&mut self, sim: &Sim, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let view = self.view;
        let charging: Vec<(Vec3, f32, f32, f32)> = sim
            .units()
            .filter(|u| !u.dead && u.carried_by.is_none())
            .filter_map(|u| {
                let charge = u.teleport?;
                let t = sim.unit_type(u);
                Some((Vec3::from_array(u.pos.to_f32()), t.radius.to_f32().max(8.0), t.model.height.to_f32().max(8.0), charge.progress().to_f32()))
            })
            .filter(|(p, ..)| view.is_none_or(|(lo, hi)| p.x >= lo.x - VIEW_MARGIN && p.x <= hi.x + VIEW_MARGIN && p.z >= lo.y - VIEW_MARGIN && p.z <= hi.y + VIEW_MARGIN))
            .collect();
        for (p, size, height, progress) in charging {
            if !self.sees(sim, p) {
                continue;
            }
            // (A dozen bursts a second at the start, three dozen as it is about to go.)
            if self.rand() < dt * (12.0 + 24.0 * progress) {
                let a = self.rand() * std::f32::consts::TAU;
                let at = p + Vec3::new(a.cos(), 0.0, a.sin()) * size * (0.8 + 0.4 * self.rand()) + Vec3::Y * height * 0.1;
                let inward = (p + Vec3::Y * height * 0.9 - at).normalize_or(Vec3::Y);
                self.burst(Burst::new(Kind::Spark, at.to_array(), 3 + (progress * 4.0) as u32).toward(inward.to_array(), 0.25).speed(30.0, 70.0).life(0.25, 0.5).size(1.6, 0.6).color(BLUE).intensity(6.0).cold().collide(false));
            }
            if self.rand() < dt * 6.0 {
                let glow = size * (0.8 + 1.2 * progress);
                self.burst(Burst::new(Kind::Glow, (p + Vec3::Y * height * 0.6).to_array(), 1).radius(size * 0.2).speed(0.0, 4.0).life(0.25, 0.4).size(glow, glow * 1.3).color(BLUE).intensity(1.2 + 2.0 * progress));
            }
        }
    }

    /// A unit gone from `from` and landed at `to`: a flash, a ring and sparks at each end the
    /// viewer sees, as big as the unit.
    pub(super) fn teleported(&mut self, sim: &Sim, id: ta_sim::UnitId, from: FxVec3, to: FxVec3, now: f32) {
        let size = sim.unit(id).map_or(16.0, |u| sim.unit_type(u).radius.to_f32().max(8.0));
        for (end, arriving) in [(from, false), (to, true)] {
            let p = Vec3::from_array(end.to_f32());
            if !self.sees(sim, p) {
                continue;
            }
            let reach = (size * 4.0).clamp(40.0, 240.0);
            self.flash(p + Vec3::Y * size, now, if arriving { 0.4 } else { 0.3 }, reach, BLUE, 2.8);
            self.light(p + Vec3::Y * 30.0, now, 0.9, reach * 3.0, [1.4, 2.4, 4.6]);
            self.ripple(p, now, 0.6, reach, [0.35, 0.75, 2.2], 0.5, 6.0);
            let n = (size * 3.0).clamp(24.0, 120.0) as u32;
            // (Thrown out where it leaves; where it comes, drawn in after the flash.)
            let sparks = Burst::new(Kind::Spark, (p + Vec3::Y * size * 0.8).to_array(), n).outward(0.6).speed(reach * 0.5, reach * 1.5).life(0.3, 0.8).size(2.0, 0.8).color(BLUE).intensity(7.0).cold().collide(false);
            self.burst(if arriving { sparks.after(0.08) } else { sparks });
            self.burst(Burst::new(Kind::Glow, (p + Vec3::Y * size).to_array(), 4).radius(size * 0.5).speed(4.0, 16.0).life(0.35, 0.7).size(size * 1.5, size * 3.0).color(BLUE).intensity(2.4));
        }
    }
}
