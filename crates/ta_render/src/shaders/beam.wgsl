// Laser beams and tracer lines: camera-facing ribbons, additive HDR with a hot core.

struct VIn {
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec4<f32>,      // xyz start, w = width
    @location(1) b: vec4<f32>,      // xyz end, w = core fraction
    @location(2) color: vec4<f32>,  // rgb HDR colour, a = alpha
    @location(3) shape: vec4<f32>,  // x = head rounding (world units), y = tail fade (share of the length), z = cover
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) across: f32,
    @location(1) color: vec4<f32>,
    @location(2) core: f32,
    // World position (the reflection pass keeps what the water shows).
    @location(3) world: vec3<f32>,
    // Along the beam from its head (0) to its tail (1); its length; how its ends are drawn.
    @location(4) along: f32,
    @location(5) @interpolate(flat) length: f32,
    @location(6) @interpolate(flat) shape: vec4<f32>,
};

@vertex
fn vs_beam(in: VIn) -> VOut {
    // Quad corners: t along the beam (0/1), s across (-1/1).
    let ts = array<vec2<f32>, 6>(vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    let c = ts[in.vi % 6u];
    let p = mix(in.a.xyz, in.b.xyz, c.x);
    let dir = in.b.xyz - in.a.xyz;
    let to_cam = normalize(frame.camera_pos.xyz - p);
    var side = cross(dir, to_cam);
    if (length(side) < 1e-4) {
        side = vec3<f32>(1.0, 0.0, 0.0);
    }
    side = normalize(side);
    let world = p + side * c.y * in.a.w * 0.5;
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.across = c.y;
    out.color = in.color;
    out.core = in.b.w;
    out.world = world;
    out.along = c.x;
    out.length = length(dir);
    out.shape = in.shape;
    return out;
}

@fragment
fn fs_beam(in: VOut) -> @location(0) vec4<f32> {
    return beam_color(in);
}

// Beams mirrored in the water (the reflection pass, see reflection.rs): what the water shows.
@fragment
fn fs_beam_reflection(in: VOut) -> @location(0) vec4<f32> {
    if (!seen_in_water(in.world)) {
        discard;
    }
    return beam_color(in);
}

fn beam_color(in: VOut) -> vec4<f32> {
    // A bolt's head rounds off and its tail narrows and fades out (see `BeamInstance::shape`),
    // so it does not end in square cuts.
    var half = 1.0;
    var fade = 1.0;
    if (in.shape.x > 0.0) {
        let k = 1.0 - saturate(in.along * in.length / in.shape.x);
        half = sqrt(max(1.0 - k * k, 0.0));
    }
    if (in.shape.y > 0.0) {
        let k = saturate((1.0 - in.along) / in.shape.y);
        fade = k * k * (3.0 - 2.0 * k);
        half *= mix(0.35, 1.0, k);
    }
    let d = abs(in.across) / max(half, 1e-3);
    if (d >= 1.0) {
        return vec4<f32>(0.0);
    }
    let glow = pow(1.0 - d, 2.0) * fade;
    let core = smoothstep(in.core, 0.0, d) * fade;
    let c = in.color.rgb * glow + vec3<f32>(1.0) * core * 2.5 * in.color.a;
    // (Out of sight, light dims as the particles' does.)
    return vec4<f32>(c * in.color.a * los_light(in.world), saturate(max(glow, core)) * in.shape.z * in.color.a);
}
