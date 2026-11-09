// Space scenes (space.rs): the sky, the stars, the asteroids and their shadows, the planets and
// moons with their air, the sun's glare. Prepended: space_common.wgsl, space_worlds.wgsl.

// How far the ground's relief tilts the light (times a sphere's `relief`).
const RELIEF: f32 = 0.06;
// How high the clouds float (of the radius): how far their shadows fall from them.
const CLOUD_HEIGHT: f32 = 0.006;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> VsOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

// ---- The sky -----------------------------------------------------------------------------------

@fragment
fn fs_sky(in: VsOut) -> @location(0) vec4<f32> {
    let d = view_ray(in.pos.xy);
    let px = frame.screen.z;
    var c = vec3<f32>(0.0);
    for (var i = 0u; i < frame.counts.z; i++) {
        c += galaxy_at(d, galaxies[i], px);
    }
    // The nebulae in front of the galaxies: their dust dims what is behind.
    for (var i = 0u; i < frame.counts.y; i++) {
        let n = nebula_at(d, nebulae[i], px);
        c = c * exp(-n.a) + n.rgb;
    }
    c += sun_disc(d, px);
    return vec4<f32>(c, 1.0);
}

// ---- The stars ---------------------------------------------------------------------------------

struct StarOut {
    @builtin(position) pos: vec4<f32>,
    // its middle (pixels), its colour times its brightness, its blur (pixels)
    @location(0) centre: vec2<f32>,
    @location(1) light: vec3<f32>,
    @location(2) sigma: f32,
};

var<private> QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
    vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_star(@location(0) dir: vec3<f32>, @location(1) flux: f32, @location(2) colour: vec4<f32>, @builtin(vertex_index) vi: u32) -> StarOut {
    var out: StarOut;
    out.pos = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    out.centre = vec2<f32>(0.0);
    out.light = vec3<f32>(0.0);
    out.sigma = 1.0;
    let at = pixel_of(dir);
    let sigma = max(frame.stars.x, 0.3);
    var b = flux * frame.ambient.w;
    // (Off the picture, or too faint to show.)
    if (at.z <= 0.0 || b < 0.003 || any(at.xy < vec2<f32>(-64.0)) || any(at.xy > frame.screen.xy + 64.0)) {
        return out;
    }
    // Behind the nebulae's dust (most are), dimmed by it.
    if (colour.a > frame.misc.y && frame.counts.y > 0u) {
        b *= exp(-nebula_dust(dir));
    }
    // A bright star's glow reaches further.
    let reach = sigma * 3.2 + sqrt(b) * sigma * 7.0 + 1.0;
    let corner = at.xy + QUAD[vi] * reach;
    out.pos = vec4<f32>(corner.x / frame.screen.x * 2.0 - 1.0, 1.0 - corner.y / frame.screen.y * 2.0, 0.0, 1.0);
    out.centre = at.xy;
    out.light = colour.rgb * b;
    out.sigma = sigma;
    return out;
}

// The error function (Abramowitz and Stegun 7.1.26).
fn erf(x: f32) -> f32 {
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return sign(x) * y;
}

@fragment
fn fs_star(in: StarOut) -> @location(0) vec4<f32> {
    // The star's Gaussian over this pixel's square, over what a pixel under its middle gets.
    let d = in.pos.xy - in.centre;
    let s = in.sigma * 1.4142136;
    let gx = erf((d.x + 0.5) / s) - erf((d.x - 0.5) / s);
    let gy = erf((d.y + 0.5) / s) - erf((d.y - 0.5) / s);
    let peak = 2.0 * erf(0.5 / s);
    let core = gx * gy / (peak * peak);
    // And the faint glow of the lens round the brightest.
    let r = length(d) / in.sigma;
    let halo = 0.012 / (1.0 + r * r * 0.12);
    return vec4<f32>(in.light * (core + halo), 0.0);
}

// ---- The asteroids -----------------------------------------------------------------------------

struct RockIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) cavity: f32,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) tint: vec4<f32>,
    @location(7) extra: vec4<f32>,
};

struct RockOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) own: vec3<f32>,
    @location(3) own_normal: vec3<f32>,
    @location(4) cavity: f32,
    @location(5) @interpolate(flat) tint: vec4<f32>,
    @location(6) @interpolate(flat) extra: vec4<f32>,
};

fn rock_world(in: RockIn, p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(in.m0.xyz, p) + in.m0.w, dot(in.m1.xyz, p) + in.m1.w, dot(in.m2.xyz, p) + in.m2.w);
}

@vertex
fn vs_rock(in: RockIn) -> RockOut {
    var out: RockOut;
    out.world = rock_world(in, in.pos);
    out.normal = normalize(vec3<f32>(dot(in.m0.xyz, in.normal), dot(in.m1.xyz, in.normal), dot(in.m2.xyz, in.normal)));
    out.pos = frame.view_proj * vec4<f32>(out.world, 1.0);
    out.own = in.pos;
    out.own_normal = in.normal;
    out.cavity = in.cavity;
    out.tint = in.tint;
    out.extra = in.extra;
    return out;
}

@vertex
fn vs_rock_shadow(in: RockIn) -> @builtin(position) vec4<f32> {
    let slot = i32(in.extra.x);
    if (slot < 0) {
        return vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return frame.rock_shadow[slot] * vec4<f32>(rock_world(in, in.pos), 1.0);
}

// How much sun reaches `world` past the asteroid itself (its shadow map, 12 taps on a disc as
// wide as the sun's softening makes its shadows).
fn rock_shadow(slot: i32, world: vec3<f32>, n: vec3<f32>, radius: f32) -> f32 {
    if (slot < 0) {
        return 1.0;
    }
    let c = frame.rock_shadow[slot] * vec4<f32>(world + n * radius * 0.004, 1.0);
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    let spread = 1.5 / 2048.0;
    var sum = 0.0;
    for (var i = 0; i < 12; i++) {
        let a = f32(i) * 2.3999632;
        let r = sqrt((f32(i) + 0.5) / 12.0) * spread;
        sum += textureSampleCompareLevel(rock_shadows, shadow_sampler, uv + vec2<f32>(cos(a), sin(a)) * r, slot, c.z);
    }
    return sum / 12.0;
}

// The rock texture laid on along the asteroid's own axes (`p`, `n` in its own space), at two
// scales so its tiles do not show.
fn rock_texel(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var w = pow(abs(n), vec3<f32>(4.0));
    w /= w.x + w.y + w.z;
    let a = textureSample(rock_texture, rock_sampler, p.zy).rgb * w.x + textureSample(rock_texture, rock_sampler, p.xz).rgb * w.y + textureSample(rock_texture, rock_sampler, p.xy).rgb * w.z;
    let q = p * 0.27 + vec3<f32>(0.31, 0.57, 0.11);
    let b = textureSample(rock_texture, rock_sampler, q.zy).rgb * w.x + textureSample(rock_texture, rock_sampler, q.xz).rgb * w.y + textureSample(rock_texture, rock_sampler, q.xy).rgb * w.z;
    return a * b * 2.2;
}

@fragment
fn fs_rock(in: RockOut) -> @location(0) vec4<f32> {
    let own_n = normalize(in.own_normal);
    let tex = rock_texel(in.own * in.tint.w, own_n);
    // The texture's cracks and grain as relief (its brightness as height, through the
    // derivatives across the screen: Mikkelsen's bump mapping of surfaces without coordinates).
    let h = luminance(tex) * in.extra.y * 0.004 / max(in.tint.w, 0.1) * in.extra.z;
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let dhx = dpdx(h);
    let dhy = dpdy(h);
    var n = normalize(in.normal);
    let r1 = cross(dpy, n);
    let r2 = cross(n, dpx);
    let det = dot(dpx, r1);
    let grad = sign(det) * (dhx * r1 + dhy * r2);
    let bumped = normalize(abs(det) * n - grad);
    n = select(n, bumped, abs(det) > 1e-12);
    let v = normalize(-in.world);
    let l = frame.sun.xyz;
    let albedo = tex * in.tint.rgb * mix(1.0, 0.72, in.cavity);
    let cos_i = dot(n, l);
    let geo = smoothstep(-0.05, 0.1, dot(normalize(in.normal), l));
    let sun = frame.sun_light.rgb * eclipse(in.world, 0xffffffffu) * rock_shadow(i32(in.extra.x), in.world, normalize(in.normal), in.extra.y) * geo;
    var c = albedo * sun * lunar(cos_i, dot(n, v));
    c += albedo * (frame.ambient.rgb * (1.0 - in.cavity * 0.6) + planetshine(in.world, n, 0xffffffffu));
    // The fill light (Lambert's, its shadows none: it only shows the shape of what the sun leaves).
    c += albedo * frame.fill_light.rgb * max(dot(n, frame.fill.xyz), 0.0) * (1.0 - in.cavity * 0.4);
    return vec4<f32>(c, 1.0);
}

// ---- Planets and moons -------------------------------------------------------------------------

struct SphereOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
};

@vertex
fn vs_sphere(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SphereOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: SphereOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.index = ii;
    return out;
}

struct SphereFrag {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// A pixel's ray against a sphere, in its radii: the eye from its middle (o), the ray (d), how
// far along it passes nearest (-b), the nearest point (perp) and its distance (r), a pixel's size
// there (pix) and how much of the pixel the sphere covers (cover).
struct SphereRay {
    o: vec3<f32>,
    d: vec3<f32>,
    b: f32,
    perp: vec3<f32>,
    r: f32,
    pix: f32,
    cover: f32,
};

fn sphere_ray(s: Sphere, frag: vec2<f32>) -> SphereRay {
    var sr: SphereRay;
    sr.d = view_ray(frag);
    sr.o = -s.centre.xyz / s.centre.w;
    sr.b = dot(sr.o, sr.d);
    sr.perp = sr.o - sr.b * sr.d;
    sr.r = length(sr.perp);
    sr.pix = frame.screen.z * max(-sr.b, 1e-6);
    sr.cover = clamp(0.5 + (1.0 - sr.r) / sr.pix, 0.0, 1.0);
    return sr;
}

fn turn(rows: array<vec4<f32>, 3>, p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(rows[0].xyz, p), dot(rows[1].xyz, p), dot(rows[2].xyz, p));
}

// The sphere's face at `p` (unit, the world's axes) as the eye sees it: lit, clouded, glinting,
// glowing, seen through its air.
fn sphere_face(s: Sphere, index: u32, sr: SphereRay, p: vec3<f32>) -> vec3<f32> {
    let n = p;
    let v = -sr.d;
    let l = frame.sun.xyz;
    let world = s.kind.x;
    let has_air = (s.kind.z & 1u) != 0u;
    let cos_v = max(dot(n, v), 0.0);
    // A pixel's size on the ball, and the octaves it shows.
    let t_hit = dot(p - sr.o, sr.d);
    let pix = frame.screen.z * t_hit;
    var ctx: WorldCtx;
    ctx.seed = s.kind.y;
    ctx.fine = pix / max(cos_v, 0.12) * exp2(-s.look.y);
    ctx.detail = clamp(log2(1.0 / ctx.fine) - 0.8, 1.0, 13.0);
    let q = turn(s.body, p);
    let g = ground(world, q, ctx);
    var albedo = srgb_linear(g.albedo) * s.tint.rgb;
    if (g.sea > 0.5 && (s.kind.z & 2u) != 0u) {
        albedo = mix(s.sea[0].rgb, s.sea[1].rgb, g.deep);
    }
    // The ground's relief, from its height a pixel's width either way.
    var nb = n;
    if (s.surface.y > 0.0 && g.sea < 0.5) {
        let e = max(ctx.fine * 1.5, 2e-6);
        let t = tangents(n);
        let h0 = relief(world, q, ctx);
        let h1 = relief(world, turn(s.body, normalize(p + t[0] * e)), ctx);
        let h2 = relief(world, turn(s.body, normalize(p + t[1] * e)), ctx);
        let grad = (t[0] * (h1 - h0) + t[1] * (h2 - h0)) / e;
        nb = normalize(n - grad * s.surface.y * RELIEF);
    }
    // The clouds, turning on their own, and their shadows on the ground.
    var cloud = 0.0;
    var shade = 1.0;
    if (g.cover < 1.5 && s.surface.x > 0.0) {
        cloud = clamp(w_clouds(turn(s.cloud, p), g.cover, ctx) * g.cloud_amount * s.surface.x, 0.0, 1.0);
        let lift = CLOUD_HEIGHT / max(dot(n, l), 0.1);
        let under = clamp(w_clouds(turn(s.cloud, normalize(p + l * lift)), g.cover, ctx) * g.cloud_amount * s.surface.x, 0.0, 1.0);
        shade = 1.0 - 0.7 * under;
    }
    // The sunlight here: past the other spheres, through the air.
    let world_p = s.centre.xyz + p * s.centre.w;
    let sunlight = frame.sun_light.rgb * eclipse(world_p, index);
    var sun = sunlight;
    var cloud_sun = sunlight;
    if (has_air) {
        sun *= sun_through_air(s, p, 1.0002);
        cloud_sun *= sun_through_air(s, p, 1.0 + CLOUD_HEIGHT);
    }
    let cos_i = dot(nb, l);
    let cos_g = dot(n, l);
    let day = smoothstep(-0.04, 0.06, cos_g);
    var lit: vec3<f32>;
    if (has_air) {
        lit = albedo * max(cos_i, 0.0);
    } else {
        lit = albedo * mix(lunar(cos_i, dot(nb, v)), max(cos_i, 0.0), s.look.x);
    }
    lit *= sun * shade * day;
    // The sea's glint (GGX, Schlick's Fresnel).
    if (g.shine > 0.0) {
        let h = normalize(l + v);
        let a2 = s.surface.z * s.surface.z;
        let ndh = max(dot(n, h), 0.0);
        let dd = ndh * ndh * (a2 - 1.0) + 1.0;
        let distribution = a2 / (PI * dd * dd);
        let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(h, v), 0.0), 5.0);
        lit += sun * shade * day * g.shine * PI * distribution * fresnel / (4.0 * max(cos_v, 0.05)) * step(0.0, cos_g);
    }
    // The clouds over it, lit softly round the terminator.
    let cloud_lit = vec3<f32>(0.92, 0.93, 0.95) * cloud_sun * clamp((cos_g + 0.08) / 1.08, 0.0, 1.0);
    var c = mix(lit, cloud_lit, cloud);
    // What glows of itself (lava, cities), brightest by night.
    let night = 1.0 - smoothstep(-0.15, 0.05, cos_g);
    c += srgb_linear(g.glow) * (0.45 + 0.55 * night) * (1.0 - 0.7 * cloud) * s.surface.w;
    // The light from everywhere, the other planets', and its own fill.
    c += mix(albedo, vec3<f32>(0.9), cloud) * (frame.ambient.rgb + planetshine(world_p, n, index) + vec3<f32>(s.look.z));
    // The scene's fill light, on the airless bodies.
    if (!has_air) {
        c += albedo * frame.fill_light.rgb * max(dot(nb, frame.fill.xyz), 0.0);
    }
    // The air between the eye and the ground.
    if (has_air) {
        let top = 1.0 + s.air.w;
        let half = sqrt(max(top * top - sr.r * sr.r, 0.0));
        let t0 = max(-sr.b - half, 0.0);
        let a = air_march(s, sr.o, sr.d, t0, t_hit);
        c = c * a.transmittance + a.inscatter;
    }
    return c;
}

@fragment
fn fs_sphere(in: SphereOut) -> SphereFrag {
    let s = spheres[in.index];
    let sr = sphere_ray(s, in.pos.xy);
    if (sr.b >= 0.0 || sr.cover <= 0.0 || dot(sr.o, sr.o) <= 1.0) {
        discard;
    }
    // The near side, or at the very edge the limb.
    var p: vec3<f32>;
    if (sr.r < 1.0) {
        p = sr.perp - sr.d * sqrt(1.0 - sr.r * sr.r);
    } else {
        p = sr.perp / sr.r;
    }
    var out: SphereFrag;
    out.colour = vec4<f32>(sphere_face(s, in.index, sr, p) * sr.cover, sr.cover);
    // At the edge the rest of the pixel is the air past the ground: its glow added, and what is
    // behind dimmed by its mean transmittance (the air's own passes leave such pixels alone, as
    // their tint would dim the ground's share too).
    if (sr.cover < 1.0 && (s.kind.z & 1u) != 0u) {
        let air = air_chord(s, sr);
        let through = dot(air.transmittance, vec3<f32>(1.0 / 3.0));
        out.colour = vec4<f32>(out.colour.rgb + air.inscatter * (1.0 - sr.cover), sr.cover + (1.0 - sr.cover) * (1.0 - through));
    }
    out.depth = depth_of(s.centre.xyz + p * s.centre.w);
    return out;
}

// The air along a ray that passes the ground by: its chord through the air.
fn air_chord(s: Sphere, sr: SphereRay) -> AirLight {
    let top = 1.0 + s.air.w;
    let half = sqrt(max(top * top - sr.r * sr.r, 0.0));
    let mid = -sr.b;
    return air_march(s, sr.o, sr.d, max(mid - half, 0.0), mid + half);
}

struct Halo {
    light: AirLight,
    // how much of the pixel is air (not the ground), and its depth
    weight: f32,
    depth: f32,
};

// The air round a sphere, past its edge: the ray's chord through it. (The edge's pixels, partly
// ground, are the sphere's own pass's.)
fn halo(in: SphereOut) -> Halo {
    let s = spheres[in.index];
    let sr = sphere_ray(s, in.pos.xy);
    let top = 1.0 + s.air.w;
    if (sr.r >= top || sr.cover > 0.0 || (sr.b >= 0.0 && dot(sr.o, sr.o) > top * top)) {
        discard;
    }
    var h: Halo;
    h.light = air_chord(s, sr);
    h.weight = 1.0;
    h.depth = depth_of(s.centre.xyz + (sr.o + sr.d * max(-sr.b, 0.0)) * s.centre.w);
    return h;
}

@fragment
fn fs_halo_tint(in: SphereOut) -> SphereFrag {
    let h = halo(in);
    var out: SphereFrag;
    out.colour = vec4<f32>(mix(vec3<f32>(1.0), h.light.transmittance, h.weight), 1.0);
    out.depth = h.depth;
    return out;
}

@fragment
fn fs_halo_glow(in: SphereOut) -> SphereFrag {
    let h = halo(in);
    var out: SphereFrag;
    out.colour = vec4<f32>(h.light.inscatter * h.weight, 0.0);
    out.depth = h.depth;
    return out;
}

// ---- The sun's glare ---------------------------------------------------------------------------

@group(1) @binding(0) var scene_depth: texture_depth_multisampled_2d;

struct GlareOut {
    @builtin(position) pos: vec4<f32>,
    // how much of the sun is seen (per channel, through the air), and where it is (pixels)
    @location(0) @interpolate(flat) seen: vec3<f32>,
    @location(1) @interpolate(flat) sun_at: vec2<f32>,
};

// How much of the sun's disc the eye sees: 32 points of it, each past the spheres (through their
// air) and the asteroids (the depth there).
fn sun_seen() -> vec3<f32> {
    let l = frame.sun.xyz;
    let t = tangents(l);
    let rad = tan(frame.sun.w);
    var seen = vec3<f32>(0.0);
    for (var i = 0u; i < 32u; i++) {
        let fi = f32(i) + 0.5;
        let r = sqrt(fi / 32.0) * rad;
        let a = fi * 2.3999632;
        let d = normalize(l + (t[0] * cos(a) + t[1] * sin(a)) * r);
        var through = vec3<f32>(1.0);
        for (var j = 0u; j < frame.counts.x; j++) {
            let s = spheres[j];
            let o = -s.centre.xyz / s.centre.w;
            let b = dot(o, d);
            if (b >= 0.0) {
                continue;
            }
            let low = length(o - b * d);
            if (low < 1.0) {
                through = vec3<f32>(0.0);
                break;
            }
            if ((s.kind.z & 1u) != 0u && low < 1.0 + s.air.w) {
                let h = low - 1.0;
                let tau = 2.0 * (s.air.rgb * exp(-h / s.haze.z) * sqrt(1.5707963 * low / s.haze.z) + vec3<f32>(s.haze.x * 1.11 * exp(-h / s.haze.w) * sqrt(1.5707963 * low / s.haze.w)));
                through *= exp(-tau);
            }
        }
        let at = pixel_of(d);
        if (at.z > 0.0 && all(at.xy >= vec2<f32>(0.0)) && all(at.xy < frame.screen.xy)) {
            if (textureLoad(scene_depth, vec2<i32>(at.xy), 0) > 0.0) {
                through = vec3<f32>(0.0);
            }
        }
        seen += through;
    }
    return seen / 32.0;
}

@vertex
fn vs_glare(@builtin(vertex_index) vi: u32) -> GlareOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: GlareOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.seen = sun_seen();
    out.sun_at = pixel_of(frame.sun.xyz).xy;
    return out;
}

// A lens's ghost of the sun: a soft disc at `at` (pixels), `size` of the picture's height across.
fn ghost(px: vec2<f32>, at: vec2<f32>, size: f32) -> f32 {
    let r = length(px - at) / (size * frame.screen.y);
    return smoothstep(1.0, 0.75, r) * (0.6 + 0.4 * smoothstep(0.5, 0.95, r));
}

@fragment
fn fs_glare(in: GlareOut) -> @location(0) vec4<f32> {
    if (max(in.seen.r, max(in.seen.g, in.seen.b)) < 1e-4) {
        discard;
    }
    let d = view_ray(in.pos.xy);
    let a = angle_between(d, frame.sun.xyz) / frame.sun.w;
    let h = frame.screen.y;
    let v = in.pos.xy - in.sun_at;
    let dist = length(v) / h;
    // How far the sun is off the picture (in its heights): the lens's own flare fades as it goes.
    let off = max(max(-in.sun_at.x, in.sun_at.x - frame.screen.x), max(-in.sun_at.y, in.sun_at.y - frame.screen.y)) / h;
    let in_view = smoothstep(0.25, 0.0, off);
    // Light scattered in the lens, falling away from the sun.
    let halo = 0.35 / (1.0 + a * a * 0.8) + (0.03 / (1.0 + dist * dist * 60.0) + 0.004 * exp(-dist * 2.0)) * in_view;
    // Streaks from the aperture's blades: thin lines through the sun (a pixel or two wide at 1080
    // lines), fading along their length.
    let width = 1.2 * h / 1080.0;
    var streak = 0.0;
    for (var k = 0; k < 3; k++) {
        let angle = 0.4 + f32(k) * 1.0471976;
        let across = abs(dot(v, vec2<f32>(-sin(angle), cos(angle))));
        streak += exp(-across * across / (width * width)) * (1.0 - 0.35 * f32(k % 2));
    }
    let fall = 1.0 + dist * 16.0;
    streak *= 0.06 / (fall * fall) * smoothstep(0.0, 0.01, dist) * in_view;
    var c = vec3<f32>(halo + streak) * frame.stars.z;
    // Ghosts along the line from the sun through the middle of the picture.
    let mid = frame.screen.xy * 0.5;
    let axis = in.sun_at - mid;
    let fade = smoothstep(1.4, 0.4, length(axis) / h);
    var gh = vec3<f32>(0.0);
    gh += vec3<f32>(1.0, 0.55, 0.25) * ghost(in.pos.xy, mid - axis * 0.45, 0.05) * 0.010;
    gh += vec3<f32>(0.35, 1.0, 0.55) * ghost(in.pos.xy, mid - axis * 0.8, 0.09) * 0.006;
    gh += vec3<f32>(0.45, 0.6, 1.0) * ghost(in.pos.xy, mid - axis * 1.25, 0.14) * 0.005;
    gh += vec3<f32>(0.9, 0.5, 1.0) * ghost(in.pos.xy, mid + axis * 0.3, 0.03) * 0.012;
    c += gh * fade * in_view * frame.stars.w;
    return vec4<f32>(frame.sun_light.rgb * in.seen * c, 0.0);
}
