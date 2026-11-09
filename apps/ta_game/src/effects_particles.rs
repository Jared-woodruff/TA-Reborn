//! GPU particle recipes (see `ta_render::gpu_particles`): spark showers, flames, embers,
//! burning chunks, grit and dust for explosions, deaths and impacts, sparks at muzzles and behind
//! the D-gun, embers over burning wrecks and flying debris, and the storm of the nuclear blast.
//! Only burst descriptions are built here; the GPU creates, moves and draws the particles.
//!
//! The blast and nuclear recipes are plain functions of the blast (`blast_bursts`,
//! `nuke_bursts`), so `crates/ta_render/examples/showcase.rs` can show exactly what the game
//! does.
//!
//! Presentation only: nothing here feeds back into the simulation.

use std::f32::consts::PI;

use glam::Vec3;
use ta_render::{ParticleBurst as Burst, ParticleKind as Kind, ParticleSettings, Renderer};
use ta_sim::{Fx, FxVec3, Sim};

use super::Effects;

const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// What set a blast off, which decides its look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Blast {
    /// A shell, rocket or bomb: sparks, flames, grit and dust.
    Shell,
    /// A laser, lightning or paralyzer hit: a spray of sparks in the beam's colour.
    Beam([f32; 3]),
    /// A dying unit or building: a bigger shower, burning chunks and embers.
    Death,
    /// The flame thrower: flames and embers.
    Flame,
}

/// What a blast went off in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ground {
    /// On (or above) dry ground.
    Land,
    Sea,
    Lava,
}

/// The bursts of a fireball `radius` wide at `p` that burns for about `life` seconds (the
/// perspective camera's blast, in place of TA's painting of one): a ball of flame swelling and
/// rising, and from a big one, smaller ones boiling out around it a moment later, so its outline
/// breaks into lobes.
pub(super) fn fireball_bursts(p: Vec3, radius: f32, life: f32) -> Vec<Burst> {
    let r = radius.max(3.0);
    let d = r * 2.0;
    // (A blast's fireball outlasts TA's painting of it a little: it cools into soot.)
    let life = (life * 1.15).clamp(0.4, 2.6);
    let mut out = vec![
        Burst::new(Kind::Fireball, [p.x, p.y + r * 0.2, p.z], 1)
            .size(d * 0.55, d * 1.15)
            .life(life * 0.92, life * 1.08)
            .toward(UP, 0.5)
            .speed(0.0, r * 0.15)
            .gravity(-r * 0.9)
            .drag(1.6),
    ];
    if r >= 16.0 {
        let n = ((r / 10.0) as u32).clamp(2, 6);
        out.push(
            Burst::new(Kind::Fireball, [p.x, p.y + r * 0.25, p.z], n)
                .radius(r * 0.4)
                .vertical(0.6)
                .outward(0.9)
                .speed(r * 0.5, r * 1.2)
                .size(d * 0.3, d * 0.75)
                .life(life * 0.6, life * 0.9)
                .gravity(-r * 0.8)
                .drag(3.0)
                .intensity(0.95)
                .over(0.06),
        );
    }
    out
}

/// The bursts of a blast `radius` wide (the weapon's area of effect) at `p`, `ground` being the
/// height of the ground under it. `fireball`: a fireball burns at its heart (the perspective
/// camera's, see `fireball_bursts`), so fewer flames lick out of it.
pub(super) fn blast_bursts(p: Vec3, radius: f32, ground: f32, under: Ground, blast: Blast, fireball: bool) -> Vec<Burst> {
    // (A mod's weapon may blast far wider than any of TA's: its smoke would hang for minutes.)
    let s = radius.clamp(4.0, 480.0);
    let at = |y: f32| [p.x, y, p.z];
    let mut out = Vec::with_capacity(8);
    // On the ground or a unit standing on it, debris flies up and out and the ground throws up
    // grit and dust; in the air, sparks fly every way.
    let low = p.y - ground < 30.0;
    let (tint, shower) = match blast {
        Blast::Beam(c) => (c, 0.6),
        Blast::Death => ([1.0; 3], 1.7),
        Blast::Flame => ([1.0, 0.8, 0.6], 0.5),
        Blast::Shell => ([1.0; 3], 1.0),
    };
    // (Dozens rather than hundreds, and not white-hot for long: a battle's showers overlap, and
    // a dense one blooms into a white fog. Thin, and flung at speeds over a wide range, so they
    // streak out unevenly rather than in a neat ring.)
    let sparks = ((10.0 + s * 2.2) * shower).min(1400.0) as u32;
    let shower = Burst::new(Kind::Spark, at(p.y + 3.0), sparks)
        .radius(s * 0.12)
        .toward(UP, if low { 1.2 } else { PI })
        .speed(30.0 + s * 0.5, (160.0 + s * 3.2).min(650.0))
        .life(0.6, 1.6 + (s / 80.0).min(1.6))
        .size(1.8 + s * 0.008, 1.0)
        .intensity(if blast == Blast::Death { 3.2 } else { 2.8 })
        .color(tint);
    // Energy beams throw sparks of their own colour, not hot metal; the sea quenches hot metal.
    if under != Ground::Sea {
        out.push(if matches!(blast, Blast::Beam(_)) { shower.cold() } else { shower });
    }
    if let Blast::Beam(c) = blast {
        // A flash where the beam burns in; beams kick up little else.
        out.push(Burst::new(Kind::Glow, at(p.y + 2.0), 1).size(12.0, 20.0).life(0.12, 0.16).color(c).intensity(4.0));
        return out;
    }
    match under {
        Ground::Lava => {
            // Molten rock thrown up, glowing as it falls back, and the melt's glare. (Fewer, lower
            // and dimmer than they were: over a lava lake under fire they filled the view.)
            let n = (8.0 + s * 0.7).min(300.0) as u32;
            out.push(Burst::new(Kind::Ember, at(p.y + 1.0), n).radius(s * 0.2).toward(UP, 0.55).speed(40.0 + s * 0.6, 90.0 + s * 1.6).life(0.5, 1.1).size(2.4, 1.2).gravity(380.0).drag(0.4).intensity(4.5).speck());
            out.push(Burst::new(Kind::Glow, at(p.y + 2.0), 1).size(s * 0.6, s * 1.0).life(0.2, 0.3).color([1.0, 0.42, 0.1]).intensity(1.8));
            return out;
        }
        Ground::Sea => {
            // A column of spray thrown up and falling back, a lower crown of droplets round its
            // foot, and mist drifting off the water.
            let n = (14.0 + s * 1.8).min(700.0) as u32;
            out.push(
                Burst::new(Kind::Spray, at(p.y + 1.0), n)
                    .radius(s * 0.12)
                    .toward(UP, 0.2)
                    .speed(60.0 + s * 1.2, 150.0 + s * 3.4)
                    .life(0.7, 1.2 + s / 120.0)
                    .size(2.2 + s * 0.02, 1.6 + s * 0.015)
                    .intensity(0.75),
            );
            let crown = (10.0 + s * 1.0).min(400.0) as u32;
            out.push(Burst::new(Kind::Spray, at(p.y + 1.0), crown).radius(s * 0.25).toward(UP, 0.95).speed(25.0 + s * 0.5, 70.0 + s * 1.5).life(0.4, 0.9).size(1.8 + s * 0.015, 1.2).intensity(0.6));
            let mist = (6.0 + s * 0.3).min(90.0) as u32;
            out.push(
                Burst::new(Kind::Dust, at(p.y + 2.0), mist)
                    .color([0.9, 0.95, 1.0])
                    .intensity(0.4)
                    .radius(s * 0.25)
                    .vertical(0.4)
                    .speed(s * 0.3, s * 0.9)
                    .life(1.0, 2.4)
                    .size(s * 0.5, s * 1.4)
                    .after(0.12),
            );
            return out;
        }
        Ground::Land => {}
    }
    if s >= 12.0 {
        // Flames boiling out of the blast and rising.
        let n = ((s * if fireball { 0.15 } else { 0.3 }).min(90.0) * if blast == Blast::Flame { 2.0 } else { 1.0 }) as u32;
        let size = s.min(120.0);
        out.push(Burst::new(Kind::Fire, at(p.y + s * 0.15), n).radius(s * 0.3).outward(0.9).speed(s * 0.3, s).life(0.35, 0.7 + s / 300.0).size(size * 0.35, size * 0.7).gravity(-70.0).drag(3.0).intensity(1.2));
    }
    if s >= 20.0 || blast != Blast::Shell {
        // Embers drifting up from the blast.
        let n = ((s * 0.45).min(220.0) * if blast == Blast::Death { 1.8 } else { 1.0 }) as u32;
        out.push(Burst::new(Kind::Ember, at(p.y + 4.0), n).radius(s * 0.3).toward(UP, 1.1).speed(20.0, 40.0 + s * 0.9).life(1.2, 2.5 + s / 80.0).size(3.0, 1.2).intensity(6.0).over(0.3));
    }
    if blast == Blast::Death || s >= 30.0 {
        // Burning chunks flung out in arcs, glowing on where they land.
        let n = ((s * 0.25).min(90.0) * if blast == Blast::Death { 1.5 } else { 1.0 }) as u32;
        out.push(Burst::new(Kind::Ember, at(p.y + 4.0), n).radius(s * 0.15).toward(UP, 1.0).speed(60.0 + s, (150.0 + s * 2.5).min(560.0)).life(1.6, 3.2).size(4.6, 2.6).gravity(300.0).drag(0.3).intensity(5.5));
    }
    if low && s >= 8.0 {
        let g = [p.x, ground + 1.0, p.z];
        // Grit and clods thrown up, falling back and bouncing.
        let grit = (10.0 + s * 1.2).min(500.0) as u32;
        out.push(
            Burst::new(Kind::Dirt, g, grit)
                .ground_color()
                .radius(s * 0.2)
                .toward(UP, 0.75)
                .speed(40.0 + s, (100.0 + s * 2.8).min(520.0))
                .life(0.7, 1.4 + s / 150.0)
                .size(2.6 + s * 0.016, 2.3 + s * 0.016),
        );
        // Dust rolling out over the ground, in the ground's own colour (thin: on pale sand a
        // thick one washes a whole battle out).
        let dust = (4.0 + s * 0.28).min(80.0) as u32;
        let size = s.min(140.0);
        out.push(
            Burst::new(Kind::Dust, [p.x, ground + 2.0, p.z], dust)
                .ground_color()
                .radius(s * 0.25)
                .vertical(0.15)
                .outward(0.35)
                .speed(s * 0.5, s * 1.5)
                .life(1.2, 2.4 + s / 120.0)
                .size(size * 0.4, size * 1.15)
                .intensity(0.55),
        );
    }
    if s >= 10.0 && blast != Blast::Flame {
        // Smoke: a dark plume boiling up out of the fireball and drifting off, blacker where
        // a unit or building burned.
        let n = ((3.0 + s * 0.22) * if blast == Blast::Death { 1.6 } else { 1.0 }).min(70.0) as u32;
        let size = s.min(130.0);
        let color = if blast == Blast::Death { [0.12, 0.11, 0.1] } else { [0.24, 0.225, 0.21] };
        out.push(
            Burst::new(Kind::Smoke, at(p.y + s * 0.2), n)
                .radius(s * 0.25)
                .vertical(0.8)
                .toward(UP, 0.5)
                .speed(8.0, 20.0 + s * 0.35)
                .life(2.2, 3.8 + s / 60.0)
                .size(size * 0.3, size * 1.0)
                .color(color)
                .intensity(0.7)
                .gravity(-18.0)
                .drag(0.8)
                .over(0.35)
                .after(0.08),
        );
    }
    out
}

/// The storm of a nuclear blast `radius` wide at `p` (`ground` = height of the ground under it),
/// the biggest show there is: white-hot sparks flung out from the fireball in every direction,
/// raining back over the whole blast; the fireball boiling up into a rising column of flame and
/// then of smoke; burning chunks arcing out; tens of thousands of embers climbing into the cloud
/// and later drifting down from it; a wall of dust racing out along the ground with the
/// shockwave; and grit thrown high.
pub(super) fn nuke_bursts(p: Vec3, radius: f32, ground: f32) -> Vec<Burst> {
    let r = radius.clamp(200.0, 480.0);
    let at = |y: f32| [p.x, y, p.z];
    let g = [p.x, ground + 2.0, p.z];
    vec![
        // Sparks from all over the fireball (not its centre, which would burn white).
        Burst::new(Kind::Spark, at(p.y + r * 0.2), 9_000).radius(r * 0.25).vertical(0.7).outward(0.6).speed(150.0, r * 2.6).life(1.2, 3.5).size(3.6, 1.8).intensity(3.0).drag(0.45).gravity(300.0).over(0.3),
        // The fireball, and the column of flame it becomes.
        Burst::new(Kind::Fire, at(p.y + r * 0.15), 300).radius(r * 0.3).vertical(0.6).outward(1.0).speed(20.0, r * 0.6).life(1.0, 2.4).size(r * 0.16, r * 0.4).gravity(-100.0).drag(1.5).intensity(0.8).over(0.5),
        Burst::new(Kind::Fire, at(p.y + r * 0.4), 300).radius(r * 0.18).vertical(2.0).toward(UP, 0.25).speed(60.0, 180.0).life(1.2, 2.6).size(r * 0.08, r * 0.22).gravity(-60.0).drag(0.9).intensity(0.9).over(2.5).after(0.3),
        // Smoke boiling up the column and spreading into the cap.
        Burst::new(Kind::Smoke, at(p.y + r * 0.5), 700).radius(r * 0.2).vertical(2.5).toward(UP, 0.3).speed(40.0, 120.0).life(5.0, 9.0).size(r * 0.15, r * 0.5).color([0.2, 0.19, 0.18]).intensity(0.6).gravity(-30.0).drag(0.6).over(3.0).after(0.5),
        // Burning chunks flung out in arcs.
        Burst::new(Kind::Ember, at(p.y + r * 0.1), 500).radius(r * 0.2).toward(UP, 1.2).speed(150.0, 450.0).life(2.0, 4.0).size(5.5, 3.0).gravity(280.0).drag(0.25).intensity(6.0),
        // Embers climbing into the cloud, then drifting down from its cap.
        Burst::new(Kind::Ember, at(p.y + r * 0.3), 10_000).radius(r * 0.7).vertical(0.8).toward(UP, 1.1).speed(30.0, 240.0).life(3.0, 9.0).size(4.0, 1.6).gravity(-14.0).drag(0.7).intensity(3.5).over(3.0),
        Burst::new(Kind::Ember, at(p.y + r * 0.95), 6000).radius(r * 0.7).vertical(0.35).toward([0.0, -1.0, 0.0], PI * 0.6).speed(0.0, 30.0).life(4.0, 8.0).size(2.8, 1.2).gravity(10.0).drag(0.5).intensity(4.5).over(4.0).after(1.5),
        // The dust wall and the grit.
        Burst::new(Kind::Dust, g, 1200).ground_color().radius(r * 0.2).vertical(0.05).outward(0.12).speed(r * 1.1, r * 2.3).life(2.5, 5.0).size(r * 0.1, r * 0.35).gravity(-3.0).drag(1.0).intensity(0.8).over(0.25),
        Burst::new(Kind::Dirt, g, 5000).ground_color().radius(r * 0.3).toward(UP, 0.9).speed(150.0, 520.0).life(1.6, 3.6).size(5.0, 4.0).drag(0.25).gravity(380.0),
    ]
}

impl Effects {
    /// Queues a GPU particle burst, unless it goes off far outside the view (it would be gone
    /// before the camera got there).
    pub(super) fn burst(&mut self, b: Burst) {
        if let Some((lo, hi)) = self.view {
            let margin = 700.0;
            let [x, _, z] = b.position;
            if x < lo.x - margin || x > hi.x + margin || z < lo.y - margin || z > hi.y + margin {
                return;
            }
        }
        self.bursts.push(b);
    }

    /// Whether the viewer can see `p` (fog of war), for emitters that keep going.
    pub(super) fn sees(&self, sim: &Sim, p: Vec3) -> bool {
        self.viewer.is_none_or(|player| sim.in_sight(player, FxVec3::new(Fx::from_f32(p.x), Fx::from_f32(p.y), Fx::from_f32(p.z))))
    }

    /// Hands this frame's bursts to the renderer with the pool for the effects quality and the
    /// map's wind, and how far the particles move this frame (`dt` = 0 while paused).
    pub fn submit_particles(&mut self, renderer: &mut Renderer, dt: f32) {
        let wind = self.wind(0.25) * 16.0;
        renderer.set_particle_settings(ParticleSettings {
            capacity: self.detail.particles,
            density: self.detail.particle_density,
            wind: [wind.x, wind.y],
            reduced_flashes: self.reduced_flashes,
            ..Default::default()
        });
        renderer.emit_particles(&self.bursts);
        renderer.step_particles(dt);
        self.bursts.clear();
        // Trees bend in the same wind.
        renderer.lighting.wind = self.wind(0.15);
        // The rain: wet ground, puddles and rings on the water.
        renderer.lighting.wetness = self.ambient.wetness.unwrap_or(0.0);
        renderer.lighting.rain = self.ambient.raining;
        // Rain clouds dim the sun while it rains.
        renderer.lighting.overcast = self.ambient.overcast;
    }

    /// A blast's sparks, flames, embers, chunks, grit and dust (see `blast_bursts`); `water` =
    /// it went off in the sea (lava on lava worlds).
    pub(super) fn blast_particles(&mut self, p: Vec3, radius: f32, ground: f32, water: bool, blast: Blast) {
        let under = match (water, self.lava_world) {
            (false, _) => Ground::Land,
            (true, false) => Ground::Sea,
            (true, true) => Ground::Lava,
        };
        for b in blast_bursts(p, radius, ground, under, blast, self.modern && under == Ground::Land) {
            self.burst(b);
        }
    }

    /// A fireball (see `fireball_bursts`).
    pub(super) fn fireball(&mut self, p: Vec3, radius: f32, life: f32) {
        for b in fireball_bursts(p, radius, life) {
            self.burst(b);
        }
    }

    /// The nuclear blast's storm (see `nuke_bursts`).
    pub(super) fn nuke_particles(&mut self, p: Vec3, radius: f32, ground: f32) {
        for b in nuke_bursts(p, radius, ground) {
            self.burst(b);
        }
    }

    /// A shower of sparks out of a gun's muzzle, towards `target`.
    pub(super) fn muzzle_particles(&mut self, p: Vec3, target: Vec3, big: bool) {
        let dir = (target - p).normalize_or(Vec3::Z);
        let n = if big { 18 } else { 7 };
        self.burst(Burst::new(Kind::Spark, p.to_array(), n).toward(dir.to_array(), 0.35).speed(90.0, if big { 360.0 } else { 260.0 }).life(0.1, 0.3).size(1.8, 0.9).intensity(7.0).collide(false));
    }

    /// One tick (every 0.12 s while burning) of a burning wreck: embers and licks of flame
    /// rising from it, spread over the tick so they keep coming; the odd ember once it only
    /// smoulders.
    pub(super) fn wreck_fire_particles(&mut self, p: Vec3, big: bool, burning: bool) {
        let r = if big { 14.0 } else { 8.0 };
        if burning {
            self.burst(Burst::new(Kind::Ember, p.to_array(), if big { 6 } else { 3 }).radius(r).toward(UP, 0.5).speed(15.0, 55.0).life(1.2, 3.2).size(3.0, 1.2).over(0.12));
            self.burst(Burst::new(Kind::Fire, (p + Vec3::Y * 3.0).to_array(), if big { 3 } else { 2 }).radius(r * 0.7).toward(UP, 0.35).speed(8.0, 25.0).life(0.4, 0.8).size(if big { 15.0 } else { 10.0 }, 3.0).gravity(-50.0).over(0.12));
        } else {
            self.burst(Burst::new(Kind::Ember, p.to_array(), 1).radius(r).toward(UP, 0.4).speed(10.0, 35.0).life(1.0, 2.2).size(2.4, 1.0).intensity(4.0));
        }
    }

    /// One tick (every 0.15 s) of a tree burning, its crown `crown` across round `p`: tongues
    /// of flame licking up out of the crown and embers whirling off it.
    pub(super) fn tree_fire_particles(&mut self, p: Vec3, crown: f32) {
        let r = crown * 0.4;
        self.burst(
            Burst::new(Kind::Fire, p.to_array(), 3)
                .radius(r)
                .vertical(0.8)
                .toward(UP, 0.3)
                .speed(10.0, 30.0)
                .life(0.5, 1.0)
                .size(crown * 0.55, crown * 0.2)
                .gravity(-60.0)
                .intensity(1.5)
                .over(0.15),
        );
        self.burst(Burst::new(Kind::Ember, (p + Vec3::Y * r * 0.5).to_array(), 4).radius(r).toward(UP, 0.6).speed(20.0, 60.0).life(1.0, 2.6).size(3.0, 1.2).over(0.15));
    }

    /// Embers trailing from a burning piece of debris flying at `vel`.
    pub(super) fn debris_trail_particles(&mut self, p: Vec3, vel: Vec3) {
        self.burst(Burst::new(Kind::Ember, p.to_array(), 2).radius(2.0).spread(PI).speed(5.0, 25.0).velocity((vel * 0.2).to_array()).life(0.5, 1.3).size(3.0, 1.2).intensity(6.0));
    }

    /// Sparks and flames spewing from the D-gun's ball as it ploughs along, `dt` seconds' worth.
    pub(super) fn dgun_particles(&mut self, p: Vec3, dir: Vec3, dt: f32) {
        let n = (dt * 220.0).ceil() as u32;
        self.burst(Burst::new(Kind::Spark, p.to_array(), n).radius(6.0).toward(UP, 1.4).speed(40.0, 200.0).velocity((dir * 40.0).to_array()).life(0.4, 1.1).size(2.4, 1.1).intensity(7.0).color([1.0, 0.85, 0.6]));
        self.burst(Burst::new(Kind::Fire, (p - dir * 8.0).to_array(), (dt * 60.0).ceil() as u32).radius(6.0).toward(UP, 0.6).speed(5.0, 25.0).life(0.25, 0.55).size(12.0, 5.0).intensity(2.0));
    }

    /// Sparks from a missile's exhaust at its nozzle `p`, back along `dir`, the way it flies;
    /// `dt` seconds' worth.
    pub(super) fn exhaust_particles(&mut self, p: Vec3, dir: Vec3, dt: f32) {
        let n = (dt * 45.0 + self.rand()).floor() as u32;
        if n > 0 {
            self.burst(Burst::new(Kind::Spark, p.to_array(), n).toward((-dir).to_array(), 0.45).speed(40.0, 120.0).life(0.15, 0.4).size(1.8, 0.9).intensity(6.0).collide(false));
        }
    }
}
