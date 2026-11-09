// Space scenes (space.rs): the frame, the scene's buffers, the sky's noise, nebulae, galaxies,
// the sun, the air round a planet, eclipses and the planets' light. Positions are round the eye
// (camera-relative), directions unit vectors; depth is reversed (1 near, 0 at infinity).

const PI: f32 = 3.14159265;

struct SpaceFrame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz towards the sun, w its angular radius (radians)
    sun: vec4<f32>,
    // rgb what a white surface facing the sun shows, w how bright its disc is over that
    sun_light: vec4<f32>,
    // the target's width and height (pixels), radians a pixel spans at the middle, seconds
    screen: vec4<f32>,
    // rgb the light from everywhere, w the stars' brightness
    ambient: vec4<f32>,
    // spheres, nebulae, galaxies, the frame's number
    counts: vec4<u32>,
    // x a star's blur (pixels), y how coloured the stars are, z the glare's strength, w the ghosts'
    stars: vec4<f32>,
    // x the near plane, y the share of the stars in front of the nebulae's dust
    misc: vec4<f32>,
    // the fill light: towards it, and its colour
    fill: vec4<f32>,
    fill_light: vec4<f32>,
    // the shadowed asteroids' shadow maps' projections
    rock_shadow: array<mat4x4<f32>, 4>,
};

struct Sphere {
    // its middle (camera-relative), w its radius
    centre: vec4<f32>,
    // from the world's axes to its surface's, and to its clouds' (rows)
    body: array<vec4<f32>, 3>,
    cloud: array<vec4<f32>, 3>,
    // its world, its seed, flags (1: air, 2: its own sea colours)
    kind: vec4<u32>,
    // the molecules' optical depth straight up (rgb), w the air's height (of the radius)
    air: vec4<f32>,
    // the haze's optical depth, its forward lean (g), the molecules' and the haze's scale heights
    haze: vec4<f32>,
    // clouds, relief, the sea's roughness, lights by night
    surface: vec4<f32>,
    // its colours times this (linear)
    tint: vec4<f32>,
    // its sea, shallow and deep (linear), when its own
    sea: array<vec4<f32>, 2>,
    // its mean colour, as it throws the sunlight back (w 1: it lights the others)
    shine: vec4<f32>,
    // x how Lambertian an airless body's light is, y octaves more (fewer) than its size shows,
    // z its fill light
    look: vec4<f32>,
};

struct Nebula {
    // towards its middle, w half its length (radians)
    centre: vec4<f32>,
    // along it, w half its width
    along: vec4<f32>,
    // across it, w how far it bends
    across: vec4<f32>,
    // its gas's colours, w its brightness and its filaments' strength
    colour: vec4<f32>,
    colour2: vec4<f32>,
    // its cores' colour, w their brightness
    core: vec4<f32>,
    // dust, the noise's scale, its swirl, its seed
    shape: vec4<f32>,
    // x how tightly its core gathers along it
    extra: vec4<f32>,
};

struct Galaxy {
    // towards its middle, w the tangent of its radius on the sky
    centre: vec4<f32>,
    // its long axis on the sky, w the cosine of its tilt
    major: vec4<f32>,
    // its short axis, w how tightly its arms wind
    minor: vec4<f32>,
    // the bulge's colour, w its brightness
    core: vec4<f32>,
    // the arms' colour, w how many
    arm: vec4<f32>,
    // the knots' colour, w its dust
    knots: vec4<f32>,
    // x its seed, y its clouds' strength
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: SpaceFrame;
@group(0) @binding(1) var<storage, read> spheres: array<Sphere>;
@group(0) @binding(2) var<storage, read> nebulae: array<Nebula>;
@group(0) @binding(3) var<storage, read> galaxies: array<Galaxy>;
@group(0) @binding(4) var rock_texture: texture_2d<f32>;
@group(0) @binding(5) var rock_sampler: sampler;
@group(0) @binding(6) var rock_shadows: texture_depth_2d_array;
@group(0) @binding(7) var shadow_sampler: sampler_comparison;

// The ray through a pixel (window pixels, y down).
fn view_ray(frag: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(frag.x / frame.screen.x * 2.0 - 1.0, 1.0 - frag.y / frame.screen.y * 2.0);
    let p = frame.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(p.xyz / p.w);
}

// A point's depth (reversed).
fn depth_of(p: vec3<f32>) -> f32 {
    let c = frame.view_proj * vec4<f32>(p, 1.0);
    return clamp(c.z / max(c.w, 1e-9), 0.0, 1.0);
}

// Where a direction lands on the picture (pixels), and whether it is in front of the eye (z).
fn pixel_of(d: vec3<f32>) -> vec3<f32> {
    let c = frame.view_proj * vec4<f32>(d, 0.0);
    let ndc = c.xy / max(c.w, 1e-9);
    return vec3<f32>((ndc.x * 0.5 + 0.5) * frame.screen.x, (0.5 - ndc.y * 0.5) * frame.screen.y, c.w);
}

fn srgb_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Two unit vectors square to `n` and to each other.
fn tangents(n: vec3<f32>) -> mat2x3<f32> {
    let a = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t = normalize(cross(a, n));
    return mat2x3<f32>(t, cross(n, t));
}

// ---- The sky's noise: gradient noise over the sphere of directions --------------------------------

fn pcg3(v_in: vec3<u32>) -> vec3<u32> {
    var v = v_in * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn sky_corner(c: vec3<i32>, f: vec3<f32>) -> f32 {
    let h = pcg3(bitcast<vec3<u32>>(c));
    let g = vec3<f32>(h & vec3<u32>(0xffffu)) / 32767.5 - 1.0;
    return dot(g, f);
}

// About -1 to 1.
fn sky_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let c = vec3<i32>(i);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let x00 = mix(sky_corner(c, f), sky_corner(c + vec3<i32>(1, 0, 0), f - vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(sky_corner(c + vec3<i32>(0, 1, 0), f - vec3<f32>(0.0, 1.0, 0.0)), sky_corner(c + vec3<i32>(1, 1, 0), f - vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(sky_corner(c + vec3<i32>(0, 0, 1), f - vec3<f32>(0.0, 0.0, 1.0)), sky_corner(c + vec3<i32>(1, 0, 1), f - vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(sky_corner(c + vec3<i32>(0, 1, 1), f - vec3<f32>(0.0, 1.0, 1.0)), sky_corner(c + vec3<i32>(1, 1, 1), f - vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z) * 1.4;
}

// Fractal noise of `octaves` (the last one fading in by its fraction): about -1 to 1.
fn sky_fbm(p: vec3<f32>, octaves: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    let n = max(octaves, 1.0);
    for (var o = 0.0; o < n; o += 1.0) {
        sum += amp * min(n - o, 1.0) * sky_noise(q);
        amp *= 0.5;
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 5.3);
    }
    return sum;
}

// Ridges (thin bright lines where the noise crosses 0), each octave weighted by the last: 0 to 1.
fn sky_ridged(p: vec3<f32>, octaves: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.6;
    var q = p;
    var prev = 1.0;
    let n = max(octaves, 1.0);
    for (var o = 0.0; o < n; o += 1.0) {
        let r = 1.0 - abs(sky_noise(q));
        let r2 = r * r;
        sum += amp * min(n - o, 1.0) * r2 * prev;
        prev = r2;
        amp *= 0.5;
        q = q * 2.07 + vec3<f32>(4.1, 2.3, 7.7);
    }
    return sum;
}

// ---- Nebulae -----------------------------------------------------------------------------------

// Where a direction lies on a nebula: along it and across it (in its half length and half width),
// its band bending.
fn nebula_place(d: vec3<f32>, n: Nebula) -> vec3<f32> {
    let z = dot(d, n.centre.xyz);
    let u = atan2(dot(d, n.along.xyz), z) / n.centre.w;
    let v = atan2(dot(d, n.across.xyz), z) / n.along.w - n.across.w * (u * u - 0.3);
    return vec3<f32>(u, v, z);
}

// A nebula's noise: its domain over the sky (`d` scaled, offset by its seed) and that domain
// swirled by noise of its own.
struct NebulaDomain {
    p: vec3<f32>,
    pw: vec3<f32>,
};

fn nebula_domain(d: vec3<f32>, n: Nebula) -> NebulaDomain {
    let seed = n.shape.w;
    var out: NebulaDomain;
    out.p = d * n.shape.y + vec3<f32>(seed * 1.731, seed * 0.917, seed * 2.313);
    let w = vec3<f32>(sky_fbm(out.p * 0.45, 3.0), sky_fbm(out.p * 0.45 + vec3<f32>(5.2, 1.3, 2.8), 3.0), sky_fbm(out.p * 0.45 + vec3<f32>(1.7, 9.2, 4.1), 3.0));
    out.pw = out.p + w * n.shape.z;
    return out;
}

// Its dust's optical depth: dark lanes in front of it (`wide`: where its haze reaches).
fn nebula_dust_depth(n: Nebula, dom: NebulaDomain, wide: f32, oct: f32) -> f32 {
    let f = sky_fbm(dom.pw * 1.25 + vec3<f32>(8.1, 2.2, 5.5), max(oct - 1.0, 2.0));
    return n.shape.x * wide * smoothstep(0.08, 0.5, f) * 2.2;
}

// A nebula's light towards `d` (rgb) and the optical depth of its dust (a); `px` a pixel's angle.
// Its gas is a few translucent layers through its depth, each billowing on its own (warped fractal
// noise), glowing and dimming what is behind it; the band's edge ragged, its brightness gathered
// in clumps along it; thin wisps (ridged noise) over it; a white-hot core; dust lanes in front.
fn nebula_at(d: vec3<f32>, n: Nebula, px: f32) -> vec4<f32> {
    let place = nebula_place(d, n);
    if (place.z < 0.05) {
        return vec4<f32>(0.0);
    }
    let u = place.x;
    let v = place.y;
    let wide = exp(-(u * u * 0.45 + v * v * 0.4));
    if (wide < 0.003) {
        return vec4<f32>(0.0);
    }
    let scale = n.shape.y;
    let dom = nebula_domain(d, n);
    // The finest octave a couple of pixels across.
    let oct = clamp(log2(0.35 / max(px * scale, 1e-6)), 2.0, 9.0);
    // A ragged edge, and clumps along the band.
    let rag = sky_fbm(dom.pw * 0.7 + vec3<f32>(3.1, 7.4, 1.9), 4.0);
    let uu = u + rag * 0.35;
    let vv = v + rag * 0.6;
    let env = exp(-(uu * uu * 1.1 + vv * vv * 1.6));
    let clump = smoothstep(-0.55, 0.55, sky_fbm(dom.p * 0.22 + vec3<f32>(6.3, 2.9, 8.8), 3.0));
    // Its core: where its gas glows white hot, a streak in the band's middle.
    let core = exp(-(uu * uu * n.extra.x + vv * vv * 8.0)) * n.core.w;
    // Its gas, layer by layer from the back.
    let axis = n.centre.xyz * scale * 0.4;
    var light = vec3<f32>(0.0);
    var tint = n.colour.rgb;
    for (var k = 0; k < 4; k++) {
        let q = dom.pw + axis * (f32(k) - 1.5);
        let f = sky_fbm(q, oct);
        let dens = pow(clamp(f * 1.35 + env * 0.75 - 0.38, 0.0, 1.6), 2.2) * env;
        tint = mix(n.colour.rgb, n.colour2.rgb, smoothstep(-0.35, 0.5, sky_fbm(q * 0.3 + vec3<f32>(2.0, 7.0, 3.0), 2.0)));
        light = light * exp(-dens * 1.4) + (tint * n.colour.w * (0.3 + 0.7 * clump) * 0.6 + n.core.rgb * core * (0.4 + dens)) * dens;
    }
    // Wisps.
    let fil = sky_ridged(dom.pw * 2.7 + vec3<f32>(3.3, 0.0, 1.1), max(oct - 1.0, 1.0));
    light += mix(tint, vec3<f32>(1.0), 0.35) * pow(fil, 4.0) * env * (0.4 + 0.6 * clump) * n.colour2.w * n.colour.w * 0.3;
    // A faint haze round it all, and its dust in front.
    light += tint * wide * 0.018 * n.colour.w;
    let dust = nebula_dust_depth(n, dom, wide, oct);
    return vec4<f32>(light * exp(-dust), dust);
}

// The nebulae's dust towards `d` (what dims the stars behind it).
fn nebula_dust(d: vec3<f32>) -> f32 {
    var dust = 0.0;
    for (var i = 0u; i < frame.counts.y; i++) {
        let n = nebulae[i];
        if (n.shape.x <= 0.0) {
            continue;
        }
        let place = nebula_place(d, n);
        if (place.z < 0.05) {
            continue;
        }
        let wide = exp(-(place.x * place.x * 0.45 + place.y * place.y * 0.4));
        if (wide < 0.003) {
            continue;
        }
        dust += nebula_dust_depth(n, nebula_domain(d, n), wide, 5.0);
    }
    return dust;
}

// ---- Galaxies ----------------------------------------------------------------------------------

// Spots scattered one to a cell of a square grid (where stars are born): the nearest's light,
// a Gaussian `size` of the cell across, some cells empty.
fn spots(p: vec2<f32>, size: f32, seed: f32) -> f32 {
    let base = floor(p);
    var best = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let c = base + vec2<f32>(f32(dx), f32(dy));
            let h = pcg3(bitcast<vec3<u32>>(vec3<i32>(i32(c.x), i32(c.y), i32(seed * 977.0))));
            let at = c + vec2<f32>(f32(h.x & 0xffffu), f32(h.y & 0xffffu)) / 65535.0;
            let lit = select(0.0, 0.3 + 0.7 * f32(h.z & 0xffffu) / 65535.0, (h.z >> 16u) % 5u == 0u);
            let r = length(p - at) / size;
            best = max(best, lit * exp(-r * r));
        }
    }
    return best;
}

// A spiral galaxy's light towards `d`: a bulge, an exponential disc, logarithmic arms broken into
// clumps, dust lanes on their inner edges, pink knots where stars are born, a faint halo.
fn galaxy_at(d: vec3<f32>, g: Galaxy, px: f32) -> vec3<f32> {
    let z = dot(d, g.centre.xyz);
    if (z < 0.2) {
        return vec3<f32>(0.0);
    }
    // On the sky's tangent plane, in its radii.
    let k = 1.0 / (z * g.centre.w);
    let x = dot(d, g.major.xyz) * k;
    let y = dot(d, g.minor.xyz) * k;
    let rr = x * x + y * y;
    if (rr > 9.0) {
        return vec3<f32>(0.0);
    }
    let ci = max(g.major.w, 0.08);
    // Its disc as seen face on.
    let yd = y / ci;
    let r = sqrt(x * x + yd * yd);
    let th = atan2(yd, x);
    let seed = g.extra.x * 0.731;
    // A pixel in its radii (at its disc's foreshortening), and the octaves that shows.
    let pr = px / g.centre.w / ci;
    let oct = clamp(log2(0.3 / max(pr * 6.0, 1e-6)), 1.0, 6.0);
    // The bulge: round, a little flattened as the disc tilts.
    let rb = sqrt(x * x + y * y / (0.35 + 0.65 * ci));
    let bulge = exp(-pow(rb / 0.045, 0.75) * 2.0) + 0.3 * exp(-rb / 0.11);
    // The disc, its arms winding out from the bulge, wavering.
    let disc = exp(-r / 0.3) * smoothstep(1.7, 0.8, r);
    let wob = sky_fbm(vec3<f32>(x * 2.0, yd * 2.0, seed), 3.0);
    let phase = th - g.minor.w * log(r + 0.03) + wob * 0.8;
    let arm = pow(0.5 + 0.5 * cos(g.arm.w * phase), 2.4);
    let clump = clamp(0.35 + 0.9 * sky_fbm(vec3<f32>(x * 6.0, yd * 6.0, seed + 3.1), oct), 0.0, 1.6);
    let grain = clamp(0.55 + 0.8 * sky_fbm(vec3<f32>(x * 17.0, yd * 17.0, seed + 5.3), max(oct - 1.0, 1.0)), 0.0, 1.5);
    let arms = disc * (0.1 + 1.7 * arm * clump * grain) * smoothstep(0.03, 0.2, r);
    // Dust lanes along the arms' inner edges.
    let lane = pow(0.5 + 0.5 * cos(g.arm.w * phase + 0.75), 6.0) * disc * smoothstep(0.05, 0.22, r) * g.knots.w * (0.5 + 0.7 * clump);
    // Knots: young stars' nebulae, pink, strung along the arms.
    let kn = spots(vec2<f32>(x, yd) * 30.0, 0.16, seed) * pow(arm, 1.2) * clump * disc * smoothstep(0.1, 0.3, r) * smoothstep(1.5, 0.4, pr * 30.0);
    // Their glowing gas: great clouds of it about the inner arms, a few big ones (a field of its own,
    // coarser than the arms' clumps) where the arms run, not a ring of them.
    let big = sky_fbm(vec3<f32>(x * 2.6, yd * 2.6, seed + 9.1), min(oct, 4.0));
    let gas = pow(clamp(big + 0.05, 0.0, 1.0), 1.6) * (0.35 + 0.65 * arm) * exp(-r / 0.3) * smoothstep(0.05, 0.18, r) * clamp(0.6 + clump * 0.6, 0.0, 1.5);
    var c = g.core.rgb * bulge * 2.4 + g.arm.rgb * arms + g.knots.rgb * (kn * 1.6 + gas * 3.0 * g.extra.y);
    c *= 1.0 - 0.8 * clamp(lane * 1.8, 0.0, 1.0);
    // Its halo of old stars and gas, wider than the disc.
    let reach = sqrt(rr);
    c += mix(g.arm.rgb, g.core.rgb, 0.5) * (exp(-reach / 0.6) * 0.035 + exp(-reach / 0.25) * 0.05);
    return c * g.core.w * smoothstep(9.0, 3.0, rr);
}

// ---- The sun -----------------------------------------------------------------------------------

// The angle between two unit vectors (exact for small ones).
fn angle_between(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return atan2(length(cross(a, b)), dot(a, b));
}

// The sun's disc towards `d`: limb-darkened, its edge antialiased over a pixel (`px` radians).
fn sun_disc(d: vec3<f32>, px: f32) -> vec3<f32> {
    let l = frame.sun.xyz;
    if (dot(d, l) <= 0.0) {
        return vec3<f32>(0.0);
    }
    let rad = frame.sun.w;
    let r = angle_between(d, l) / rad;
    let cover = clamp((1.0 - r) * rad / max(px, 1e-6) + 0.5, 0.0, 1.0);
    let rc = min(r, 1.0);
    let mu = sqrt(max(1.0 - rc * rc, 0.0));
    let limb = 1.0 - 0.6 * (1.0 - mu) - 0.15 * (1.0 - mu * mu);
    return frame.sun_light.rgb * frame.sun_light.w * cover * limb;
}

// ---- The air round a planet (in its radii; the ground at 1) -----------------------------------

struct AirLight {
    inscatter: vec3<f32>,
    transmittance: vec3<f32>,
};

// Schüler's approximation of the Chapman function: the optical depth along a ray at cosine `mu`
// from the vertical, from `x` scale heights from the middle, over that straight up.
fn chapman(x: f32, mu: f32) -> f32 {
    let c = sqrt(1.5707963 * x);
    if (mu >= 0.0) {
        return c / ((c - 1.0) * mu + 1.0);
    }
    let x0 = x * sqrt(max(1.0 - mu * mu, 0.0));
    let c0 = sqrt(1.5707963 * x0);
    return 2.0 * c0 * exp(x - x0) - c / ((c - 1.0) * (-mu) + 1.0);
}

// How much of the sun a point `r` from the middle sees past the ground, the sun at cosine `mu`
// from its vertical.
fn past_the_ground(r: f32, mu: f32) -> f32 {
    if (mu >= 0.0) {
        return 1.0;
    }
    let low = r * sqrt(max(1.0 - mu * mu, 0.0));
    return smoothstep(0.9988, 1.0012, low);
}

// The optical depth from `r` towards the sun, both kinds of scattering.
fn depth_to_sun(s: Sphere, r: f32, mu: f32) -> vec3<f32> {
    let h = max(r - 1.0, 0.0);
    let rayleigh = s.air.rgb * exp(-h / s.haze.z) * chapman(r / s.haze.z, mu);
    let mie = s.haze.x * 1.11 * exp(-h / s.haze.w) * chapman(r / s.haze.w, mu);
    return rayleigh + vec3<f32>(mie);
}

// The sunlight reaching `p` (unit, on the ground; `r` from the middle: clouds float higher)
// through the air: the beam, dimmed, and of the light the molecules scattered out of it, the half
// that comes down again as the sky's light (blue), which keeps white things white.
fn sun_through_air(s: Sphere, p: vec3<f32>, r: f32) -> vec3<f32> {
    let mu = dot(p, frame.sun.xyz);
    // (Past the terminator as far as the sun is still seen over the ground from that height.)
    let seen = past_the_ground(r, mu);
    if (seen <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = max(r - 1.0, 0.0);
    let rayleigh = s.air.rgb * exp(-h / s.haze.z) * chapman(r / s.haze.z, mu);
    let mie = s.haze.x * 1.11 * exp(-h / s.haze.w) * chapman(r / s.haze.w, mu);
    let beam = exp(-(rayleigh + vec3<f32>(mie)));
    let sky = (exp(-vec3<f32>(mie)) - beam) * 0.5 * smoothstep(-0.05, 0.25, mu);
    return (beam + sky) * seen;
}

fn phase_rayleigh(nu: f32) -> f32 {
    return 0.0596831 * (1.0 + nu * nu);
}

fn phase_mie(nu: f32, g: f32) -> f32 {
    let g2 = g * g;
    return 0.1193662 * (1.0 - g2) * (1.0 + nu * nu) / ((2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * nu, 1e-4), 1.5));
}

// Single scattering along the ray `o + d t` from `t0` to `t1`: the light scattered towards the eye,
// and how much of what lies behind comes through.
fn air_march(s: Sphere, o: vec3<f32>, d: vec3<f32>, t0: f32, t1: f32) -> AirLight {
    let l = frame.sun.xyz;
    let hr = s.haze.z;
    let hm = s.haze.w;
    let beta_r = s.air.rgb / hr;
    let beta_m = s.haze.x / hm;
    let steps = 24;
    let ds = max(t1 - t0, 0.0) / f32(steps);
    var tau = vec3<f32>(0.0);
    var sum_r = vec3<f32>(0.0);
    var sum_m = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        let t = t0 + (f32(i) + 0.5) * ds;
        let p = o + d * t;
        let r = length(p);
        let h = max(r - 1.0, 0.0);
        let dr = exp(-h / hr);
        let dm = exp(-h / hm);
        let ext = beta_r * dr + vec3<f32>(beta_m * dm * 1.11);
        let mid = tau + ext * (ds * 0.5);
        let mu = dot(p, l) / r;
        let seen = past_the_ground(r, mu);
        if (seen > 0.0) {
            let att = exp(-(mid + depth_to_sun(s, r, mu))) * seen;
            sum_r += dr * att * ds;
            sum_m += dm * att * ds;
        }
        tau += ext * ds;
    }
    let nu = dot(d, l);
    var out: AirLight;
    out.inscatter = PI * frame.sun_light.rgb * (beta_r * sum_r * phase_rayleigh(nu) + beta_m * sum_m * phase_mie(nu, s.haze.y));
    out.transmittance = exp(-tau);
    return out;
}

// ---- Light between the bodies ------------------------------------------------------------------

// How much of the sun `p` sees past the other spheres (the eclipses; a disc over a disc).
fn eclipse(p: vec3<f32>, skip: u32) -> f32 {
    var lit = 1.0;
    let l = frame.sun.xyz;
    let a_sun = frame.sun.w;
    for (var j = 0u; j < frame.counts.x; j++) {
        if (j == skip) {
            continue;
        }
        let s = spheres[j];
        let to = s.centre.xyz - p;
        let along = dot(to, l);
        if (along <= 0.0) {
            continue;
        }
        let dist = length(to);
        let a_body = asin(min(s.centre.w / dist, 1.0));
        let sep = atan2(length(cross(to, l)), along);
        let cover = 1.0 - smoothstep(abs(a_body - a_sun), a_body + a_sun, sep);
        let most = min(a_body * a_body / (a_sun * a_sun), 1.0);
        lit *= 1.0 - cover * most;
    }
    return lit;
}

// The sunlight the planets throw back onto `p` (facing `n`), as a white surface facing them shows.
fn planetshine(p: vec3<f32>, n: vec3<f32>, skip: u32) -> vec3<f32> {
    var e = vec3<f32>(0.0);
    for (var j = 0u; j < frame.counts.x; j++) {
        if (j == skip) {
            continue;
        }
        let s = spheres[j];
        if (s.shine.w <= 0.0) {
            continue;
        }
        let to = s.centre.xyz - p;
        let dist = length(to);
        let dir = to / dist;
        let r = min(s.centre.w / dist, 1.0);
        // How much of the face it shows `p` is lit.
        let lit = 0.5 * (1.0 - dot(dir, frame.sun.xyz));
        e += s.shine.rgb * r * r * lit * max(dot(n, dir) + 0.1 * r, 0.0);
    }
    return e * frame.sun_light.rgb;
}

// The moon's and the asteroids' dusty light: half Lambert, half Lommel-Seeliger (a full moon's flat face).
fn lunar(cos_i: f32, cos_e: f32) -> f32 {
    if (cos_i <= 0.0) {
        return 0.0;
    }
    let ls = 2.0 * cos_i / (cos_i + max(cos_e, 0.0) + 1e-3);
    return mix(cos_i, ls, 0.55);
}
