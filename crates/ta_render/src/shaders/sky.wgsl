// Sky pass: one full-screen triangle on the far plane, drawn after the opaque scene with the
// depth test on (and no depth writes), so only background pixels are shaded. Prepended:
// common.wgsl, noise.wgsl, clouds.wgsl, sky_common.wgsl.
//
// In a battle nothing is drawn beyond the map (`void_on`, see `sky::Beyond::Clouds`): under the
// horizon the sky pass draws a sea of dark clouds some way below the map instead, so the
// battlefield stands out, as in Supreme Commander.

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
    // footprints on the cloud layer above and on the sea of clouds below.
    let pixel = length(fwidth(dir));
    let plane = cloud_plane(dir, frame.world.x);
    let footprint = max(length(dpdx(plane.xy)), length(dpdy(plane.xy)));
    let under = void_plane(dir);
    let under_footprint = max(length(dpdx(under.xy)), length(dpdy(under.xy)));

    var color = sky_gradient(dir) + sun_disc(dir, pixel) + star_field(dir, pixel);
    let clouds = sky_clouds(dir, plane, footprint);
    // Clouds pass in front of the sun (their colour is premultiplied by coverage).
    color = color * (1.0 - clouds.a) + clouds.rgb;
    if (void_on()) {
        color = beyond_void(dir, color, under, under_footprint);
    }
    return vec4<f32>(color, SKY_ALPHA);
}

// Alpha the sky leaves in the scene target (geometry leaves 1): the heat haze never distorts
// the sky, nor pulls it into distorted pixels (see heat.rs and post.wgsl). Negative enough that
// hot air in front of the sky, or an edge pixel shared with geometry, stays below 0.
const SKY_ALPHA: f32 = -16.0;

// The sea of clouds under the map: how far under the sea level it lies, the size of its
// largest billows and how fast they drift (world units, and a second), and how far along a ray
// it is looked for (beyond, the ray runs into its haze).
const VOID_DEPTH: f32 = 160.0;
const VOID_SCALE: f32 = 1600.0;
const VOID_DRIFT: f32 = 5.0;
const VOID_FAR: f32 = 360000.0;

// True in a battle: nothing beyond the map, the sea of clouds under it (perspective camera
// with the sky; never in space).
fn void_on() -> bool {
    return frame.liquid.z > 0.5;
}

// Where the view ray `dir` meets the sea of clouds: world xz and the distance along the ray.
// Rays at or above the horizon are taken far out along it.
fn void_plane(dir: vec3<f32>) -> vec3<f32> {
    let eye = frame.camera_pos.xyz;
    let drop = max(eye.y - (frame.world.y - VOID_DEPTH), 1.0);
    let t = min(drop / max(-dir.y, 1e-5), VOID_FAR);
    return vec3<f32>(eye.xz + dir.xz * t, t);
}

// Fractal noise of the sea of clouds at `p`, band-limited by the pixel footprint there as
// `cloud_fbm` is (so it never shimmers far out). Mean 0.
fn void_fbm(p: vec2<f32>, footprint: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var wavelength = VOID_SCALE;
    var q = p / VOID_SCALE;
    for (var i = 0; i < 5; i++) {
        let band = smoothstep(2.5, 6.0, wavelength / max(footprint, 1e-3));
        // (Each octave finer than the last: once one is lost in the pixel, so are the rest.)
        if (band <= 0.0) {
            break;
        }
        sum += amp * band * noise(q + vec2<f32>(f32(i) * 23.1, f32(i) * -11.7));
        q = vec2<f32>(q.x * 1.6 - q.y * 1.2, q.x * 1.2 + q.y * 1.6);
        amp *= 0.5;
        wavelength *= 0.5;
    }
    return sum;
}

// How far world xz `p` lies outside the map (0 over it).
fn void_outside(p: vec2<f32>) -> f32 {
    return length(max(max(-p, p - frame.world.zw), vec2<f32>(0.0)));
}

// The sea of dark clouds along `dir` over the sky's colour `sky`: everything under the horizon,
// and a dark bank along it the sky rises from. `under` and `footprint` from `void_plane`.
fn beyond_void(dir: vec3<f32>, sky: vec3<f32>, under: vec3<f32>, footprint: f32) -> vec3<f32> {
    let drift = CLOUD_WIND * (VOID_DRIFT * frame.world.x);
    let p = under.xy;
    // Billows drifting with the wind over a broader murk drifting slower, a slow swirl through
    // them.
    let swirl = vec2<f32>(noise(p / 9000.0 + vec2<f32>(4.3, -2.1)), noise(p / 9000.0 + vec2<f32>(-1.6, 7.7))) * 1200.0;
    let q = p + swirl - drift;
    let billows = void_fbm(q, footprint);
    let murk = void_fbm(p * 0.45 - drift * 0.2 + vec2<f32>(5113.0, -2971.0), footprint * 0.45);
    let density = saturate(0.5 + billows * 1.7 + murk * 0.8);
    // Lit faintly from above: a billow's side towards the sun brighter, the side away from it in
    // its own shade (denser towards the sun), as the sky's clouds are; the hollows near black.
    let sun_flat = frame.sun_dir.xz / max(length(frame.sun_dir.xz), 1e-3);
    let towards = void_fbm(q + sun_flat * (VOID_SCALE * 0.08), footprint);
    let lit = saturate(0.5 + (billows - towards) * 4.0);
    let sun = frame.sun_color.rgb * frame.sun_dir.w * saturate(frame.sun_dir.y * 1.5 + 0.1);
    let skylight = mix(frame.fog.rgb, frame.sky_zenith.rgb, 0.5) + frame.night.rgb;
    let dark = frame.fog.rgb * 0.004 + vec3<f32>(0.0015, 0.0018, 0.0024);
    let tops = smoothstep(0.3, 0.95, density);
    var c = dark + (sun * (0.003 + 0.014 * lit) + skylight * 0.011) * tops;
    // The map overhead shades them along its edges.
    c *= mix(0.35, 1.0, smoothstep(0.0, 900.0, void_outside(p)));
    // On a lava world, a dull glow from below between the billows.
    if (frame.water_extra.y > 0.5) {
        c += vec3<f32>(0.02, 0.005, 0.0012) * (1.0 - smoothstep(0.2, 0.7, density));
    }
    // Beyond what the view is on they sink into a dim haze, as the map's ground sinks into the
    // haze (`fog_factor`); it lies along the horizon as a dark bank the sky rises from.
    let haze = dark * 1.5 + skylight * 0.004;
    let thick = max(frame.fog.a * 2.0, 1.0 / 15000.0);
    c = mix(c, haze, 1.0 - exp(-max(under.z - frame.los.w, 0.0) * thick));
    return mix(sky, c, 1.0 - smoothstep(-0.015, 0.08, dir.y));
}
