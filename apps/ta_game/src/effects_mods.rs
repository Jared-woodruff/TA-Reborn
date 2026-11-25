//! Effects of a mod's own (ta_data's `WeaponLook` and `EffectDef`): a weapon's muzzle flash and
//! trail animations (`firegaf`, `trailgaf`), its particle effects where its shots leave, along
//! their flight and where they hit (`fireeffect`, `traileffect`, `hiteffect`,
//! `waterhiteffect`), its smoke trail's pace and the smoke where a shot ends (TA's `smokedelay`
//! and `endsmoke`), a unit's own effects played by its script (`emit-sfx 1024 + n`), and the beams
//! between buildings that help each other (`AdjacencyBeamColor`). Each effect's bursts become
//! GPU particle bursts (or rings on the ground), in the frame the effect plays in.
//!
//! What they cost is bounded: an effect makes at most `effect::MAX_BURSTS` bursts of at most
//! `effect::MAX_COUNT` particles each (held there as it is read; a trail's so many a second), the
//! effects of mods queue at most [`MOD_PARTICLES`] particles a frame (times the effects quality's
//! budget; past it they are thinned), and a trail's animation is left at most [`TRAIL_CATCH_UP`]
//! times a frame.
//!
//! Presentation only: nothing here feeds back into the simulation.

use glam::Vec3;
use ta_data::effect::{BurstDef, BurstDirection, BurstKind};
use ta_render::{BeamInstance, ParticleBurst as Burst, ParticleKind as Kind};
use ta_sim::{Sim, SimEvent};

use super::{Anim, Effects};

/// The most particles the mods' effects queue in a frame at the High effects quality (Low a
/// quarter, Ultra twice): past it, their bursts are thinned.
const MOD_PARTICLES: f32 = 6000.0;
/// The most times a shot's trail animation is left in a frame however short its `smokedelay` (it
/// then comes as often as the frames do).
const TRAIL_CATCH_UP: u32 = 3;
/// The shortest and longest pace of a trail (seconds).
const TRAIL_PACE: (f32, f32) = (1.0 / 120.0, 10.0);
/// How many frames a second a muzzle flash's and a trail's animations play at.
const ANIM_FPS: f32 = 20.0;
/// Reborn's smoke trail puffs this many times as often as TA's `smokedelay` says (its puffs are
/// smaller): TA's 0.1 s, every 0.04 s (in f64, so that TA's 0.1 comes to the very 0.04 a puff
/// always came at: the battles of TA's own weapons smoke as they did).
const SMOKE_PACE: f64 = 0.4;
/// How high over the ground an adjacency beam runs (pixels) and how fast its pulse crosses it
/// (shares of its length a second).
const ADJACENCY_LIFT: f32 = 5.0;
const ADJACENCY_PULSE: f32 = 0.6;
/// The most adjacency beams drawn in a frame.
const MOST_ADJACENCY_BEAMS: usize = 800;

/// What a weapon plays of a mod's own (none of it for TA's own weapons): its animations (indices
/// into `Effects::anims`) and its effects (indices into the game data's `effects`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct WeaponFx {
    pub fire_anim: Option<usize>,
    pub trail_anim: Option<usize>,
    pub fire: Option<usize>,
    pub trail: Option<usize>,
    pub hit: Option<usize>,
    pub water_hit: Option<usize>,
}

impl WeaponFx {
    /// Whether it draws a muzzle of its own (TA Reborn's own flash, sparks and smoke for its kind
    /// then left out).
    pub fn own_fire(&self) -> bool {
        self.fire_anim.is_some() || self.fire.is_some()
    }

    /// The effect where it hits: the water's where it hit the water and it has one.
    pub fn hit(&self, water: bool) -> Option<usize> {
        if water { self.water_hit.or(self.hit) } else { self.hit }
    }
}

/// The effects of mods a battle plays, as found when it starts.
#[derive(Default)]
pub(super) struct ModFx {
    /// Per weapon type.
    pub weapons: Vec<WeaponFx>,
    /// Per unit type: its own effects (`SfxEffects`), by their places in its list.
    pub units: Vec<Vec<Option<usize>>>,
    /// Particles the mods' effects queued this frame.
    pub queued: f32,
    /// When each shot's trail plays next (by projectile id).
    pub trail_due: rustc_hash::FxHashMap<u32, f32>,
    /// Where each shot that ends in smoke (TA's `endsmoke`) was last drawn, and its weapon.
    pub ending: rustc_hash::FxHashMap<u32, (usize, Vec3)>,
}

impl ModFx {
    /// What the weapons and the unit types of `sim` play, `fire_trail` the weapons' animations as
    /// loaded (`effects_anims.rs`).
    pub fn new(sim: &Sim, fire_trail: &[[Option<usize>; 2]]) -> ModFx {
        let data = &sim.data;
        let weapons = sim
            .weapon_types
            .types
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let look = &w.def.look;
                let [fire_anim, trail_anim] = fire_trail.get(i).copied().unwrap_or([None; 2]);
                WeaponFx {
                    fire_anim,
                    trail_anim,
                    fire: data.effect_id(&look.fire_effect),
                    trail: data.effect_id(&look.trail_effect),
                    hit: data.effect_id(&look.hit_effect),
                    water_hit: data.effect_id(&look.water_hit_effect),
                }
            })
            .collect();
        let units = sim.types.types.iter().map(|t| t.def.sfx_effects.iter().map(|e| data.effect_id(e)).collect()).collect();
        ModFx { weapons, units, ..Default::default() }
    }

    pub fn weapon(&self, weapon: usize) -> WeaponFx {
        self.weapons.get(weapon).copied().unwrap_or_default()
    }

    pub fn clear(&mut self) {
        self.trail_due.clear();
        self.ending.clear();
        self.queued = 0.0;
    }
}

/// The frame an effect plays in: its forward (see `BurstDirection`), up and right.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pub forward: Vec3,
    pub up: Vec3,
    pub right: Vec3,
}

impl Frame {
    /// The frame whose forward is `forward` (any length; none: up), its up as near the world's as
    /// it can be, its right to the right of one looking along it (the world's x east, y up, z
    /// south: looking south, right is west).
    pub fn new(forward: Vec3) -> Frame {
        let forward = forward.normalize_or(Vec3::Y);
        let side = if forward.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let right = forward.cross(side).normalize_or(Vec3::NEG_X);
        let up = right.cross(forward).normalize_or(Vec3::Y);
        Frame { forward, up, right }
    }

    fn at(&self, p: Vec3, offset: [f32; 3]) -> Vec3 {
        p + self.right * offset[0] + self.up * offset[1] + self.forward * offset[2]
    }
}

/// How often a smoke trail puffs (seconds): [`SMOKE_PACE`] of its weapon's `smokedelay`.
pub(super) fn smoke_pace(smoke_delay: f64) -> f32 {
    ((smoke_delay * SMOKE_PACE) as f32).clamp(TRAIL_PACE.0, TRAIL_PACE.1)
}

/// The player colour a shot of player `owner` is drawn in, a missile's, a bomb's (its model's
/// player-coloured faces): its owner's, as the unit that fired it is drawn (the first colour where
/// there is no such player).
pub(super) fn owner_color(sim: &Sim, owner: u8) -> u32 {
    sim.players.get(owner as usize).map_or(0, |p| p.color as u32)
}

/// The particle kind a burst's kind is (none: a ring, which is no particle).
fn particle_kind(k: BurstKind) -> Option<Kind> {
    Some(match k {
        BurstKind::Spark => Kind::Spark,
        BurstKind::Ember => Kind::Ember,
        BurstKind::Glow => Kind::Glow,
        BurstKind::Fire => Kind::Fire,
        BurstKind::Dust => Kind::Dust,
        BurstKind::Smoke => Kind::Smoke,
        BurstKind::Dirt => Kind::Dirt,
        BurstKind::Spray => Kind::Spray,
        BurstKind::Fireball => Kind::Fireball,
        BurstKind::Ring => return None,
    })
}

/// The GPU burst of a mod's burst `b` played at `at` in `frame`, `carry` the velocity of what plays
/// it (a shot's, for `inherit`), `count` its particles (thinned): what it leaves out is its kind's
/// own (`ParticleBurst::new`).
pub(super) fn particle_burst(b: &BurstDef, at: Vec3, frame: &Frame, carry: Vec3, count: u32) -> Option<Burst> {
    let kind = particle_kind(b.kind)?;
    let mut p = Burst::new(kind, frame.at(at, b.offset).to_array(), count);
    if b.ground {
        p = p.ground_color();
    }
    if let Some(r) = b.radius {
        p = p.radius(r);
    }
    if let Some(v) = b.vertical {
        p = p.vertical(v);
    }
    let cone = b.cone.map_or(p.spread, f32::to_radians);
    p = match b.direction {
        Some(BurstDirection::Out) => p.outward(cone),
        Some(d) => {
            let way = match d {
                BurstDirection::Forward => frame.forward,
                BurstDirection::Back => -frame.forward,
                BurstDirection::Up => Vec3::Y,
                _ => Vec3::NEG_Y,
            };
            p.toward(way.to_array(), cone)
        }
        None => p.spread(cone),
    };
    if let Some([lo, hi]) = b.speed {
        p = p.speed(lo.min(hi), hi.max(lo));
    }
    if let Some([lo, hi]) = b.life {
        p = p.life(lo.min(hi), hi.max(lo).max(0.02));
    }
    if let Some([start, end]) = b.size {
        p = p.size(start, end);
    }
    if let Some(c) = b.color {
        p = p.color(c);
    }
    if let Some(c) = b.end_color {
        p = p.end_color(c);
    }
    if let Some(i) = b.intensity {
        p = p.intensity(i);
    }
    if let Some(g) = b.gravity {
        p = p.gravity(g);
    }
    if let Some(d) = b.drag {
        p = p.drag(d);
    }
    if let Some(c) = b.collide {
        p = p.collide(c);
    }
    if b.cold {
        p = p.cold();
    }
    if b.inherit > 0.0 {
        p = p.velocity((carry * b.inherit).to_array());
    }
    Some(p.over(b.over).after(b.delay))
}

impl Effects {
    /// Plays the mod's effect `effect` (an index into the game data's) at `at`, `forward` its
    /// frame's forward, `carry` the velocity of what plays it; within this frame's budget.
    pub(super) fn play_effect(&mut self, sim: &Sim, effect: usize, at: Vec3, forward: Vec3, carry: Vec3, now: f32) {
        self.play_effect_for(sim, effect, at, forward, carry, now, None);
    }

    /// [`Effects::play_effect`], or, `seconds` given, `seconds` of a trail's: its bursts' counts
    /// so many a second (as many as fall due, the rest left to chance, so that over the frames
    /// they come to their count a second whatever the frame rate); its rings none.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn play_effect_for(&mut self, sim: &Sim, effect: usize, at: Vec3, forward: Vec3, carry: Vec3, now: f32, seconds: Option<f32>) {
        let Some(def) = sim.data.effects.get(effect) else { return };
        // (Played far off the view, it would be gone before the camera got there: `burst`.)
        if self.view.is_some_and(|(lo, hi)| at.x < lo.x - 700.0 || at.x > hi.x + 700.0 || at.z < lo.y - 700.0 || at.z > hi.y + 700.0) {
            return;
        }
        let frame = Frame::new(forward);
        let budget = MOD_PARTICLES * self.detail.budget;
        for b in &def.bursts {
            if b.kind == BurstKind::Ring {
                if seconds.is_none() {
                    self.effect_ring(sim, b, frame.at(at, b.offset), now);
                }
                continue;
            }
            let wanted = match seconds {
                Some(dt) => (b.count as f32 * dt + self.rand()).floor() as u32,
                None => b.count,
            };
            // (Past the frame's budget, thinned: as many as are left of it, a few at least.)
            let left = (budget - self.mods.queued).max(0.0);
            let count = if (wanted as f32) <= left { wanted } else { (left as u32).max(u32::from(wanted > 0 && left > 0.0)) };
            if count == 0 {
                continue;
            }
            self.mods.queued += count as f32;
            if let Some(burst) = particle_burst(b, at, &frame, carry, count) {
                self.burst(burst);
            }
        }
    }

    /// A ring burst: running out over the ground (or the water over it) from `at`.
    fn effect_ring(&mut self, sim: &Sim, b: &BurstDef, at: Vec3, now: f32) {
        let ground = crate::scene::drawn_ground(&sim.world, at.x, at.z).max(sim.world.sea_level.to_f32());
        let [width, across] = b.size.unwrap_or([6.0, 80.0]);
        let life = b.life.map_or(0.6, |[lo, hi]| (lo + hi) * 0.5).max(0.05);
        let colour = b.color.unwrap_or([1.0; 3]);
        self.ripple(Vec3::new(at.x, ground, at.z), now + b.delay, life, (across * 0.5).max(1.0), colour, b.intensity.unwrap_or(0.5).min(1.0), width.max(0.5));
    }

    /// A shot leaving its weapon's muzzle at `p` towards `target`: its animation and its effect,
    /// where the weapon has them of its own.
    pub(super) fn mod_fire(&mut self, sim: &Sim, weapon: usize, p: Vec3, target: Vec3, now: f32) {
        let fx = self.mods.weapon(weapon);
        if let Some(anim) = fx.fire_anim {
            self.active.push(Anim { frames: anim, start: now, pos: p, fps: ANIM_FPS, scale: 1.0, brightness: 1.0 });
        }
        if let Some(effect) = fx.fire {
            self.play_effect(sim, effect, p, target - p, Vec3::ZERO, now);
        }
    }

    /// The direction a shot of `weapon` was flying as it hit at `pos` (one ended this tick: from
    /// the simulation's spent shots, the nearest of that weapon), if known.
    pub(super) fn hit_direction(&self, sim: &Sim, weapon: usize, pos: Vec3) -> Option<Vec3> {
        sim.spent
            .iter()
            .map(|(p, _)| p)
            .filter(|p| p.weapon == weapon)
            .map(|p| (Vec3::from_array(p.pos.to_f32()).distance_squared(pos), Vec3::from_array(p.vel.to_f32())))
            .filter(|(d, v)| *d < 64.0 * 64.0 && v.length_squared() > 1e-6)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, v)| v.normalize())
    }

    /// Where a shot of `weapon` hit at `p` (in the water or not, flying along `along` as it hit,
    /// zero where not known): the weapon's own effect, if it has one; whether it had (TA Reborn's
    /// own sparks, flames, grit, smoke and flash for the weapon's kind are then left out).
    pub(super) fn mod_hit(&mut self, sim: &Sim, weapon: usize, p: Vec3, water: bool, along: Vec3, now: f32) -> bool {
        let Some(effect) = self.mods.weapon(weapon).hit(water) else { return false };
        // (Forward, for a hit: back the way the shot came; up where that is not known.)
        let forward = if along.length_squared() > 1e-6 { -along } else { Vec3::Y };
        self.play_effect(sim, effect, p, forward, Vec3::ZERO, now);
        true
    }

    /// A unit's script asked for one of its own effects (`emit-sfx 1024 + n`) at `pos`.
    pub(super) fn unit_effect(&mut self, sim: &Sim, e: &SimEvent, now: f32) -> bool {
        let SimEvent::Sfx { unit, pos, kind } = *e else { return false };
        let Some(n) = ta_data::effect::sfx_effect(kind) else { return false };
        let Some(u) = sim.unit(unit) else { return true };
        let Some(Some(effect)) = self.mods.units.get(u.type_id).and_then(|list| list.get(n)).copied() else { return true };
        // (Forward: the way the unit faces.)
        let heading = u.heading as f32 * std::f32::consts::TAU / 65536.0;
        let forward = Vec3::new(heading.sin(), 0.0, heading.cos());
        self.play_effect(sim, effect, Vec3::from_array(pos.to_f32()), forward, Vec3::ZERO, now);
        true
    }

    /// A shot in flight at `pos`, flying at `vel` (world units a second), `dt` seconds since the
    /// last frame: its trail effect, so many particles a second, its trail's animation as its
    /// weapon's pace (`smokedelay`) comes round, and where it is, should it end in smoke.
    pub(super) fn mod_flight(&mut self, sim: &Sim, p: &ta_sim::Projectile, pos: Vec3, vel: Vec3, now: f32, dt: f32) {
        let w = &sim.weapon_types.types[p.weapon];
        if w.def.end_smoke {
            self.mods.ending.insert(p.id, (p.weapon, pos));
        }
        let fx = self.mods.weapon(p.weapon);
        if let Some(effect) = fx.trail
            && dt > 0.0
        {
            self.play_effect_for(sim, effect, pos, vel, vel, now, Some(dt));
        }
        let Some(anim) = fx.trail_anim else { return };
        let pace = (w.def.smoke_delay as f32).clamp(TRAIL_PACE.0, TRAIL_PACE.1);
        let due = *self.mods.trail_due.entry(p.id).or_insert(now);
        if due > now {
            return;
        }
        let mut next = due;
        let mut played = 0;
        while next <= now && played < TRAIL_CATCH_UP {
            // (Where it was then, along its flight.)
            let at = pos - vel * (now - next);
            self.active.push(Anim { frames: anim, start: now, pos: at, fps: ANIM_FPS, scale: 1.0, brightness: 1.0 });
            next += pace;
            played += 1;
        }
        // (Behind by more than it may catch up: from now on.)
        self.mods.trail_due.insert(p.id, if next <= now { now + pace } else { next });
    }

    /// The shots no longer in flight (`flying`, those that are): their trails forgotten, and a
    /// puff of smoke where those of a weapon that ends in smoke were last drawn.
    pub(super) fn mod_landed(&mut self, sim: &Sim, flying: &rustc_hash::FxHashSet<u32>, now: f32) {
        self.mods.trail_due.retain(|id, _| flying.contains(id));
        let ended: Vec<(usize, Vec3)> = self.mods.ending.iter().filter(|(id, _)| !flying.contains(id)).map(|(_, &e)| e).collect();
        self.mods.ending.retain(|id, _| flying.contains(id));
        for (_, at) in ended {
            if self.view.is_some_and(|(lo, hi)| at.x < lo.x - 64.0 || at.x > hi.x + 64.0 || at.z < lo.y - 64.0 || at.z > hi.y + 64.0) || !self.sees(sim, at) {
                continue;
            }
            self.puff(at, Vec3::new(0.0, 10.0, 0.0), now, 1.2, 8.0, 26.0, 0.4);
        }
    }

    /// The beams between the buildings in view that help each other (Supreme Commander's
    /// adjacency), from each giving one whose type gives a beam's colour (`AdjacencyBeamColor`) to
    /// the one it helps, along the ground, a pulse running along it; a pair helping each other, one
    /// beam. Out of the viewer's sight, none.
    pub(super) fn adjacency_beams(&mut self, sim: &Sim, now: f32) {
        let mut drawn = 0;
        for u in sim.units() {
            let Some(taken) = u.adjacency.as_ref() else { continue };
            let receiver = Vec3::from_array(u.pos.to_f32());
            if self.view.is_some_and(|(lo, hi)| receiver.x < lo.x - 200.0 || receiver.x > hi.x + 200.0 || receiver.z < lo.y - 200.0 || receiver.z > hi.y + 200.0) || !self.sees(sim, receiver) {
                continue;
            }
            for &(g, _) in &taken.from {
                let Some(giver) = sim.unit(g) else { continue };
                // (Both ways: drawn once, by the lower id's, in whichever's colour there is.)
                let mutual = giver.adjacency.as_ref().is_some_and(|a| a.from.iter().any(|&(o, _)| o == u.id));
                if mutual && g.slot > u.id.slot {
                    continue;
                }
                let look = sim.types.get(giver.type_id).def.adjacency_beam.or_else(|| if mutual { sim.types.get(u.type_id).def.adjacency_beam } else { None });
                let Some((colour, thickness)) = look else { continue };
                let from = Vec3::from_array(giver.pos.to_f32());
                let lift = |p: Vec3| Vec3::new(p.x, crate::scene::drawn_ground(&sim.world, p.x, p.z) + ADJACENCY_LIFT, p.z);
                let (a, b) = (lift(from), lift(receiver));
                let glow = colour.map(|c| c * 1.4);
                self.beams.push(BeamInstance::new(a.to_array(), b.to_array(), thickness * 2.0, glow, 0.55).core(0.2).cover(0.25));
                // (A pulse running from the giver to the one it helps.)
                let phase = (now * ADJACENCY_PULSE + (g.slot as f32) * 0.37).fract();
                let head = a.lerp(b, phase);
                let tail = a.lerp(b, (phase - 0.18).max(0.0));
                self.beams.push(BeamInstance::new(head.to_array(), tail.to_array(), thickness * 2.6, colour.map(|c| c * 2.4), 0.8).core(0.3).tapered(thickness, 0.9));
                drawn += 1;
                if drawn >= MOST_ADJACENCY_BEAMS {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    fn burst(text: &str) -> BurstDef {
        ta_data::EffectDef::from_tdf(&tdf::parse(&format!("[E] {{ [B] {{ {text} }} }}")).sections[0]).0.bursts.remove(0)
    }

    /// A mod's burst as the GPU makes it: what it gives in the effect's frame (forward the shot's
    /// way, its offset along it), what it leaves out its kind's own.
    #[test]
    fn a_mods_burst_is_made_in_its_effects_frame() {
        let frame = Frame::new(Vec3::X);
        // (Looking east, up is up and right is south.)
        assert!((frame.up - Vec3::Y).length() < 1e-5 && (frame.right - Vec3::Z).length() < 1e-5, "{frame:?}");
        assert!((Frame::new(Vec3::Z).right - Vec3::NEG_X).length() < 1e-5, "looking south, right is west");
        let b = burst("kind=spark; count=9; direction=forward; cone=30; speed=50 150; life=0.3 0.2; size=2 1; color=1 0.5 0; endcolor=0 0 1; offset=0 2 10; inherit=0.5; cold=1;");
        let p = particle_burst(&b, Vec3::new(100.0, 0.0, 50.0), &frame, Vec3::new(40.0, 0.0, 0.0), 9).unwrap();
        assert_eq!((p.kind, p.count, p.position), (Kind::Spark, 9, [110.0, 2.0, 50.0]), "ten pixels forward, two up");
        assert!((p.direction[0] - 1.0).abs() < 1e-5 && (p.spread - 30f32.to_radians()).abs() < 1e-5);
        assert_eq!((p.speed, p.life, p.size, p.color, p.end_color, p.velocity, p.cold), ([50.0, 150.0], [0.2, 0.3], [2.0, 1.0], [1.0, 0.5, 0.0], Some([0.0, 0.0, 1.0]), [20.0, 0.0, 0.0], true));
        // Left out: the kind's own (a spark falls and bounces).
        let plain = particle_burst(&burst("kind=spark;"), Vec3::ZERO, &frame, Vec3::ZERO, 1).unwrap();
        let own = Burst::new(Kind::Spark, [0.0; 3], 1);
        assert_eq!((plain.gravity, plain.collide, plain.speed, plain.end_color, plain.direction), (own.gravity, own.collide, own.speed, None, own.direction));
        // Out from the middle; a ring is no particle.
        assert_eq!(particle_burst(&burst("kind=glow; direction=out;"), Vec3::ZERO, &frame, Vec3::ZERO, 1).unwrap().direction, [0.0; 3]);
        assert!(particle_burst(&burst("kind=ring;"), Vec3::ZERO, &frame, Vec3::ZERO, 1).is_none());
    }

    /// TA's own `smokedelay` (0.1 s, written or not) puffs a smoke trail at the very pace it always
    /// did, 0.04 s; a mod's at the same share of its own.
    #[test]
    fn a_smoke_trail_keeps_tas_pace() {
        assert_eq!(smoke_pace(0.1), 0.04f32);
        let unwritten = ta_data::WeaponDef::from_tdf(&tdf::parse("[W] { smoketrail=1; }").sections[0]);
        assert_eq!(smoke_pace(unwritten.smoke_delay), 0.04f32);
        assert_eq!(smoke_pace(0.05), 0.02f32);
        assert_eq!(smoke_pace(0.0), TRAIL_PACE.0);
    }

    /// A shot drawn as a model takes its owner's colour (as the unit that fired it does), not the
    /// first player's.
    #[test]
    fn a_shot_is_drawn_in_its_owners_colour() {
        use ta_sim::{Fx, FxVec3};
        let data = std::sync::Arc::new(ta_data::GameData::load(&ta_vfs::Vfs::new()));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 32, height: 32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![0; 1024], features: vec![ta_formats::tnt::FEATURE_NONE; 1024], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
        let player = |color| ta_sim::PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false };
        let sim = Sim::new(types, data, &map, &Default::default(), 0, &[player(3), player(7)], 7);
        assert_eq!((owner_color(&sim, 0), owner_color(&sim, 1)), (3, 7));
        assert_eq!(owner_color(&sim, 5), 0, "no such player: the first colour");
    }

    /// A frame for any forward: straight up and down too.
    #[test]
    fn a_frame_is_square_whichever_way_it_faces() {
        for f in [Vec3::X, Vec3::Y, Vec3::NEG_Y, Vec3::new(0.3, -0.2, -1.0), Vec3::ZERO] {
            let frame = Frame::new(f);
            for (a, b) in [(frame.forward, frame.up), (frame.up, frame.right), (frame.right, frame.forward)] {
                assert!(a.dot(b).abs() < 1e-4 && (a.length() - 1.0).abs() < 1e-4, "{f}: {frame:?}");
            }
            assert!((frame.forward.cross(frame.up) - frame.right).length() < 1e-4, "{f}: right-handed, {frame:?}");
        }
    }
}
