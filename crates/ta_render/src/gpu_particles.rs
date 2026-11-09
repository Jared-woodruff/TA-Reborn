//! GPU particles: sparks, embers, fire, dust, smoke, grit and spray that the GPU creates, moves
//! and draws itself, so a battle can throw hundreds of thousands of them.
//!
//! The game describes *bursts* ([`ParticleBurst`]: how many particles of which
//! [`ParticleKind`], where, which way and how fast, how long they live, how they look) and
//! queues them with [`Renderer::emit_particles`](crate::Renderer::emit_particles); only those
//! descriptors (128 bytes each) cross to the GPU, so tens of thousands of particles per frame
//! cost the CPU next to nothing. [`Renderer::step_particles`](crate::Renderer::step_particles)
//! sets how far they move per frame (0 = frozen, e.g. a paused game).
//!
//! ```ignore
//! // A shell hits the ground: sparks, grit and dust in the ground's own colour.
//! renderer.emit_particles(&[
//!     ParticleBurst::new(ParticleKind::Spark, pos, 60).speed(60.0, 220.0),
//!     ParticleBurst::new(ParticleKind::Dirt, pos, 30),
//!     ParticleBurst::new(ParticleKind::Dust, pos, 12).ground_color(),
//! ]);
//! renderer.step_particles(frame_seconds);
//! ```
//!
//! The particles live in two buffers of [`ParticleSettings::capacity`] particles used in turn:
//! additive ones packed from the front of a buffer, alpha-blended ones from its back. Each frame
//! one compute pass (after the shadow, cloud and contact occlusion passes, before the scene
//! pass) simulates the pool and sorts its translucent draw order:
//! 1. `emit`: one thread per requested particle finds its burst (a binary search over the bursts'
//!    first indices), initialises the particle from a random stream seeded by its index (so the
//!    result does not depend on thread order) and appends it to this frame's buffer;
//! 2. `plan`: sizes the simulation dispatch from the live count (an indirect dispatch);
//! 3. `simulate`: advances every live particle (gravity, drag through the moving air, swirling
//!    noise for embers, flames and billows), bounces sparks, embers and grit off the heightmap
//!    until they come to rest, quenches them in water and lava, and packs the survivors into the
//!    other buffer (one atomic per workgroup and side, not per particle);
//! 4. `finalize`: writes the two indirect draws and the pool state the next frame reads.
//! 5. Sort draw indices back to front for the actual camera and, when used, the reflected
//!    camera. Blocks of 64 keys are sorted in shared memory then merged; indirect dispatches
//!    stop after the largest live layer is sorted. Pure additive light needs no sorting.
//!
//! The work is proportional to the live particles, never to the pool size, and when nothing is
//! alive the pass and the draws are skipped altogether (the frame renders exactly as without
//! particles).
//!
//! The scene pass draws the survivors with two indexed indirect draws that pull four vertices
//! per particle straight from the buffer, in order (no instancing: quads of four vertices would
//! leave most of every vertex warp idle): the alpha-blended kinds (dust, smoke, grit, spray)
//! right after the smoke sprites, the additive ones (sparks, embers, fire, glows, fireballs)
//! right after the additive effect sprites; with the perspective camera the water's reflection
//! pass draws them once more, mirrored. Particles are camera-facing quads, depth tested without
//! writing depth, dimmed by the fog of war and hazed with distance (`apply_fog`, per vertex),
//! lit like the smoke (alpha-blended kinds), fading out near the ground instead of being cut by
//! it (soft particles), and never narrower than about a pixel and a half (they fade instead),
//! so distant sparks do not flicker. Additive colours are HDR: white-hot sparks bloom; a
//! fireball also hides what lies behind it, as its soot does. Both cameras draw them; see
//! `shaders/particle_draw.wgsl`.
//!
//! When the pool is more than half full, new particles are thinned out evenly over all bursts,
//! and past 70 % everything ages faster, so mass destruction degrades gracefully instead of new
//! explosions finding no free particles.
//!
//! Particles are presentation only: nothing is read back into the game except the pool's
//! statistics ([`Renderer::particle_stats`](crate::Renderer::particle_stats)), a few frames late.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use bytemuck::{Pod, Zeroable};

use crate::timing::PassTimestamps;
use crate::util::depth_state;

mod sort;

/// Pool size of [`ParticleSettings::default`].
pub const DEFAULT_CAPACITY: u32 = 1 << 18;
/// The largest pool.
pub const MAX_CAPACITY: u32 = 1 << 20;
const MIN_CAPACITY: u32 = 1 << 10;
/// Bursts emitted per frame at most (later ones wait for the next frame).
pub const MAX_BURSTS: usize = 8192;
/// Threads per workgroup of the emission and simulation dispatches (as in the shader).
const WORKGROUP: u32 = 64;
/// Frames whose statistics can be in flight at once.
const READBACK_SLOTS: usize = 3;

/// What a particle looks like and how it behaves; [`ParticleBurst::new`] fills in suitable
/// defaults for each kind. The first four are drawn additively (they glow, HDR), the others are
/// alpha-blended and lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ParticleKind {
    /// White-hot sparks that streak along their flight and cool through yellow and orange to
    /// dark red; they fall, bounce off the ground and lie there glowing, and die in water.
    /// [`ParticleBurst::cold`] ones keep their colour instead (electric arcs, beam hits).
    Spark = 0,
    /// Glowing embers: small flickering motes that rise on the hot air, swirl and cool.
    Ember = 1,
    /// A soft glow in the burst's colour that fades out (flashes, plasma).
    Glow = 2,
    /// Flames: soft billows that rise, swirl and cool from yellow to red as they burn out.
    Fire = 3,
    /// Dust kicked up from the ground: soft, lit billows that spread, grow and settle.
    Dust = 4,
    /// Smoke: rising, growing, swirling billows.
    Smoke = 5,
    /// Grit and clods of earth thrown up by impacts: small dark bits that fall and bounce.
    Dirt = 6,
    /// Water spray: droplets thrown up that fall back into the sea.
    Spray = 7,
    /// A fireball: a ball of flame that swells, churns and cools into soot as it rises, hiding
    /// what burns behind it. Drawn per pixel (a turbulence of noise, not a texture), so it holds
    /// its detail however close the camera comes; each one churns its own way.
    Fireball = 8,
    /// Rain: fine pale streaks falling on the wind, a pixel or so wide and never more than a
    /// few dozen long on screen, gone near the eye; seen from above, faint specks.
    Rain = 9,
}

impl ParticleKind {
    /// Drawn additively (glowing, the fireball hiding some of what lies behind it) rather than
    /// alpha-blended.
    pub fn additive(self) -> bool {
        matches!(self, ParticleKind::Spark | ParticleKind::Ember | ParticleKind::Glow | ParticleKind::Fire | ParticleKind::Fireball | ParticleKind::Rain)
    }
}

/// A burst of particles to emit (see the [module docs](self)). Build one with
/// [`ParticleBurst::new`], which fills in the kind's defaults, and adjust it with the builder
/// methods; all distances are world units, times seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleBurst {
    pub kind: ParticleKind,
    /// Particles to emit (scaled by [`ParticleSettings::density`]).
    pub count: u32,
    /// Centre of the emission volume.
    pub position: [f32; 3],
    /// Particles start at random points of a ball this big around `position`...
    pub radius: f32,
    /// ...squashed vertically by this factor (0 = a flat disc, 1 = a ball).
    pub vertical: f32,
    /// Mean direction of flight (any length); zero = outward from `position`, like a blast
    /// (with a flat volume: out along the ground).
    pub direction: [f32; 3],
    /// Half angle (radians) of the cone of directions around `direction` (π = any direction).
    pub spread: f32,
    /// Speed range.
    pub speed: [f32; 2],
    /// Velocity added to every particle (a moving emitter).
    pub velocity: [f32; 3],
    /// Lifetime range.
    pub life: [f32; 2],
    /// Diameter at birth and at death.
    pub size: [f32; 2],
    /// Colour (linear): tints the built-in colours of sparks, embers and flames, and is the
    /// colour of glows, dust, smoke, grit and spray. With `ground_color` it multiplies the
    /// painted ground's colour instead (greyed a little and pulled towards a mid tone, so dust
    /// shows against the ground it comes from; above 1 lightens it).
    pub color: [f32; 3],
    /// HDR brightness of the additive kinds; opacity (0..1) of the alpha-blended ones.
    pub intensity: f32,
    /// Downward acceleration (world units / s²; negative rises).
    pub gravity: f32,
    /// Air drag: the speed relative to the wind falls by a factor e^-drag per second.
    pub drag: f32,
    /// Births are spread over this long...
    pub duration: f32,
    /// ...starting this long after the burst is emitted.
    pub delay: f32,
    /// Bounces off (or settles on) the ground, and dies in water or lava (dust and smoke ride
    /// on it).
    pub collide: bool,
    /// Colour each particle like the painted ground where it starts (multiplied by `color`).
    pub ground_color: bool,
    /// Sparks: a white flash that settles to `color` and fades, instead of cooling through
    /// incandescent yellow, orange and red (electricity, energy beams).
    pub cold: bool,
    /// Kept a speck however near the eye it passes (a little fainter there): ambient motes that
    /// fill the air round a low camera (the embers over lava), which at their own size grew into
    /// big soft discs across the view.
    pub speck: bool,
}

impl ParticleBurst {
    /// `count` particles of `kind` at `position` with the kind's defaults: sparks fly up and out
    /// fast, fall and bounce; embers drift up; glows sit and fade; flames billow up; dust spreads
    /// out low; smoke rises; grit is thrown up and falls; spray is thrown up and falls back; a
    /// fireball swells where it is, rising a little; rain falls.
    pub fn new(kind: ParticleKind, position: [f32; 3], count: u32) -> ParticleBurst {
        use std::f32::consts::PI;
        let base = ParticleBurst {
            kind,
            count,
            position,
            radius: 2.0,
            vertical: 1.0,
            direction: [0.0, 1.0, 0.0],
            spread: PI * 0.5,
            speed: [20.0, 60.0],
            velocity: [0.0; 3],
            life: [1.0, 2.0],
            size: [4.0, 4.0],
            color: [1.0; 3],
            intensity: 1.0,
            gravity: 0.0,
            drag: 1.0,
            duration: 0.0,
            delay: 0.0,
            collide: false,
            ground_color: false,
            cold: false,
            speck: false,
        };
        match kind {
            ParticleKind::Spark => ParticleBurst {
                spread: 1.2,
                speed: [60.0, 240.0],
                life: [0.4, 1.1],
                size: [1.6, 0.9],
                intensity: 4.5,
                gravity: 380.0,
                drag: 0.7,
                collide: true,
                ..base
            },
            ParticleKind::Ember => ParticleBurst {
                spread: 0.9,
                speed: [15.0, 60.0],
                life: [1.5, 3.5],
                size: [2.6, 1.2],
                intensity: 5.0,
                gravity: -25.0,
                drag: 1.4,
                collide: true,
                ..base
            },
            ParticleKind::Glow => ParticleBurst { spread: PI, speed: [0.0, 8.0], life: [0.15, 0.35], size: [10.0, 16.0], intensity: 3.0, drag: 2.0, ..base },
            ParticleKind::Fire => ParticleBurst {
                radius: 6.0,
                spread: 0.8,
                speed: [10.0, 40.0],
                life: [0.5, 1.0],
                size: [12.0, 22.0],
                intensity: 2.2,
                gravity: -60.0,
                drag: 2.5,
                collide: true,
                ..base
            },
            ParticleKind::Dust => ParticleBurst {
                radius: 8.0,
                vertical: 0.3,
                direction: [0.0; 3],
                spread: 0.5,
                speed: [20.0, 70.0],
                life: [1.5, 3.0],
                size: [14.0, 42.0],
                color: [0.78, 0.68, 0.52],
                intensity: 0.45,
                gravity: 8.0,
                drag: 2.2,
                collide: true,
                ..base
            },
            ParticleKind::Smoke => ParticleBurst {
                radius: 6.0,
                spread: 0.6,
                speed: [10.0, 30.0],
                life: [2.0, 4.0],
                size: [14.0, 50.0],
                color: [0.32, 0.31, 0.3],
                intensity: 0.5,
                gravity: -14.0,
                drag: 1.2,
                collide: true,
                ..base
            },
            ParticleKind::Dirt => ParticleBurst {
                radius: 4.0,
                spread: 0.7,
                speed: [60.0, 180.0],
                life: [0.8, 1.6],
                size: [2.8, 2.4],
                color: [0.28, 0.23, 0.18],
                intensity: 0.95,
                gravity: 420.0,
                drag: 0.35,
                collide: true,
                ..base
            },
            ParticleKind::Spray => ParticleBurst {
                radius: 4.0,
                spread: 0.45,
                speed: [60.0, 200.0],
                life: [0.7, 1.4],
                size: [3.0, 6.0],
                color: [0.85, 0.92, 1.0],
                intensity: 0.7,
                gravity: 400.0,
                drag: 0.5,
                collide: true,
                ..base
            },
            ParticleKind::Rain => ParticleBurst {
                direction: [0.0, -1.0, 0.0],
                spread: 0.03,
                speed: [560.0, 640.0],
                life: [0.55, 0.8],
                size: [0.9, 0.8],
                color: [0.85, 0.9, 1.0],
                intensity: 0.3,
                drag: 0.0,
                ..base
            },
            ParticleKind::Fireball => ParticleBurst {
                radius: 0.0,
                spread: 0.6,
                speed: [0.0, 6.0],
                life: [0.9, 1.1],
                size: [16.0, 40.0],
                intensity: 1.0,
                gravity: -40.0,
                drag: 2.0,
                collide: true,
                ..base
            },
        }
    }

    pub fn count(mut self, count: u32) -> Self {
        self.count = count;
        self
    }

    /// Radius of the emission volume.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Vertical scale of the emission volume (0 = a flat disc).
    pub fn vertical(mut self, scale: f32) -> Self {
        self.vertical = scale;
        self
    }

    /// Fly within `spread` radians of `direction`.
    pub fn toward(mut self, direction: [f32; 3], spread: f32) -> Self {
        self.direction = direction;
        self.spread = spread;
        self
    }

    /// Fly outward from the centre (within `spread` radians of straight out).
    pub fn outward(mut self, spread: f32) -> Self {
        self.direction = [0.0; 3];
        self.spread = spread;
        self
    }

    /// Half angle of the cone of directions.
    pub fn spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    pub fn speed(mut self, min: f32, max: f32) -> Self {
        self.speed = [min, max];
        self
    }

    /// Velocity added to every particle.
    pub fn velocity(mut self, velocity: [f32; 3]) -> Self {
        self.velocity = velocity;
        self
    }

    pub fn life(mut self, min: f32, max: f32) -> Self {
        self.life = [min, max];
        self
    }

    /// Diameter at birth and at death.
    pub fn size(mut self, start: f32, end: f32) -> Self {
        self.size = [start, end];
        self
    }

    pub fn color(mut self, rgb: [f32; 3]) -> Self {
        self.color = rgb;
        self
    }

    /// HDR brightness (additive kinds) or opacity (alpha-blended kinds).
    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// Downward acceleration (negative rises).
    pub fn gravity(mut self, gravity: f32) -> Self {
        self.gravity = gravity;
        self
    }

    pub fn drag(mut self, drag: f32) -> Self {
        self.drag = drag;
        self
    }

    /// Spread the births over `seconds` (a fire that keeps going, a shower that keeps coming).
    pub fn over(mut self, seconds: f32) -> Self {
        self.duration = seconds;
        self
    }

    /// Wait `seconds` before the first birth.
    pub fn after(mut self, seconds: f32) -> Self {
        self.delay = seconds;
        self
    }

    pub fn collide(mut self, collide: bool) -> Self {
        self.collide = collide;
        self
    }

    /// Sparks keep their colour (a white flash, then `color` fading out) rather than cooling
    /// like hot metal: electric arcs and energy beams.
    pub fn cold(mut self) -> Self {
        self.cold = true;
        self
    }

    /// Never drawn wider than a speck, however near the eye (see [`ParticleBurst::speck`]).
    pub fn speck(mut self) -> Self {
        self.speck = true;
        self
    }

    /// Take the painted ground's colour where each particle starts (greyer and pulled towards a
    /// mid tone, so it shows against the ground it comes from), darkened for grit; `color` can
    /// adjust it after.
    pub fn ground_color(mut self) -> Self {
        self.ground_color = true;
        self.color = match self.kind {
            ParticleKind::Dust | ParticleKind::Smoke => [1.0; 3],
            ParticleKind::Dirt => [0.55, 0.5, 0.45],
            _ => [1.0; 3],
        };
        self
    }

    fn is_valid(&self) -> bool {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        finite(&self.position)
            && finite(&self.direction)
            && finite(&self.velocity)
            && finite(&self.speed)
            && finite(&self.life)
            && finite(&self.size)
            && finite(&self.color)
            && finite(&[self.radius, self.vertical, self.spread, self.intensity, self.gravity, self.drag, self.duration, self.delay])
            && self.life[1] > 0.0
    }
}

/// Pool size, density and the air the particles move in; see
/// [`Renderer::set_particle_settings`](crate::Renderer::set_particle_settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleSettings {
    /// Most particles alive at once (clamped to 1024..=1 048 576). Their memory is 144 bytes
    /// each; their cost follows the live count only. Changing it empties the pool.
    pub capacity: u32,
    /// Multiplies every burst's count (e.g. fewer at low effects quality).
    pub density: f32,
    /// Horizontal wind (world units / s along x and z) that carries embers, flames, smoke and
    /// dust (sparks and spray a little).
    pub wind: [f32; 2],
    /// Height over which dust and smoke fade out where they meet the ground (soft particles;
    /// 0 = cut off by the ground). Flames always fade.
    pub soft_ground: f32,
    /// Dim additive effects at draw time, including particles emitted before a toggle.
    pub reduced_flashes: bool,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        ParticleSettings { capacity: DEFAULT_CAPACITY, density: 1.0, wind: [0.0; 2], soft_ground: 12.0, reduced_flashes: false }
    }
}

/// The particle pool as measured a few frames ago.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParticleStats {
    /// Particles alive (including those waiting to be born).
    pub live: u32,
    /// Particles emitted in that frame (after thinning).
    pub emitted: u32,
    /// Pool size.
    pub capacity: u32,
}

/// One particle (mirrors `Particle` in `particle_common.wgsl`; created on the GPU only).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuParticle {
    pos: [f32; 3],
    age: f32,
    vel: [f32; 3],
    life: f32,
    data: [u32; 4],
}

/// Mirrors `ParticleParams` in `particle_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuParams {
    step: [f32; 4],
    counts: [u32; 4],
    state: [u32; 4],
    wind: [f32; 4],
}

/// Mirrors `Burst` in `particle_sim.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
struct GpuBurst {
    pos: [f32; 4],
    dir: [f32; 4],
    vel: [f32; 4],
    speed_life: [f32; 4],
    size_phys: [f32; 4],
    color: [f32; 4],
    info: [u32; 4],
    extra: [f32; 4],
}

/// Mirrors `Counters` in `particle_sim.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCounters {
    emitted: u32,
    free: u32,
    pressure: f32,
    live: u32,
    last_emitted: u32,
    covered: u32,
    pad: [u32; 2],
    front: [u32; 2],
    back: [u32; 2],
    total: [u32; 2],
    pad2: [u32; 2],
}

/// Byte offsets of the indirect arguments (mirrors `Indirect` in `particle_sim.wgsl`).
const INDIRECT_DISPATCH: u64 = 0;
const INDIRECT_ADDITIVE: u64 = 16;
const INDIRECT_ALPHA: u64 = 48;
const INDIRECT_SIZE: u64 = sort::SORT_INDIRECT + sort::SORT_STEPS as u64 * 16;

/// Burst flags (`info.z >> 8`, as in `particle_sim.wgsl`).
const EMIT_COLLIDE: u32 = 1;
const EMIT_GROUND_COLOR: u32 = 2;
const EMIT_COLD: u32 = 4;
const EMIT_SPECK: u32 = 8;

impl GpuBurst {
    fn new(b: &ParticleBurst, first: u32, count: u32) -> GpuBurst {
        let flags = if b.collide { EMIT_COLLIDE } else { 0 }
            | if b.ground_color { EMIT_GROUND_COLOR } else { 0 }
            | if b.cold { EMIT_COLD } else { 0 }
            | if b.speck { EMIT_SPECK } else { 0 };
        let (min_speed, max_speed) = (b.speed[0].max(0.0), b.speed[1].max(0.0));
        let (min_life, max_life) = (b.life[0].max(0.0), b.life[1].max(0.0));
        GpuBurst {
            pos: [b.position[0], b.position[1], b.position[2], b.radius.max(0.0)],
            dir: [b.direction[0], b.direction[1], b.direction[2], b.spread.clamp(0.0, std::f32::consts::PI).cos()],
            vel: [b.velocity[0], b.velocity[1], b.velocity[2], b.vertical.max(0.0)],
            speed_life: [min_speed.min(max_speed), max_speed, min_life.min(max_life), max_life],
            size_phys: [b.size[0].max(0.0), b.size[1].max(0.0), b.gravity, b.drag.max(0.0)],
            color: [b.color[0].max(0.0), b.color[1].max(0.0), b.color[2].max(0.0), b.intensity.max(0.0)],
            info: [first, count, b.kind as u32 | flags << 8, 0],
            extra: [b.duration.max(0.0), b.delay.max(0.0), 0.0, 0.0],
        }
    }
}

/// Scales the queued bursts by `density`, fits them into `limit` particles and lays them out for
/// the emission dispatch (each with the index of its first particle). Fractional counts are
/// dithered with `rng` so small bursts at low density still emit on average what they should.
/// Returns the GPU bursts and their total particle count.
fn pack_bursts(queued: &[ParticleBurst], density: f32, limit: u32, rng: &mut u32) -> (Vec<GpuBurst>, u32) {
    let density = if density.is_finite() { density.max(0.0) } else { 1.0 };
    let wanted: f64 = queued.iter().map(|b| b.count as f64 * density as f64).sum();
    let scale = if wanted > limit as f64 { limit as f64 / wanted } else { 1.0 };
    let mut out = Vec::with_capacity(queued.len().min(MAX_BURSTS));
    let mut total = 0u32;
    for b in queued.iter().take(MAX_BURSTS) {
        let exact = b.count as f64 * density as f64 * scale;
        // xorshift32: dithering only.
        *rng ^= *rng << 13;
        *rng ^= *rng >> 17;
        *rng ^= *rng << 5;
        let dither = (*rng >> 8) as f64 / (1u32 << 24) as f64;
        let n = ((exact + dither).floor() as u32).min(limit - total);
        if n == 0 {
            continue;
        }
        out.push(GpuBurst::new(b, total, n));
        total += n;
    }
    (out, total)
}

/// Blend layer of a particle draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParticleLayer {
    /// Dust, smoke, grit, spray (drawn with the smoke sprites).
    Alpha,
    /// Sparks, embers, flames, glows (drawn with the additive effect sprites).
    Additive,
}

/// Buffers sized by the pool capacity.
struct Pool {
    capacity: u32,
    /// The two particle buffers, used in turn.
    particles: [wgpu::Buffer; 2],
    counters: wgpu::Buffer,
    indirect: wgpu::Buffer,
    /// Two triangles per particle: 4q, 4q+1, 4q+2 and 4q+2, 4q+1, 4q+3 (static).
    indices: wgpu::Buffer,
}

/// Bind groups: emission and simulation per input buffer, the dispatch/draw bookkeeping, and
/// the draws per buffer.
struct Groups {
    sim: [wgpu::BindGroup; 2],
    control: wgpu::BindGroup,
    draw: [[wgpu::BindGroup; 2]; 2],
}

const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

/// A staging buffer the pool's counters are copied into for [`ParticleStats`].
struct Readback {
    buffer: wgpu::Buffer,
    frame: u64,
    state: Arc<AtomicU8>,
}

/// The particle system (owned by the renderer).
pub(crate) struct GpuParticles {
    settings: ParticleSettings,
    emit: wgpu::ComputePipeline,
    plan: wgpu::ComputePipeline,
    simulate: wgpu::ComputePipeline,
    finalize: wgpu::ComputePipeline,
    draw_additive: wgpu::RenderPipeline,
    draw_alpha: wgpu::RenderPipeline,
    /// The same draws mirrored in the water (the reflection pass).
    reflect_additive: wgpu::RenderPipeline,
    reflect_alpha: wgpu::RenderPipeline,
    sim_layout: wgpu::BindGroupLayout,
    control_layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    bursts: wgpu::Buffer,
    billows: wgpu::TextureView,
    billow_sampler: wgpu::Sampler,
    linear_clamp: wgpu::Sampler,
    /// The heightmap and the tile colours of the current map (or stand-ins).
    heights: wgpu::TextureView,
    ground_colors: wgpu::TextureView,
    /// A 1x1 sand-coloured stand-in for the tile colours before a map is loaded.
    no_ground_colors: wgpu::TextureView,
    pool: Pool,
    sorter: sort::Sorter,
    /// Always present once constructed (the groups need the finished struct to be made).
    groups: Option<Groups>,
    /// The pool must be (re)initialised before its next use.
    reset: bool,
    /// Bursts queued for the next frame.
    queued: Vec<ParticleBurst>,
    dt: f32,
    /// Particle clock (s; advances with the steps).
    clock: f32,
    /// The buffer the next simulation reads (the draws read the other).
    list: u32,
    frame: u64,
    rng: u32,
    /// This frame's emission (particles) and whether the passes run.
    requested: u32,
    active: bool,
    /// Particles may be alive: set by emission, cleared by a readback showing none.
    maybe_alive: bool,
    last_emit_frame: u64,
    readbacks: Vec<Readback>,
    next_readback: usize,
    /// The readback slot filled this frame (mapped after the submit).
    pending_readback: Option<usize>,
    stats: ParticleStats,
    stats_frame: u64,
}

impl GpuParticles {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32, heights: &wgpu::TextureView) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S};
        let buffer = |binding, visibility, read_only: Option<bool>| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: B::Buffer {
                ty: match read_only {
                    None => wgpu::BufferBindingType::Uniform,
                    Some(read_only) => wgpu::BufferBindingType::Storage { read_only },
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: B::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let sampler = |binding, visibility| wgpu::BindGroupLayoutEntry { binding, visibility, ty: B::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle simulation"),
            entries: &[
                buffer(0, S::COMPUTE, None),
                buffer(1, S::COMPUTE, Some(false)),
                buffer(2, S::COMPUTE, Some(false)),
                buffer(3, S::COMPUTE, Some(false)),
                buffer(4, S::COMPUTE, Some(true)),
                texture(5, S::COMPUTE),
                texture(6, S::COMPUTE),
                sampler(7, S::COMPUTE),
            ],
        });
        // The indirect arguments are written by the bookkeeping dispatches only: a buffer cannot
        // be bound for writing in the dispatch that reads its arguments.
        let control_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle control"),
            entries: &[buffer(0, S::COMPUTE, None), buffer(3, S::COMPUTE, Some(false)), buffer(8, S::COMPUTE, Some(false))],
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle draw"),
            entries: &[buffer(0, S::VERTEX | S::FRAGMENT, None), buffer(1, S::VERTEX, Some(true)), texture(2, S::FRAGMENT), sampler(3, S::FRAGMENT), buffer(4, S::VERTEX, Some(true)), buffer(5, S::VERTEX, Some(true))],
        });

        let sim_shader = crate::util::shader_standalone(device, "particle simulation", &sim_source());
        let compute = |label: &str, layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &sim_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let emit = compute("particle emit", &sim_layout, "cs_emit");
        let plan = compute("particle plan", &control_layout, "cs_plan");
        let simulate = compute("particle simulate", &sim_layout, "cs_simulate");
        let finalize = compute("particle finalize", &control_layout, "cs_finalize");

        let draw_shader = crate::util::shader(device, "particle draw", &draw_source());
        let draw_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle draw"),
            bind_group_layouts: &[Some(frame_layout), Some(&draw_layout)],
            immediate_size: 0,
        });
        // The scene pass's pipelines, or with `mirrored` those of the water's reflection pass
        // (one sample a pixel, the mirror turning triangles over; see reflection.rs).
        let render = |label: &str, vs: &str, fs: &str, blend: wgpu::BlendState, mirrored: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&draw_pl),
                vertex: wgpu::VertexState { module: &draw_shader, entry_point: Some(vs), buffers: &[], compilation_options: Default::default() },
                primitive: if mirrored { crate::reflection::primitive() } else { wgpu::PrimitiveState { cull_mode: None, ..Default::default() } },
                depth_stencil: Some(wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) }),
                multisample: wgpu::MultisampleState { count: if mirrored { 1 } else { samples }, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // Additive light leaves the target's alpha (the heat haze channel) alone, like the
        // additive sprites; the alpha-blended kinds blend like the smoke sprites. A fragment's
        // own alpha is how much of what lies behind it it hides: 0 for plain light (sparks,
        // embers), more for flames (see particle_draw.wgsl).
        let additive_blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let draw_additive = render("particles additive", "vs_particle_additive", "fs_particle_additive", additive_blend, false);
        let draw_alpha = render("particles alpha", "vs_particle_alpha", "fs_particle_alpha", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING, false);
        let reflect_additive = render("particles additive reflection", "vs_particle_additive", "fs_particle_additive_reflection", additive_blend, true);
        let reflect_alpha = render("particles alpha reflection", "vs_particle_alpha", "fs_particle_alpha_reflection", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING, true);

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle params"),
            size: std::mem::size_of::<GpuParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bursts = make_bursts(device, 64);
        let billows = billow_texture(device, queue);
        let billow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle billows"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let linear_clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle ground"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let no_ground_colors = {
            let t = crate::util::Target::new(device, "particle ground colour stand-in", 1, 1, wgpu::TextureFormat::Rgba8UnormSrgb, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
            crate::util::write_texture_level(queue, &t.texture, 0, 1, 1, &[196, 178, 146, 255], 4);
            t.view
        };
        let settings = ParticleSettings::default();
        // A token pool: the real one is made when there is first something to emit.
        let pool = Pool::new(device, MIN_CAPACITY);
        let sorter = sort::Sorter::new(device, &pool);
        let readbacks = (0..READBACK_SLOTS)
            .map(|_| Readback {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("particle statistics"),
                    size: std::mem::size_of::<GpuCounters>() as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                frame: 0,
                state: Arc::new(AtomicU8::new(IDLE)),
            })
            .collect();
        let mut particles = GpuParticles {
            settings,
            emit,
            plan,
            simulate,
            finalize,
            draw_additive,
            draw_alpha,
            reflect_additive,
            reflect_alpha,
            groups: None,
            sim_layout,
            control_layout,
            draw_layout,
            params,
            bursts,
            billows,
            billow_sampler,
            linear_clamp,
            heights: heights.clone(),
            ground_colors: no_ground_colors.clone(),
            no_ground_colors,
            pool,
            sorter,
            reset: true,
            queued: Vec::new(),
            dt: 0.0,
            clock: 0.0,
            list: 0,
            frame: 0,
            rng: 0x9E37_79B9,
            requested: 0,
            active: false,
            maybe_alive: false,
            last_emit_frame: 0,
            readbacks,
            next_readback: 0,
            pending_readback: None,
            stats: ParticleStats::default(),
            stats_frame: 0,
        };
        particles.groups = Some(particles.make_groups(device));
        particles
    }

    pub fn settings(&self) -> ParticleSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: ParticleSettings) {
        self.settings = settings;
    }

    pub fn queue(&mut self, bursts: &[ParticleBurst]) {
        enqueue(&mut self.queued, bursts, MAX_BURSTS * 4);
    }

    /// Drops the bursts queued for a frame that is not drawn (their particles are never seen):
    /// kept, a capture running on ahead of its shot let loose a minute's queue when it began.
    pub fn drop_queued(&mut self) {
        self.queued.clear();
    }

    pub fn set_step(&mut self, dt: f32) {
        self.dt = if dt.is_finite() { dt.clamp(0.0, 0.5) } else { 0.0 };
    }

    /// Empties the pool (a new map).
    pub fn clear(&mut self) {
        self.reset = true;
        self.queued.clear();
        self.maybe_alive = false;
        self.stats = ParticleStats::default();
        // Statistics still in flight describe the old particles.
        self.stats_frame = self.frame + 1;
    }

    pub fn stats(&self) -> ParticleStats {
        ParticleStats { capacity: capacity_for(&self.settings), ..self.stats }
    }

    /// Uses a new map's heightmap and tile colours (`None`: the stand-ins).
    pub fn set_ground(&mut self, device: &wgpu::Device, heights: &wgpu::TextureView, colors: Option<&wgpu::TextureView>) {
        self.heights = heights.clone();
        self.ground_colors = colors.unwrap_or(&self.no_ground_colors).clone();
        self.groups = Some(self.make_groups(device));
        self.clear();
    }

    fn make_groups(&self, device: &wgpu::Device) -> Groups {
        let pool = &self.pool;
        let sim = |input: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle simulation"),
                layout: &self.sim_layout,
                entries: &[
                    entry(0, &self.params),
                    entry(1, &pool.particles[input]),
                    entry(2, &pool.particles[1 - input]),
                    entry(3, &pool.counters),
                    entry(4, &self.bursts),
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.heights) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.ground_colors) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.linear_clamp) },
                ],
            })
        };
        let draw = |list: usize, view: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle draw"),
                layout: &self.draw_layout,
                entries: &[
                    entry(0, &self.params),
                    entry(1, &pool.particles[list]),
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.billows) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.billow_sampler) },
                    entry(4, &self.sorter.order[view]),
                    entry(5, &pool.counters),
                ],
            })
        };
        let control = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle control"),
            layout: &self.control_layout,
            entries: &[entry(0, &self.params), entry(3, &pool.counters), entry(8, &pool.indirect)],
        });
        Groups { sim: [sim(0), sim(1)], control, draw: [[draw(0, 0), draw(0, 1)], [draw(1, 0), draw(1, 1)]] }
    }

    /// Picks up statistics that have arrived (never blocks).
    fn collect(&mut self, device: &wgpu::Device) {
        if self.readbacks.iter().any(|r| r.state.load(Ordering::Acquire) == MAPPING) {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        for r in &mut self.readbacks {
            match r.state.load(Ordering::Acquire) {
                READY => {
                    if r.frame >= self.stats_frame && let Ok(data) = r.buffer.slice(..).get_mapped_range() {
                        let c: GpuCounters = bytemuck::pod_read_unaligned(&data[..std::mem::size_of::<GpuCounters>()]);
                        self.stats = ParticleStats { live: c.live, emitted: c.last_emitted, capacity: self.pool.capacity };
                        self.stats_frame = r.frame;
                    }
                    r.buffer.unmap();
                    r.state.store(IDLE, Ordering::Release);
                }
                FAILED => r.state.store(IDLE, Ordering::Release),
                _ => {}
            }
        }
        // Nothing alive since the last emission: go idle until the next one.
        if self.maybe_alive && self.stats_frame >= self.last_emit_frame && self.stats.live == 0 {
            self.maybe_alive = false;
        }
    }

    /// Uploads this frame's bursts and parameters; returns whether the particle passes run this
    /// frame (anything emitted or possibly alive). `liquid` = height of the sea or lava surface.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, liquid: Option<f32>, view_depth: [f32; 4], reflected_depth: Option<[f32; 4]>) -> bool {
        self.frame += 1;
        self.collect(device);
        if self.queued.is_empty() && !self.maybe_alive {
            self.active = false;
            return false;
        }
        let capacity = capacity_for(&self.settings);
        if capacity != self.pool.capacity {
            self.pool = Pool::new(device, capacity);
            self.sorter = sort::Sorter::new(device, &self.pool);
            self.groups = Some(self.make_groups(device));
            self.reset = true;
        }
        if self.reset {
            self.reset = false;
            self.pool.initialise(queue);
            self.list = 0;
        }
        // At most half the pool per frame: one frame's bursts never flush everything else.
        let limit = (self.pool.capacity / 2).max(1);
        let take = self.queued.len().min(MAX_BURSTS);
        let (bursts, requested) = pack_bursts(&self.queued[..take], self.settings.density, limit, &mut self.rng);
        self.queued.drain(..take);
        self.requested = requested;
        if requested > 0 {
            if (bursts.len() * std::mem::size_of::<GpuBurst>()) as u64 > self.bursts.size() {
                self.bursts = make_bursts(device, bursts.len().next_power_of_two());
                self.groups = Some(self.make_groups(device));
            }
            queue.write_buffer(&self.bursts, 0, bytemuck::cast_slice(&bursts));
            self.maybe_alive = true;
            self.last_emit_frame = self.frame;
        }
        self.active = self.maybe_alive;
        if !self.active {
            return false;
        }
        // (Wrapped: it only sways the noise and the flicker, and a clock of hours would step.)
        self.clock = (self.clock + self.dt) % 4096.0;
        let seed = {
            let mut h = (self.frame as u32).wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
            h ^= h >> 16;
            h.wrapping_mul(0x7FEB_352D)
        };
        let s = &self.settings;
        let params = GpuParams {
            step: [self.dt, self.clock, liquid.unwrap_or(-1.0e6), s.soft_ground.max(0.0)],
            counts: [self.pool.capacity, requested, bursts.len() as u32, seed],
            state: [self.list, 0, 0, 0],
            wind: [s.wind[0], s.wind[1], if s.reduced_flashes { 1.0 } else { 0.0 }, 0.0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        self.sorter.prepare(queue, self.pool.capacity, self.list, view_depth, reflected_depth);
        true
    }

    /// Records this frame's compute pass (after [`GpuParticles::prepare`] returned true).
    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, timestamps: Option<&PassTimestamps>) {
        let Some(groups) = self.groups.as_ref().filter(|_| self.active) else { return };
        let list = self.list as usize;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("particles"), timestamp_writes: timestamps.map(|t| t.compute_writes()) });
            if self.requested > 0 {
                pass.set_pipeline(&self.emit);
                pass.set_bind_group(0, &groups.sim[list], &[]);
                pass.dispatch_workgroups(self.requested.div_ceil(WORKGROUP), 1, 1);
            }
            pass.set_pipeline(&self.plan);
            pass.set_bind_group(0, &groups.control, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.simulate);
            pass.set_bind_group(0, &groups.sim[list], &[]);
            pass.dispatch_workgroups_indirect(&self.pool.indirect, INDIRECT_DISPATCH);
            pass.set_pipeline(&self.finalize);
            pass.set_bind_group(0, &groups.control, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            self.sorter.encode(&mut pass, &self.pool, list);
        }
        let slot = self.next_readback;
        if self.readbacks[slot].state.load(Ordering::Acquire) == IDLE {
            let r = &mut self.readbacks[slot];
            encoder.copy_buffer_to_buffer(&self.pool.counters, 0, &r.buffer, 0, std::mem::size_of::<GpuCounters>() as u64);
            r.frame = self.frame;
            self.pending_readback = Some(slot);
        }
    }

    /// Draws one layer of the live particles (inside the scene pass, frame bind group set).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layer: ParticleLayer) {
        let pipeline = match layer {
            ParticleLayer::Alpha => &self.draw_alpha,
            ParticleLayer::Additive => &self.draw_additive,
        };
        self.draw_with(pass, layer, pipeline, 0);
    }

    /// Draws one layer mirrored in the water (inside the reflection pass, its frame bind group
    /// set): what flies above the water, where the water shows it.
    pub fn draw_reflection<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layer: ParticleLayer) {
        let pipeline = match layer {
            ParticleLayer::Alpha => &self.reflect_alpha,
            ParticleLayer::Additive => &self.reflect_additive,
        };
        self.draw_with(pass, layer, pipeline, 1);
    }

    fn draw_with<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layer: ParticleLayer, pipeline: &'a wgpu::RenderPipeline, view: usize) {
        let Some(groups) = self.groups.as_ref().filter(|_| self.active) else { return };
        let offset = match layer {
            ParticleLayer::Alpha => INDIRECT_ALPHA,
            ParticleLayer::Additive => INDIRECT_ADDITIVE,
        };
        pass.set_pipeline(pipeline);
        // The simulation wrote the other buffer.
        pass.set_bind_group(1, &groups.draw[1 - self.list as usize][view], &[]);
        pass.set_index_buffer(self.pool.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed_indirect(&self.pool.indirect, offset);
    }

    /// After the frame was submitted: starts reading its statistics back and switches lists.
    pub fn after_submit(&mut self) {
        if let Some(slot) = self.pending_readback.take() {
            let r = &self.readbacks[slot];
            r.state.store(MAPPING, Ordering::Release);
            let state = r.state.clone();
            r.buffer.map_async(wgpu::MapMode::Read, .., move |result| state.store(if result.is_ok() { READY } else { FAILED }, Ordering::Release));
            self.next_readback = (slot + 1) % READBACK_SLOTS;
        }
        if self.active {
            self.list = 1 - self.list;
        }
    }
}

impl Pool {
    fn new(device: &wgpu::Device, capacity: u32) -> Pool {
        use crate::gpu::CheckedBufferInit;
        let storage = |label: &str, size: u64, extra: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | extra, mapped_at_creation: false })
        };
        let bytes = capacity as u64 * std::mem::size_of::<GpuParticle>() as u64;
        let indices: Vec<u32> = (0..capacity).flat_map(|q| [0, 1, 2, 2, 1, 3].map(|c| 4 * q + c)).collect();
        Pool {
            capacity,
            particles: [storage("particles", bytes, wgpu::BufferUsages::empty()), storage("particles", bytes, wgpu::BufferUsages::empty())],
            counters: storage("particle counters", std::mem::size_of::<GpuCounters>() as u64, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC),
            indirect: storage("particle indirect", INDIRECT_SIZE, wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST),
            indices: device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
                label: Some("particle indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        }
    }

    /// Both buffers empty, nothing to draw.
    fn initialise(&self, queue: &wgpu::Queue) {
        let counters = GpuCounters { free: self.capacity, ..Zeroable::zeroed() };
        queue.write_buffer(&self.counters, 0, bytemuck::bytes_of(&counters));
        queue.write_buffer(&self.indirect, 0, &[0u8; INDIRECT_SIZE as usize]);
    }
}

/// The pool size for `settings` (clamped, a whole number of workgroups).
fn capacity_for(settings: &ParticleSettings) -> u32 {
    settings.capacity.clamp(MIN_CAPACITY, MAX_CAPACITY).next_multiple_of(WORKGROUP)
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

fn make_bursts(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle bursts"),
        size: (n.max(1) * std::mem::size_of::<GpuBurst>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Size of one billow shape of the atlas (2 x 2 shapes), and its mip levels.
const BILLOW: u32 = 128;
const BILLOW_LEVELS: usize = 5;

/// Four billow shapes for flames, dust and smoke: lumpy blobs of fractal noise, soft-edged and
/// transparent at their cells' borders, in a 2 x 2 R8 atlas with box-filtered mips.
fn billow_pixels() -> Vec<u8> {
    let size = BILLOW * 2;
    let mut pixels = vec![0u8; (size * size) as usize];
    let hash = |x: i32, y: i32, s: u32| {
        let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ s.wrapping_mul(0xCB1A_B31F);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5BD1_E995);
        h ^= h >> 15;
        (h & 0xFFFF) as f32 / 65535.0
    };
    let value_noise = |x: f32, y: f32, s: u32| {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let (ix, iy) = (x0 as i32, y0 as i32);
        let a = hash(ix, iy, s) + (hash(ix + 1, iy, s) - hash(ix, iy, s)) * sx;
        let b = hash(ix, iy + 1, s) + (hash(ix + 1, iy + 1, s) - hash(ix, iy + 1, s)) * sx;
        a + (b - a) * sy
    };
    let smoothstep = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for shape in 0..4u32 {
        let (ox, oy) = ((shape % 2) * BILLOW, (shape / 2) * BILLOW);
        for y in 0..BILLOW {
            for x in 0..BILLOW {
                let u = (x as f32 + 0.5) / BILLOW as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / BILLOW as f32 * 2.0 - 1.0;
                let d = (u * u + v * v).sqrt();
                let mut n = 0.0;
                let mut amplitude = 0.5;
                let mut frequency = 2.6;
                for octave in 0..4 {
                    n += value_noise(u * frequency + 17.0, v * frequency + 31.0, shape * 4 + octave) * amplitude;
                    amplitude *= 0.5;
                    frequency *= 2.0;
                }
                let n = n / 0.9375;
                // A lumpy outline, denser inside, and nothing at the cell's border.
                let edge = 1.0 - smoothstep(0.3, 0.95, d + (n - 0.5) * 0.55);
                let density = edge * (0.35 + 0.95 * n) * (1.0 - smoothstep(0.85, 1.0, d));
                pixels[((oy + y) * size + ox + x) as usize] = (density.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    pixels
}

fn billow_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let size = BILLOW * 2;
    // Down to 8 x 8 texels per shape: coarser levels would average each shape into a flat
    // square (and the shapes into each other).
    let mut levels = crate::terrain::byte_mips(size, size, 1, &billow_pixels());
    levels.truncate(BILLOW_LEVELS);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("particle billows"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (w, h, data)) in levels.iter().enumerate() {
        crate::util::write_texture_level(queue, &texture, level as u32, *w, *h, data, 1);
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The compute shader: noise (for the swirls), the shared declarations, the simulation.
pub(crate) fn sim_source() -> String {
    format!("{}\n{}\n{}", include_str!("shaders/noise.wgsl"), include_str!("shaders/particle_common.wgsl"), include_str!("shaders/particle_sim.wgsl"))
}

/// The draw shader: noise (the fireballs' turbulence), the shared declarations, the draws
/// (`common.wgsl` is prepended by [`crate::util::shader`]).
pub(crate) fn draw_source() -> String {
    format!("{}\n{}\n{}", include_str!("shaders/noise.wgsl"), include_str!("shaders/particle_common.wgsl"), include_str!("shaders/particle_draw.wgsl"))
}

/// Adds `bursts` to the queue of those still to be emitted, keeping at most `room`: a window that
/// stops presenting (minimised) must not let it grow forever, and past its room the oldest go,
/// so what it emits when it is back is what was just happening (not the blasts of when it went).
fn enqueue(queued: &mut Vec<ParticleBurst>, bursts: &[ParticleBurst], room: usize) {
    queued.extend(bursts.iter().filter(|b| b.count > 0 && b.is_valid()));
    let over = queued.len().saturating_sub(room);
    queued.drain(..over);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(count: u32) -> ParticleBurst {
        ParticleBurst::new(ParticleKind::Spark, [0.0; 3], count)
    }

    #[test]
    fn bursts_are_laid_out_back_to_back() {
        let mut rng = 1;
        let (packed, total) = pack_bursts(&[burst(10), burst(0), burst(5), burst(20)], 1.0, 1000, &mut rng);
        assert_eq!(total, 35);
        let firsts: Vec<(u32, u32)> = packed.iter().map(|b| (b.info[0], b.info[1])).collect();
        assert_eq!(firsts, vec![(0, 10), (10, 5), (15, 20)]);
    }

    #[test]
    fn a_full_queue_keeps_the_newest_bursts() {
        let mut queued = Vec::new();
        enqueue(&mut queued, &[burst(1), burst(0), burst(2)], 4);
        enqueue(&mut queued, &[burst(3), burst(4), burst(5)], 4);
        let counts: Vec<u32> = queued.iter().map(|b| b.count).collect();
        assert_eq!(counts, vec![2, 3, 4, 5]);
    }

    #[test]
    fn density_scales_counts_on_average() {
        let mut rng = 7;
        let queued: Vec<ParticleBurst> = (0..1000).map(|_| burst(3)).collect();
        let (_, total) = pack_bursts(&queued, 0.25, u32::MAX, &mut rng);
        // 750 expected; dithering keeps small bursts alive at low density.
        assert!((700..=800).contains(&total), "{total}");
    }

    #[test]
    fn emission_fits_the_limit() {
        let mut rng = 3;
        let (packed, total) = pack_bursts(&[burst(50_000), burst(50_000), burst(10)], 1.0, 20_000, &mut rng);
        assert!((19_990..=20_000).contains(&total), "{total}");
        let sum: u32 = packed.iter().map(|b| b.info[1]).sum();
        assert_eq!(sum, total);
        // Every burst keeps its share.
        assert!(packed[0].info[1].abs_diff(packed[1].info[1]) <= 1);
    }

    #[test]
    fn burst_encoding() {
        let b = ParticleBurst::new(ParticleKind::Dust, [1.0, 2.0, 3.0], 4).ground_color().toward([0.0, 1.0, 0.0], std::f32::consts::PI).collide(false);
        let g = GpuBurst::new(&b, 7, 4);
        assert_eq!(g.info, [7, 4, ParticleKind::Dust as u32 | EMIT_GROUND_COLOR << 8, 0]);
        assert!((g.dir[3] + 1.0).abs() < 1e-6);
        assert!(ParticleKind::Fire.additive() && !ParticleKind::Dust.additive());
        // A speck's flag reaches the particle as the shaders' FLAG_SPECK.
        let speck = GpuBurst::new(&ParticleBurst::new(ParticleKind::Ember, [0.0; 3], 1).speck(), 0, 1);
        assert_eq!(speck.info[2], ParticleKind::Ember as u32 | (EMIT_COLLIDE | EMIT_SPECK) << 8);
        assert!(include_str!("shaders/particle_common.wgsl").contains(&format!("const FLAG_SPECK: u32 = {EMIT_SPECK}u;")));
        // Invalid bursts are dropped before they reach the GPU.
        assert!(!ParticleBurst::new(ParticleKind::Spark, [f32::NAN, 0.0, 0.0], 3).is_valid());
    }

    /// The structs shared with the GPU match their WGSL layouts.
    #[test]
    fn gpu_structs_match_wgsl_layouts() {
        use wgpu::naga;
        let source = sim_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).expect("layout");
        let size = |name: &str| {
            let (handle, _) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no type {name}"));
            layouter[handle].size as usize
        };
        assert_eq!(size("Particle"), std::mem::size_of::<GpuParticle>());
        assert_eq!(size("ParticleParams"), std::mem::size_of::<GpuParams>());
        assert_eq!(size("Burst"), std::mem::size_of::<GpuBurst>());
        assert_eq!(size("Counters"), std::mem::size_of::<GpuCounters>());
        assert_eq!(size("Indirect") as u64, INDIRECT_SIZE);
    }

    #[test]
    fn capacity_is_clamped_to_whole_workgroups() {
        let with = |capacity| capacity_for(&ParticleSettings { capacity, ..Default::default() });
        assert_eq!(with(0), MIN_CAPACITY);
        assert_eq!(with(u32::MAX), MAX_CAPACITY);
        assert_eq!(with(100_001) % WORKGROUP, 0);
        assert_eq!(with(DEFAULT_CAPACITY), DEFAULT_CAPACITY);
    }

    #[test]
    fn billows_vanish_at_their_borders() {
        let pixels = billow_pixels();
        let size = (BILLOW * 2) as usize;
        for i in 0..size {
            for (x, y) in [(i, 0), (0, i), (i, size - 1), (size - 1, i), (i, size / 2), (size / 2, i)] {
                assert!(pixels[y * size + x] < 8, "texel ({x}, {y}) = {}", pixels[y * size + x]);
            }
        }
        // ...and are dense in the middle.
        let centre = pixels[(size / 4) * size + size / 4];
        assert!(centre > 60, "centre {centre}");
    }
}
