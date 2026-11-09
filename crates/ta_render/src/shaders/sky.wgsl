// Sky pass: one full-screen triangle on the far plane, drawn after the opaque scene with the
// depth test on (and no depth writes), so only background pixels are shaded. Prepended:
// common.wgsl, noise.wgsl, sky_common.wgsl.

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    // z = w: exactly on the far plane, so the depth test passes only where nothing was drawn.
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let near = frame.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = frame.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - near.xyz / near.w);
    // Derivatives first, in uniform control flow: the angular size of a pixel, and the pixel
    // footprint on the cloud plane.
    let pixel = length(fwidth(dir));
    let plane = cloud_plane(dir, frame.world.x);
    let footprint = max(length(dpdx(plane.xy)), length(dpdy(plane.xy)));

    var color = sky_gradient(dir) + sun_disc(dir, pixel) + star_field(dir, pixel);
    let clouds = sky_clouds(dir, plane, footprint);
    // Clouds pass in front of the sun (their colour is premultiplied by coverage).
    color = color * (1.0 - clouds.a) + clouds.rgb;
    return vec4<f32>(color, SKY_ALPHA);
}

// Alpha the sky leaves in the scene target (geometry leaves 1): the heat haze never distorts
// the sky, nor pulls it into distorted pixels (see heat.rs and post.wgsl). Negative enough that
// hot air in front of the sky, or an edge pixel shared with geometry, stays below 0.
const SKY_ALPHA: f32 = -16.0;
