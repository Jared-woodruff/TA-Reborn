// Declarations shared by the GPU particle compute and draw shaders (see gpu_particles.rs).
// No bindings: each module binds its own.

// One particle (48 bytes; mirrors `GpuParticle`).
struct Particle {
    pos: vec3<f32>,
    // Seconds since birth (negative while waiting to be born).
    age: f32,
    vel: vec3<f32>,
    // Lifetime (s).
    life: f32,
    // x = diameters at birth and at death (2 x f16), y = gravity and drag (2 x f16),
    // z = colour and opacity (4 x unorm8),
    // w = kind (bits 0-3) | flags (4-7) | random seed (8-15) | brightness (f16, 16-31)
    data: vec4<u32>,
};

// This frame's parameters (mirrors `GpuParams`).
struct ParticleParams {
    // x = time step (s; 0 = paused), y = particle clock (s), z = height of the sea or lava
    // surface (far below the map without one), w = height over which alpha-blended particles
    // fade out towards the ground (0 = off)
    step: vec4<f32>,
    // x = pool capacity, y = particles to emit this frame, z = bursts this frame, w = random seed
    counts: vec4<u32>,
    // x = the alive list simulated this frame (0 or 1; the draws read the other one), yzw = unused
    state: vec4<u32>,
    // xy = wind (world units / s along x and z), z = reduce flashes, w = unused
    wind: vec4<f32>,
};

// Kinds (`ParticleKind`). The first four, the fireball and rain are additive (light, or light
// with what they hide behind them), the rest alpha-blended.
const KIND_SPARK: u32 = 0u;
const KIND_EMBER: u32 = 1u;
const KIND_GLOW: u32 = 2u;
const KIND_FIRE: u32 = 3u;
const KIND_DUST: u32 = 4u;
const KIND_SMOKE: u32 = 5u;
const KIND_DIRT: u32 = 6u;
const KIND_SPRAY: u32 = 7u;
const KIND_FIREBALL: u32 = 8u;
const KIND_RAIN: u32 = 9u;

// Particle flags: collides with the ground and liquids; came to rest on the ground; keeps its
// colour instead of cooling (sparks of electricity and energy beams); kept a speck however near
// the eye (ambient motes: the embers over lava).
const FLAG_COLLIDE: u32 = 1u;
const FLAG_RESTING: u32 = 2u;
const FLAG_COLD: u32 = 4u;
const FLAG_SPECK: u32 = 8u;

fn particle_kind(p: Particle) -> u32 {
    return p.data.w & 15u;
}

fn particle_flags(p: Particle) -> u32 {
    return (p.data.w >> 4u) & 15u;
}

fn particle_seed(p: Particle) -> u32 {
    return (p.data.w >> 8u) & 255u;
}

fn particle_brightness(p: Particle) -> f32 {
    return unpack2x16float(p.data.w >> 16u).x;
}

// Additive kinds are drawn with the additive effects, the others with the smoke.
fn kind_additive(kind: u32) -> bool {
    return kind <= KIND_FIRE || kind == KIND_FIREBALL || kind == KIND_RAIN;
}

// A number in 0..1 of the particle's own (`salt` picks which), so that particles of one burst
// differ in size, shape and shade.
fn particle_random(seed: u32, salt: u32) -> f32 {
    var h = ((seed + 1u) * 0x9E3779B1u) ^ (salt * 0x85EBCA77u);
    h ^= h >> 15u;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12u;
    return f32(h & 0xFFFFu) / 65535.0;
}
