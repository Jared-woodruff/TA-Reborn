// Ray tracing is on (see raytrace.rs): appended to common.wgsl.

// The ray-traced light of the front-most surface at each pixel: r = the sun's (or moon's), g =
// ambient occlusion, b = the brightest point light's, a = the pixel's depth.
@group(0) @binding(10) var rt_result: texture_2d<f32>;
// What the mirror ray from each pixel met (rgb, lit; a = 1 where it met something).
@group(0) @binding(11) var rt_reflections: texture_2d<f32>;
// The 3D trees' and rocks' own shadow map (the traced scene holds the models alone).
@group(0) @binding(12) var feature_shadow_map: texture_depth_2d;

// The 3D trees' and rocks' shadow at `world` (1 = lit), for a surface whose sun is traced (see
// `feature_shadow_map`): a wide percentage-closer filter, as soft as the rest.
fn rt_feature_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (frame.rt.x <= 0.0 || frame.screen.w <= 0.0) {
        return 1.0;
    }
    let clip = frame.shadow_view_proj * vec4<f32>(world + n * 1.5, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z > 1.0) {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(feature_shadow_map));
    var lit = 0.0;
    for (var i = 0; i < 16; i++) {
        lit += textureSampleCompareLevel(feature_shadow_map, shadow_sampler, uv + SHADOW_DISC[i] * 2.5 * texel, ndc.z - shadow_bias());
    }
    return mix(1.0, lit / 16.0, frame.screen.w * shadow_edge_keep(ndc.xy));
}

// Distance from the eye of a depth buffer value.
fn rt_linear_depth(z: f32) -> f32 {
    let n = frame.rt.y;
    let f = frame.rt.z;
    return n * f / max(f - z * (f - n), 1e-6);
}

// The pixel whose traced result is a fragment's at `frag` (its @builtin(position)): its own where
// the depth prepass saw this very surface there. On a silhouette edge a fragment covers some of
// a pixel's samples while the prepass saw, at its centre, what lies in front of or behind it:
// then the neighbour that saw the surface nearest the fragment's depth, so each side of the edge
// is shaded as its own side's pixels are (left to the shadow map, the ground under a unit's
// edge came out lit and its samples a dotted line of light along every track and foot). x < 0:
// none (trees, water, effects), and the shadow map and contact occlusion are used.
fn rt_pixel(frag: vec4<f32>) -> vec2<i32> {
    let pix = vec2<i32>(frag.xy);
    let here = rt_linear_depth(frag.z);
    if (abs(here - rt_linear_depth(textureLoad(rt_result, pix, 0).a)) <= 0.3 + here * 0.002) {
        return pix;
    }
    // (A wider tolerance: a neighbour sees its own point of the surface, a step down a slope
    // seen at a glance further off.)
    let last = vec2<i32>(textureDimensions(rt_result)) - 1;
    var best = vec2<i32>(-1);
    var nearest = 0.3 + here * 0.01;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let q = clamp(pix + vec2<i32>(i, j), vec2<i32>(0), last);
            let d = abs(here - rt_linear_depth(textureLoad(rt_result, q, 0).a));
            if (d < nearest && any(q != pix)) {
                nearest = d;
                best = q;
            }
        }
    }
    return best;
}

// The ray-traced result for a fragment at `frag` (its @builtin(position)), when the fragment is
// the surface it was traced for, at its pixel or one beside it (`rt_pixel`; else nothing: the
// shadow map and contact occlusion are used, for trees, water, effects). See `RtLight`.
fn rt_visibility(frag: vec4<f32>) -> RtLight {
    var r = RtLight(false, 1.0, 1.0, 1.0);
    if (frame.rt.x <= 0.0) {
        return r;
    }
    let pix = rt_pixel(frag);
    if (pix.x < 0) {
        return r;
    }
    let t = textureLoad(rt_result, pix, 0);
    return RtLight(true, t.r, t.g, t.b);
}

// What the mirror ray from a fragment at `frag` met (rgb), where it met something (a = 1) and
// the fragment is the surface it was traced from (`rt_pixel`); else a = 0 (the shader's own sky
// and ground).
fn rt_reflection(frag: vec4<f32>) -> vec4<f32> {
    if (frame.rt.x <= 0.0) {
        return vec4<f32>(0.0);
    }
    let pix = rt_pixel(frag);
    if (pix.x < 0) {
        return vec4<f32>(0.0);
    }
    return textureLoad(rt_reflections, pix, 0);
}
