// Liveries (ta_skins::Livery; model/skin.rs `LiveryUniform`): paint laid over a unit's own,
// fixed to each of its pieces (in the piece's space, so it moves and turns with it). Prepended
// to unit.wgsl and raytrace.wgsl, which both paint with `livery_paint`; the raster one adds the
// trim, the wear and the grime, which need the face's edges.

struct Livery {
    // Its colours (linear; w: 1 for each it has).
    colours: array<vec4<f32>, 3>,
    // Along the convex edges (linear; w: 1 with one).
    trim: vec4<f32>,
    // The colours as they blend seen from afar.
    average: vec4<f32>,
    // x: the pattern (ta_skins::Pattern's order), y: its frequency (1 / scale), z: sharpness,
    // w: the number of colours.
    pattern: vec4<f32>,
    // Where in the noise its seed puts it.
    offset: vec4<f32>,
    // x: how much of the paint under it it covers, y: how much of that paint's light and dark
    // shows through, z: roughness, w: metallic.
    finish: vec4<f32>,
    // x: wear (0.5: Reborn's own), y: grime, z: 1 when laid over TA's own paint as TA drew it
    // (unit.wgsl and raytrace.wgsl sample it in place of Reborn's), w: fade.
    weather: vec4<f32>,
};

// The own bank's, then the enemy bank's.
struct Liveries {
    banks: array<Livery, 2>,
};

// A pattern's unit in model units at scale 1: patches about a third of a light tank.
const LIVERY_PERIOD: f32 = 16.0;

fn livery_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn livery_bits(c: vec3<i32>) -> u32 {
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u) ^ (bitcast<u32>(c.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    return h ^ (h >> 16u);
}

fn livery_hash(c: vec3<i32>) -> f32 {
    return f32(livery_bits(c) >> 8u) / 16777216.0;
}

// A point in the cell (each coordinate 0..1).
fn livery_point(c: vec3<i32>) -> vec3<f32> {
    let h = livery_bits(c);
    return vec3<f32>(f32(h & 1023u), f32((h >> 10u) & 1023u), f32((h >> 20u) & 1023u)) / 1023.0;
}

// Gradient noise, about 0..1 (most of it 0.3..0.7).
fn livery_noise(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let i = vec3<i32>(cell);
    let f = p - cell;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var corners: array<f32, 8>;
    for (var k = 0; k < 8; k++) {
        let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let g = livery_point(i + o) * 2.0 - 1.0;
        corners[k] = dot(g, f - vec3<f32>(o));
    }
    let x0 = mix(corners[0], corners[1], u.x);
    let x1 = mix(corners[2], corners[3], u.x);
    let x2 = mix(corners[4], corners[5], u.x);
    let x3 = mix(corners[6], corners[7], u.x);
    return mix(mix(x0, x1, u.y), mix(x2, x3, u.y), u.z) * 0.9 + 0.5;
}

// Three octaves, spread back out to cover about 0..1.
fn livery_field(p: vec3<f32>) -> f32 {
    let n = livery_noise(p) * 0.57 + livery_noise(p * 2.07 + vec3<f32>(17.3, 3.1, 9.7)) * 0.29
        + livery_noise(p * 4.13 + vec3<f32>(5.9, 23.1, 1.3)) * 0.14;
    return (n - 0.5) * 2.4 + 0.5;
}

// One of its colours, by `h` (0..1) in the pattern's shares: the first the most.
fn livery_pick(l: Livery, h: f32) -> vec3<f32> {
    let count = l.pattern.w;
    if (count < 1.5) {
        return l.colours[0].rgb;
    }
    if (count < 2.5) {
        return select(l.colours[1].rgb, l.colours[0].rgb, h < 0.58);
    }
    return select(select(l.colours[2].rgb, l.colours[1].rgb, h < 0.78), l.colours[0].rgb, h < 0.45);
}

// Patches of the second and third colours over the first, their edges warped into fingers.
// `soft`: half the width of their edges, in the field's terms.
fn livery_blotch(l: Livery, p: vec3<f32>, soft: f32) -> vec3<f32> {
    let warp = vec3<f32>(livery_noise(p * 0.7 + vec3<f32>(3.7, 1.1, 8.3)), livery_noise(p * 0.7 + vec3<f32>(9.1, 4.7, 2.9)),
        livery_noise(p * 0.7 + vec3<f32>(6.3, 7.9, 5.1))) - 0.5;
    let q = p * 1.6 + warp * 1.4;
    let a = smoothstep(0.5 - soft, 0.5 + soft, livery_field(q));
    let b = smoothstep(0.66 - soft, 0.66 + soft, livery_field(q * 1.3 + vec3<f32>(41.0, 7.0, 19.0)));
    var colour = mix(l.colours[0].rgb, l.colours[1].rgb, a * l.colours[1].w);
    return mix(colour, l.colours[2].rgb, b * l.colours[2].w);
}

// The same patches in square pixels; pixels smaller than a few of the screen's blend into the
// smooth patches (no shimmer).
fn livery_digital(l: Livery, p: vec3<f32>, aa: f32) -> vec3<f32> {
    let size = 0.12;
    let hard = livery_blotch(l, (floor(p / size) + 0.5) * size, 1e-4);
    let soft = livery_blotch(l, p, 0.02 + aa * 2.0);
    return mix(hard, soft, smoothstep(0.25, 0.5, aa / size));
}

// Shards: the cells round scattered points, slanted and stretched; each a colour of its own.
fn livery_splinter(l: Livery, p: vec3<f32>, soft: f32) -> vec3<f32> {
    let q = vec3<f32>(p.x * 0.8 + p.z * 0.45, p.y * 1.7 + p.x * 0.3, p.z * 0.8 - p.x * 0.45) * 1.5;
    let cell = vec3<i32>(floor(q));
    var best = 1e9;
    var second = 1e9;
    var id = cell;
    var next = cell;
    for (var k = 0; k < 27; k++) {
        let c = cell + vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        let d = q - (vec3<f32>(c) + 0.15 + livery_point(c) * 0.7);
        let dist = dot(d, d);
        if (dist < best) {
            second = best;
            next = id;
            best = dist;
            id = c;
        } else if (dist < second) {
            second = dist;
            next = c;
        }
    }
    // (How far from the edge with the next cell: half the difference of their distances.)
    let edge = (sqrt(second) - sqrt(best)) * 0.5;
    let near = livery_pick(l, livery_hash(id));
    let far = livery_pick(l, livery_hash(next));
    return mix(mix(far, near, 0.5), near, smoothstep(0.0, soft * 0.6 + 1e-4, edge));
}

// Bands across the length, bent and tapering; thinner bands of the third colour between them.
fn livery_stripes(l: Livery, p: vec3<f32>, soft: f32) -> vec3<f32> {
    let bend = (livery_field(p * vec3<f32>(0.45, 0.9, 0.45) + vec3<f32>(2.3, 5.1, 7.7)) - 0.5) * 1.3;
    let d = p.z * 1.3 + p.x * 0.25 + bend;
    let along = vec3<f32>(p.x * 0.8, p.y * 0.8, d * 2.0);
    let width = 0.34 + 0.5 * (livery_noise(along + vec3<f32>(11.0, 3.0, 5.0)) - 0.5);
    let s = abs(fract(d) - 0.5) * 2.0;
    let band = 1.0 - smoothstep(width - soft, width + soft, s);
    var colour = mix(l.colours[0].rgb, l.colours[1].rgb, band * l.colours[1].w);
    let thin = 1.0 - smoothstep(width * 0.35 - soft, width * 0.35 + soft, abs(fract(d + 0.5) - 0.5) * 2.0);
    return mix(colour, l.colours[2].rgb, thin * l.colours[2].w);
}

// The plane the face lies most nearly in (`normal`, in the piece): `p` on it (xy; up is up on a
// side, ahead on a top), and which (z: 0 facing ahead or behind, 1 to a side, 2 up or down).
fn livery_plane(p: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let a = abs(normal);
    if (a.x >= a.y && a.x >= a.z) {
        return vec3<f32>(p.zy, 1.0);
    } else if (a.y >= a.z) {
        return vec3<f32>(p.xz, 2.0);
    }
    return vec3<f32>(p.xy, 0.0);
}

// Hexagonal plates laid on the plane the face lies most nearly in (`normal`, in the piece),
// each a colour, with dark gaps between them.
fn livery_hex(l: Livery, p: vec3<f32>, normal: vec3<f32>, aa: f32) -> vec3<f32> {
    let on = livery_plane(p, normal);
    let plane = i32(on.z);
    var uv = on.xy * 2.4;
    let r = vec2<f32>(1.0, 1.7320508);
    let h = r * 0.5;
    let ga = uv - r * floor(uv / r) - h;
    let gb = uv - h - r * floor((uv - h) / r) - h;
    let g = select(gb, ga, dot(ga, ga) < dot(gb, gb));
    let centre = uv - g;
    let q = abs(g);
    let edge = 0.5 - max(dot(q, vec2<f32>(0.5, 0.8660254)), q.x);
    let id = vec3<i32>(vec2<i32>(round(centre * 2.0)), plane);
    let gap = 1.0 - smoothstep(0.03, 0.03 + aa * 2.4 + 0.02, edge);
    return livery_pick(l, livery_hash(id)) * (1.0 - 0.55 * gap * (1.0 - smoothstep(0.08, 0.2, aa)));
}

// Waves of the first two colours with snowflakes of the third (the Christmas pattern of before).
fn livery_festive(l: Livery, p: vec3<f32>, aa: f32) -> vec3<f32> {
    let q = p * 1.28;
    let phase = sin(q.x * 1.7 + q.y * 0.6 + q.z * 1.1);
    let edge = mix(0.5, 0.05, l.pattern.z) + aa * 3.0;
    let colour = mix(l.colours[0].rgb, l.colours[1].rgb, smoothstep(-edge, edge, phase) * l.colours[1].w);
    let uv = fract(vec2<f32>(q.x + q.z * 0.35, q.y + q.z * 0.65)) - 0.5;
    let line = min(min(abs(uv.x), abs(uv.y)), min(abs(uv.x - uv.y), abs(uv.x + uv.y)) * 0.7071);
    let footprint = aa * 1.28;
    let width = max(footprint * 0.6, 0.008);
    let snow = (1.0 - smoothstep(0.018 - width, 0.018 + width, line))
        * (1.0 - smoothstep(0.21 - width, 0.21 + width, length(uv)))
        * (1.0 - smoothstep(0.12, 0.35, footprint));
    return mix(colour, l.colours[2].rgb, snow * 0.88 * l.colours[2].w);
}

// A colour of the livery's, round by its count.
fn livery_colour(l: Livery, i: f32) -> vec3<f32> {
    let count = max(l.pattern.w, 1.0);
    return l.colours[u32(i - count * floor(i / count))].rgb;
}

// Equal bands of the colours in turn across the length, straight, repeating: a tricolour's.
fn livery_tricolour(l: Livery, p: vec3<f32>, soft: f32) -> vec3<f32> {
    let count = max(l.pattern.w, 1.0);
    let t = fract(p.z * 0.7) * count;
    let band = floor(t);
    let u = t - band;
    let w = soft * count * 0.7 + 1e-4;
    let colour = mix(livery_colour(l, band), livery_colour(l, band + 1.0), smoothstep(1.0 - w, 1.0 + w, u));
    return mix(livery_colour(l, band - 1.0), colour, smoothstep(-w, w, u));
}

// Crosses upright and diagonal on the plane the face lies most nearly in, broad ones of the
// second colour and narrow ones of the third over the first: a union flag's, repeating.
fn livery_union(l: Livery, p: vec3<f32>, normal: vec3<f32>, soft: f32) -> vec3<f32> {
    let g = fract(livery_plane(p, normal).xy * 0.9) - 0.5;
    let upright = min(abs(g.x), abs(g.y));
    let diagonal = min(abs(g.x - g.y), abs(g.x + g.y)) * 0.70710678;
    let broad = 1.0 - smoothstep(-soft, soft, min(upright - 0.1, diagonal - 0.065));
    let narrow = 1.0 - smoothstep(-soft, soft, min(upright - 0.045, diagonal - 0.025));
    let colour = mix(l.colours[0].rgb, l.colours[1].rgb, broad * l.colours[1].w);
    return mix(colour, l.colours[2].rgb, narrow * l.colours[2].w);
}

// A five-pointed star's mirrors (cos and sin of a fifth of a turn's halves). (Constants of the
// module: one built from another inside a function, pipelines' constant processing emits twice.)
const LIVERY_STAR_K1: vec2<f32> = vec2<f32>(0.809016994375, -0.587785252292);
const LIVERY_STAR_K2: vec2<f32> = vec2<f32>(-0.809016994375, -0.587785252292);
const LIVERY_STAR_SIDE: vec2<f32> = vec2<f32>(0.587785252292, 0.809016994375);

// How far `q` lies outside a five-pointed star `r` across its points, its inner corners at
// `inner` of that (negative inside), its top point up.
fn livery_star(q: vec2<f32>, r: f32, inner: f32) -> f32 {
    var p = vec2<f32>(abs(q.x), q.y);
    p = p - 2.0 * max(dot(LIVERY_STAR_K1, p), 0.0) * LIVERY_STAR_K1;
    p = p - 2.0 * max(dot(LIVERY_STAR_K2, p), 0.0) * LIVERY_STAR_K2;
    p = vec2<f32>(abs(p.x), p.y - r);
    let ba = inner * LIVERY_STAR_SIDE - vec2<f32>(0.0, 1.0);
    let h = clamp(dot(p, ba) / dot(ba, ba), 0.0, r);
    return length(p - ba * h) * sign(p.y * ba.x - p.x * ba.y);
}

// Rows of five-pointed stars of the second colour on the first, each row half a star along from
// the last; with a third colour, every other row a band of it on the second instead (a
// star-spangled banner's stars and stripes).
fn livery_stars(l: Livery, p: vec3<f32>, normal: vec3<f32>, soft: f32) -> vec3<f32> {
    let at = livery_plane(p, normal).xy * 1.8;
    let row = floor(at.y);
    let g = fract(vec2<f32>(at.x + row * 0.5, at.y)) - 0.5;
    if (l.pattern.w > 2.5 && row - 2.0 * floor(row * 0.5) > 0.5) {
        let band = 1.0 - smoothstep(0.2 - soft, 0.2 + soft, abs(g.y));
        return mix(l.colours[1].rgb, l.colours[2].rgb, band);
    }
    let star = 1.0 - smoothstep(-soft, soft, livery_star(g, 0.3, 0.42));
    return mix(l.colours[0].rgb, l.colours[1].rgb, star * l.colours[1].w);
}

// The livery's colour at `p` (the pattern's space), `normal` the face's in its piece, `aa` the
// size of a pixel there. Patterns finer than the eye can follow blend into their average.
fn livery_pattern(l: Livery, p: vec3<f32>, normal: vec3<f32>, aa: f32) -> vec3<f32> {
    let soft = mix(0.16, 0.006, l.pattern.z) + aa * 1.5;
    var colour = l.colours[0].rgb;
    switch (u32(l.pattern.x + 0.5)) {
        case 1u: { colour = livery_blotch(l, p, soft); }
        case 2u: { colour = livery_splinter(l, p, soft); }
        case 3u: { colour = livery_digital(l, p, aa); }
        case 4u: { colour = livery_stripes(l, p, soft); }
        case 5u: { colour = livery_hex(l, p, normal, aa); }
        case 6u: { colour = livery_festive(l, p, aa); }
        case 7u: { colour = livery_tricolour(l, p, soft); }
        case 8u: { colour = livery_union(l, p, normal, soft); }
        case 9u: { colour = livery_stars(l, p, normal, soft); }
        default: {}
    }
    return mix(colour, l.average.rgb, smoothstep(0.2, 0.6, aa));
}

// What the paint's own light and dark makes of the livery's colours: a part's overall
// lightness, softened (a dark part stays a little darker), and the detail painted into its
// texture (`albedo` against its average `mean`): panel lines, seams and markings.
fn livery_shade(l: Livery, albedo: vec3<f32>, mean: vec3<f32>) -> f32 {
    let average = max(livery_luma(mean), 0.004);
    let grain = clamp(livery_luma(albedo) / average, 0.0, 2.5);
    return pow(clamp(average / 0.16, 0.2, 3.0), 0.3) * mix(1.0, grain, l.finish.y);
}

// Paint bleached by the sun as much as the livery's fade: its colour washed out towards a dusty
// grey of its own lightness, a little lighter, in patches faded more than others.
fn livery_faded(l: Livery, c: vec3<f32>, local: vec3<f32>) -> vec3<f32> {
    let fade = l.weather.w * (0.75 + 0.5 * livery_noise(local * 0.11 + vec3<f32>(3.7, 8.1, 1.9)));
    if (fade <= 0.0) {
        return c;
    }
    let luma = livery_luma(c);
    let dust = vec3<f32>(luma) * vec3<f32>(1.06, 1.0, 0.88);
    return mix(c, dust, saturate(fade * 0.85)) * (1.0 + 0.3 * fade) + vec3<f32>(0.012, 0.011, 0.008) * fade;
}

// The livery over paint `albedo` (linear) whose texture averages `mean`, at `local` in its piece
// (model units) where a pixel spans `footprint`; `trim` (0..1) how much of it is trim. Faded as
// it says, paint and livery alike.
fn livery_paint(l: Livery, albedo: vec3<f32>, mean: vec3<f32>, local: vec3<f32>, normal: vec3<f32>, footprint: f32, trim: f32) -> vec3<f32> {
    let frequency = l.pattern.y / LIVERY_PERIOD;
    let shade = livery_shade(l, albedo, mean);
    let colour = mix(livery_pattern(l, local * frequency + l.offset.xyz, normal, footprint * frequency), l.trim.rgb, trim * l.trim.w);
    return livery_faded(l, mix(albedo, colour * shade, l.finish.x), local);
}

// Its finish over the paint's (roughness, metallic) as much as it covers; faded paint chalky.
fn livery_material(l: Livery, material: vec2<f32>) -> vec2<f32> {
    let m = mix(material, l.finish.zw, l.finish.x);
    return vec2<f32>(mix(m.x, 0.92, saturate(l.weather.w * 0.8)), m.y);
}

// Whether a face this livery paints shows TA's own paint (as TA drew it) under it, not Reborn's.
fn livery_original(l: Livery) -> bool {
    return l.weather.z > 0.5;
}

// How much of a face is trim (0..1): a band along its convex edges (model.rs `worn_edges`:
// `dist` to them, `weight` their sharpness, `aa` how fast the distance changes across a pixel),
// fading where it would be thinner than a pixel.
fn livery_trim(dist: vec4<f32>, weight: vec4<f32>, aa: vec4<f32>) -> f32 {
    let width = vec4<f32>(0.42);
    let visible = saturate(width / max(aa * 1.5, vec4<f32>(1e-4)) - 0.5);
    let band = (vec4<f32>(1.0) - smoothstep(width - aa - 1e-4, width + aa + 1e-4, dist)) * visible * step(vec4<f32>(0.3), weight);
    return max(max(band.x, band.y), max(band.z, band.w));
}

// Paint worn through to the metal (0..1): past Reborn's own wear (0.5), chips flaking off along
// stretches of the convex edges, ragged and wider the more worn, and a few scratches across the
// faces.
fn livery_chips(l: Livery, local: vec3<f32>, dist: vec4<f32>, weight: vec4<f32>, aa: vec4<f32>) -> f32 {
    let chips = saturate(l.weather.x * 2.0 - 1.0);
    if (chips <= 0.0) {
        return 0.0;
    }
    let reach = vec4<f32>(0.5 + 1.6 * chips);
    let visible = saturate(reach / max(aa * 1.5, vec4<f32>(1e-4)) - 0.5);
    let near = (vec4<f32>(1.0) - smoothstep(vec4<f32>(0.0), reach + aa, dist)) * visible * saturate(weight * 2.0);
    let edge = max(max(near.x, near.y), max(near.z, near.w));
    let ragged = livery_noise(local * 0.6) * 0.6 + livery_noise(local * 2.1 + vec3<f32>(4.1, 9.3, 2.7)) * 0.4;
    // (Which stretches of the edges flake; slightly worn, only their most exposed corners, worn
    // out, a ragged band.)
    let stretches = smoothstep(0.42, 0.62, livery_noise(local * 0.3 + vec3<f32>(9.0, 2.0, 5.0)));
    let flaked = smoothstep(0.46, 0.54, edge * mix(0.35, 1.0, stretches) - (1.0 - chips) * 0.85 - (ragged - 0.5) * 0.6);
    let scratch = smoothstep(0.86, 0.9, livery_noise(local * vec3<f32>(0.35, 3.1, 0.35) + vec3<f32>(7.7, 1.3, 3.9))) * chips * 0.6;
    return saturate(flaked + scratch);
}

// Dirt (0..1): in the creases (`crease`, 0..1), on what faces down (`down`, 0..1), in patches
// and in streaks run down the faces, as much as the livery's grime.
fn livery_grime(l: Livery, local: vec3<f32>, crease: f32, down: f32) -> f32 {
    let grime = l.weather.y;
    if (grime <= 0.0) {
        return 0.0;
    }
    let patches = livery_noise(local * 0.09 + vec3<f32>(5.3, 1.7, 2.9)) * 0.6 + livery_noise(local * 0.31 + vec3<f32>(8.1, 2.2, 6.4)) * 0.4;
    let streaks = livery_noise(vec3<f32>(local.x * 0.7, local.y * 0.06, local.z * 0.7) + vec3<f32>(13.0, 4.0, 9.0));
    let dirt = crease * 0.9 + smoothstep(0.5, 0.75, patches) * 0.55 + smoothstep(0.58, 0.75, streaks) * 0.35 + down * 0.35;
    return saturate(dirt * grime * 1.3);
}
