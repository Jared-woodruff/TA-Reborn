// The cockpit round a Perspective view (cockpit.rs): drawn in the eye's own space (x right,
// y up, looking along -z), lit as an interior. The sky's light comes in dimmed, the sun only
// through the openings, onto what faces it; a cabin lamp over the eye, every instrument's glow
// on what is round it, and in danger a red alarm light.
//
// It is a crew's station after years in the field: painted steel (the side's own paint, a black
// enamel, hazard stripes), satin to matte, chipped to red primer and to bare steel along its
// edges and where hands go, scratched, grime and soot in its seams and corners, dust on what
// faces up; bare steel and brass dull and darkened, rusting in the crevices; rubber and padding
// worn smooth where held; stencils and plates painted on (the decal atlas), wearing with the
// paint under them. The model's vertices carry the masks that place all this (build.py):
// occlusion, the distance to the nearest edge, occlusion close in, where hands go; the wear
// texture gives it its shapes (chips, scratches, stains, grain). A closed cab reflects itself,
// dark, not the sky: only what looks out through the window sees the world in its gloss.
//
// The instruments are pictures on their glass, from the atlas the game paints: lit displays
// under the glass's reflections.

struct Uniform {
    proj: mat4x4<f32>,
    mount: mat4x4<f32>,
    // xyz towards the sun (eye space), w how much of it gets in.
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    trim: vec4<f32>,
    cabin: vec4<f32>,
    glow: vec4<f32>,
    // Seconds, alarm (0..1), a hit's flash (0..1), lamps on (0..1).
    params: vec4<f32>,
    // How hard it rains (0..1), and whether it flies (1: drops are blown back over a canopy).
    weather: vec4<f32>,
    // The side's paint (linear); w: how many screen lights there are.
    paint: vec4<f32>,
    // The instruments' glow as lights, two each: their middle (eye space) and how far their
    // light reaches; the way they face and how bright they are.
    lights: array<vec4<f32>, 16>,
};

struct Material {
    base_rough: vec4<f32>,
    emit_metal: vec4<f32>,
    // Its role, how much it wears and gathers grime, 1 where it takes the side's paint.
    role: vec4<f32>,
};

struct Screen {
    centre: vec4<f32>,
    // Half extents across and up; w: one over their length squared.
    right: vec4<f32>,
    up: vec4<f32>,
    // The atlas region: corner and size (0..1).
    region: vec4<f32>,
    // The region's height and width in pixels.
    info: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var<storage, read> parts: array<mat4x4<f32>>;
@group(0) @binding(2) var<storage, read> mats: array<Material>;
@group(0) @binding(3) var atlas: texture_2d<f32>;
@group(0) @binding(4) var atlas_sampler: sampler;
@group(0) @binding(5) var<storage, read> screens: array<Screen>;
@group(0) @binding(6) var wear_tex: texture_2d<f32>;
@group(0) @binding(7) var decal_tex: texture_2d<f32>;
@group(0) @binding(8) var wrap_sampler: sampler;
@group(0) @binding(9) var clamp_sampler: sampler;

// How bright a lit display's phosphor is (the atlas's colours, linear).
const PHOSPHOR_GAIN: f32 = 2.2;
const PI: f32 = 3.14159265;
// The edge mask's reach (metres at its 1: build.py's EDGE_REACH).
const EDGE_REACH: f32 = 0.03;
// The wear texture's tiles per metre: fine (its chips, scratches and grain), coarse (stains).
const TILE: f32 = 5.6;
const TILE_COARSE: f32 = 1.3;
// The cabin lamp, over and behind the eye.
const CABIN_LAMP: vec3<f32> = vec3<f32>(0.0, 0.3, 0.18);
// Roles (cockpit.rs's `Role`).
const SURFACE: u32 = 0u;
const TRIM: u32 = 1u;
const SCREEN: u32 = 2u;
const LAMP: u32 = 3u;
const ALARM: u32 = 4u;
const GLASS: u32 = 5u;
const TRIM_GLOW: u32 = 6u;
const METAL: u32 = 7u;
const RUBBER: u32 = 8u;
const FABRIC: u32 = 9u;
const DECAL: u32 = 10u;
const CONCRETE: u32 = 11u;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) masks: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) material: u32,
    @location(5) part: u32,
};

// (The normal and the masks at the triangle's centroid: multisampled, a pixel whose centre
// lies off a thin triangle (a bevel seen edge-on) would carry them on past its corners.)
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) eye_pos: vec3<f32>,
    @location(1) @interpolate(perspective, centroid) normal: vec3<f32>,
    @location(2) @interpolate(perspective, centroid) masks: vec4<f32>,
    @location(3) @interpolate(flat) material: u32,
    // The point and its normal in its part's own space (where the wear's patterns stay put).
    @location(4) local: vec3<f32>,
    @location(5) local_n: vec3<f32>,
    @location(6) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VIn) -> VOut {
    let m = u.mount * parts[in.part];
    let p = m * vec4<f32>(in.pos, 1.0);
    var out: VOut;
    out.clip = u.proj * p;
    out.eye_pos = p.xyz;
    out.normal = (m * vec4<f32>(in.normal, 0.0)).xyz;
    out.masks = in.masks;
    out.material = in.material;
    out.local = in.pos;
    out.local_n = in.normal;
    out.uv = in.uv;
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let d = q + dot(q, q.yxz + 33.33);
    return fract((d.x + d.y) * d.z);
}

fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let s = f * f * (3.0 - 2.0 * f);
    let n000 = hash3(i);
    let n100 = hash3(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash3(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash3(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash3(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash3(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash3(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash3(i + vec3<f32>(1.0, 1.0, 1.0));
    return mix(mix(mix(n000, n100, s.x), mix(n010, n110, s.x), s.y), mix(mix(n001, n101, s.x), mix(n011, n111, s.x), s.y), s.z);
}

// A light at `pos` (eye space) of colour `c`: its diffuse and a highlight, falling off with
// distance (`reach`: the distance at which it is at half strength). (The instruments' glass and
// the windows' are lit by this, as they always were.)
fn point_light(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, pos: vec3<f32>, c: vec3<f32>, reach: f32, shine: f32) -> vec2<f32> {
    let d = pos - p;
    let dist = length(d);
    let l = d / max(dist, 1e-4);
    let fall = 1.0 / (1.0 + (dist / reach) * (dist / reach));
    let diff = max(dot(n, l), 0.0) * fall;
    let h = normalize(l + v);
    // (A lamp an inch from a curved rim would blaze: highlights are held to a few times the
    // light.)
    let spec = min(pow(max(dot(n, h), 0.0), shine) * (shine + 2.0) / 8.0, 3.0) * fall * step(0.0, dot(n, l));
    return vec2<f32>(diff, spec);
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Smith's visibility, with the 1 / (4 n·l n·v) folded in.
fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let k = a * 0.5;
    return 0.25 / ((nv * (1.0 - k) + k) * (nl * (1.0 - k) + k));
}

// What a light from `l` reflects towards `v` (times the light, as the diffuse is albedo times
// the light), held to a few times the light (a lamp an inch from a bolt's rim would blaze).
fn gloss(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a: f32, f0: vec3<f32>) -> vec3<f32> {
    let h = normalize(l + v);
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 1e-3);
    let f = f0 + (1.0 - f0) * pow(1.0 - max(dot(v, h), 0.0), 5.0);
    return min(d_ggx(max(dot(n, h), 0.0), a) * v_smith(nv, nl, a) * nl * PI, 6.0) * f;
}

fn hash2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

// Rain on a pane: what of a drop is at a point. `cover`: how much (its body); `lens`: where in
// it (-1 its top to 1 its foot: a drop shows the world upside down, the sky at its foot);
// `rim`: its dark edge; `glint`: the light caught on it; `trail`: the wet line a running drop
// leaves.
struct Drops {
    cover: f32,
    lens: f32,
    rim: f32,
    glint: f32,
    trail: f32,
};

// A drop `rad` across at `centre`, seen from `f` (any one unit for all three), added to `d` where
// it is the nearer drop's (as far as it is there: `present`).
fn add_drop(d: Drops, f: vec2<f32>, centre: vec2<f32>, rad: f32, present: f32) -> Drops {
    var o = d;
    let q = (f - centre) / rad;
    let r = length(q * vec2<f32>(1.0, 1.12));
    let cover = (1.0 - smoothstep(0.78, 1.0, r)) * present;
    if (cover > o.cover) {
        o.cover = cover;
        o.lens = clamp(-q.y, -1.0, 1.0);
        o.rim = (smoothstep(0.62, 0.92, r) - smoothstep(0.92, 1.04, r)) * present;
        o.glint = (1.0 - smoothstep(0.0, 0.26, length(q - vec2<f32>(-0.32, 0.4)))) * present;
    }
    return o;
}

// Rain on a pane at `p` (metres across and up): drops clinging, a few large and more small (more
// as it rains harder), each coming and going; and others running down their column, a trail
// behind (up and out across a canopy in flight, blown back: `flies`).
fn raindrops(p: vec2<f32>, t: f32, rain: f32, flies: f32) -> Drops {
    var d = Drops(0.0, 0.0, 0.0, 0.0, 0.0);
    for (var layer = 0; layer < 2; layer++) {
        let large = layer == 0;
        let scale = select(85.0, 34.0, large);
        let q = p * scale + f32(layer) * 0.5;
        let cell = floor(q);
        let h = hash2(cell + f32(layer) * 31.0);
        let life = fract(t / (5.0 + h.y * 7.0) + h.x * 3.7);
        let present = step(h.x, rain * select(0.3, 0.16, large)) * smoothstep(0.0, 0.1, life) * (1.0 - smoothstep(0.8, 0.9, life));
        let centre = (hash2(cell + 17.0 + f32(layer)) - 0.5) * 0.4;
        let rad = select(0.1 + 0.16 * h.y, 0.16 + 0.2 * h.y, large);
        d = add_drop(d, fract(q) - 0.5, centre, rad, present);
    }
    let cols = 24.0;
    let x = select(p.x, abs(p.x), flies > 0.5);
    let c = floor(x * cols);
    let hc = hash2(vec2<f32>(c, 7.0));
    if (hc.x < rain * 0.4) {
        let span = 1.4;
        let speed = 0.05 + 0.12 * hc.y;
        let dir = select(-1.0, 1.0, flies > 0.5);
        let y0 = (fract(hc.y * 13.7 + t * speed / span) - 0.5) * span * dir;
        let dx = (fract(x * cols) - 0.5) / cols;
        let behind = (y0 - p.y) * dir;
        let r = 0.0035 + 0.003 * hc.y;
        d = add_drop(d, vec2<f32>(dx, p.y - y0), vec2<f32>(0.0), r, 1.0);
        d.trail = max(d.trail, smoothstep(r * 0.45, r * 0.1, abs(dx)) * step(0.0, behind) * (1.0 - smoothstep(0.0, 0.1, behind)));
    }
    return d;
}

// Where a point of a screen's glass is on its picture: 0..1 across (left to right) and down.
fn on_glass(s: Screen, local: vec3<f32>) -> vec2<f32> {
    let d = local - s.centre.xyz;
    let a = dot(d, s.right.xyz) * s.right.w;
    let b = dot(d, s.up.xyz) * s.up.w;
    return vec2<f32>(a * 0.5 + 0.5, 0.5 - b * 0.5);
}

// The wear texture is laid on each face along the axis it most nearly looks along (boxes,
// plates and consoles have faces that do): which axis, and a vector's two other components.
fn axis_of(n: vec3<f32>) -> u32 {
    let a = abs(n);
    if (a.x >= a.y && a.x >= a.z) {
        return 0u;
    }
    if (a.y >= a.z) {
        return 1u;
    }
    return 2u;
}

fn across(v: vec3<f32>, axis: u32) -> vec2<f32> {
    if (axis == 0u) {
        return v.zy;
    }
    if (axis == 1u) {
        return v.xz;
    }
    return v.xy;
}

// A surface as the light finds it.
struct Surf {
    albedo: vec3<f32>,
    rough: f32,
    metal: f32,
    // How far the surface stands out of its face (metres: chips and scratches sink into the
    // paint), for the bump.
    height: f32,
    // How much of it there is (a decal's paint, chipped).
    alpha: f32,
};

// What wears a painted surface (shared by its decals, which wear with the paint under them).
struct Wear {
    // Chipped through the paint (to primer), and on through the primer (to bare steel).
    chip: f32,
    bare: f32,
    scratch: f32,
    // The edge's band (1 on it), and where the paint is rubbed thin, broader.
    edge: f32,
    rim: f32,
    grime: f32,
    dust: f32,
};

// m: the vertex masks (occlusion, edge distance, occlusion close in, hands); d1, d2: the wear
// texture fine and coarse; streak: its stains drawn down a face; up: how much the face looks up.
fn wear_of(wear: f32, grime_amt: f32, m: vec4<f32>, d1: vec4<f32>, d2: vec4<f32>, streak: f32, up: f32) -> Wear {
    var w: Wear;
    let hands = m.w;
    let edge_d = m.y * EDGE_REACH;
    w.edge = exp(-edge_d / 0.0032);
    w.rim = exp(-edge_d / 0.01);
    // Chips where the chip field rises over a threshold that falls on edges and where hands go
    // (a few anywhere: knocks); only the cores of the big ones go through to the steel.
    let exposure = clamp((w.edge * 0.5 + hands * 0.28 + 0.004) * wear, 0.0, 0.8);
    let t = 1.0 - exposure;
    w.chip = smoothstep(t - 0.008, t + 0.008, d1.r);
    w.bare = smoothstep(t + 0.1, t + 0.13, d1.r);
    // Scratches: most where hands go and on what faces up, a few anywhere.
    w.scratch = smoothstep(0.45, 0.85, d1.g) * clamp(0.1 + hands * 0.8 + up * 0.2, 0.0, 1.0) * min(wear, 1.0);
    // Grime and soot: thick in the seams and crevices, some in the cabin's corners, run down
    // the faces in streaks; wiped off where hands go.
    let crevice = 1.0 - m.z;
    let corner = 1.0 - m.x;
    let g = (crevice * 1.15 + corner * 0.5 + streak * 0.25) * (0.5 + 0.95 * d1.b) * grime_amt * (1.0 - hands * 0.65);
    w.grime = clamp(g, 0.0, 1.0);
    // Dust on what faces up, in drifts, where nothing wipes it.
    w.dust = smoothstep(0.5, 0.92, up) * smoothstep(0.15, 0.7, d2.b * 0.6 + d1.a * 0.4) * (1.0 - hands) * 0.75 * min(grime_amt, 1.0);
    return w;
}

// Painted steel: the paint (mottled, rubbed thin and lighter along its edges, polished where
// hands go), red primer in its chips, bare steel in the deep ones and in its scratches; then
// the grime and the dust over it all.
fn painted(base: vec3<f32>, rough0: f32, w: Wear, hands: f32, d1: vec4<f32>, d2: vec4<f32>) -> Surf {
    var s: Surf;
    var col = base * (0.86 + 0.28 * d2.b) * (0.96 + 0.08 * d1.a);
    col = mix(col, col * 1.3 + vec3<f32>(0.008), w.rim * 0.25);
    col *= 1.0 + hands * 0.1;
    var rough = rough0 + (d1.a - 0.5) * 0.08 - hands * 0.14;
    let primer = vec3<f32>(0.075, 0.034, 0.02) * (0.85 + 0.3 * d1.a);
    let steel = vec3<f32>(0.2, 0.2, 0.195) * (0.82 + 0.36 * d1.a);
    col = mix(col, primer, w.chip);
    rough = mix(rough, 0.82, w.chip);
    let metal_part = max(w.bare, w.scratch * (1.0 - w.chip));
    col = mix(col, steel, metal_part);
    rough = mix(rough, 0.48 + 0.16 * d1.a, metal_part);
    var metal = metal_part;
    // Grime and soot, then dust.
    col = mix(col, vec3<f32>(0.016, 0.014, 0.012), w.grime * 0.88);
    rough = mix(rough, 0.92, w.grime);
    metal *= 1.0 - w.grime;
    col = mix(col, vec3<f32>(0.15, 0.135, 0.11), w.dust * 0.55);
    rough = mix(rough, 0.97, w.dust);
    metal *= 1.0 - w.dust;
    s.albedo = col;
    s.rough = clamp(rough, 0.2, 1.0);
    s.metal = metal;
    s.height = -(w.chip * 0.7 + w.bare * 0.3) * 0.00012 - w.scratch * 0.00005 + (d1.a - 0.5) * 0.000008;
    s.alpha = 1.0;
    return s;
}

// Bare metal (steel, brass, a weld): dull, darker and rougher in places, polished where hands
// go and along its edges, scratched brighter; tarnished and rusting in the crevices; grime and
// dust over it.
fn bare_metal(base: vec3<f32>, rough0: f32, w: Wear, m: vec4<f32>, d1: vec4<f32>, d2: vec4<f32>) -> Surf {
    var s: Surf;
    let hands = m.w;
    var col = base * (0.78 + 0.38 * d2.b) * (0.9 + 0.2 * d1.a);
    var rough = rough0 + (d1.a - 0.5) * 0.16 + (d2.b - 0.5) * 0.1;
    rough = mix(rough, 0.38, hands * 0.7 + w.edge * 0.2);
    col *= 1.0 + hands * 0.2 + w.edge * 0.12;
    let scr = smoothstep(0.35, 0.75, d1.g) * (0.35 + hands);
    col = mix(col, col * 1.3, min(scr, 1.0));
    rough = mix(rough, 0.3, min(scr, 1.0));
    var metal = 1.0;
    let crevice = 1.0 - m.z;
    let rust = smoothstep(0.5, 0.9, crevice * (0.35 + 0.9 * d1.b)) * (1.0 - hands);
    col = mix(col, vec3<f32>(0.09, 0.04, 0.02) * (0.8 + 0.4 * d1.a), rust);
    rough = mix(rough, 0.9, rust);
    metal *= 1.0 - rust;
    col = mix(col, vec3<f32>(0.02, 0.018, 0.015), w.grime * 0.8);
    rough = mix(rough, 0.9, w.grime);
    metal *= 1.0 - w.grime * 0.9;
    col = mix(col, vec3<f32>(0.15, 0.135, 0.11), w.dust * 0.5);
    rough = mix(rough, 0.97, w.dust);
    metal *= 1.0 - w.dust;
    s.albedo = col;
    s.rough = clamp(rough, 0.2, 1.0);
    s.metal = metal;
    s.height = -scr * 0.00003 + (d1.a - 0.5) * 0.00001;
    s.alpha = 1.0;
    return s;
}

// Rubber and padding: matte, worn smooth and a little shiny where held, scuffed grey on its
// edges, dusty on top.
fn rubber(base: vec3<f32>, rough0: f32, w: Wear, hands: f32, d1: vec4<f32>, d2: vec4<f32>) -> Surf {
    var s: Surf;
    var col = base * (0.85 + 0.3 * d2.b) * (0.92 + 0.16 * d1.a);
    var rough = rough0 + (d1.a - 0.5) * 0.12;
    rough = mix(rough, rough0 * 0.6, hands);
    col = mix(col, vec3<f32>(0.055, 0.053, 0.05), w.edge * 0.45 + smoothstep(0.5, 0.8, d1.g) * 0.3);
    col = mix(col, vec3<f32>(0.012, 0.011, 0.01), w.grime * 0.6);
    col = mix(col, vec3<f32>(0.14, 0.13, 0.11), w.dust * 0.45);
    rough = mix(rough, 0.97, w.dust);
    s.albedo = col;
    s.rough = clamp(rough, 0.25, 1.0);
    s.metal = 0.0;
    s.height = (d1.a - 0.5) * 0.00003;
    s.alpha = 1.0;
    return s;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let mat = mats[in.material];
    let role = u32(mat.role.x + 0.5);
    // (Every fragment works out what needs a neighbour's value before anything branches: the
    // picture's footprint on an instrument's glass, the wear texture's, the decal's, the bump.)
    let scr = screens[in.material];
    let glass_uv = on_glass(scr, in.local);
    let atlas_uv = scr.region.xy + glass_uv * scr.region.zw;
    let fx = dpdx(atlas_uv);
    let fy = dpdy(atlas_uv);
    // Scan lines on the glass, one every 1.6 mm, and how many of them a pixel spans.
    let rows = glass_uv.y * 2.0 / sqrt(max(scr.up.w, 1e-6)) / 0.0016;
    let rows_per_px = fwidth(rows);
    // The wear texture along the face's axis (with the gradients of the place itself, smooth
    // where the axis changes), fine and coarse; and its stains stretched down the face.
    let axis = axis_of(in.local_n);
    let place = across(in.local, axis);
    let gx = across(dpdx(in.local), axis);
    let gy = across(dpdy(in.local), axis);
    let d1 = textureSampleGrad(wear_tex, wrap_sampler, place * TILE, gx * TILE, gy * TILE);
    let d2 = textureSampleGrad(wear_tex, wrap_sampler, place * TILE_COARSE + vec2<f32>(0.37, 0.61), gx * TILE_COARSE, gy * TILE_COARSE);
    // (Read blurred, a few mips down: runs, not hairlines.)
    let stretch = vec2<f32>(TILE * 1.1, TILE * 0.12);
    let d3 = textureSampleGrad(wear_tex, wrap_sampler, place * stretch + vec2<f32>(0.11, 0.29), gx * stretch * 6.0, gy * stretch * 6.0);
    let picture = textureSample(decal_tex, clamp_sampler, in.uv);
    // How fast the normal turns across a pixel: a glint narrower than a pixel only shimmers, so
    // the gloss is widened to what a pixel shows.
    let turn = fwidth(in.normal);
    let widen = min(2.0 * dot(turn, turn), 0.25);
    var base = mat.base_rough.rgb;
    var rough = mat.base_rough.a;
    var metal = mat.emit_metal.a;
    let emit = mat.emit_metal.rgb;
    let t = u.params.x;
    let alarm = u.params.y;
    let flash = u.params.z;
    let lamps = u.params.w;
    if (role == TRIM || role == TRIM_GLOW) {
        base = u.trim.rgb;
    }
    if (mat.role.w > 0.5) {
        base = u.paint.rgb;
    }
    let v = normalize(-in.eye_pos);
    var n = normalize(in.normal);
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    let masks = clamp(in.masks, vec4<f32>(0.0), vec4<f32>(1.0));
    let up = n.y;
    // Streaks run down what stands upright.
    let streak = smoothstep(0.55, 0.85, d3.b) * smoothstep(0.4, 0.8, d2.b) * (1.0 - abs(up));
    let w = wear_of(mat.role.y, mat.role.z, masks, d1, d2, streak, up);
    var s = Surf(base, rough, metal, 0.0, 1.0);
    if (role == SURFACE || role == TRIM || role == DECAL) {
        s = painted(base, rough, w, masks.w, d1, d2);
        if (role == DECAL) {
            // The decal's own paint, as worn as the paint under it: gone where that is chipped
            // or scratched through, dulled by the grime and the dust.
            let under = painted(picture.rgb, rough, w, masks.w, d1, d2);
            s.albedo = under.albedo;
            s.alpha = picture.a * (1.0 - w.chip * 0.9) * (1.0 - w.scratch * 0.7) * (0.9 + 0.1 * d1.a);
        }
    } else if (role == METAL) {
        s = bare_metal(base, rough, w, masks, d1, d2);
    } else if (role == RUBBER) {
        s = rubber(base, rough, w, masks.w, d1, d2);
    } else if (role == FABRIC) {
        s = rubber(base * (0.8 + 0.4 * d1.a), rough, w, masks.w, d1, d2);
        s.rough = 0.97;
    } else if (role == CONCRETE) {
        var c = base * (0.72 + 0.5 * d2.b) * (0.84 + 0.32 * d1.a);
        c *= 1.0 - smoothstep(0.86, 0.95, d1.r) * 0.45;
        c = mix(c, c * 1.3 + vec3<f32>(0.02), w.chip * 0.6);
        c = mix(c, vec3<f32>(0.03, 0.028, 0.025), w.grime * 0.75);
        c = mix(c, vec3<f32>(0.16, 0.15, 0.13), w.dust * 0.5);
        s = Surf(c, 0.95, 0.0, -smoothstep(0.86, 0.95, d1.r) * 0.0003, 1.0);
    }
    // The bump: chips and scratches sink into the paint, the grain stands in it (a height's
    // slope across the pixel tilts the normal: no tangents needed).
    let dpx = dpdx(in.eye_pos);
    let dpy = dpdy(in.eye_pos);
    let dhx = dpdx(s.height);
    let dhy = dpdy(s.height);
    let r1 = cross(dpy, n);
    let r2 = cross(n, dpx);
    let det = dot(dpx, r1);
    let grad = sign(det) * (dhx * r1 + dhy * r2);
    let bumped = normalize(abs(det) * n - grad);
    let nb = select(n, bumped, abs(det) > 1e-12 && (role == SURFACE || role == TRIM || role == DECAL || role == METAL || role == RUBBER || role == CONCRETE));
    let ao = mix(0.25, 1.0, masks.x);
    let a = clamp(sqrt(s.rough * s.rough * s.rough * s.rough + widen), 0.02, 1.0);
    let f0 = mix(vec3<f32>(0.04), s.albedo, s.metal);
    // The sky's light, dim inside, less in the corners.
    let hemi = mix(u.ground.rgb, u.sky.rgb, n.y * 0.5 + 0.5) * 0.42;
    // The sun, on what faces it, as much as gets in: through the openings ahead, little into
    // the corners.
    let sun_l = u.sun_dir.xyz;
    let through = mix(0.3, 1.0, smoothstep(-0.25, 0.45, -sun_l.z)) * u.sun_dir.w * mix(0.35, 1.0, masks.x * masks.x);
    let sun = u.sun_color.rgb * max(dot(nb, sun_l), 0.0) * through;
    var diffuse = hemi * ao + sun;
    var spec = u.sun_color.rgb * through * gloss(nb, v, sun_l, a, f0);
    // The cabin lamp over the eye.
    let to_lamp = CABIN_LAMP - in.eye_pos;
    let lamp_d = length(to_lamp);
    let lamp_l = to_lamp / max(lamp_d, 1e-4);
    let lamp_fall = 1.0 / (1.0 + (lamp_d / 0.55) * (lamp_d / 0.55)) * mix(0.55, 1.0, masks.x);
    diffuse += u.cabin.rgb * max(dot(nb, lamp_l), 0.0) * lamp_fall;
    spec += u.cabin.rgb * lamp_fall * gloss(nb, v, lamp_l, a, f0);
    // Every instrument's glow on what is round it, most on what it faces.
    let count = u32(u.paint.w + 0.5);
    for (var k = 0u; k < count; k++) {
        let at = u.lights[k * 2u];
        let face = u.lights[k * 2u + 1u];
        let d = at.xyz - in.eye_pos;
        let dist = length(d);
        let l = d / max(dist, 1e-4);
        let lobe = mix(0.35, 1.0, max(dot(face.xyz, -l), 0.0));
        let fall = face.w * lobe / (1.0 + (dist / at.w) * (dist / at.w));
        diffuse += u.glow.rgb * max(dot(nb, l), 0.0) * fall;
        spec += u.glow.rgb * fall * gloss(nb, v, l, a, f0) * 0.5;
    }
    // Alarm: a red wash, pulsing; a hit's flash.
    let pulse = 0.5 + 0.5 * sin(t * 7.0);
    let red = vec3<f32>(1.0, 0.08, 0.04) * (alarm * pulse * 0.5 + flash * 1.4);
    diffuse += red;
    // What the gloss shows of all round it: the cab, dark, except where it looks out of the
    // window; rough surfaces see the cab's average.
    let rr = reflect(-v, nb);
    let window = smoothstep(0.25, 0.7, -rr.z) * smoothstep(-0.35, -0.06, rr.y) * (1.0 - smoothstep(0.38, 0.75, rr.y));
    let outside = mix(u.ground.rgb * 0.6, u.sky.rgb, smoothstep(-0.1, 0.2, rr.y));
    let inside = u.sky.rgb * 0.05 + u.ground.rgb * 0.035 + u.cabin.rgb * 0.06;
    let env = mix(mix(inside, outside, window), inside * 0.8 + outside * 0.12, s.rough);
    let nv = max(dot(nb, v), 1e-3);
    let fresnel = f0 + (max(vec3<f32>(1.0 - s.rough), f0) - f0) * pow(1.0 - nv, 5.0);
    spec += env * fresnel * ao + red * 0.3 * f0;
    var color = s.albedo * (1.0 - s.metal) * diffuse * ao + spec;
    // The instruments' glass and the windows' are lit as they always were (their own gloss).
    let cab = point_light(in.eye_pos, n, v, CABIN_LAMP, u.cabin.rgb, 0.55, 160.0);
    if (role == SCREEN) {
        // An instrument's glass: dark, a faint glow of its phosphor, the lamp's reflection; and
        // behind it the instrument's picture, lit. (Four taps across the footprint: seen at an
        // angle the lettering stays whole.)
        let fres = pow(1.0 - max(dot(n, v), 0.0), 5.0);
        var ink = vec4<f32>(0.0);
        if (scr.centre.w > 0.5) {
            let lo = scr.region.xy;
            let hi = scr.region.xy + scr.region.zw;
            for (var k = 0; k < 4; k++) {
                let o = vec2<f32>(f32(k & 1) - 0.5, f32(k >> 1u) - 0.5) * 0.5;
                let q = clamp(atlas_uv + fx * o.x + fy * o.y, lo, hi);
                ink += textureSampleLevel(atlas, atlas_sampler, q, 0.0) * 0.25;
            }
        }
        // Scan lines, fading out where they would be finer than two of the screen's pixels.
        let scan = 1.0 - 0.2 * (0.5 + 0.5 * cos(rows * 6.2831853)) * (1.0 - smoothstep(0.3, 0.5, rows_per_px));
        // The tube's edge darker, as on a curved face.
        let edge = glass_uv * (1.0 - glass_uv);
        let vignette = clamp(pow(edge.x * edge.y * 16.0, 0.25), 0.0, 1.0);
        let lit = ink.rgb * PHOSPHOR_GAIN * lamps * scan * mix(0.7, 1.0, vignette);
        color = vec3<f32>(0.002, 0.007, 0.004) + emit * 0.04 * lamps + lit + u.cabin.rgb * cab.y * 0.08 + (u.sky.rgb * 0.1 + hemi * 0.05) * fres;
    } else if (role == LAMP) {
        color += emit * lamps * (0.9 + 0.1 * sin(t * 2.3 + in.local.x * 31.0));
    } else if (role == ALARM) {
        let blink = step(0.5, fract(t * 2.2));
        color += mix(emit, vec3<f32>(5.0, 0.35, 0.12) * blink, alarm) * lamps;
    } else if (role == TRIM_GLOW) {
        // The side's colour, glowing (as bright as the material's own glow).
        let strength = max(max(emit.r, emit.g), emit.b);
        color += u.trim.rgb * strength * 1.6 * lamps;
    } else if (role == GLASS) {
        // The openings' glass: faint, more at a glance along it; dust and fine scratches that
        // catch the sun and the cabin lamp; and in the rain, drops on it, lensing the sky.
        let fres = pow(1.0 - max(dot(n, v), 0.0), 3.0);
        let sun_spec = min(pow(max(dot(n, normalize(u.sun_dir.xyz + v)), 0.0), 160.0) * (162.0) / 8.0, 4.0) * step(0.0, dot(n, u.sun_dir.xyz));
        let glint = min(sun_spec * u.sun_dir.w + cab.y * 0.5, 2.0);
        let dust = smoothstep(0.8, 0.96, noise3(in.local * 230.0)) * 0.6 + smoothstep(0.93, 0.99, noise3(in.local * vec3<f32>(900.0, 30.0, 900.0))) * 0.4;
        var col = u.sky.rgb * 0.15 + u.cabin.rgb * cab.y * 0.2 + vec3<f32>(0.55) * dust * (0.12 + glint);
        var alpha = 0.03 + fres * 0.26 + dust * (0.04 + 0.2 * min(glint, 1.0));
        let rain = u.weather.x;
        if (rain > 0.01) {
            // A drop shows the world upside down (the sky at its foot, the ground at its top),
            // a dark rim round it and the light caught on it; a running one's trail is wet.
            let d = raindrops(in.local.xy, t, rain, u.weather.y);
            let world = mix(u.ground.rgb * 0.6, u.sky.rgb * 0.95, 0.5 + 0.5 * d.lens);
            col = mix(col, world, d.cover * 0.75);
            col *= 1.0 - d.rim * 0.45;
            col += vec3<f32>(1.0) * d.glint * (0.6 + glint) + u.sky.rgb * 0.15 * d.trail;
            alpha = max(alpha, d.cover * 0.2 + d.rim * 0.28 + d.glint * 0.75 + d.trail * 0.08);
        }
        return vec4<f32>(col, alpha);
    }
    return vec4<f32>(color, s.alpha);
}
