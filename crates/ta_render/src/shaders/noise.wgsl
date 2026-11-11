// Procedural noise shared by the water, terrain, extended ground and sky shaders. No textures
// or bindings are used.

const TAU: f32 = 6.2831853;

// Integer hash (PCG output permutation) -> 32 random bits.
fn hash_u32(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

// `hash_u32` of four values at once, lane by lane.
fn hash_u32x4(x: vec4<u32>) -> vec4<u32> {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> vec4<u32>(28u)) + 4u)) ^ h) * 277803737u;
    return (h >> vec4<u32>(22u)) ^ h;
}

// Random gradient in [-1, 1]² for an integer lattice point.
fn lattice_gradient(i: vec2<i32>) -> vec2<f32> {
    let h = hash_u32(bitcast<u32>(i.x) ^ hash_u32(bitcast<u32>(i.y) + 0x9E3779B9u));
    return vec2<f32>(f32(h & 0xFFFFu), f32(h >> 16u)) * (2.0 / 65535.0) - 1.0;
}

// 2D gradient noise with a quintic fade and analytic derivatives: x = value (std ~0.18,
// range about ±0.75), yz = gradient with respect to `p` (variance ~0.26 per axis).
fn noise_d(p: vec2<f32>) -> vec3<f32> {
    let fl = floor(p);
    let i = vec2<i32>(fl);
    let f = p - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    // (The four corners' `lattice_gradient`s, (0, 0), (1, 0), (0, 1) and (1, 1), in one pass: the
    // same bits, in a quarter of the code. Every shader inlines noise wherever it is called, and
    // Direct3D's shader compilers took minutes over the terrain's hundreds of calls.)
    let x = bitcast<vec4<u32>>(vec4<i32>(i.x, i.x + 1, i.x, i.x + 1));
    let y = bitcast<vec4<u32>>(vec4<i32>(i.y, i.y, i.y + 1, i.y + 1));
    let h = hash_u32x4(x ^ hash_u32x4(y + 0x9E3779B9u));
    let gx = vec4<f32>(h & vec4<u32>(0xFFFFu)) * (2.0 / 65535.0) - 1.0;
    let gy = vec4<f32>(h >> vec4<u32>(16u)) * (2.0 / 65535.0) - 1.0;
    let ga = vec2<f32>(gx.x, gy.x);
    let gb = vec2<f32>(gx.y, gy.y);
    let gc = vec2<f32>(gx.z, gy.z);
    let gd = vec2<f32>(gx.w, gy.w);
    let va = dot(ga, f);
    let vb = dot(gb, f - vec2<f32>(1.0, 0.0));
    let vc = dot(gc, f - vec2<f32>(0.0, 1.0));
    let vd = dot(gd, f - vec2<f32>(1.0, 1.0));
    let k = va - vb - vc + vd;
    let value = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    let grad = ga + u.x * (gb - ga) + u.y * (gc - ga) + u.x * u.y * (ga - gb - gc + gd)
        + du * (u.yx * k + vec2<f32>(vb, vc) - va);
    return vec3<f32>(value, grad);
}

// Value of the gradient noise only (the compiler drops the unused derivative terms).
fn noise(p: vec2<f32>) -> f32 {
    return noise_d(p).x;
}

// `start` plus `noise` at the first `count` of `points`, each times its `weights`, in their
// order: the sum as written out term by term, with `noise` written once. (Every shader inlines
// every call, and Direct3D's shader compilers slow down a great deal over copies of noise
// written out octave by octave.)
fn noise_sum(points: array<vec2<f32>, 4>, weights: vec4<f32>, count: u32, start: f32) -> f32 {
    var sum = start;
    for (var k = 0u; k < count; k++) {
        sum += weights[k] * noise(points[k]);
    }
    return sum;
}

// `noise` at the first `count` of `points`, in a loop (see `noise_sum`).
fn noise_at(points: array<vec2<f32>, 4>, count: u32) -> vec4<f32> {
    var n = vec4<f32>(0.0);
    for (var k = 0u; k < count; k++) {
        n = select(n, vec4<f32>(noise(points[k])), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(k));
    }
    return n;
}
