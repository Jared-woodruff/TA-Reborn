// GPU particles, compute side (see gpu_particles.rs): emission from this frame's bursts and the
// simulation (gravity, drag through the moving air, swirling, bounces off the terrain). The
// particles live in two buffers used in turn: each frame the simulation reads one and packs the
// survivors into the other, additive particles from its front and alpha-blended ones from its
// back, which the draws then read in order.
// Prepended: noise.wgsl, particle_common.wgsl.

// One burst to emit this frame (mirrors `GpuBurst`).
struct Burst {
    // xyz = centre of the emission volume, w = its radius
    pos: vec4<f32>,
    // xyz = direction of flight (zero = outward from the centre), w = cosine of the half angle
    // of the cone of directions around it
    dir: vec4<f32>,
    // xyz = velocity added to every particle, w = vertical scale of the emission volume
    vel: vec4<f32>,
    // xy = speed range, zw = lifetime range
    speed_life: vec4<f32>,
    // xy = diameters at birth and at death, z = gravity, w = drag
    size_phys: vec4<f32>,
    // rgb = colour, a = brightness (additive kinds) or opacity (alpha-blended kinds)
    color: vec4<f32>,
    // x = index of its first particle among this frame's, y = count, z = kind | flags << 8,
    // w = unused
    info: vec4<u32>,
    // x = seconds the births are spread over, y = seconds before the first, zw = unused
    extra: vec4<f32>,
};

// Pool bookkeeping (mirrors `GpuCounters`).
struct Counters {
    // Particles emitted this frame.
    emitted: atomic<u32>,
    // After the last simulation: free room, the pool's pressure (0..1), live particles and the
    // particles emitted in that frame.
    free: u32,
    pressure: f32,
    live: u32,
    last_emitted: u32,
    // Surviving flames/fireballs hide the background, so their additive layer needs sorting.
    covered: atomic<u32>,
    pad1: u32,
    pad2: u32,
    // Per buffer: particles at its front (additive) and back (alpha-blended), and both (checked
    // against the capacity while emitting).
    front: array<atomic<u32>, 2>,
    back: array<atomic<u32>, 2>,
    total: array<atomic<u32>, 2>,
    pad3: u32,
    pad4: u32,
};

// Indirect arguments (read by `dispatch_workgroups_indirect` and `draw_indexed_indirect`).
struct Indirect {
    // The simulation's workgroups (x, y, z; w unused).
    dispatch: vec4<u32>,
    // The two draws: index count, instance count, first index, base vertex, first instance
    // (padded to 8 words).
    additive: array<u32, 8>,
    alpha: array<u32, 8>,
    // Sorting: blocks of 64, then merges up to the largest supported pool (1 << 20).
    sorting: array<vec4<u32>, 15>,
};

// Burst flags (`info.z >> 8`): bits 0, 2 and 3 are the particle's FLAG_COLLIDE, FLAG_COLD and
// FLAG_SPECK.
const EMIT_GROUND_COLOR: u32 = 2u;

@group(0) @binding(0) var<uniform> params: ParticleParams;
// This frame's particles (emission appends to them) and the survivors' buffer.
@group(0) @binding(1) var<storage, read_write> particles_in: array<Particle>;
@group(0) @binding(2) var<storage, read_write> particles_out: array<Particle>;
@group(0) @binding(3) var<storage, read_write> counters: Counters;
@group(0) @binding(4) var<storage, read> bursts: array<Burst>;
// The map's heightmap (r = height / 255 per 16-unit cell) and the mean painted colour of each
// 32 px tile (1x1 stand-ins before a map is loaded).
@group(0) @binding(5) var heights: texture_2d<f32>;
@group(0) @binding(6) var ground_colors: texture_2d<f32>;
@group(0) @binding(7) var linear_clamp: sampler;
@group(0) @binding(8) var<storage, read_write> indirect: Indirect;

// ---- Random numbers ------------------------------------------------------------------------
//
// Every particle draws from its own stream, seeded by its index among the frame's emissions,
// so the result does not depend on the order threads run in.

fn rand(state: ptr<function, u32>) -> f32 {
    *state = hash_u32(*state);
    return f32(*state >> 8u) * (1.0 / 16777216.0);
}

// Uniformly distributed on the unit sphere.
fn rand_unit(state: ptr<function, u32>) -> vec3<f32> {
    let z = rand(state) * 2.0 - 1.0;
    let a = rand(state) * TAU;
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(a), z, r * sin(a));
}

// Uniformly distributed over the cone of directions within acos(cos_max) of the unit `axis`.
fn rand_cone(state: ptr<function, u32>, axis: vec3<f32>, cos_max: f32) -> vec3<f32> {
    let c = mix(1.0, cos_max, rand(state));
    let s = sqrt(max(1.0 - c * c, 0.0));
    let a = rand(state) * TAU;
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(axis.x) > 0.7);
    let u = normalize(cross(axis, helper));
    let v = cross(axis, u);
    return axis * c + (u * cos(a) + v * sin(a)) * s;
}

// ---- The ground ----------------------------------------------------------------------------

// Ground height (bilinear heightmap) at world xz; clamped at the map edges.
fn terrain_height(xz: vec2<f32>) -> f32 {
    let uv = (xz / 16.0 + 0.5) / vec2<f32>(textureDimensions(heights));
    return textureSampleLevel(heights, linear_clamp, uv, 0.0).r * 255.0;
}

// Unit normal of the ground at world xz.
fn terrain_normal(xz: vec2<f32>) -> vec3<f32> {
    let e = 8.0;
    let dx = terrain_height(xz + vec2<f32>(e, 0.0)) - terrain_height(xz - vec2<f32>(e, 0.0));
    let dz = terrain_height(xz + vec2<f32>(0.0, e)) - terrain_height(xz - vec2<f32>(0.0, e));
    return normalize(vec3<f32>(-dx, 2.0 * e, -dz));
}

// Painted colour (linear) of the ground at world xz, `height` high: the map art shows ground
// point (x, h, z) at art pixel (x, z - h / 2) (FORMATS.md), averaged per 32 px tile.
fn painted_ground(xz: vec2<f32>, height: f32) -> vec3<f32> {
    let art = vec2<f32>(xz.x, xz.y - height * 0.5);
    let uv = art / (32.0 * vec2<f32>(textureDimensions(ground_colors)));
    return textureSampleLevel(ground_colors, linear_clamp, uv, 0.0).rgb;
}

// ---- Emission ------------------------------------------------------------------------------

@compute @workgroup_size(64)
fn cs_emit(@builtin(global_invocation_id) id: vec3<u32>) {
    let g = id.x;
    if (g >= params.counts.y) {
        return;
    }
    // This particle's burst: the last one whose first particle is at or before it.
    var lo = 0u;
    var hi = params.counts.z;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (bursts[mid].info.x <= g) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let b = bursts[lo];
    var rng = hash_u32(g ^ params.counts.w);
    // Thin out evenly while the pool is more than half full, and never ask for more than it has
    // room for: every burst keeps the same share.
    let cap = params.counts.x;
    let free = f32(counters.free);
    let keep = min(free / (0.5 * f32(cap)), free / max(f32(params.counts.y), 1.0));
    if (rand(&rng) >= keep) {
        return;
    }
    let list = params.state.x;
    if (atomicAdd(&counters.total[list], 1u) >= cap) {
        return;
    }
    let kind = b.info.z & 255u;
    let flags = (b.info.z >> 8u) & 255u;
    // A random point of the emission volume: a ball, squashed vertically.
    let out = rand_unit(&rng);
    var offset = out * (b.pos.w * pow(rand(&rng), 1.0 / 3.0));
    offset.y *= b.vel.w;
    var pos = b.pos.xyz + offset;
    // A direction within the cone around the burst's direction, or around the way out from its
    // centre (squashed like the volume: a flat burst spreads along the ground).
    var axis = b.dir.xyz;
    if (dot(axis, axis) < 1e-8) {
        let flat = vec3<f32>(out.x, out.y * b.vel.w, out.z);
        axis = select(vec3<f32>(0.0, 1.0, 0.0), flat, dot(flat, flat) > 1e-6);
    }
    let dir = rand_cone(&rng, normalize(axis), b.dir.w);
    let vel = dir * mix(b.speed_life.x, b.speed_life.y, rand(&rng)) + b.vel.xyz;
    let life = max(mix(b.speed_life.z, b.speed_life.w, rand(&rng)), 0.02);
    let birth = b.extra.y + b.extra.x * rand(&rng);
    let ground = terrain_height(pos.xz);
    var color = b.color.rgb;
    if ((flags & EMIT_GROUND_COLOR) != 0u) {
        // The ground's colour, greyer and pulled towards a mid tone: dust stands out a little
        // against the very ground it comes from, light ground or dark.
        let painted = painted_ground(pos.xz, ground);
        let grey = mix(painted, vec3<f32>(dot(painted, vec3<f32>(0.2126, 0.7152, 0.0722))), 0.35);
        color *= mix(grey, vec3<f32>(0.42), 0.3);
    }
    if ((flags & FLAG_COLLIDE) != 0u) {
        pos.y = max(pos.y, max(ground, params.step.z) + 0.5);
    }
    let additive = kind_additive(kind);
    let opacity = select(saturate(b.color.a), 1.0, additive);
    let brightness = select(1.0, b.color.a, additive);
    var p: Particle;
    p.pos = pos;
    p.age = -birth;
    p.vel = vel;
    p.life = life;
    p.data = vec4<u32>(
        pack2x16float(b.size_phys.xy),
        pack2x16float(b.size_phys.zw),
        pack4x8unorm(vec4<f32>(saturate(color), opacity)),
        kind | ((flags & (FLAG_COLLIDE | FLAG_COLD | FLAG_SPECK)) << 4u) | ((hash_u32(rng) & 255u) << 8u) | (pack2x16float(vec2<f32>(brightness, 0.0)) << 16u),
    );
    if (additive) {
        particles_in[atomicAdd(&counters.front[list], 1u)] = p;
    } else {
        particles_in[cap - 1u - atomicAdd(&counters.back[list], 1u)] = p;
    }
    atomicAdd(&counters.emitted, 1u);
}

// ---- Simulation ----------------------------------------------------------------------------

// Sizes the simulation dispatch for this frame's buffer and empties the other one.
@compute @workgroup_size(1)
fn cs_plan() {
    let list = params.state.x;
    let n = atomicLoad(&counters.front[list]) + atomicLoad(&counters.back[list]);
    indirect.dispatch = vec4<u32>((n + 63u) / 64u, 1u, 1u, 0u);
    let other = 1u - list;
    atomicStore(&counters.front[other], 0u);
    atomicStore(&counters.back[other], 0u);
    atomicStore(&counters.total[other], 0u);
    atomicStore(&counters.covered, 0u);
}

// How much the wind carries each kind (0 = not at all).
fn windiness(kind: u32) -> f32 {
    if (kind == KIND_DIRT) {
        return 0.0;
    }
    if (kind == KIND_SPARK || kind == KIND_SPRAY || kind == KIND_RAIN) {
        return 0.25;
    }
    if (kind == KIND_FIREBALL) {
        return 0.4;
    }
    return 1.0;
}

// Strength (world units / s²) of the swirling air each kind is pushed around by.
fn swirl(kind: u32) -> f32 {
    if (kind == KIND_EMBER) {
        return 70.0;
    }
    if (kind == KIND_FIRE) {
        return 45.0;
    }
    if (kind == KIND_SMOKE) {
        return 18.0;
    }
    if (kind == KIND_DUST) {
        return 10.0;
    }
    return 0.0;
}

// One step of a live particle: forces, motion and collisions. A particle that dies (quenched
// in water or lava) comes back with its age at its lifetime.
fn integrate(p_in: Particle, dt: f32) -> Particle {
    var p = p_in;
    let kind = particle_kind(p);
    var flags = particle_flags(p);
    if ((flags & FLAG_RESTING) != 0u) {
        return p;
    }
    let phys = unpack2x16float(p.data.y);
    var v = p.vel;
    v.y -= phys.x * dt;
    let turbulence = swirl(kind);
    if (turbulence > 0.0) {
        // The curl of drifting gradient noise: swirls without sources or sinks, so billows and
        // embers twist around each other instead of bunching up.
        let seed = f32(particle_seed(p));
        let n = noise_d(p.pos.xz * 0.025 + vec2<f32>(seed * 0.013, params.step.y * 0.21)).yz;
        v += vec3<f32>(n.y, 0.0, -n.x) * (turbulence * dt);
    }
    // Drag relative to the moving air.
    let air = vec3<f32>(params.wind.x, 0.0, params.wind.y) * windiness(kind);
    v = air + (v - air) * exp(-phys.y * dt);
    p.pos += v * dt;
    if ((flags & FLAG_COLLIDE) != 0u) {
        let ground = terrain_height(p.pos.xz);
        let liquid = params.step.z;
        if (ground < liquid && p.pos.y < liquid) {
            // In the sea or lava: hot and heavy things are gone, billows ride on the surface.
            if (kind == KIND_DUST || kind == KIND_SMOKE || kind == KIND_FIREBALL) {
                p.pos.y = liquid;
                v.y = max(v.y, 0.0);
            } else {
                p.age = p.life;
            }
        } else if (p.pos.y < ground) {
            p.pos.y = ground;
            if (kind == KIND_SPARK || kind == KIND_DIRT || kind == KIND_EMBER) {
                // Bounce off the slope, losing most of the speed into it and some along it;
                // slow ones come to rest (and sparks lie glowing as they cool).
                let n = terrain_normal(p.pos.xz);
                let vn = dot(v, n);
                if (vn < 0.0) {
                    let bounce = select(0.25, 0.45, kind == KIND_SPARK);
                    v = (v - n * vn) * 0.7 - n * (vn * bounce);
                }
                if (dot(v, v) < 144.0) {
                    v = vec3<f32>(0.0);
                    flags |= FLAG_RESTING;
                }
            } else if (kind == KIND_SPRAY) {
                p.age = p.life;
            } else {
                // Billows and flames slide over the ground.
                v.y = max(v.y, 0.0);
            }
        }
    }
    p.vel = v;
    p.data.w = (p.data.w & ~(15u << 4u)) | (flags << 4u);
    return p;
}

// Survivors of this workgroup per side of the output buffer, and where they go in it: one
// global atomic per workgroup and side instead of one per particle.
var<workgroup> group_count: array<atomic<u32>, 2>;
var<workgroup> group_base: array<u32, 2>;
var<workgroup> group_covered: atomic<u32>;

@compute @workgroup_size(64)
fn cs_simulate(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) local: u32) {
    if (local == 0u) {
        atomicStore(&group_count[0], 0u);
        atomicStore(&group_count[1], 0u);
        atomicStore(&group_covered, 0u);
    }
    workgroupBarrier();
    let list = params.state.x;
    let other = 1u - list;
    let cap = params.counts.x;
    let front = atomicLoad(&counters.front[list]);
    let back = atomicLoad(&counters.back[list]);
    let i = id.x;
    var p: Particle;
    var side = 2u;
    var slot = 0u;
    if (i < front + back) {
        if (i < front) {
            p = particles_in[i];
        } else {
            p = particles_in[cap - 1u - (i - front)];
        }
        let dt = params.step.x;
        // A nearly full pool ages everything faster, making room for new bursts.
        p.age += dt * (1.0 + 3.0 * counters.pressure);
        if (p.age >= 0.0 && dt > 0.0) {
            p = integrate(p, dt);
        }
        if (p.age < p.life) {
            side = select(1u, 0u, kind_additive(particle_kind(p)));
            slot = atomicAdd(&group_count[side], 1u);
            let kind = particle_kind(p);
            if (kind == KIND_FIRE || kind == KIND_FIREBALL) { atomicAdd(&group_covered, 1u); }
        }
    }
    workgroupBarrier();
    if (local == 0u) {
        let additive = atomicLoad(&group_count[0]);
        let alpha = atomicLoad(&group_count[1]);
        group_base[0] = atomicAdd(&counters.front[other], additive);
        group_base[1] = atomicAdd(&counters.back[other], alpha);
        atomicAdd(&counters.total[other], additive + alpha);
        atomicAdd(&counters.covered, atomicLoad(&group_covered));
    }
    workgroupBarrier();
    if (side == 0u) {
        particles_out[group_base[0] + slot] = p;
    } else if (side == 1u) {
        particles_out[cap - 1u - (group_base[1] + slot)] = p;
    }
}

// Writes the draws of the survivors and the pool's state for the next frame.
@compute @workgroup_size(1)
fn cs_finalize() {
    let other = 1u - params.state.x;
    let additive = atomicLoad(&counters.front[other]);
    let alpha = atomicLoad(&counters.back[other]);
    // Six indices (two triangles) per particle; see `GpuParticles` for the index buffer.
    indirect.additive = array<u32, 8>(additive * 6u, 1u, 0u, 0u, 0u, 0u, 0u, 0u);
    indirect.alpha = array<u32, 8>(alpha * 6u, 1u, 0u, 0u, 0u, 0u, 0u, 0u);
    let sorted_additive = select(0u, additive, atomicLoad(&counters.covered) > 0u);
    let groups = (sorted_additive + 63u) / 64u + (alpha + 63u) / 64u;
    let largest = max(sorted_additive, alpha);
    for (var step = 0u; step < 15u; step++) {
        let needed = step == 0u || largest > (32u << step);
        indirect.sorting[step] = vec4<u32>(select(0u, groups, needed), 1u, 1u, 0u);
    }
    let live = additive + alpha;
    counters.free = params.counts.x - min(live, params.counts.x);
    counters.live = live;
    counters.pressure = saturate((f32(live) / f32(params.counts.x) - 0.7) / 0.3);
    counters.last_emitted = atomicLoad(&counters.emitted);
    atomicStore(&counters.emitted, 0u);
}
