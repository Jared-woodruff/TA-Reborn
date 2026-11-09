// Procedural noise shared by the water, terrain, extended ground and sky shaders. No textures
// or bindings are used.

const TAU: f32 = 6.2831853;

// Integer hash (PCG output permutation) -> 32 random bits.
fn hash_u32(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
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
    let ga = lattice_gradient(i);
    let gb = lattice_gradient(i + vec2<i32>(1, 0));
    let gc = lattice_gradient(i + vec2<i32>(0, 1));
    let gd = lattice_gradient(i + vec2<i32>(1, 1));
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
