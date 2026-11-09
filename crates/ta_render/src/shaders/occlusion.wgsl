// Contact occlusion map (see occlusion.rs): soft footprints of units, buildings, wrecks and
// trees drawn top-down into a world-space R8 map around the view, multiplied together (the map
// is cleared to 1 = open ground). Standalone: no frame bindings.

struct OcclusionParams {
    // xy = world xz of the map's minimum corner, z = 1 / its size (world units), w = unused
    region: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: OcclusionParams;
// Footprint masks of the models (see `footprint_mask` in occlusion.rs): 1 = full occlusion.
@group(0) @binding(1) var masks: texture_2d<f32>;
@group(0) @binding(2) var mask_sampler: sampler;

struct VIn {
    @builtin(vertex_index) vi: u32,
    // World xz of the footprint's centre, the unit direction of its first axis, its half
    // extents along and across that axis, and (falloff width, strength).
    @location(0) center: vec2<f32>,
    @location(1) axis: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) shape: vec2<f32>,
    // The footprint's mask in the atlas: xy = uv origin, zw = uv size (0 = a rounded box).
    @location(4) mask: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // Position relative to the footprint's centre, in its own axes (world units).
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half_size: vec2<f32>,
    @location(2) @interpolate(flat) shape: vec2<f32>,
    @location(3) @interpolate(flat) mask: vec4<f32>,
};

// How far beyond the footprint the occlusion reaches, in falloff widths (REACH in occlusion.rs).
const REACH: f32 = 1.6;

@vertex
fn vs_occluder(in: VIn) -> VOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    // A mask covers its own square (falloff included); a rounded box needs room around it.
    var extent = in.half_size;
    if (in.mask.z <= 0.0) {
        extent += vec2<f32>(in.shape.x * REACH);
    }
    let local = corners[in.vi % 6u] * extent;
    let across = vec2<f32>(-in.axis.y, in.axis.x);
    let world = in.center + in.axis * local.x + across * local.y;
    // Map texel rows run towards +z (texture v down = world south).
    let uv = (world - params.region.xy) * params.region.z;
    var out: VOut;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.half_size = in.half_size;
    out.shape = in.shape;
    out.mask = in.mask;
    return out;
}

@fragment
fn fs_occluder(in: VOut) -> @location(0) vec4<f32> {
    if (in.mask.z > 0.0) {
        // Half a texel in from the tile's border, so its neighbours never bleed in.
        let texel = in.mask.zw / 32.0;
        let uv = clamp(in.local / in.half_size * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
        let at = clamp(in.mask.xy + uv * in.mask.zw, in.mask.xy + texel * 0.5, in.mask.xy + in.mask.zw - texel * 0.5);
        let occlusion = in.shape.y * textureSampleLevel(masks, mask_sampler, at, 0.0).r;
        return vec4<f32>(1.0 - occlusion, 0.0, 0.0, 1.0);
    }
    let w = in.shape.x;
    // Signed distance to the footprint, a box with rounded corners.
    let r = min(in.half_size.x, in.half_size.y) * 0.5;
    let q = abs(in.local) - (in.half_size - vec2<f32>(r));
    let sd = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    // A contact shadow, not a sky-occlusion tail: full under the footprint, still almost full
    // at its edge, a quarter one falloff width out, gone at REACH widths (overlapping tails of
    // many occluders would grey whole areas).
    let occlusion = in.shape.y * (1.0 - smoothstep(-0.25 * w, REACH * w, sd));
    return vec4<f32>(1.0 - occlusion, 0.0, 0.0, 1.0);
}
