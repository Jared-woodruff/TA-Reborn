//! Battle damage on units that still fight: a badly damaged unit burns (licks of flame, embers
//! and a column of smoke rising from its hull, its firelight on the ground and the air
//! shimmering over it), throws sparks when it is nearly gone, and a damaged aircraft trails
//! smoke, flame too when it is burning, across the sky. Smoke at lighter damage comes from the
//! units' own scripts (TA's SmokeUnit), as TA had it.

use glam::Vec3;
use ta_render::{ParticleBurst as Burst, ParticleKind as Kind};
use ta_sim::{Locomotion, Sim};

use super::Effects;

/// Health (a share of the most) below which a unit burns, and below which it throws sparks
/// too; below which a flying aircraft trails smoke.
const FIRE_BELOW: f32 = 0.33;
const SPARKS_BELOW: f32 = 0.15;
const TRAIL_BELOW: f32 = 0.5;
/// Seconds between an aircraft's puffs of smoke (its trail), and the longest between a
/// burning unit's flames (shorter the worse it burns).
const TRAIL_EVERY: f32 = 0.05;
const FIRE_EVERY: f32 = 0.24;
/// The most burning units lit and shimmering at once, and the most giving off flame and smoke
/// (the nearest to the middle of the view: a battle of thousands burns where it is watched).
const MOST_FIRES: usize = 48;
const MOST_ABLAZE: usize = 160;
/// How far past the view a unit still smokes (world units: a column of smoke rises into it).
const VIEW_MARGIN: f32 = 300.0;

/// What a damaged unit gives off, by how badly damaged and what it is.
enum Ablaze {
    /// Flying: a trail of smoke behind it, flame in it when it burns (`fire`).
    Trail { fire: bool },
    /// On the ground or the water: flames, embers and smoke; sparks too.
    Hull { sparks: bool },
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];

impl Effects {
    /// This frame's battle damage (see the module): what the units below the thresholds give
    /// off, when it is due; their firelight and shimmer for `collect_lights`.
    pub(super) fn update_damage(&mut self, sim: &Sim, now: f32) {
        self.unit_fires.clear();
        let sea = sim.world.sea_level.to_f32();
        let view = self.view;
        let mut due: Vec<(ta_sim::UnitId, Vec3, f32, f32, f32, Ablaze)> = Vec::new();
        for u in sim.units() {
            if u.dead || u.cloaked || u.carried_by.is_some() || !u.is_complete() {
                continue;
            }
            // (Only what is in view, or nearly.)
            if let Some((lo, hi)) = view {
                let (x, z) = (u.pos.x.to_f32(), u.pos.z.to_f32());
                if x < lo.x - VIEW_MARGIN || x > hi.x + VIEW_MARGIN || z < lo.y - VIEW_MARGIN || z > hi.y + VIEW_MARGIN {
                    continue;
                }
            }
            let t = sim.unit_type(u);
            if t.max_health.is_zero() || t.def.is_feature {
                continue;
            }
            let health = (u.health / t.max_health).to_f32();
            let flying = t.locomotion == Locomotion::Air && !u.landed;
            if health >= if flying { TRAIL_BELOW } else { FIRE_BELOW } {
                continue;
            }
            let p = Vec3::from_array(u.pos.to_f32());
            // (Nothing burns under the water.)
            if p.y < sea - 4.0 {
                continue;
            }
            let height = t.model.height.to_f32().max(6.0);
            let size = t.radius.to_f32().max(6.0);
            let ablaze = if flying { Ablaze::Trail { fire: health < FIRE_BELOW } } else { Ablaze::Hull { sparks: health < SPARKS_BELOW } };
            // How badly it burns, 0 at the line to 1 about to go.
            let severity = (1.0 - health / FIRE_BELOW).clamp(0.0, 1.0);
            due.push((u.id, p + Vec3::Y * height * if flying { 0.3 } else { 0.55 }, size, height, severity, ablaze));
        }
        // (The nearest the middle of the view, when there are more than enough.)
        let middle = self.view.map(|(lo, hi)| (lo + hi) * 0.5);
        if let Some(m) = middle
            && due.len() > MOST_ABLAZE
        {
            let far = |p: Vec3| (glam::Vec2::new(p.x, p.z) - m).length_squared();
            due.select_nth_unstable_by(MOST_ABLAZE - 1, |a, b| far(a.1).total_cmp(&far(b.1)));
            due.truncate(MOST_ABLAZE);
        }
        // (Firelight and shimmer for the burning ones nearest the middle of the view.)
        let mut fires: Vec<(Vec3, f32, f32)> = due
            .iter()
            .filter(|d| matches!(d.5, Ablaze::Hull { .. }) || matches!(d.5, Ablaze::Trail { fire: true }))
            .map(|d| (d.1, d.2, 0.35 + 0.65 * d.4))
            .collect();
        if let Some(m) = middle {
            fires.sort_by(|a, b| (glam::Vec2::new(a.0.x, a.0.z) - m).length_squared().total_cmp(&(glam::Vec2::new(b.0.x, b.0.z) - m).length_squared()));
        }
        fires.truncate(MOST_FIRES);
        // (Those no longer burning, or gone, forgotten now and then.)
        if self.damage_fx.len() > 4 * MOST_ABLAZE {
            let live: rustc_hash::FxHashSet<ta_sim::UnitId> = due.iter().map(|d| d.0).collect();
            self.damage_fx.retain(|id, _| live.contains(id));
        }
        for (id, p, size, height, severity, ablaze) in due {
            if !self.sees(sim, p) {
                continue;
            }
            let next = self.damage_fx.get(&id).copied().unwrap_or(0.0);
            if now < next {
                continue;
            }
            let every = match ablaze {
                Ablaze::Trail { .. } => TRAIL_EVERY,
                Ablaze::Hull { .. } => FIRE_EVERY * (1.0 - 0.55 * severity),
            };
            // (Each unit in its own rhythm.)
            let jitter = 0.8 + 0.4 * self.rand();
            self.damage_fx.insert(id, now + every * jitter);
            match ablaze {
                Ablaze::Trail { fire } => self.smoke_trail(p, size, fire, now),
                Ablaze::Hull { sparks } => self.hull_fire(p, size, height, severity, sparks, now),
            }
        }
        for (p, size, k) in fires {
            if self.sees(sim, p) {
                self.unit_fires.push((p, size, k));
            }
        }
    }

    /// A puff of an aircraft's smoke trail (and a tongue of flame in it, when it burns).
    fn smoke_trail(&mut self, p: Vec3, size: f32, fire: bool, now: f32) {
        let s = size.clamp(8.0, 30.0);
        let drift = Vec3::new(self.rand() * 4.0 - 2.0, 3.0, self.rand() * 4.0 - 2.0);
        let life = 1.8 + self.rand() * 0.6;
        self.dark_puff(p, drift, now, life, s * 0.5, s * 1.8, if fire { 0.55 } else { 0.38 });
        if fire {
            self.burst(Burst::new(Kind::Fire, p.to_array(), 1).radius(s * 0.15).speed(2.0, 8.0).life(0.25, 0.45).size(s * 0.7, s * 0.2).gravity(-20.0));
        }
    }

    /// A burning hull's flames, embers and smoke (sparks too when it is nearly gone), spread
    /// over its top as big as it is.
    fn hull_fire(&mut self, p: Vec3, size: f32, height: f32, severity: f32, sparks: bool, now: f32) {
        let r = (size * 0.45).clamp(5.0, 40.0);
        let at = p + Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * r * 1.2;
        let flame = (size * 0.45).clamp(8.0, 26.0) * (0.75 + 0.5 * severity);
        let n = 1 + (severity * 2.0) as u32 + (size >= 30.0) as u32;
        self.burst(Burst::new(Kind::Fire, at.to_array(), n).radius(r * 0.5).toward(UP, 0.35).speed(8.0, 26.0).life(0.35, 0.75).size(flame, flame * 0.3).gravity(-55.0).over(0.2));
        self.burst(Burst::new(Kind::Ember, at.to_array(), 1 + n / 2).radius(r * 0.6).toward(UP, 0.5).speed(14.0, 50.0).life(1.0, 2.6).size(2.6, 1.0).over(0.2));
        // Smoke rising off it, thicker the worse it burns.
        let drift = Vec3::new(self.rand() * 6.0 - 3.0, 24.0 + height * 0.2, self.rand() * 6.0 - 3.0);
        let (s0, s1) = (r * 1.1, r * (3.0 + 2.0 * severity));
        self.dark_puff(at + Vec3::Y * 4.0, drift, now, 2.6 + severity, s0, s1, 0.32 + 0.25 * severity);
        if sparks && self.rand() < 0.35 {
            // A shorting system: a crackle of sparks off the hull.
            let dir = Vec3::new(self.rand() - 0.5, 0.6, self.rand() - 0.5).normalize_or(Vec3::Y);
            self.burst(Burst::new(Kind::Spark, (at + Vec3::Y * height * 0.2).to_array(), 6).toward(dir.to_array(), 0.6).speed(60.0, 160.0).life(0.15, 0.45).size(1.4, 0.7).intensity(6.0));
        }
    }
}
