//! Fireworks for the New Year (TA Reborn's own racks of them, `rebornfireworks`): by night each
//! rack in view sends up a shell now and then, which climbs on a trail of sparks, slows and
//! bursts into a sphere of sparks of one colour or two, falling as they fade, its flash lighting
//! the snow below. Seen, not simulated: nothing in the battle comes of them.

use glam::Vec3;

use super::{SPARK_FALL, Effects, Particle};

/// A shell on its way up: where it is, how fast it rises, when it bursts and in what colours.
pub(super) struct Shell {
    pos: Vec3,
    vel: Vec3,
    burst: f32,
    colours: [[f32; 3]; 2],
    big: bool,
}

/// What colour fireworks burst in (linear, bright).
const COLOURS: [[f32; 3]; 7] = [
    [1.0, 0.18, 0.12],
    [0.2, 1.0, 0.3],
    [0.25, 0.45, 1.0],
    [1.0, 0.75, 0.2],
    [0.85, 0.3, 1.0],
    [1.0, 0.95, 0.85],
    [0.2, 0.95, 1.0],
];

/// How far a shell climbs before it bursts (world units), and how fast its sparks fly out.
const CLIMB: f32 = 240.0;
const SPREAD: f32 = 120.0;

impl Effects {
    /// Fireworks over the `racks` in view (feature indices and where their shells go up from) by
    /// night (`lamps`, 0..1, how dark it is: none by day), at game time `now`, `dt` seconds on.
    pub fn fireworks(&mut self, racks: &[(usize, Vec3)], lamps: f32, now: f32, dt: f32) {
        if dt > 0.0 {
            let volley = self.volley.filter(|v| now < v.2);
            for (index, at) in racks {
                // (In a volley, one shell after another.)
                let quick = volley.is_some_and(|(round, reach, _)| at.distance(round) <= reach);
                let next = self.firework_due.get(index).copied();
                match next {
                    None => {
                        let wait = self.rand() * if quick { 0.4 } else { 4.0 };
                        self.firework_due.insert(*index, now + wait);
                    }
                    Some(t) if now >= t => {
                        // (Shells now and then, more of them the darker it is.)
                        let wait = if quick { 0.2 + self.rand() * 0.45 } else { 1.2 + self.rand() * 4.5 / lamps.max(0.25) };
                        self.firework_due.insert(*index, now + wait);
                        if lamps > 0.3 {
                            self.launch(*at, now);
                        }
                    }
                    Some(t) if quick && t > now + 0.7 => {
                        let wait = self.rand() * 0.4;
                        self.firework_due.insert(*index, now + wait);
                    }
                    _ => {}
                }
            }
            if self.firework_due.len() > 512 {
                let near: Vec<usize> = racks.iter().map(|r| r.0).collect();
                self.firework_due.retain(|k, _| near.contains(k));
            }
        }
        // The shells in flight: a trail behind each, and the burst at the top.
        let mut shells = std::mem::take(&mut self.shells);
        shells.retain_mut(|s| {
            s.pos += s.vel * dt;
            s.vel.y -= 90.0 * dt;
            if now < s.burst {
                if self.rand() < 0.8 {
                    let p = s.pos + self.rand_dir() * 1.5;
                    self.emit(Particle { sprite: self.glow, pos: p, vel: Vec3::new(0.0, -12.0, 0.0), start: now, life: 0.5, size0: 4.5, size1: 1.2, alpha: 0.9, additive: true, gravity: SPARK_FALL, tint: [1.0, 0.7, 0.35], brightness: 3.2, rotation: 0.0 });
                }
                return true;
            }
            self.firework_burst(s, now);
            false
        });
        self.shells = shells;
    }

    /// The racks within `reach` of `round` send their shells up one after another until `until`
    /// (game time): a volley, at midnight (a staged scene's `fireworks`).
    pub fn firework_volley(&mut self, round: Vec3, reach: f32, until: f32) {
        self.volley = Some((round, reach, until));
    }

    fn launch(&mut self, at: Vec3, now: f32) {
        let a = self.rand();
        let b = self.rand();
        let colours = [COLOURS[(a * 7.0) as usize % 7], COLOURS[(b * 7.0) as usize % 7]];
        let big = self.rand() < 0.3;
        let lean = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5) * 40.0;
        let rise = (2.0 * 90.0 * CLIMB * (0.8 + 0.4 * self.rand())).sqrt();
        self.shells.push(Shell { pos: at, vel: Vec3::new(lean.x, rise, lean.z), burst: now + rise / 90.0 * 0.92, colours, big });
        self.flash(at + Vec3::Y * 4.0, now, 0.15, 10.0, [1.0, 0.8, 0.5], 3.0);
        self.sounds.push(("xplosml2", at, 0.25));
    }

    fn firework_burst(&mut self, s: &Shell, now: f32) {
        let sparks = if s.big { 150 } else { 90 };
        let speed = SPREAD * if s.big { 1.35 } else { 1.0 };
        for k in 0..sparks {
            // (Evenly round a sphere: the golden spiral.)
            let t = (k as f32 + 0.5) / sparks as f32;
            let up = 1.0 - 2.0 * t;
            let r = (1.0 - up * up).max(0.0).sqrt();
            let phi = k as f32 * 2.399_963;
            let dir = Vec3::new(r * phi.cos(), up, r * phi.sin());
            let colour = s.colours[k % 2];
            let jitter = 0.85 + 0.3 * self.rand();
            let life = 1.6 + 0.8 * self.rand();
            self.emit(Particle {
                sprite: self.glow,
                pos: s.pos,
                vel: dir * speed * jitter + Vec3::Y * 40.0,
                start: now,
                life,
                size0: 9.0,
                size1: 3.0,
                alpha: 1.0,
                additive: true,
                gravity: SPARK_FALL,
                tint: colour,
                brightness: 5.5,
                rotation: 0.0,
            });
        }
        let mean = [0, 1, 2].map(|i| (s.colours[0][i] + s.colours[1][i]) * 0.5);
        self.flash(s.pos, now, 0.35, if s.big { 60.0 } else { 40.0 }, mean, 2.5);
        self.light(s.pos, now, 0.9, if s.big { 420.0 } else { 320.0 }, mean.map(|c| c * 3.0));
        self.sounds.push((if s.big { "xplomed2" } else { "xplosml2" }, s.pos, if s.big { 0.55 } else { 0.4 }));
    }
}
