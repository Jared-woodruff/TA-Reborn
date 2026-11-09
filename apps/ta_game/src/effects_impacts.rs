//! What weapons do to the world as they fire and hit, beyond TA's own art: shockwaves running
//! out over the ground from big blasts (flat, as rings on the ground: a sprite facing the
//! camera stood up like a hoop), ripples on the water where shots splash down, the pulse of an
//! EMP (blue, no fireball: it is no nuke), a missile's exhaust and a missile shot down in the
//! sky, impacts in the weapon's own colour, the flash and smoke of a gun's muzzle thrown forward
//! along the shot, and the shake of the camera that TA's weapons ask for (`shakemagnitude`).

use glam::Vec3;
use ta_render::{BeamInstance, ParticleBurst as Burst, ParticleKind as Kind, RingInstance, SpriteInstance};
use ta_sim::Sim;

use super::{Effects, Particle};

/// A ring running out over the ground or the water: its middle (y: the lowest it lies, the
/// water's surface for a ripple), when, for how long and how far it runs, its colour (linear,
/// over 1 to glow), opacity at the start and width (pixels).
pub(super) struct Ripple {
    pub pos: Vec3,
    pub start: f32,
    pub life: f32,
    pub radius: f32,
    pub color: [f32; 3],
    pub alpha: f32,
    pub width: f32,
}

/// How far a blast's shockwave runs (times its radius) and from what radius a blast has one.
const SHOCK_REACH: f32 = 2.6;
const SHOCK_FROM: f32 = 40.0;
/// How far from the middle of the view a weapon's shake fades out (world units).
const SHAKE_REACH: f32 = 1600.0;

impl Effects {
    /// A ring running out (see [`Ripple`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ripple(&mut self, pos: Vec3, start: f32, life: f32, radius: f32, color: [f32; 3], alpha: f32, width: f32) {
        self.ripples.push(Ripple { pos, start, life, radius, color, alpha, width });
    }

    /// This frame's rings on the ground and the water, for the renderer (`rings`).
    pub(super) fn update_ripples(&mut self, now: f32) {
        self.rings.clear();
        self.ripples.retain(|r| now - r.start < r.life);
        for r in &self.ripples {
            if now < r.start {
                continue;
            }
            let t = ((now - r.start) / r.life).clamp(0.0, 1.0);
            // Fast at first, slowing as it spreads, fading as it goes.
            let radius = r.radius * (1.0 - (1.0 - t).powi(3));
            let fade = (1.0 - t).powf(1.5);
            if radius > 1.0 {
                self.rings.push(RingInstance::new([r.pos.x, r.pos.z], radius, r.pos.y, r.color, r.alpha * fade, r.width * (1.0 - 0.5 * t), 0.0));
            }
        }
    }

    /// A blast's shockwave over the ground: a bright ring running out and fading.
    pub(super) fn shockwave(&mut self, p: Vec3, radius: f32, now: f32) {
        if radius < SHOCK_FROM {
            return;
        }
        let reach = (radius * SHOCK_REACH).min(900.0);
        let width = (5.0 + radius / 20.0).min(16.0);
        let life = 0.4 + radius / 600.0;
        // A soft band of heat, and its hard front.
        self.ripple(p, now, life, reach, [2.4, 1.4, 0.6], 0.2, width * 2.4);
        self.ripple(p, now, life, reach, [3.4, 2.5, 1.6], 0.55, width * 0.5);
    }

    /// Ripples spreading on the water where a shot splashed down.
    pub(super) fn splash_ripples(&mut self, p: Vec3, sea: f32, radius: f32, now: f32) {
        let reach = (radius * 3.0).clamp(24.0, 300.0);
        let at = Vec3::new(p.x, sea, p.z);
        self.ripple(at, now, 1.4, reach, [0.9, 1.0, 1.05], 0.35, 2.0);
        self.ripple(at, now + 0.25, 2.0, reach * 0.6, [0.9, 1.0, 1.05], 0.25, 1.5);
    }

    /// An EMP's pulse: a blue flash and light, rings of it running out and electric sparks
    /// (no fireball, no mushroom: nothing burns).
    pub(super) fn emp_pulse(&mut self, p: Vec3, radius: f32, now: f32) {
        let blue = [0.55, 0.75, 1.0];
        self.flash(p + Vec3::Y * 12.0, now, 0.35, (radius * 0.9).min(500.0), blue, 2.6);
        self.light(p + Vec3::Y * 60.0, now, 1.2, (radius * 2.0).min(1200.0), [1.6, 2.6, 5.0]);
        self.ripple(p, now, 0.7, radius * 1.15, [0.3, 0.7, 2.0], 0.3, 18.0);
        self.ripple(p, now, 0.7, radius * 1.15, [0.45, 0.95, 2.4], 0.6, 5.0);
        self.ripple(p, now + 0.15, 1.0, radius * 0.8, [0.45, 0.95, 2.4], 0.45, 4.0);
        let n = (radius / 3.0).clamp(30.0, 220.0) as u32;
        self.burst(Burst::new(Kind::Spark, (p + Vec3::Y * 10.0).to_array(), n).outward(0.5).speed(radius * 0.4, radius * 1.4).life(0.3, 0.9).size(2.2, 1.0).color(blue).intensity(7.0).cold().collide(false));
        self.burst(Burst::new(Kind::Glow, (p + Vec3::Y * 20.0).to_array(), 6).radius(radius * 0.3).speed(5.0, 20.0).life(0.4, 0.9).size(radius * 0.25, radius * 0.6).color(blue).intensity(2.0));
    }

    /// A missile's exhaust, from the nozzle at its tail back along `dir`, the way it flies: a
    /// flame white-hot at the nozzle and orange to its tip (a ribbon on the body's line: from any
    /// side it points along the body, and the body hides what is behind it), a glow on the nozzle
    /// (seen end on, from behind), sparks; as wide as the body is thick (`girth`), longer while it
    /// climbs out of its tube (`boost`), flickering. Where its smoke starts: the flame's tip.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn missile_flame(&mut self, id: u32, nozzle: Vec3, dir: Vec3, girth: f32, boost: bool, now: f32, dt: f32) -> Vec3 {
        let seed = id as f32 * 1.37;
        let flicker = 1.0 + 0.1 * (now * 29.0 + seed).sin() + 0.06 * (now * 47.0 + seed * 2.1).sin();
        let width = (girth * 0.6).clamp(2.0, 12.0);
        let length = width * if boost { 4.5 } else { 3.0 } * flicker;
        let tip = nozzle - dir * length;
        self.beams.push(BeamInstance::new(nozzle.to_array(), tip.to_array(), width, [2.4, 1.15, 0.4], 1.0).core(0.45).tapered(width * 0.5, 0.85).cover(0.35));
        self.additive.push(SpriteInstance::new(nozzle.to_array(), self.glow).scale(width * 0.034).tint([1.0, 0.72, 0.42]).brightness(4.0));
        self.exhaust_particles(nozzle, dir, dt);
        tip
    }

    /// A missile shot down in the sky: a flare, its pieces falling burning, a puff of smoke.
    pub(super) fn interception(&mut self, p: Vec3, now: f32) {
        self.flash(p, now, 0.25, 110.0, [1.0, 0.85, 0.6], 3.0);
        self.light(p, now, 0.6, 420.0, [4.0, 3.0, 1.8]);
        self.burst(Burst::new(Kind::Spark, p.to_array(), 60).outward(1.0).speed(80.0, 260.0).life(0.4, 1.2).size(2.4, 1.0).intensity(8.0).gravity(160.0).collide(false));
        self.burst(Burst::new(Kind::Ember, p.to_array(), 24).outward(1.0).speed(30.0, 120.0).life(2.0, 4.0).size(4.0, 2.0).gravity(140.0).intensity(5.0));
        self.burst(Burst::new(Kind::Fireball, p.to_array(), 2).radius(10.0).speed(4.0, 12.0).life(0.6, 0.9).size(40.0, 70.0));
        self.dark_puff(p, Vec3::new(0.0, 6.0, 0.0), now + 0.2, 4.0, 30.0, 110.0, 0.45);
    }

    /// A gun's muzzle: a flash thrown forward along the shot (a few glows, smaller the further
    /// out), and a puff of smoke drifting off it; `size` as the gun's shells are big.
    pub(super) fn muzzle_blast(&mut self, p: Vec3, dir: Vec3, size: f32, now: f32) {
        // (Not out of view: a battle of thousands fires a thousand times a second.)
        if let Some((lo, hi)) = self.view
            && (p.x < lo.x - 60.0 || p.x > hi.x + 60.0 || p.z < lo.y - 60.0 || p.z > hi.y + 60.0)
        {
            return;
        }
        for k in 0..3 {
            let f = k as f32;
            let rotation = self.rand() * std::f32::consts::TAU;
            self.emit(Particle {
                sprite: self.glow,
                pos: p + dir * (4.0 + f * 7.0) * size,
                vel: dir * 20.0,
                start: now,
                life: 0.07 + 0.02 * f,
                size0: (16.0 - f * 4.0) * size,
                size1: (8.0 - f * 2.0) * size,
                alpha: 0.9,
                additive: true,
                gravity: 0.0,
                tint: [1.0, 0.72, 0.38],
                brightness: 3.2 - f * 0.6,
                rotation,
            });
        }
        // (Smoke from the bigger guns.)
        if size >= 0.9 {
            let drift = dir * 18.0 + Vec3::new(0.0, 6.0, 0.0);
            self.puff_tinted(p + dir * 6.0 * size, drift, now + 0.02, 0.9 + 0.3 * size, 6.0 * size, 22.0 * size, 0.32, [0.82, 0.8, 0.77]);
        }
    }

    /// The camera's shake for a weapon's blast (TA's `shakemagnitude`), fading out with the
    /// distance from the middle of the view.
    pub(super) fn weapon_shake(&mut self, w: &ta_sim::WeaponType, p: Vec3) {
        let magnitude = w.def.shake_magnitude as f32;
        if magnitude <= 0.0 {
            return;
        }
        let near = match self.view {
            Some((lo, hi)) => {
                let middle = (lo + hi) * 0.5;
                1.0 - (glam::Vec2::new(p.x, p.z) - middle).length() / SHAKE_REACH
            }
            None => 1.0,
        };
        if near > 0.0 {
            self.shake = self.shake.max((magnitude * 0.6 * near).min(24.0));
        }
    }
}

/// The colour of a weapon's impact (its flash and the light it casts): a laser's or lightning's
/// own, an EMP's blue, the warm orange of everything that burns.
pub(super) fn impact_tint(w: &ta_sim::WeaponType, sim: &Sim) -> [f32; 3] {
    use ta_data::RenderType;
    if w.def.paralyzer {
        return [0.55, 0.75, 1.0];
    }
    match w.def.render_type {
        RenderType::Laser | RenderType::Lightning => {
            let c = super::beam_color(super::weapon_color(w, sim));
            // (Paled a little: a pure green or violet flash reads as paint, not light.)
            c.map(|v| 0.35 + 0.65 * v)
        }
        _ => [1.0, 0.62, 0.3],
    }
}
