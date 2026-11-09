//! What is left of a unit the moment it dies, for the seconds after: an aircraft falls out of
//! the sky trailing fire and smoke and blows up where it hits; a ship lists, settles and slips
//! under; a building blown to nothing sags into the ground behind a cloud of dust; and a big
//! unit's death sets off a few blasts more. Whatever its death script threw clear flies as debris
//! of its own (see `spawn_debris`) and is left out here.
//!
//! What is left is shown once. A building's wreck stands where it stood from the moment it dies,
//! so the building itself does not come down too (it sagged and tilted through the ground beside
//! its own wreck); a wreck under a sinking hull, or under a falling aircraft, waits unseen until
//! the hull has settled on the bottom or the aircraft has come down. A frame shot before it was
//! finished, or a unit taken off the map (reclaimed, its construction given up), leaves no
//! whole charred model behind.
//!
//! Presentation only: the game has already moved on.

use glam::{Mat4, Quat, Vec3};
use ta_render::{DrawInstance, ModelId, ParticleBurst as Burst, ParticleKind as Kind};
use ta_sim::{Locomotion, Sim};

use super::particles::Blast;
use super::{Burning, Effects, Particle, Scorch};
use crate::scene::Remnant;

/// How a unit came to its end (see [`Effects::unit_died`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Death {
    /// The wreck (or heap) its death left, a feature of the simulation's.
    pub wreck: Option<usize>,
    /// It was finished: a frame still being built leaves its blast and nothing more.
    pub complete: bool,
}

/// How what is left of a unit goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    /// Out of the sky, tumbling, to blow up on the ground or in the sea.
    Crash,
    /// Listing and settling, under the water.
    Sink,
    /// Sagging into the ground behind a cloud of dust.
    Collapse,
}

/// What is left to show of a unit that moves as `locomotion` does and died as `death` tells
/// (`flying`: in the air; `wet`: over the sea), and how it goes; none for a unit that leaves
/// only its wreck, if that.
fn fate_of(locomotion: Locomotion, flying: bool, wet: bool, death: Death) -> Option<Fate> {
    if !death.complete {
        return None;
    }
    let fate = match locomotion {
        Locomotion::Air if flying => Fate::Crash,
        Locomotion::Naval => Fate::Sink,
        Locomotion::Hover if wet => Fate::Sink,
        Locomotion::Static if wet => Fate::Sink,
        Locomotion::Static => Fate::Collapse,
        _ => return None,
    };
    // A building's wreck stands in its place at once: the building coming down as well showed it
    // twice. (A sinking hull and a falling aircraft go on, their wreck hidden until they settle.)
    (fate != Fate::Collapse || death.wreck.is_none()).then_some(fate)
}

/// How far under where it floated a hull has sunk `k` of the way through its sinking (0..1),
/// `height` tall; with `room` (the sea's depth there, its wreck waiting on the bottom) it settles
/// there rather than going on through the sea floor.
fn sunk(k: f32, height: f32, room: Option<f32>) -> f32 {
    let depth = (height + 14.0) * k.clamp(0.0, 1.0).powf(1.6);
    room.map_or(depth, |room| depth.min(room.max(0.0)))
}

pub(super) struct Remains {
    model: ModelId,
    fate: Fate,
    /// Its wreck, a feature not drawn until this has come to rest (see `Effects::unsettled_wrecks`),
    /// and how far the bottom lies under its pivot (a hull settles on it).
    wreck: Option<usize>,
    room: Option<f32>,
    /// Its pieces' world matrices when it died (thrown pieces zero).
    at_death: Vec<Mat4>,
    /// This frame's.
    matrices: Vec<Mat4>,
    /// The point it turns about (its origin when it died), and where that point is now.
    pivot: Vec3,
    pos: Vec3,
    /// World units a second.
    vel: Vec3,
    /// How far it has turned about the pivot, and about what, how fast (radians a second).
    turn: Quat,
    axis: Vec3,
    spin: f32,
    /// Across and up (world units).
    size: f32,
    height: f32,
    team: u32,
    start: f32,
    /// Seconds it takes (sinking, collapsing).
    duration: f32,
    next_puff: f32,
    done: bool,
}

/// A blast set off a moment after a big unit's death.
pub(super) struct Pop {
    at: f32,
    pos: Vec3,
    size: f32,
}

/// Gravity on falling wrecks (world units a second²; a little under the debris' own, as
/// airframes glide a while).
const FALL: f32 = 240.0;

impl Effects {
    /// A unit of type `type_id` died at `pos` (`death`: what it left, whether it was finished):
    /// big ones go off in a few blasts more, and what is left of it (`remnant`, when it was in
    /// view) lingers as its kind decides, without the pieces its death script threw clear
    /// (`thrown`), and shown once with its wreck (see the module's notes).
    pub fn unit_died(&mut self, remnant: Option<Remnant>, type_id: usize, pos: Vec3, thrown: &[usize], death: Death, sim: &Sim, now: f32) {
        if !self.sees(sim, pos) {
            return;
        }
        let t = sim.types.get(type_id);
        let size = t.radius.to_f32().max(8.0);
        let height = t.model.height.to_f32().max(8.0);
        // Big ones don't go at once: fuel and ammunition cook off around the wreck (not round a
        // frame, which has none).
        if size >= 24.0 && death.complete {
            let n = 2 + (size / 24.0) as usize + (self.rand() * 2.0) as usize;
            for k in 0..n.min(6) {
                let off = Vec3::new(self.rand() * 2.0 - 1.0, 0.0, self.rand() * 2.0 - 1.0) * size * 0.7;
                let at = now + 0.15 + k as f32 * (0.18 + self.rand() * 0.25);
                let lift = Vec3::Y * (4.0 + self.rand() * height * 0.6);
                self.pops.push(Pop { at, pos: pos + off + lift, size: (size * 0.35).clamp(10.0, 30.0) });
            }
        }
        let ground = height_at(sim, pos);
        let sea = sim.world.sea_level.to_f32();
        let wet = ground < sea - 4.0;
        // What it leaves on the ground burns a while, then smoulders, the longer the bigger it
        // was (a sinking ship and a crashing aircraft have fates of their own; a unit without a
        // death blast is already burning, see `SimEvent::UnitExploded`).
        let grounded = matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Static) || remnant.as_ref().is_some_and(|r| !r.flying);
        if grounded && !wet && !t.def.is_feature && !self.burning.iter().any(|b| b.pos.distance(pos) < 8.0 && now - b.start < 1.0) {
            let fire = (2.5 + size * 0.15).min(12.0);
            let smoulder = (10.0 + size * 0.5).min(45.0);
            self.burn(super::Burning { pos: Vec3::new(pos.x, ground.max(pos.y.min(ground + height * 0.3)), pos.z), start: now + 0.3, big: size >= 24.0, next_puff: now + 0.3, fire, smoulder });
        }
        let Some(r) = remnant else { return };
        let Some(fate) = fate_of(t.locomotion, r.flying, wet, death) else {
            // A building whose wreck stands in its place: only the dust it throws up and the rumble.
            if t.locomotion == Locomotion::Static && !wet && death.complete {
                self.collapse_dust(pos, ground, size);
            }
            return;
        };
        let mut at_death = r.pieces;
        for &p in thrown {
            if let Some(m) = at_death.get_mut(p) {
                *m = Mat4::ZERO;
            }
        }
        if at_death.iter().all(|m| *m == Mat4::ZERO) {
            return;
        }
        let pivot = at_death.iter().find(|m| **m != Mat4::ZERO).map_or(pos, |m| m.w_axis.truncate());
        let pivot = Vec3::new(pos.x, pivot.y.min(pos.y + 4.0).max(pos.y - 4.0), pos.z);
        let side = if self.rand() < 0.5 { -1.0 } else { 1.0 };
        let (axis, spin, vel, duration) = match fate {
            Fate::Crash => {
                // Tumbling about its line of flight, nose dropping: fighters spiral, heavy
                // craft roll over slowly.
                let dir = if r.velocity.length_squared() > 1.0 { r.velocity.normalize() } else { Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5).normalize_or(Vec3::X) };
                let across = Vec3::Y.cross(dir).normalize_or(Vec3::X);
                let axis = (dir * 0.85 + across * 0.45 * side).normalize();
                let spin = side * (1.4 + self.rand() * 2.6) * (30.0 / size).clamp(0.5, 1.6);
                // Hovering craft drop and drift; planes keep most of their speed.
                let drift = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * 30.0;
                (axis, spin, r.velocity * 0.9 + drift + Vec3::Y * 10.0, 8.0)
            }
            // Over onto one side, the bow or stern coming up.
            Fate::Sink => {
                let tilt = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5).normalize_or(Vec3::X);
                (tilt, side, Vec3::ZERO, if t.def.water_line > 0.0 { 4.0 } else { 5.5 + size / 40.0 })
            }
            Fate::Collapse => {
                let tilt = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5).normalize_or(Vec3::X);
                (tilt, side, Vec3::ZERO, 1.2 + (size / 80.0).min(1.0))
            }
        };
        // (A hull settles on the bottom where its wreck lies, rather than sinking on through it.)
        let room = (fate == Fate::Sink && death.wreck.is_some()).then(|| pivot.y - ground);
        let remains = Remains {
            model: r.model,
            fate,
            wreck: death.wreck,
            room,
            matrices: at_death.clone(),
            at_death,
            pivot,
            pos: pivot,
            vel,
            turn: Quat::IDENTITY,
            axis,
            spin,
            size,
            height,
            team: r.team,
            start: now,
            duration,
            next_puff: now,
            done: false,
        };
        match fate {
            Fate::Collapse => self.collapse_dust(pos, ground, size),
            Fate::Sink => self.sounds.push(("xplosml2", pos, 0.5)),
            Fate::Crash => {}
        }
        self.remains.push(remains);
        if self.remains.len() > MAX_REMAINS {
            self.remains.remove(0);
        }
    }

    /// Moves what is left of the dead, sets off the blasts due, and poses them for drawing.
    pub(super) fn update_remains(&mut self, sim: &Sim, now: f32, dt: f32) {
        let pops: Vec<Pop> = {
            let (due, later): (Vec<Pop>, Vec<Pop>) = std::mem::take(&mut self.pops).into_iter().partition(|p| p.at <= now);
            self.pops = later;
            due
        };
        for p in pops {
            self.pop(sim, p, now);
        }
        let sea = sim.world.sea_level.to_f32();
        let mut remains = std::mem::take(&mut self.remains);
        for r in &mut remains {
            let age = now - r.start;
            let k = (age / r.duration).clamp(0.0, 1.0);
            match r.fate {
                Fate::Crash => {
                    r.vel.y -= FALL * dt;
                    let drag = (1.0 - 0.35 * dt).max(0.0);
                    r.vel.x *= drag;
                    r.vel.z *= drag;
                    r.pos += r.vel * dt;
                    r.turn = (Quat::from_axis_angle(r.axis, r.spin * dt) * r.turn).normalize();
                    if now >= r.next_puff {
                        r.next_puff = now + 0.035;
                        if self.sees(sim, r.pos) {
                            self.crash_trail(r.pos, r.vel, now, r.size);
                        }
                    }
                    let ground = height_at(sim, r.pos);
                    let floor = ground.max(sea);
                    if r.pos.y <= floor + 2.0 {
                        let at = Vec3::new(r.pos.x, floor, r.pos.z);
                        if self.sees(sim, at) {
                            self.crash(at, ground < sea, r.size, now);
                        }
                        r.done = true;
                    } else if age > r.duration {
                        r.done = true;
                    }
                }
                Fate::Sink => {
                    // Settling slowly at first, then going: over and down.
                    let depth = sunk(k, r.height, r.room);
                    let list = 0.55 * (30.0 / r.size).clamp(0.5, 1.0) * smooth(k * 1.6);
                    let pitch = 0.18 * smooth(k * 1.2) * r.spin;
                    let across = Vec3::Y.cross(r.axis).normalize_or(Vec3::Z);
                    r.turn = Quat::from_axis_angle(r.axis, list * r.spin) * Quat::from_axis_angle(across, pitch);
                    r.pos = r.pivot - Vec3::Y * depth;
                    if now >= r.next_puff {
                        r.next_puff = now + 0.09;
                        self.sinking(r.pivot, sea, r.size, k, now);
                    }
                    r.done = age > r.duration;
                }
                Fate::Collapse => {
                    // Sagging faster and faster into its rubble, shuddering as it goes.
                    let depth = r.height * 0.9 * k * k;
                    let shudder = (1.0 - k) * 1.2;
                    let jitter = Vec3::new((now * 61.0).sin(), 0.0, (now * 47.0).cos()) * shudder;
                    r.turn = Quat::from_axis_angle(r.axis, 0.14 * k * r.spin);
                    r.pos = r.pivot - Vec3::Y * depth + jitter;
                    if now >= r.next_puff && k < 0.8 {
                        r.next_puff = now + 0.14;
                        let off = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * r.size * 1.4;
                        let drift = Vec3::new(self.rand() * 10.0 - 5.0, 12.0, self.rand() * 10.0 - 5.0);
                        self.dark_puff(r.pivot + off + Vec3::Y * 4.0, drift, now, 3.0, r.size * 0.5, r.size * 1.4, 0.4);
                    }
                    r.done = age > r.duration;
                }
            }
            // Its pieces: turned about the pivot, then moved with it.
            let m = Mat4::from_translation(r.pos) * Mat4::from_quat(r.turn) * Mat4::from_translation(-r.pivot);
            for (out, &d) in r.matrices.iter_mut().zip(&r.at_death) {
                *out = if d == Mat4::ZERO { Mat4::ZERO } else { m * d };
            }
        }
        remains.retain(|r| !r.done);
        self.remains = remains;
    }

    /// The wrecks not to draw yet: those of hulls still sinking and aircraft still falling, which
    /// would show the dead twice (each appears as what is left of its unit comes to rest).
    pub fn unsettled_wrecks(&self) -> impl Iterator<Item = usize> + '_ {
        self.remains.iter().filter_map(|r| r.wreck)
    }

    /// What is left of the dead, as model instances (charred).
    pub(super) fn remains_instances(&self) -> impl Iterator<Item = DrawInstance<'_>> {
        let in_view = |p: Vec3, r: f32| self.view.is_none_or(|(lo, hi)| p.x >= lo.x - r && p.x <= hi.x + r && p.z >= lo.y - r && p.z <= hi.y + r);
        self.remains
            .iter()
            .filter(move |r| in_view(r.pos, r.size + 60.0))
            .map(|r| DrawInstance { model: r.model, pieces: &r.matrices, team: r.team, flags: super::DEAD_LIGHTS, build: 1.0, damage: 1.0, travel: [0.0; 2], piece: None, playing: 0 })
    }

    /// Falling wrecks light the ground under them.
    pub(super) fn remains_lights(&mut self, now: f32) {
        for r in self.remains.iter().filter(|r| r.fate == Fate::Crash) {
            let flicker = 0.75 + 0.25 * ((now * 21.0 + r.start * 5.0).sin() * 0.5 + 0.5);
            self.lights.push(ta_render::PointLight::new(r.pos.to_array(), [2.0 * flicker, 0.85 * flicker, 0.22 * flicker], 60.0 + r.size));
        }
    }

    /// Fire and black smoke pouring from a falling wreck, and embers.
    fn crash_trail(&mut self, p: Vec3, vel: Vec3, now: f32, size: f32) {
        let s = (size / 20.0).clamp(0.6, 2.0);
        let rotation = self.rand() * std::f32::consts::TAU;
        self.emit(Particle {
            sprite: self.glow,
            pos: p,
            vel: vel * 0.1,
            start: now,
            life: 0.3,
            size0: 16.0 * s,
            size1: 5.0 * s,
            alpha: 0.9,
            additive: true,
            gravity: 0.0,
            tint: [1.0, 0.45, 0.12],
            brightness: 2.2,
            rotation,
        });
        self.dark_puff(p, vel * 0.05 + Vec3::Y * 6.0, now, 2.4, 8.0 * s, 30.0 * s, 0.55);
        self.debris_trail_particles(p, vel);
    }

    /// A falling wreck hits the ground (or the sea) at `p`.
    fn crash(&mut self, p: Vec3, water: bool, size: f32, now: f32) {
        let big = size >= 24.0;
        let key = if big { "fx/explode4" } else { "fx/explode3" };
        if let Some(&a) = self.generic_blasts.get(key) {
            self.blast_anim(a, p + Vec3::Y * 6.0, 20.0, if big { 1.6 } else { 1.0 }, 1.1, now);
        }
        self.blast_particles(p, if big { 70.0 } else { 40.0 }, p.y, water, Blast::Death);
        self.light(p + Vec3::Y * 20.0, now, 0.6, if big { 240.0 } else { 150.0 }, [2.0, 0.85, 0.22]);
        self.heat(p, now, 1.6, if big { 110.0 } else { 60.0 }, 0.7);
        if !water {
            let rotation = self.rand() * std::f32::consts::TAU;
            self.scorches.push(Scorch { pos: p, size: if big { 130.0 } else { 70.0 }, rotation, start: now });
            self.burn(Burning { pos: p, start: now, big, next_puff: now, fire: super::FIRE_TIME, smoulder: super::SMOULDER_TIME });
        }
        for _ in 0..4 {
            let off = self.rand_dir() * 12.0;
            let life = 2.5 + self.rand() * 2.0;
            self.dark_puff(p + off + Vec3::Y * 6.0, Vec3::new(0.0, 16.0, 0.0), now + 0.1, life, 24.0, 70.0, 0.55);
        }
        self.sounds.push((if big { "xplolrg1" } else { "xplomed2" }, p, 0.8));
        self.shake = self.shake.max(if big { 3.0 } else { 1.5 });
    }

    /// A sinking hull: foam and bubbles boiling up around it at the surface, and smoke and
    /// flames while its deck is still above the water.
    fn sinking(&mut self, at: Vec3, sea: f32, size: f32, k: f32, now: f32) {
        let off = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * size * 1.2;
        let foam = Vec3::new(at.x + off.x, sea + 1.0, at.z + off.z);
        self.puff_tinted(foam, Vec3::new(0.0, 4.0, 0.0), now, 1.6, size * 0.25, size * 0.7, 0.35 * (1.0 - k * 0.5), [0.92, 0.96, 1.0]);
        if self.rand() < 0.35 {
            self.burst(Burst::new(Kind::Spray, foam.to_array(), (4.0 + size * 0.2) as u32).radius(size * 0.3).speed(20.0, 60.0).life(0.5, 1.0).size(2.5, 4.0));
        }
        if k < 0.6 {
            let deck = at + Vec3::Y * (8.0 * (1.0 - k)) + off * 0.5;
            self.dark_puff(deck, Vec3::new(0.0, 18.0, 0.0), now, 3.0, size * 0.4, size * 1.2, 0.5 * (1.0 - k));
            if self.rand() < 0.5 {
                self.wreck_fire_particles(deck, size >= 24.0, true);
            }
        }
    }

    /// The dust a building throws up as it comes down, and the rumble.
    fn collapse_dust(&mut self, p: Vec3, ground: f32, size: f32) {
        let g = [p.x, ground + 2.0, p.z];
        let n = (10.0 + size * 0.9).min(120.0) as u32;
        self.burst(
            Burst::new(Kind::Dust, g, n)
                .ground_color()
                .radius(size * 0.7)
                .vertical(0.25)
                .outward(0.6)
                .speed(size * 0.4, size * 1.3)
                .life(1.8, 3.6)
                .size(size * 0.35, size * 1.1)
                .intensity(0.6)
                .over(0.6),
        );
        self.burst(Burst::new(Kind::Dirt, g, (8.0 + size * 0.8).min(160.0) as u32).ground_color().radius(size * 0.6).toward([0.0, 1.0, 0.0], 0.8).speed(30.0, 90.0 + size).life(0.6, 1.4).size(3.0, 2.6));
        self.sounds.push(("xplobld1", p, 0.7));
    }

    /// One of a big unit's later blasts.
    fn pop(&mut self, sim: &Sim, p: Pop, now: f32) {
        if !self.sees(sim, p.pos) {
            return;
        }
        if let Some(&a) = self.generic_blasts.get("fx/explode2") {
            self.blast_anim(a, p.pos, 24.0, p.size / 45.0 + 0.2, 0.85, now);
        }
        let ground = height_at(sim, p.pos);
        self.blast_particles(p.pos, p.size, ground, false, Blast::Shell);
        self.light(p.pos + Vec3::Y * 10.0, now, 0.3, p.size * 5.0, [1.6, 0.7, 0.2]);
        self.dark_puff(p.pos, Vec3::new(0.0, 14.0, 0.0), now, 2.6, p.size, p.size * 2.5, 0.5);
        if self.rand() < 0.5 {
            self.sounds.push(("xplosml2", p.pos, 0.45));
        }
    }
}

/// Most falling, sinking and collapsing wrecks at once (the oldest go first).
const MAX_REMAINS: usize = 120;

#[cfg(test)]
mod tests {
    use super::*;

    const SHOT: Death = Death { wreck: Some(7), complete: true };
    const BLOWN_UP: Death = Death { wreck: None, complete: true };

    #[test]
    fn a_dead_unit_is_shown_once() {
        // A building that leaves a wreck: the wreck stands in its place, and the building does not
        // come down beside it (it sagged and tilted through the ground next to its own wreck).
        assert_eq!(fate_of(Locomotion::Static, false, false, SHOT), None);
        // Blown to nothing, it comes down into its dust.
        assert_eq!(fate_of(Locomotion::Static, false, false, BLOWN_UP), Some(Fate::Collapse));
        // A frame shot before it was finished leaves no whole building to come down.
        assert_eq!(fate_of(Locomotion::Static, false, false, Death { wreck: None, complete: false }), None);
        assert_eq!(fate_of(Locomotion::Naval, false, true, Death { wreck: None, complete: false }), None);
        // Hulls sink and aircraft fall either way (their wrecks wait unseen until they settle).
        for death in [SHOT, BLOWN_UP] {
            assert_eq!(fate_of(Locomotion::Naval, false, true, death), Some(Fate::Sink));
            assert_eq!(fate_of(Locomotion::Hover, false, true, death), Some(Fate::Sink));
            assert_eq!(fate_of(Locomotion::Static, false, true, death), Some(Fate::Sink));
            assert_eq!(fate_of(Locomotion::Air, true, false, death), Some(Fate::Crash));
            // Ground units, hovercraft ashore and landed aircraft: their wreck alone.
            assert_eq!(fate_of(Locomotion::Ground, false, false, death), None);
            assert_eq!(fate_of(Locomotion::Hover, false, false, death), None);
            assert_eq!(fate_of(Locomotion::Air, false, false, death), None);
        }
    }

    #[test]
    fn a_hull_settles_where_its_wreck_lies() {
        // Without a wreck below, it sinks out of sight; with one, it goes no deeper than the
        // bottom, where the wreck takes its place.
        assert_eq!(sunk(0.0, 30.0, None), 0.0);
        assert_eq!(sunk(1.0, 30.0, None), 44.0);
        assert_eq!(sunk(1.0, 30.0, Some(12.0)), 12.0);
        assert!((0..=10).all(|i| sunk(i as f32 / 10.0, 30.0, Some(12.0)) <= 12.0));
        assert!((1..=10).all(|i| sunk(i as f32 / 10.0, 30.0, Some(12.0)) >= sunk((i - 1) as f32 / 10.0, 30.0, Some(12.0))), "it never rises again");
        assert_eq!(sunk(0.5, 30.0, Some(-3.0)), 0.0, "a hull aground stays where it is");
    }
}

fn height_at(sim: &Sim, p: Vec3) -> f32 {
    crate::scene::drawn_ground(&sim.world, p.x, p.z)
}

/// Smoothstep on 0..1 (clamped).
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}
