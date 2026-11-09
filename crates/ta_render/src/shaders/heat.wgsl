// Heat haze sources (see heat.rs): soft columns of hot air rising from explosions and fires.
// They add to the alpha channel of the HDR scene target (the colour is left alone) and are
// depth tested against the scene; the composite (post.wgsl) turns that heat into a drifting
// distortion. Prepended: common.wgsl.

struct VIn {
    @builtin(vertex_index) vi: u32,
    // xyz = the point the hot air rises from, w = radius
    @location(0) pos_radius: vec4<f32>,
    // x = strength, yzw = unused
    @location(1) params: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // Position on the column in radii: x across (-1..1), y up from the source.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) amount: f32,
};

// Extent of a column, in radii: from a little below its source to well above it.
const COLUMN_BOTTOM: f32 = -0.5;
const COLUMN_TOP: f32 = 2.4;

@vertex
fn vs_heat(in: VIn) -> VOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[in.vi % 6u];
    let r = in.pos_radius.w;
    let source = in.pos_radius.xyz;
    let y = mix(COLUMN_BOTTOM, COLUMN_TOP, c.y);
    // Facing the camera and rising up the screen, as hot air does.
    let right = normalize(frame.billboard_right.xyz);
    let up = normalize(frame.billboard_up.xyz);
    let world = source + (right * c.x + up * y) * r;
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    // The whole column at the depth of its source (nudged towards the camera), like the
    // sprites: the ground behind the source shimmers, units in front of it do not, and the
    // column is never cut by the ground it rises from.
    let to_cam = normalize(frame.camera_pos.xyz - source);
    let anchor = frame.view_proj * vec4<f32>(source + to_cam * (r * 0.5), 1.0);
    out.clip.z = anchor.z / anchor.w * out.clip.w;
    out.local = vec2<f32>(c.x, y);
    // The distortion is measured in screen pixels: far fires shimmer less.
    let dist = distance(frame.camera_pos.xyz, source);
    out.amount = in.params.x * frame.heat.x * min(1.0, 900.0 / max(dist, 1.0));
    return out;
}

@fragment
fn fs_heat(in: VOut) -> @location(0) vec4<f32> {
    let y = in.local.y;
    // Narrowing as it rises, soft at the sides, fading in at the bottom and out at the top.
    let width = mix(1.0, 0.6, saturate(y / COLUMN_TOP));
    let across = 1.0 - smoothstep(0.3, 1.0, abs(in.local.x) / width);
    let along = smoothstep(COLUMN_BOTTOM, 0.35, y) * (1.0 - smoothstep(1.0, COLUMN_TOP, y));
    return vec4<f32>(0.0, 0.0, 0.0, in.amount * across * along);
}
