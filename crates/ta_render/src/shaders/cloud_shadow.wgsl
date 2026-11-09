// Cloud shadow map (see clouds.rs): how much of the cloud layer lies between each point of the
// ground and the sun. Texel t (-1..1 per axis) holds the point `centre + K t / (1 - |t|)` of the
// plane y = 0 (a compressed mapping: fine near the view, reaching the horizon at the edges);
// points at other heights look it up where the sun ray through them crosses that plane (see
// `cloud_light` in common.wgsl). Prepended: fullscreen.wgsl, noise.wgsl, clouds.wgsl.

struct CloudShadowParams {
    // xy = centre of the map (world xz), z = scale K of the mapping, w = texels per side
    map: vec4<f32>,
    // xy = offset from a point of the plane y = 0 to the cloud layer along the sun (world xz),
    // z = how far the wind has carried the clouds (cloud speed * time), w = unused
    layer: vec4<f32>,
    // x = cloud cover (0..1), yz = the map's offset of the cloud field (as the sky's), w = unused
    cover: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: CloudShadowParams;

@fragment
fn fs_cloud_shadow(in: VsOut) -> @location(0) vec4<f32> {
    let t = in.uv * 2.0 - 1.0;
    let k = params.map.z;
    let rest = max(1.0 - abs(t), vec2<f32>(1e-4));
    let d = k * t / rest;
    // World units per texel here (the mapping's slope), to band-limit the noise. Shadows are
    // soft anyway (a cloud's penumbra alone is some 50 units wide), so never finer than 100.
    let texel = k / (rest * rest) * (2.0 / params.map.w);
    let footprint = max(max(texel.x, texel.y), 100.0);
    // The drifting layer coordinates of the cloud above this point, exactly as the sky has them.
    let p = params.map.xy + d + params.layer.xy - CLOUD_WIND * params.layer.z + params.cover.yz;
    let density = cloud_fbm(p + cloud_warp(p), footprint, 5);
    return vec4<f32>(cloud_coverage(density, cloud_threshold(params.cover.x)), 0.0, 0.0, 1.0);
}
