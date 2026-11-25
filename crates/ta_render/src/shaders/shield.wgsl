// Shields (see shields.rs): a dome round its unit, an ellipsoid of its radius across and its
// height, in its owner's colour: faint glass, its rim brighter where the eye grazes it, a slow
// shimmer of bands drifting up it, rings running out from each of its latest hits over its face,
// and the whole of it flashing as it collapses and comes back up. A personal shield is a thinner,
// fainter shell. Unlit; premultiplied over the picture, its brightest parts over 1 so that they
// bloom.

struct VIn {
    @builtin(vertex_index) vi: u32,
    // xyz: its middle, w: its radius across
    @location(0) centre: vec4<f32>,
    // x: its height, y: flash 0..1, z: its hit points' share, w: 1 a personal shield's shell
    @location(1) shape: vec4<f32>,
    // rgb: linear colour, a: opacity
    @location(2) color: vec4<f32>,
    // Its latest hits: xyz the way from its middle (its round shape), w seconds since (< 0 none)
    @location(3) hit0: vec4<f32>,
    @location(4) hit1: vec4<f32>,
    @location(5) hit2: vec4<f32>,
    @location(6) hit3: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // The point on the round dome (a unit ball), its normal in the world, and where it is.
    @location(0) round: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) shape: vec4<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
    @location(5) @interpolate(flat) hit0: vec4<f32>,
    @location(6) @interpolate(flat) hit1: vec4<f32>,
    @location(7) @interpolate(flat) hit2: vec4<f32>,
    @location(8) @interpolate(flat) hit3: vec4<f32>,
};

// Sides round it and bands down it.
const AROUND: u32 = 32u;
const DOWN: u32 = 16u;
const PI: f32 = 3.14159265;

@vertex
fn vs_shield(in: VIn) -> VOut {
    // Each piece of it a quad: one corner of it, a step round (x) and down (y).
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    let c = corners[in.vi % 6u];
    let quad = in.vi / 6u;
    let around = (f32(quad % AROUND) + c.x) / f32(AROUND) * 2.0 * PI;
    let down = (f32(quad / AROUND) + c.y) / f32(DOWN) * PI;
    let round = vec3<f32>(sin(down) * cos(around), cos(down), sin(down) * sin(around));
    let size = vec3<f32>(in.centre.w, in.shape.x, in.centre.w);
    let world = in.centre.xyz + round * size;
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.round = round;
    // (An ellipsoid's normal: the ball's, squeezed the other way.)
    out.normal = round / max(size, vec3<f32>(1.0));
    out.world = world;
    out.shape = in.shape;
    out.color = in.color;
    out.hit0 = in.hit0;
    out.hit1 = in.hit1;
    out.hit2 = in.hit2;
    out.hit3 = in.hit3;
    return out;
}

// A hit's ring running out over the dome from where it struck, and its first glare there:
// `d` how far round the dome (radians) the point is from it, `age` seconds since.
fn ripple(d: f32, age: f32) -> f32 {
    if (age < 0.0 || age > 0.9) {
        return 0.0;
    }
    let fade = 1.0 - age / 0.9;
    // (A ring running out from it, a quarter of the way round the dome at the most, thinning.)
    let front = age * 0.9;
    let ring = exp(-pow((d - front) / 0.06, 2.0)) * fade * fade * 0.8;
    let glare = exp(-pow(d / 0.12, 2.0)) * max(0.0, 1.0 - age * 2.5);
    return ring + glare * 2.0;
}

fn hit_at(round: vec3<f32>, hit: vec4<f32>) -> f32 {
    if (hit.w < 0.0) {
        return 0.0;
    }
    let d = acos(clamp(dot(normalize(round), normalize(hit.xyz)), -1.0, 1.0));
    return ripple(d, hit.w);
}

@fragment
fn fs_shield(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = view_dir(in.world);
    let facing = abs(dot(n, v));
    // Glass: faint face on, brighter where the eye grazes it.
    let rim = pow(1.0 - facing, 3.0);
    let personal = in.shape.w > 0.5;
    // (The far side seen through the near one, fainter.)
    let side = select(0.55, 1.0, front);
    // A slow shimmer of bands drifting up it.
    let time = frame.world.x;
    let bands = 0.5 + 0.5 * sin(in.round.y * 22.0 - time * 1.7 + atan2(in.round.z, in.round.x) * 3.0);
    var glow = (0.10 + 0.90 * rim) * (0.85 + 0.15 * bands);
    if (personal) {
        glow = rim * 0.6;
    }
    let ripples = hit_at(in.round, in.hit0) + hit_at(in.round, in.hit1) + hit_at(in.round, in.hit2) + hit_at(in.round, in.hit3);
    let flash = in.shape.y;
    // (Weaker as its hit points run down.)
    let strength = 0.55 + 0.45 * clamp(in.shape.z, 0.0, 1.0);
    let base = in.color.a * glow * strength * side;
    let amount = base + ripples * 0.45 + flash * (0.35 + 0.65 * rim);
    // Brighter than white where it is hit and as it flashes: those parts bloom.
    let light = in.color.rgb * (1.0 + ripples * 2.5 + flash * 3.0) + vec3<f32>(ripples * 0.25 + flash * 0.4);
    let alpha = clamp(amount, 0.0, 1.0);
    if (alpha < 0.003) {
        discard;
    }
    return vec4<f32>(light * alpha, alpha * 0.55);
}
