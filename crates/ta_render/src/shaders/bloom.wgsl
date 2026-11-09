// ---- Bloom: bright-pass downsample chain + tent upsample ------------------------------

struct BloomParams {
    // x,y = source texel size, z = threshold (first pass only), w = unused
    texel: vec4<f32>,
};
@group(0) @binding(0) var<uniform> bp: BloomParams;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

fn karis(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)));
}

@fragment
fn fs_bloom_down(in: VsOut) -> @location(0) vec4<f32> {
    let t = bp.texel.xy;
    let uv = in.uv;
    // 13-tap downsample (Call of Duty: AW style), with Karis average on the first pass.
    let a = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(-2.0, -2.0), 0.0).rgb;
    let b = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(0.0, -2.0), 0.0).rgb;
    let c = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(2.0, -2.0), 0.0).rgb;
    let d = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(-2.0, 0.0), 0.0).rgb;
    let e = textureSampleLevel(src, src_sampler, uv, 0.0).rgb;
    let f = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(2.0, 0.0), 0.0).rgb;
    let g = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(-2.0, 2.0), 0.0).rgb;
    let h = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(0.0, 2.0), 0.0).rgb;
    let i = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(2.0, 2.0), 0.0).rgb;
    let j = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    let k = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb;
    let l = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    let m = textureSampleLevel(src, src_sampler, uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb;
    var o: vec3<f32>;
    if (bp.texel.z > 0.0) {
        // Bright pass with soft knee, Karis-weighted groups to kill fireflies.
        let g0 = (j + k + l + m) * 0.25;
        let g1 = (a + b + d + e) * 0.25;
        let g2 = (b + c + e + f) * 0.25;
        let g3 = (d + e + g + h) * 0.25;
        let g4 = (e + f + h + i) * 0.25;
        let w0 = karis(g0) * 0.5;
        let w1 = karis(g1) * 0.125;
        let w2 = karis(g2) * 0.125;
        let w3 = karis(g3) * 0.125;
        let w4 = karis(g4) * 0.125;
        o = (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / (w0 + w1 + w2 + w3 + w4);
        let peak = max(max(o.r, o.g), o.b);
        let knee = bp.texel.z * 0.5;
        let soft = clamp(peak - bp.texel.z + knee, 0.0, 2.0 * knee);
        let contrib = max(soft * soft / (4.0 * knee + 1e-4), peak - bp.texel.z) / max(peak, 1e-4);
        o *= max(contrib, 0.0);
    } else {
        o = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
    }
    return vec4<f32>(o, 1.0);
}

@fragment
fn fs_bloom_up(in: VsOut) -> @location(0) vec4<f32> {
    let t = bp.texel.xy;
    let uv = in.uv;
    var s = textureSampleLevel(src, src_sampler, uv, 0.0).rgb * 4.0;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(-t.x, 0.0), 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(t.x, 0.0), 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(0.0, -t.y), 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(0.0, t.y), 0.0).rgb * 2.0;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(-t.x, -t.y), 0.0).rgb;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(t.x, -t.y), 0.0).rgb;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(-t.x, t.y), 0.0).rgb;
    s += textureSampleLevel(src, src_sampler, uv + vec2<f32>(t.x, t.y), 0.0).rgb;
    return vec4<f32>(s / 16.0, 1.0);
}
