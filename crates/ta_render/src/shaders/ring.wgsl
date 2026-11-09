// Rings on the ground (a selected unit's): lying round the unit as a decal would, draped over the
// ground and kept over the water's surface (see rings.rs), so that the unit standing in it, its
// neighbours and the hills hide what they stand in front of; a line of so many pixels whatever
// the zoom, in its colour, unlit, edged dark to read on pale ground.

struct VIn {
    @builtin(vertex_index) vi: u32,
    // xz: the middle, w: the radius (world units)
    @location(0) centre: vec4<f32>,
    // rgb: linear colour, a: opacity
    @location(1) color: vec4<f32>,
    // x: the lowest the ring lies (world y: a ship's, the water's surface), y: the coloured
    // line's width, z: the dark edge's (pixels)
    @location(2) shape: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // Pixels from the ring's middle line, across it.
    @location(0) across: f32,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) widths: vec2<f32>,
};

// Sides round a ring, and how far over the ground it lies (world units).
const SEGMENTS: u32 = 96u;
const LIFT: f32 = 2.5;

fn ring_point(centre: vec4<f32>, lowest: f32, a: f32) -> vec3<f32> {
    let xz = centre.xz + vec2<f32>(cos(a), sin(a)) * centre.w;
    return vec3<f32>(xz.x, max(ground_height(xz), lowest) + LIFT, xz.y);
}

// A clip-space point in pixels from the middle of the screen.
fn ring_pixels(p: vec4<f32>) -> vec2<f32> {
    return p.xy / p.w * 0.5 * frame.screen.xy;
}

@vertex
fn vs_ring(in: VIn) -> VOut {
    // Each side a quad: x along the ring (this corner's end of the side), y across it.
    let corners = array<vec2<f32>, 6>(vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    let c = corners[in.vi % 6u];
    let a = (f32(in.vi / 6u) + c.x) / f32(SEGMENTS) * 6.2831853;
    let here = frame.view_proj * vec4<f32>(ring_point(in.centre, in.shape.x, a), 1.0);
    let ahead = frame.view_proj * vec4<f32>(ring_point(in.centre, in.shape.x, a + 0.01), 1.0);
    // Out across the line as the screen shows it, as wide as its dark edge and a pixel more
    // (to smooth its sides), at the depth of its middle.
    var along = ring_pixels(ahead) - ring_pixels(here);
    if (dot(along, along) < 1e-8) {
        along = vec2<f32>(1.0, 0.0);
    }
    let normal = normalize(vec2<f32>(-along.y, along.x));
    let half = in.shape.z * 0.5 + 1.0;
    var out: VOut;
    out.clip = here + vec4<f32>(normal * (c.y * half) / (0.5 * frame.screen.xy) * here.w, 0.0, 0.0);
    out.across = c.y * half;
    out.color = in.color;
    out.widths = in.shape.yz;
    return out;
}

@fragment
fn fs_ring(in: VOut) -> @location(0) vec4<f32> {
    // The coloured line over its dark edge, each smoothed over a pixel (premultiplied).
    let d = abs(in.across);
    let core = clamp(in.widths.x * 0.5 + 0.5 - d, 0.0, 1.0);
    let edge = clamp(in.widths.y * 0.5 + 0.5 - d, 0.0, 1.0) * 0.4;
    let alpha = (core + edge * (1.0 - core)) * in.color.a;
    // (Nothing past its edge: its depth is kept, and the water would not be drawn there.)
    if (alpha < 0.02) {
        discard;
    }
    return vec4<f32>(in.color.rgb * core * in.color.a, alpha);
}
